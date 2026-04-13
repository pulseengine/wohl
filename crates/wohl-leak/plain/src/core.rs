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
