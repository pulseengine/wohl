//! Wohl Alert Dispatcher — uses Relay Telemetry Output for subscription routing.
//!
//! Architecture:
//!   - relay-to::SubscriptionTable handles alert routing configuration (VERIFIED)
//!   - This module adds: deduplication, rate limiting, channel encoding
//!
//! Wohl provides DEDUP + RATE LIMITING + CHANNEL MAPPING.
//! Relay provides VERIFIED SUBSCRIPTION FILTERING.

use relay_to::engine::{SubscriptionTable, ToDecision};

pub const MAX_RECENT_ALERTS: usize = 64;
pub const MAX_OUTPUT_QUEUE: usize = 16;
pub const DEDUP_COOLDOWN_SEC: u64 = 300;
pub const MAX_ALERTS_PER_MINUTE: u32 = 10;
pub const MAX_CHANNELS: usize = 8;

#[derive(Clone, Copy)]
pub struct AlertEntry { pub zone_id: u32, pub alert_type: u8, pub time: u64 }

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DispatchAction { Send, Deduplicated, RateLimited, NotSubscribed }

#[derive(Clone, Copy)]
pub struct DispatchResult { pub action: DispatchAction, pub queue_depth: u32 }

/// Encodes an alert type + zone into a subscription message ID.
/// Layout: upper 8 bits = alert_type, lower 24 bits = zone_id.
pub const fn subscription_msg_id(zone_id: u32, alert_type: u8) -> u32 {
    ((alert_type as u32) << 24) | (zone_id & 0x00FF_FFFF)
}

pub struct AlertDispatcher {
    recent: [AlertEntry; MAX_RECENT_ALERTS],
    pub recent_count: u32,
    minute_count: u32,
    minute_start: u64,
    /// Relay's verified subscription table handles alert routing configuration.
    subscriptions: SubscriptionTable,
}

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
            subscriptions: SubscriptionTable::new(),
        }
    }

    /// Subscribe a specific alert type + zone to a notification channel.
    /// Uses relay-to's verified SubscriptionTable for routing decisions.
    /// Priority maps to notification urgency (0 = highest).
    pub fn subscribe(&mut self, zone_id: u32, alert_type: u8, priority: u8) -> bool {
        let msg_id = subscription_msg_id(zone_id, alert_type);
        self.subscriptions.subscribe(msg_id, priority)
    }

    /// Unsubscribe an alert type + zone from notifications.
    pub fn unsubscribe(&mut self, zone_id: u32, alert_type: u8) -> bool {
        let msg_id = subscription_msg_id(zone_id, alert_type);
        self.subscriptions.unsubscribe(msg_id)
    }

    /// Check whether an alert type + zone is subscribed for delivery.
    /// Uses relay-to (VERIFIED) for the subscription evaluation.
    pub fn is_subscribed(&self, zone_id: u32, alert_type: u8) -> bool {
        let msg_id = subscription_msg_id(zone_id, alert_type);
        self.subscriptions.evaluate(msg_id) == ToDecision::Include
    }

    /// Count active subscriptions (delegates to relay-to).
    pub fn active_subscription_count(&self) -> u32 {
        self.subscriptions.get_active_count()
    }

    pub fn process_alert(&mut self, zone_id: u32, alert_type: u8, time: u64) -> DispatchResult {
        // ── Phase 0: subscription check via relay-to (VERIFIED) ──
        let msg_id = subscription_msg_id(zone_id, alert_type);
        let decision = self.subscriptions.evaluate(msg_id);
        if decision == ToDecision::Exclude || decision == ToDecision::NotSubscribed {
            return DispatchResult { action: DispatchAction::NotSubscribed, queue_depth: self.recent_count };
        }

        // ── Phase 1: dedup check (domain-specific) ──

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

        // ── Phase 2: rate limit check (domain-specific) ──

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

    /// Helper: create dispatcher with a subscription for the given zone+type.
    fn subscribed_dispatcher(zone_id: u32, alert_type: u8) -> AlertDispatcher {
        let mut d = AlertDispatcher::new();
        d.subscribe(zone_id, alert_type, 1);
        d
    }

    #[test] fn test_first_alert_sends() {
        let mut d = subscribed_dispatcher(1, 1);
        let r = d.process_alert(1, 1, 1000);
        assert_eq!(r.action, DispatchAction::Send);
    }

    #[test] fn test_not_subscribed_rejects() {
        let mut d = AlertDispatcher::new();
        let r = d.process_alert(1, 1, 1000);
        assert_eq!(r.action, DispatchAction::NotSubscribed);
    }

    #[test] fn test_unsubscribed_rejects() {
        let mut d = AlertDispatcher::new();
        d.subscribe(1, 1, 1);
        d.unsubscribe(1, 1);
        let r = d.process_alert(1, 1, 1000);
        assert_eq!(r.action, DispatchAction::NotSubscribed);
    }

    #[test] fn test_subscription_check() {
        let mut d = AlertDispatcher::new();
        assert!(!d.is_subscribed(1, 1));
        d.subscribe(1, 1, 1);
        assert!(d.is_subscribed(1, 1));
        assert!(!d.is_subscribed(2, 1));
    }

    #[test] fn test_active_count() {
        let mut d = AlertDispatcher::new();
        d.subscribe(1, 1, 1);
        d.subscribe(2, 1, 1);
        assert_eq!(d.active_subscription_count(), 2);
        d.unsubscribe(1, 1);
        assert_eq!(d.active_subscription_count(), 1);
    }

    #[test] fn test_duplicate_within_cooldown() {
        let mut d = subscribed_dispatcher(1, 1);
        d.process_alert(1, 1, 1000);
        let r = d.process_alert(1, 1, 1100);
        assert_eq!(r.action, DispatchAction::Deduplicated);
    }

    #[test] fn test_duplicate_after_cooldown() {
        let mut d = subscribed_dispatcher(1, 1);
        d.process_alert(1, 1, 1000);
        let r = d.process_alert(1, 1, 1300);
        assert_eq!(r.action, DispatchAction::Send);
    }

    #[test] fn test_rate_limit_kicks_in() {
        let mut d = AlertDispatcher::new();
        for i in 0..MAX_ALERTS_PER_MINUTE {
            d.subscribe(100 + i, 1, 1);
        }
        d.subscribe(200, 1, 1);
        for i in 0..MAX_ALERTS_PER_MINUTE {
            let r = d.process_alert(100 + i, 1, 1000);
            assert_eq!(r.action, DispatchAction::Send);
        }
        let r = d.process_alert(200, 1, 1000);
        assert_eq!(r.action, DispatchAction::RateLimited);
    }

    #[test] fn test_rate_limit_resets_after_minute() {
        let mut d = AlertDispatcher::new();
        for i in 0..MAX_ALERTS_PER_MINUTE {
            d.subscribe(100 + i, 1, 1);
        }
        d.subscribe(200, 1, 1);
        for i in 0..MAX_ALERTS_PER_MINUTE {
            d.process_alert(100 + i, 1, 1000);
        }
        let r = d.process_alert(200, 1, 1060);
        assert_eq!(r.action, DispatchAction::Send);
    }

    #[test] fn test_clear_expired() {
        let mut d = subscribed_dispatcher(1, 1);
        d.process_alert(1, 1, 1000);
        assert_eq!(d.recent_count, 1);
        d.clear_expired(1301);
        assert_eq!(d.recent_count, 0);
        let r = d.process_alert(1, 1, 1301);
        assert_eq!(r.action, DispatchAction::Send);
    }

    #[test] fn test_different_zone_not_deduplicated() {
        let mut d = AlertDispatcher::new();
        d.subscribe(1, 1, 1);
        d.subscribe(2, 1, 1);
        d.process_alert(1, 1, 1000);
        let r = d.process_alert(2, 1, 1000);
        assert_eq!(r.action, DispatchAction::Send);
    }

    #[test] fn test_msg_id_encoding() {
        // alert_type=1, zone_id=42 -> (1 << 24) | 42 = 0x0100_002A
        assert_eq!(subscription_msg_id(42, 1), 0x0100_002A);
        // alert_type=0xFF, zone_id=0x00FF_FFFF -> (0xFF << 24) | 0x00FF_FFFF
        assert_eq!(subscription_msg_id(0x00FF_FFFF, 0xFF), 0xFFFF_FFFF);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn dedup_always_works(
            zone_id in 0u32..100,
            alert_type in 0u8..10,
            time in 0u64..1000,
        ) {
            let mut disp = AlertDispatcher::new();
            disp.subscribe(zone_id, alert_type, 1);
            let r1 = disp.process_alert(zone_id, alert_type, time);
            prop_assert_eq!(r1.action, DispatchAction::Send);
            // Same alert within cooldown should dedup
            let r2 = disp.process_alert(zone_id, alert_type, time + 1);
            prop_assert_eq!(r2.action, DispatchAction::Deduplicated);
        }

        #[test]
        fn rate_limit_kicks_in(
            zone_id in 0u32..10,
        ) {
            let mut disp = AlertDispatcher::new();
            // Subscribe and send MAX_ALERTS_PER_MINUTE different types
            for t in 0..MAX_ALERTS_PER_MINUTE as u8 + 1 {
                disp.subscribe(zone_id, t, 1);
            }
            for t in 0..MAX_ALERTS_PER_MINUTE as u8 {
                let r = disp.process_alert(zone_id, t, 100);
                prop_assert!(r.action == DispatchAction::Send || r.action == DispatchAction::NotSubscribed);
            }
            // Next one should be rate limited
            let r = disp.process_alert(zone_id, MAX_ALERTS_PER_MINUTE as u8, 100);
            prop_assert_eq!(r.action, DispatchAction::RateLimited);
        }
    }
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
        // Subscribe first (required for routing via relay-to)
        d.subscribe(zone_id, alert_type, 1);
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
        // Subscribe and send MAX_ALERTS_PER_MINUTE alerts with distinct zone_ids
        let mut i: u32 = 0;
        while i < MAX_ALERTS_PER_MINUTE {
            d.subscribe(1000 + i, 1, 1);
            let r = d.process_alert(1000 + i, 1, time);
            assert_eq!(r.action, DispatchAction::Send);
            i += 1;
        }
        // Next alert within same minute should be RateLimited
        d.subscribe(9999, 1, 1);
        let r = d.process_alert(9999, 1, time);
        assert_eq!(r.action, DispatchAction::RateLimited);
    }

    /// ALERT-P05: unsubscribed alert returns NotSubscribed (via relay-to)
    #[kani::proof]
    fn verify_not_subscribed() {
        let d = AlertDispatcher::new();
        let zone_id: u32 = kani::any();
        let alert_type: u8 = kani::any();
        // No subscriptions — relay-to should return NotSubscribed
        let decision = d.subscriptions.evaluate(subscription_msg_id(zone_id, alert_type));
        assert_eq!(decision, ToDecision::NotSubscribed);
    }

    /// No panics for any combination of symbolic inputs
    #[kani::proof]
    fn verify_no_panic() {
        let mut d = AlertDispatcher::new();
        let zone_id: u32 = kani::any();
        let alert_type: u8 = kani::any();
        let time: u64 = kani::any();
        d.subscribe(zone_id, alert_type, 1);
        let _ = d.process_alert(zone_id, alert_type, time);
        let _ = d.process_alert(zone_id, alert_type, time);
        let clear_time: u64 = kani::any();
        d.clear_expired(clear_time);
    }
}
