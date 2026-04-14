//! Wohl Water Leak Detector — plain Rust (generated from Verus source).
//! Source of truth: ../src/core.rs. Do not edit manually.

pub const MAX_ZONES: usize = 32;

#[derive(Clone, Copy)]
pub struct ZoneState { pub zone_id: u32, pub wet: bool, pub detected_at: u64, pub active: bool }

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LeakAction { NewLeak, AlreadyWet, Cleared, AlreadyDry, Unknown }

pub struct LeakDetector { zones: [ZoneState; MAX_ZONES], zone_count: u32 }

impl ZoneState {
    pub const fn empty() -> Self { ZoneState { zone_id: 0, wet: false, detected_at: 0, active: false } }
}

impl LeakDetector {
    pub fn new() -> Self { LeakDetector { zones: [ZoneState::empty(); MAX_ZONES], zone_count: 0 } }

    pub fn register_zone(&mut self, zone_id: u32) -> bool {
        if self.zone_count as usize >= MAX_ZONES { return false; }
        let idx = self.zone_count as usize;
        self.zones[idx] = ZoneState { zone_id, wet: false, detected_at: 0, active: true };
        self.zone_count = self.zone_count + 1;
        true
    }

    pub fn process_event(&mut self, zone_id: u32, wet: bool, timestamp_sec: u64) -> LeakAction {
        let count = self.zone_count;
        let mut i: u32 = 0;
        while i < count {
            let idx = i as usize;
            if self.zones[idx].active && self.zones[idx].zone_id == zone_id {
                let was_wet = self.zones[idx].wet;
                if wet {
                    if was_wet { return LeakAction::AlreadyWet; }
                    else { self.zones[idx].wet = true; self.zones[idx].detected_at = timestamp_sec; return LeakAction::NewLeak; }
                } else {
                    if was_wet { self.zones[idx].wet = false; return LeakAction::Cleared; }
                    else { return LeakAction::AlreadyDry; }
                }
            }
            i = i + 1;
        }
        LeakAction::Unknown
    }

    pub fn any_wet(&self) -> bool {
        let count = self.zone_count;
        let mut i: u32 = 0;
        while i < count {
            if self.zones[i as usize].active && self.zones[i as usize].wet { return true; }
            i = i + 1;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn test_new_leak() { let mut d = LeakDetector::new(); d.register_zone(1); assert_eq!(d.process_event(1, true, 1000), LeakAction::NewLeak); }
    #[test] fn test_already_wet() { let mut d = LeakDetector::new(); d.register_zone(1); d.process_event(1, true, 1000); assert_eq!(d.process_event(1, true, 1001), LeakAction::AlreadyWet); }
    #[test] fn test_cleared() { let mut d = LeakDetector::new(); d.register_zone(1); d.process_event(1, true, 1000); assert_eq!(d.process_event(1, false, 2000), LeakAction::Cleared); }
    #[test] fn test_no_auto_clear() { let mut d = LeakDetector::new(); d.register_zone(1); d.process_event(1, true, 1000); assert!(d.any_wet()); }
    #[test] fn test_unknown() { let mut d = LeakDetector::new(); assert_eq!(d.process_event(99, true, 1000), LeakAction::Unknown); }
    #[test] fn test_multi_zone() { let mut d = LeakDetector::new(); d.register_zone(1); d.register_zone(2); assert_eq!(d.process_event(2, true, 100), LeakAction::NewLeak); assert_eq!(d.process_event(2, false, 200), LeakAction::Cleared); }
    #[test] fn test_already_dry() { let mut d = LeakDetector::new(); d.register_zone(1); assert_eq!(d.process_event(1, false, 1000), LeakAction::AlreadyDry); }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn wet_always_detects_for_registered_zone(
            zone_id in 0u32..50,
            time in 0u64..1_000_000,
        ) {
            let mut det = LeakDetector::new();
            det.register_zone(zone_id);
            let action = det.process_event(zone_id, true, time);
            prop_assert!(action == LeakAction::NewLeak || action == LeakAction::AlreadyWet);
        }

        #[test]
        fn unregistered_zone_always_unknown(
            zone_id in 100u32..200,
            wet in proptest::bool::ANY,
            time in 0u64..1_000_000,
        ) {
            let mut det = LeakDetector::new();
            // Don't register zone_id
            let action = det.process_event(zone_id, wet, time);
            prop_assert_eq!(action, LeakAction::Unknown);
        }

        #[test]
        fn wet_then_dry_clears(
            zone_id in 0u32..30,
            t1 in 0u64..500_000,
            t2 in 500_001u64..1_000_000,
        ) {
            let mut det = LeakDetector::new();
            det.register_zone(zone_id);
            det.process_event(zone_id, true, t1);
            let action = det.process_event(zone_id, false, t2);
            prop_assert_eq!(action, LeakAction::Cleared);
            prop_assert!(!det.any_wet());
        }
    }
}

// ── Kani bounded model checking harnesses ────────────────────

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// LEAK-P05: zone_count never exceeds MAX_ZONES
    #[kani::proof]
    fn verify_zone_count_bounded() {
        let mut det = LeakDetector::new();
        let zone_id: u32 = kani::any();
        // Register up to MAX_ZONES + 1 times
        for _ in 0..MAX_ZONES + 1 {
            det.register_zone(kani::any());
        }
        assert!(det.zone_count <= MAX_ZONES as u32);
    }

    /// LEAK-P01: wet event always produces NewLeak or AlreadyWet (never Unknown for registered zone)
    #[kani::proof]
    fn verify_wet_always_detects() {
        let mut det = LeakDetector::new();
        let zone_id: u32 = kani::any();
        kani::assume(zone_id < 100); // bound search space
        det.register_zone(zone_id);
        let time: u64 = kani::any();
        let action = det.process_event(zone_id, true, time);
        assert!(action == LeakAction::NewLeak || action == LeakAction::AlreadyWet);
    }

    /// LEAK-P04: dry event only clears if zone was wet
    #[kani::proof]
    fn verify_dry_clear_requires_wet() {
        let mut det = LeakDetector::new();
        det.register_zone(1);
        // No wet event sent
        let action = det.process_event(1, false, 1000);
        assert!(action == LeakAction::AlreadyDry);
    }

    /// No panics for any combination of inputs
    #[kani::proof]
    fn verify_no_panic() {
        let mut det = LeakDetector::new();
        let zone_id: u32 = kani::any();
        let wet: bool = kani::any();
        let time: u64 = kani::any();
        kani::assume(zone_id < 100);
        det.register_zone(zone_id);
        let _ = det.process_event(zone_id, wet, time);
        let _ = det.any_wet();
    }
}
