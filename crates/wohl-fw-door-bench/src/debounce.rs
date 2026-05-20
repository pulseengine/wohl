//! Edge-debouncer for the reed-switch input.
//!
//! The reed switch bounces for a few milliseconds when the magnet
//! enters or leaves the activation field. We sample the GPIO at a
//! fixed cadence (typically every 1 ms via the SysTick handler) and
//! only commit a state change after the input has held the new value
//! for `STABLE_TICKS` consecutive samples (default 50, i.e. 50 ms).
//!
//! The debouncer is **purely combinatorial** — caller provides the
//! current GPIO level and a tick reference; we return `Some(edge)` on
//! a confirmed transition or `None` otherwise. No timers, no async,
//! no globals: the caller drives time.
//!
//! Spec:
//! - On `Debouncer::new(initial)` the committed state is `initial`
//!   and the counter is `0`.
//! - On every `update(level)`:
//!   - If `level` matches the committed state → counter is reset to
//!     `0`, returns `None`.
//!   - Else counter is incremented. If counter reaches `STABLE_TICKS`,
//!     commit `level`, reset counter to `0`, return `Some(edge)`.
//!   - Else returns `None`.
//!
//! This deliberately ignores both the **first** N samples of a glitch
//! and any glitch shorter than `STABLE_TICKS` consecutive ticks — both
//! of which are the standard mechanical-contact-debouncing semantics.

/// Default debounce window: 50 samples at 1 kHz → 50 ms. Chosen to
/// match `SYSREQ-WOHL-002`-style mechanical-bounce assumptions and the
/// 100 ms `Deadline` on the `DoorFirmware` AADL thread.
pub const DEFAULT_STABLE_TICKS: u16 = 50;

/// Logical level of the reed input. We name them by *door state* —
/// the wire-level voltage is the inverse because the line is pulled
/// high and pulled low by a closed reed switch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DoorLevel {
    /// Reed shorted, line low → door closed.
    Closed,
    /// Reed open, line high → door open.
    Open,
}

impl DoorLevel {
    /// Map a raw GPIO read (true = high) to a door level.
    pub fn from_high(is_high: bool) -> Self {
        if is_high { Self::Open } else { Self::Closed }
    }

    /// `1` for open, `0` for closed — matches `SENSOR_CONTACT` value.
    pub fn as_value(self) -> i32 {
        match self {
            Self::Closed => 0,
            Self::Open => 1,
        }
    }
}

/// A confirmed transition reported by [`Debouncer::update`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    /// Door went from closed to open.
    Opened,
    /// Door went from open to closed.
    Closed,
}

/// Debouncer state. Generic only over the stable-tick count so the
/// firmware and tests can vary the window.
#[derive(Clone, Copy, Debug)]
pub struct Debouncer<const STABLE_TICKS: u16 = DEFAULT_STABLE_TICKS> {
    committed: DoorLevel,
    /// Number of consecutive samples disagreeing with `committed`.
    streak: u16,
}

impl<const N: u16> Debouncer<N> {
    /// Construct a debouncer with a known initial level (typically
    /// the level read once at boot, after the GPIO has settled).
    pub const fn new(initial: DoorLevel) -> Self {
        Self {
            committed: initial,
            streak: 0,
        }
    }

    /// The currently-committed door level (no glitches reflected).
    pub const fn level(&self) -> DoorLevel {
        self.committed
    }

    /// Feed a new sample. Returns `Some(edge)` exactly once per
    /// confirmed transition, `None` otherwise.
    pub fn update(&mut self, sample: DoorLevel) -> Option<Edge> {
        if sample == self.committed {
            self.streak = 0;
            return None;
        }
        // Sample disagrees with the committed value.
        self.streak = self.streak.saturating_add(1);
        if self.streak >= N {
            let edge = match (self.committed, sample) {
                (DoorLevel::Closed, DoorLevel::Open) => Edge::Opened,
                (DoorLevel::Open, DoorLevel::Closed) => Edge::Closed,
                // Same-state case ruled out above by the equality check.
                _ => unreachable!(),
            };
            self.committed = sample;
            self.streak = 0;
            Some(edge)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_input_never_fires() {
        let mut d: Debouncer = Debouncer::new(DoorLevel::Closed);
        for _ in 0..1000 {
            assert!(d.update(DoorLevel::Closed).is_none());
        }
        assert_eq!(d.level(), DoorLevel::Closed);
    }

    #[test]
    fn short_glitch_is_ignored() {
        let mut d: Debouncer = Debouncer::new(DoorLevel::Closed);
        // 10 ms of "open" then back to closed → never crosses 50 ticks.
        for _ in 0..10 {
            assert!(d.update(DoorLevel::Open).is_none());
        }
        for _ in 0..5 {
            assert!(d.update(DoorLevel::Closed).is_none());
        }
        assert_eq!(d.level(), DoorLevel::Closed);
    }

    #[test]
    fn fifty_ms_hold_fires_once() {
        let mut d: Debouncer = Debouncer::new(DoorLevel::Closed);
        let mut edges = 0;
        for i in 0..200 {
            // First 49 "open" samples must NOT fire; the 50th does.
            let r = d.update(DoorLevel::Open);
            if r.is_some() {
                edges += 1;
                assert_eq!(r, Some(Edge::Opened));
                assert_eq!(i, 49, "edge must fire on the 50th sample");
            }
        }
        assert_eq!(edges, 1);
        assert_eq!(d.level(), DoorLevel::Open);
    }

    #[test]
    fn opens_then_closes() {
        let mut d: Debouncer<3> = Debouncer::new(DoorLevel::Closed);
        for _ in 0..3 {
            d.update(DoorLevel::Open);
        }
        assert_eq!(d.level(), DoorLevel::Open);
        let mut last: Option<Edge> = None;
        for _ in 0..3 {
            if let Some(e) = d.update(DoorLevel::Closed) {
                last = Some(e);
            }
        }
        assert_eq!(last, Some(Edge::Closed));
        assert_eq!(d.level(), DoorLevel::Closed);
    }

    #[test]
    fn level_from_high() {
        assert_eq!(DoorLevel::from_high(true), DoorLevel::Open);
        assert_eq!(DoorLevel::from_high(false), DoorLevel::Closed);
        assert_eq!(DoorLevel::Open.as_value(), 1);
        assert_eq!(DoorLevel::Closed.as_value(), 0);
    }

    proptest::proptest! {
        /// Pumping noise into the debouncer never produces an edge if
        /// the disagreeing run never reaches the stable threshold.
        #[test]
        fn no_spurious_edge_below_threshold(
            run_len in 0u16..50,
        ) {
            let mut d: Debouncer = Debouncer::new(DoorLevel::Closed);
            for _ in 0..run_len {
                proptest::prop_assert!(d.update(DoorLevel::Open).is_none());
            }
            // Return to closed before stable threshold reached.
            proptest::prop_assert_eq!(d.update(DoorLevel::Closed), None);
            proptest::prop_assert_eq!(d.level(), DoorLevel::Closed);
        }
    }
}
