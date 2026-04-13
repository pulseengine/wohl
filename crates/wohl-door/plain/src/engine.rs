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
pub enum DoorAlertType { OpenTooLong, OpenedAtNight }

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
        ContactConfig { contact_id: 0, zone_id: 0, max_open_sec: 0, night_start_hour: 0, night_end_hour: 0, enabled: false }
    }
}

impl ContactState {
    pub const fn empty() -> Self {
        ContactState { contact_id: 0, open: false, opened_at: 0, active: false }
    }
}

impl DoorAlert {
    pub const fn empty() -> Self {
        DoorAlert { contact_id: 0, zone_id: 0, alert_type: DoorAlertType::OpenTooLong, open_duration_sec: 0, time: 0 }
    }
}

impl DoorResult {
    pub const fn empty() -> Self {
        DoorResult { alerts: [DoorAlert::empty(); MAX_ALERTS_PER_CHECK], alert_count: 0 }
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
        if self.contact_count as usize >= MAX_CONTACTS { return false; }
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
                    ) {
                        if (res.alert_count as usize) < MAX_ALERTS_PER_CHECK {
                            res.alerts[res.alert_count as usize] = DoorAlert {
                                contact_id,
                                zone_id: self.configs[idx].zone_id,
                                alert_type: DoorAlertType::OpenedAtNight,
                                open_duration_sec: 0,
                                time,
                            };
                            res.alert_count += 1;
                        }
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
            if self.states[idx].active
                && self.configs[idx].enabled
                && self.states[idx].open
            {
                let opened_at = self.states[idx].opened_at;
                if current_time >= opened_at {
                    let duration = current_time - opened_at;
                    if duration > self.configs[idx].max_open_sec as u64 {
                        if (res.alert_count as usize) < MAX_ALERTS_PER_CHECK {
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
        ContactConfig { contact_id, zone_id, max_open_sec: 300, night_start_hour: 22, night_end_hour: 6, enabled: true }
    }

    #[test] fn test_open_close() { let mut w = DoorWatch::new(); w.register_contact(make_config(1, 10)); let r = w.process_event(1, true, 43200); assert_eq!(r.alert_count, 0); let r = w.process_event(1, false, 43260); assert_eq!(r.alert_count, 0); }
    #[test] fn test_open_too_long() { let mut w = DoorWatch::new(); w.register_contact(make_config(1, 10)); w.process_event(1, true, 43200); let r = w.check_timeouts(43200 + 400); assert_eq!(r.alert_count, 1); assert_eq!(r.alerts[0].alert_type, DoorAlertType::OpenTooLong); }
    #[test] fn test_opened_at_night() { let mut w = DoorWatch::new(); w.register_contact(make_config(1, 10)); let r = w.process_event(1, true, 82800); assert_eq!(r.alert_count, 1); assert_eq!(r.alerts[0].alert_type, DoorAlertType::OpenedAtNight); }
    #[test] fn test_normal_day() { let mut w = DoorWatch::new(); w.register_contact(make_config(1, 10)); let r = w.process_event(1, true, 50400); assert_eq!(r.alert_count, 0); }
    #[test] fn test_unknown_contact() { let mut w = DoorWatch::new(); let r = w.process_event(99, true, 1000); assert_eq!(r.alert_count, 0); }
    #[test] fn test_multiple_contacts() { let mut w = DoorWatch::new(); w.register_contact(make_config(1, 10)); w.register_contact(make_config(2, 20)); w.process_event(1, true, 43200); w.process_event(2, true, 43200); let r = w.check_timeouts(43200 + 400); assert_eq!(r.alert_count, 2); }
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
