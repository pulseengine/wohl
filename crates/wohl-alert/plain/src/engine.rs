//! Wohl Alert Dispatcher — plain Rust (generated from Verus source).
//! Source of truth: ../src/core.rs. Do not edit manually.

pub const MAX_RECENT_ALERTS: usize = 64;
pub const MAX_OUTPUT_QUEUE: usize = 16;
pub const DEDUP_COOLDOWN_SEC: u64 = 300;
pub const MAX_ALERTS_PER_MINUTE: u32 = 10;

#[derive(Clone, Copy)]
pub struct AlertEntry { pub zone_id: u32, pub alert_type: u8, pub time: u64 }

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DispatchAction { Send, Deduplicated, RateLimited }

#[derive(Clone, Copy)]
pub struct DispatchResult { pub action: DispatchAction, pub queue_depth: u32 }

pub struct AlertDispatcher { recent: [AlertEntry; MAX_RECENT_ALERTS], pub recent_count: u32, minute_count: u32, minute_start: u64 }

impl AlertEntry {
    pub const fn empty() -> Self { AlertEntry { zone_id: 0, alert_type: 0, time: 0 } }
}

impl AlertDispatcher {
    pub fn new() -> Self {
        AlertDispatcher {
            recent: [AlertEntry::empty(); MAX_RECENT_ALERTS],
            recent_count: 0,
            minute_count: 0,
            minute_start: 0,
        }
    }

    pub fn process_alert(&mut self, zone_id: u32, alert_type: u8, time: u64) -> DispatchResult {
        // Reset minute counter if new minute window
        if time >= self.minute_start + 60 {
            self.minute_count = 0;
            self.minute_start = time;
        }

        // Dedup check
        let rc = self.recent_count;
        let mut i: u32 = 0;
        while i < rc {
            let idx = i as usize;
            if self.recent[idx].zone_id == zone_id
                && self.recent[idx].alert_type == alert_type
                && time < self.recent[idx].time + DEDUP_COOLDOWN_SEC
            {
                return DispatchResult { action: DispatchAction::Deduplicated, queue_depth: self.recent_count };
            }
            i = i + 1;
        }

        // Rate limit check
        if self.minute_count >= MAX_ALERTS_PER_MINUTE {
            return DispatchResult { action: DispatchAction::RateLimited, queue_depth: self.recent_count };
        }

        // Record in recent window
        if (self.recent_count as usize) < MAX_RECENT_ALERTS {
            let ridx = self.recent_count as usize;
            self.recent[ridx] = AlertEntry { zone_id, alert_type, time };
            self.recent_count = self.recent_count + 1;
        }

        self.minute_count = self.minute_count + 1;

        DispatchResult { action: DispatchAction::Send, queue_depth: self.recent_count }
    }

    pub fn clear_expired(&mut self, current_time: u64) {
        let mut write: u32 = 0;
        let rc = self.recent_count;
        let mut read: u32 = 0;

        while read < rc {
            let ridx = read as usize;
            if self.recent[ridx].time + DEDUP_COOLDOWN_SEC >= current_time {
                if write != read {
                    self.recent[write as usize] = self.recent[ridx];
                }
                write = write + 1;
            }
            read = read + 1;
        }

        self.recent_count = write;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn test_first_alert_sends() { let mut d = AlertDispatcher::new(); let r = d.process_alert(1, 1, 1000); assert_eq!(r.action, DispatchAction::Send); }
    #[test] fn test_duplicate_within_cooldown() { let mut d = AlertDispatcher::new(); d.process_alert(1, 1, 1000); let r = d.process_alert(1, 1, 1100); assert_eq!(r.action, DispatchAction::Deduplicated); }
    #[test] fn test_duplicate_after_cooldown() { let mut d = AlertDispatcher::new(); d.process_alert(1, 1, 1000); let r = d.process_alert(1, 1, 1300); assert_eq!(r.action, DispatchAction::Send); }
    #[test] fn test_rate_limit_kicks_in() { let mut d = AlertDispatcher::new(); for i in 0..MAX_ALERTS_PER_MINUTE { let r = d.process_alert(100 + i, 1, 1000); assert_eq!(r.action, DispatchAction::Send); } let r = d.process_alert(200, 1, 1000); assert_eq!(r.action, DispatchAction::RateLimited); }
    #[test] fn test_rate_limit_resets_after_minute() { let mut d = AlertDispatcher::new(); for i in 0..MAX_ALERTS_PER_MINUTE { d.process_alert(100 + i, 1, 1000); } let r = d.process_alert(200, 1, 1060); assert_eq!(r.action, DispatchAction::Send); }
    #[test] fn test_clear_expired() { let mut d = AlertDispatcher::new(); d.process_alert(1, 1, 1000); assert_eq!(d.recent_count, 1); d.clear_expired(1301); assert_eq!(d.recent_count, 0); let r = d.process_alert(1, 1, 1301); assert_eq!(r.action, DispatchAction::Send); }
    #[test] fn test_different_zone_not_deduplicated() { let mut d = AlertDispatcher::new(); d.process_alert(1, 1, 1000); let r = d.process_alert(2, 1, 1000); assert_eq!(r.action, DispatchAction::Send); }
}

// ── Kani bounded model checking harnesses ────────────────────

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// ALERT-P03: same (zone, type) within cooldown returns Deduplicated
    #[kani::proof]
    fn verify_dedup_works() {
        let mut d = AlertDispatcher::new();
        let zone_id: u32 = kani::any();
        let alert_type: u8 = kani::any();
        let time1: u64 = kani::any();
        kani::assume(time1 < u64::MAX - DEDUP_COOLDOWN_SEC);
        // First alert should be Send
        let r1 = d.process_alert(zone_id, alert_type, time1);
        assert_eq!(r1.action, DispatchAction::Send);
        // Same zone+type within cooldown should be Deduplicated
        let time2: u64 = kani::any();
        kani::assume(time2 >= time1 && time2 < time1 + DEDUP_COOLDOWN_SEC);
        let r2 = d.process_alert(zone_id, alert_type, time2);
        assert_eq!(r2.action, DispatchAction::Deduplicated);
    }

    /// ALERT-P04: after MAX_ALERTS_PER_MINUTE distinct alerts, returns RateLimited
    #[kani::proof]
    fn verify_rate_limit() {
        let mut d = AlertDispatcher::new();
        let time: u64 = kani::any();
        kani::assume(time < u64::MAX - 60);
        // Send MAX_ALERTS_PER_MINUTE alerts with distinct zone_ids
        let mut i: u32 = 0;
        while i < MAX_ALERTS_PER_MINUTE {
            let r = d.process_alert(1000 + i, 1, time);
            assert_eq!(r.action, DispatchAction::Send);
            i += 1;
        }
        // Next alert within same minute should be RateLimited
        let r = d.process_alert(9999, 1, time);
        assert_eq!(r.action, DispatchAction::RateLimited);
    }

    /// No panics for any combination of symbolic inputs
    #[kani::proof]
    fn verify_no_panic() {
        let mut d = AlertDispatcher::new();
        let zone_id: u32 = kani::any();
        let alert_type: u8 = kani::any();
        let time: u64 = kani::any();
        let _ = d.process_alert(zone_id, alert_type, time);
        let _ = d.process_alert(zone_id, alert_type, time);
        let clear_time: u64 = kani::any();
        d.clear_expired(clear_time);
    }
}
