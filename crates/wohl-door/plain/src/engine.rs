//! Wohl Door Watch — plain Rust (generated from Verus source).
//! Source of truth: ../src/core.rs. Do not edit manually.

pub const MAX_CONTACTS: usize = 32;
pub const MAX_ALERTS_PER_CHECK: usize = 4;

#[derive(Clone, Copy)]
pub struct ContactConfig {
    pub contact_id: u32,
    pub zone_id: u32,
    pub max_open_sec: u32,
    pub night_start_hour: u8,
    pub night_end_hour: u8,
    pub enabled: bool,
}

#[derive(Clone, Copy)]
pub struct ContactState {
    pub contact_id: u32,
    pub open: bool,
    pub opened_at: u64,
    pub active: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DoorAlertType {
    OpenTooLong,
    OpenedAtNight,
}

#[derive(Clone, Copy)]
pub struct DoorAlert {
    pub contact_id: u32,
    pub zone_id: u32,
    pub alert_type: DoorAlertType,
    pub open_duration_sec: u64,
    pub time: u64,
}

#[derive(Clone, Copy)]
pub struct DoorResult {
    pub alerts: [DoorAlert; MAX_ALERTS_PER_CHECK],
    pub alert_count: u32,
}

pub struct DoorWatch {
    configs: [ContactConfig; MAX_CONTACTS],
    states: [ContactState; MAX_CONTACTS],
    contact_count: u32,
}

impl ContactConfig {
    pub const fn empty() -> Self {
        ContactConfig {
            contact_id: 0,
            zone_id: 0,
            max_open_sec: 0,
            night_start_hour: 0,
            night_end_hour: 0,
            enabled: false,
        }
    }
}

impl ContactState {
    pub const fn empty() -> Self {
        ContactState {
            contact_id: 0,
            open: false,
            opened_at: 0,
            active: false,
        }
    }
}

impl DoorAlert {
    pub const fn empty() -> Self {
        DoorAlert {
            contact_id: 0,
            zone_id: 0,
            alert_type: DoorAlertType::OpenTooLong,
            open_duration_sec: 0,
            time: 0,
        }
    }
}

impl DoorResult {
    pub const fn empty() -> Self {
        DoorResult {
            alerts: [DoorAlert::empty(); MAX_ALERTS_PER_CHECK],
            alert_count: 0,
        }
    }
}

impl Default for DoorWatch {
    fn default() -> Self {
        Self::new()
    }
}

impl DoorWatch {
    pub fn new() -> Self {
        DoorWatch {
            configs: [ContactConfig::empty(); MAX_CONTACTS],
            states: [ContactState::empty(); MAX_CONTACTS],
            contact_count: 0,
        }
    }

    pub fn register_contact(&mut self, config: ContactConfig) -> bool {
        if self.contact_count as usize >= MAX_CONTACTS {
            return false;
        }
        let idx = self.contact_count as usize;
        self.configs[idx] = config;
        self.states[idx] = ContactState {
            contact_id: config.contact_id,
            open: false,
            opened_at: 0,
            active: true,
        };
        self.contact_count += 1;
        true
    }

    fn is_night_hour(hour: u8, night_start: u8, night_end: u8) -> bool {
        if night_start <= night_end {
            hour >= night_start && hour < night_end
        } else {
            hour >= night_start || hour < night_end
        }
    }

    pub fn process_event(&mut self, contact_id: u32, open: bool, time: u64) -> DoorResult {
        let mut res = DoorResult::empty();
        let count = self.contact_count;
        let mut i: u32 = 0;
        while i < count {
            let idx = i as usize;
            if self.states[idx].active
                && self.configs[idx].enabled
                && self.configs[idx].contact_id == contact_id
            {
                if open {
                    self.states[idx].open = true;
                    self.states[idx].opened_at = time;

                    let hour_of_day: u8 = ((time % 86400) / 3600) as u8;
                    if Self::is_night_hour(
                        hour_of_day,
                        self.configs[idx].night_start_hour,
                        self.configs[idx].night_end_hour,
                    ) && (res.alert_count as usize) < MAX_ALERTS_PER_CHECK
                    {
                        res.alerts[res.alert_count as usize] = DoorAlert {
                            contact_id,
                            zone_id: self.configs[idx].zone_id,
                            alert_type: DoorAlertType::OpenedAtNight,
                            open_duration_sec: 0,
                            time,
                        };
                        res.alert_count += 1;
                    }
                } else {
                    self.states[idx].open = false;
                }

                return res;
            }
            i += 1;
        }

        res
    }

    pub fn check_timeouts(&mut self, current_time: u64) -> DoorResult {
        let mut res = DoorResult::empty();
        let count = self.contact_count;
        let mut i: u32 = 0;
        while i < count {
            let idx = i as usize;
            if self.states[idx].active && self.configs[idx].enabled && self.states[idx].open {
                let opened_at = self.states[idx].opened_at;
                if current_time >= opened_at {
                    let duration = current_time - opened_at;
                    if duration > self.configs[idx].max_open_sec as u64
                        && (res.alert_count as usize) < MAX_ALERTS_PER_CHECK
                    {
                        res.alerts[res.alert_count as usize] = DoorAlert {
                            contact_id: self.configs[idx].contact_id,
                            zone_id: self.configs[idx].zone_id,
                            alert_type: DoorAlertType::OpenTooLong,
                            open_duration_sec: duration,
                            time: current_time,
                        };
                        res.alert_count += 1;
                    }
                }
            }
            i += 1;
        }

        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config(contact_id: u32, zone_id: u32) -> ContactConfig {
        ContactConfig {
            contact_id,
            zone_id,
            max_open_sec: 300,
            night_start_hour: 22,
            night_end_hour: 6,
            enabled: true,
        }
    }

    #[test]
    fn test_open_close() {
        let mut w = DoorWatch::new();
        w.register_contact(make_config(1, 10));
        let r = w.process_event(1, true, 43200);
        assert_eq!(r.alert_count, 0);
        let r = w.process_event(1, false, 43260);
        assert_eq!(r.alert_count, 0);
    }
    #[test]
    fn test_open_too_long() {
        let mut w = DoorWatch::new();
        w.register_contact(make_config(1, 10));
        w.process_event(1, true, 43200);
        let r = w.check_timeouts(43200 + 400);
        assert_eq!(r.alert_count, 1);
        assert_eq!(r.alerts[0].alert_type, DoorAlertType::OpenTooLong);
    }
    #[test]
    fn test_opened_at_night() {
        let mut w = DoorWatch::new();
        w.register_contact(make_config(1, 10));
        let r = w.process_event(1, true, 82800);
        assert_eq!(r.alert_count, 1);
        assert_eq!(r.alerts[0].alert_type, DoorAlertType::OpenedAtNight);
    }
    #[test]
    fn test_normal_day() {
        let mut w = DoorWatch::new();
        w.register_contact(make_config(1, 10));
        let r = w.process_event(1, true, 50400);
        assert_eq!(r.alert_count, 0);
    }
    #[test]
    fn test_unknown_contact() {
        let mut w = DoorWatch::new();
        let r = w.process_event(99, true, 1000);
        assert_eq!(r.alert_count, 0);
    }
    #[test]
    fn test_multiple_contacts() {
        let mut w = DoorWatch::new();
        w.register_contact(make_config(1, 10));
        w.register_contact(make_config(2, 20));
        w.process_event(1, true, 43200);
        w.process_event(2, true, 43200);
        let r = w.check_timeouts(43200 + 400);
        assert_eq!(r.alert_count, 2);
    }
}

// ── Kani bounded model checking harnesses ────────────────────

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// DOOR-P03: alert_count never exceeds MAX_ALERTS_PER_CHECK
    #[kani::proof]
    fn verify_alert_count_bounded() {
        let mut w = DoorWatch::new();
        let config = ContactConfig {
            contact_id: 1,
            zone_id: 10,
            max_open_sec: kani::any(),
            night_start_hour: kani::any(),
            night_end_hour: kani::any(),
            enabled: true,
        };
        w.register_contact(config);
        let open: bool = kani::any();
        let time: u64 = kani::any();
        let r = w.process_event(1, open, time);
        assert!(r.alert_count as usize <= MAX_ALERTS_PER_CHECK);
        // Also verify check_timeouts is bounded
        let current: u64 = kani::any();
        let r2 = w.check_timeouts(current);
        assert!(r2.alert_count as usize <= MAX_ALERTS_PER_CHECK);
    }

    /// DOOR-P04: process_event correctly tracks open/close state
    #[kani::proof]
    fn verify_open_close_state() {
        let mut w = DoorWatch::new();
        let config = ContactConfig {
            contact_id: 1,
            zone_id: 10,
            max_open_sec: 300,
            night_start_hour: 22,
            night_end_hour: 6,
            enabled: true,
        };
        w.register_contact(config);
        // Open the door during daytime (no night alert)
        w.process_event(1, true, 43200); // noon
        // State should be open
        assert!(w.states[0].open);
        // Close the door
        w.process_event(1, false, 43260);
        // State should be closed
        assert!(!w.states[0].open);
    }

    /// No panics for any combination of symbolic inputs
    #[kani::proof]
    fn verify_no_panic() {
        let mut w = DoorWatch::new();
        let contact_id: u32 = kani::any();
        kani::assume(contact_id < 100);
        let config = ContactConfig {
            contact_id,
            zone_id: kani::any(),
            max_open_sec: kani::any(),
            night_start_hour: kani::any(),
            night_end_hour: kani::any(),
            enabled: kani::any(),
        };
        w.register_contact(config);
        let open: bool = kani::any();
        let time: u64 = kani::any();
        let _ = w.process_event(contact_id, open, time);
        let current: u64 = kani::any();
        let _ = w.check_timeouts(current);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    fn contact() -> ContactConfig {
        ContactConfig {
            contact_id: 1,
            zone_id: 1,
            max_open_sec: 300,
            night_start_hour: 22,
            night_end_hour: 6,
            enabled: true,
        }
    }

    proptest! {
        /// Any contact event yields no more alerts than the fixed buffer holds.
        #[test]
        fn alert_count_always_bounded(
            open in proptest::bool::ANY,
            time in 0u64..1_000_000,
        ) {
            let mut w = DoorWatch::new();
            w.register_contact(contact());
            let r = w.process_event(1, open, time);
            prop_assert!(r.alert_count as usize <= MAX_ALERTS_PER_CHECK);
        }

        /// A close event (contact returning to closed) never raises an alert.
        #[test]
        fn closing_never_alerts(
            time in 0u64..1_000_000,
        ) {
            let mut w = DoorWatch::new();
            w.register_contact(contact());
            let r = w.process_event(1, false, time);
            prop_assert_eq!(r.alert_count, 0);
        }
    }
}

/// Unit tests pinning behaviors a cargo-mutants pass reported untested.
#[cfg(test)]
mod mutation_tests {
    use super::*;

    fn cfg_same_day(contact_id: u32) -> ContactConfig {
        ContactConfig {
            contact_id,
            zone_id: 10,
            max_open_sec: 300,
            night_start_hour: 1,
            night_end_hour: 5,
            enabled: true,
        }
    }

    fn cfg_wrapped(contact_id: u32) -> ContactConfig {
        ContactConfig {
            contact_id,
            zone_id: 10,
            max_open_sec: 300,
            night_start_hour: 22,
            night_end_hour: 6,
            enabled: true,
        }
    }

    // is_night_hour, same-day window: >= start, && , < end (vs <, ||, ==, >)
    #[test]
    fn non_wrapping_night_window() {
        let mut w = DoorWatch::new();
        w.register_contact(cfg_same_day(1));
        let r = w.process_event(1, true, 3 * 3600);
        assert_eq!(r.alert_count, 1);
        assert_eq!(r.alerts[0].alert_type, DoorAlertType::OpenedAtNight);
        let r = w.process_event(1, true, 12 * 3600);
        assert_eq!(r.alert_count, 0);
        let r = w.process_event(1, true, 5 * 3600);
        assert_eq!(r.alert_count, 0);
    }

    // is_night_hour: end hour exclusive (< vs <=), asserted directly too
    #[test]
    fn night_window_end_hour_is_exclusive() {
        let mut w = DoorWatch::new();
        w.register_contact(cfg_same_day(1));
        let r = w.process_event(1, true, 3 * 3600);
        assert_eq!(r.alert_count, 1);
        let r = w.process_event(1, true, 5 * 3600);
        assert_eq!(r.alert_count, 0);
        assert!(!DoorWatch::is_night_hour(5, 1, 5));
        assert!(DoorWatch::is_night_hour(1, 1, 5));
    }

    // is_night_hour, wrapped window: end hour exclusive (< vs <=)
    #[test]
    fn night_end_hour_is_exclusive_in_wrapped_window() {
        let mut w = DoorWatch::new();
        w.register_contact(cfg_wrapped(1));
        let r = w.process_event(1, true, 6 * 3600);
        assert_eq!(r.alert_count, 0);
        let r2 = w.process_event(1, true, 5 * 3600 + 3599);
        assert_eq!(r2.alert_count, 1);
    }

    // is_night_hour, wrapped window: hour < end (vs ==)
    #[test]
    fn early_morning_is_night_in_wrapping_window() {
        let mut w = DoorWatch::new();
        w.register_contact(cfg_wrapped(1));
        let r = w.process_event(1, true, 3 * 3600);
        assert_eq!(r.alert_count, 1);
        assert_eq!(r.alerts[0].alert_type, DoorAlertType::OpenedAtNight);
    }

    // process_event: loop bound < contact_count (vs <=) with a full table
    #[test]
    fn unknown_contact_with_full_table_does_not_overrun() {
        let mut w = DoorWatch::new();
        for i in 0..MAX_CONTACTS as u32 {
            assert!(w.register_contact(cfg_wrapped(i + 1)));
        }
        let r = w.process_event(9999, true, 43200);
        assert_eq!(r.alert_count, 0);
        let r = w.process_event(9999, false, 43200);
        assert_eq!(r.alert_count, 0);
    }

    // check_timeouts: loop bound < contact_count (vs <=) with a full table
    #[test]
    fn check_timeouts_with_full_contact_table() {
        let mut w = DoorWatch::new();
        for i in 0..MAX_CONTACTS as u32 {
            let mut c = cfg_wrapped(i + 1);
            c.zone_id = 7;
            assert!(w.register_contact(c));
        }
        w.process_event(1, true, 43200);
        let r = w.check_timeouts(43200 + 400);
        assert_eq!(r.alert_count, 1);
        assert_eq!(r.alerts[0].contact_id, 1);
        assert_eq!(r.alerts[0].open_duration_sec, 400);
    }

    // check_timeouts: is_open && over-limit (vs ||)
    #[test]
    fn closed_contact_never_times_out() {
        let mut w = DoorWatch::new();
        w.register_contact(cfg_wrapped(1));
        let r = w.check_timeouts(100_000);
        assert_eq!(r.alert_count, 0);
    }

    #[test]
    fn closed_after_open_never_times_out() {
        let mut w = DoorWatch::new();
        w.register_contact(cfg_wrapped(1));
        w.process_event(1, true, 43200);
        w.process_event(1, false, 43210);
        let r = w.check_timeouts(43200 + 5000);
        assert_eq!(r.alert_count, 0);
    }

    // check_timeouts: duration is now - opened_at (vs +)
    #[test]
    fn no_alert_before_timeout() {
        let mut w = DoorWatch::new();
        w.register_contact(cfg_wrapped(1));
        w.process_event(1, true, 43200);
        let r = w.check_timeouts(43300);
        assert_eq!(r.alert_count, 0);
    }

    #[test]
    fn alert_reports_duration() {
        let mut w = DoorWatch::new();
        w.register_contact(cfg_wrapped(1));
        w.process_event(1, true, 43200);
        let r = w.check_timeouts(43600);
        assert_eq!(r.alert_count, 1);
        assert_eq!(r.alerts[0].open_duration_sec, 400);
    }

    // check_timeouts: strictly over max_open_sec (> vs >=)
    #[test]
    fn no_alert_when_open_exactly_max_duration() {
        let mut w = DoorWatch::new();
        w.register_contact(cfg_wrapped(1));
        w.process_event(1, true, 43200);
        let r = w.check_timeouts(43200 + 300);
        assert_eq!(r.alert_count, 0);
        let r = w.check_timeouts(43200 + 301);
        assert_eq!(r.alert_count, 1);
        assert_eq!(r.alerts[0].open_duration_sec, 301);
    }

    // check_timeouts: open && over-limit (vs ||) for a short open
    #[test]
    fn short_open_no_timeout_alert() {
        let mut w = DoorWatch::new();
        w.register_contact(cfg_wrapped(1));
        w.process_event(1, true, 43200);
        let r = w.check_timeouts(43200 + 100);
        assert_eq!(r.alert_count, 0);
        let r = w.check_timeouts(43200 + 300);
        assert_eq!(r.alert_count, 0);
        let r = w.check_timeouts(43200 + 301);
        assert_eq!(r.alert_count, 1);
    }

    // check_timeouts: alert array bound < MAX_ALERTS_PER_CHECK (vs <=)
    #[test]
    fn alerts_capped_at_max_per_check() {
        let mut w = DoorWatch::new();
        let mut id: u32 = 1;
        while id <= 5 {
            let mut c = cfg_wrapped(id);
            c.zone_id = id * 10;
            assert!(w.register_contact(c));
            w.process_event(id, true, 43200);
            id += 1;
        }
        let r = w.check_timeouts(43200 + 400);
        assert_eq!(r.alert_count as usize, MAX_ALERTS_PER_CHECK);
        assert_eq!(r.alerts[0].contact_id, 1);
        assert_eq!(r.alerts[3].contact_id, 4);
    }
}
