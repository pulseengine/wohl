// Wohl Alert Dispatcher — P3 WASM component (self-contained).
//
// This file contains both:
//   1. The verified core engine (from plain/src/engine.rs)
//   2. The P3 async Guest trait implementation
//
// Built by: bazel build //:wohl-alert (rules_wasm_component, wasi_version="p3")

// ═══════════════════════════════════════════════════════════════
// Verified core engine (plain Rust, identical to plain/src/engine.rs)
// ═══════════════════════════════════════════════════════════════

mod engine {
    pub const MAX_RECENT_ALERTS: usize = 64;
    pub const MAX_OUTPUT_QUEUE: usize = 16;
    pub const DEDUP_COOLDOWN_SEC: u64 = 300;
    pub const MAX_ALERTS_PER_MINUTE: u32 = 10;

    #[derive(Clone, Copy)]
    pub struct AlertEntry {
        pub zone_id: u32,
        pub alert_type: u8,
        pub time: u64,
    }

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum DispatchAction {
        Send,
        Deduplicated,
        RateLimited,
    }

    #[derive(Clone, Copy)]
    pub struct DispatchResult {
        pub action: DispatchAction,
        pub queue_depth: u32,
    }

    pub struct AlertDispatcher {
        recent: [AlertEntry; MAX_RECENT_ALERTS],
        pub recent_count: u32,
        minute_count: u32,
        minute_start: u64,
    }

    impl AlertEntry {
        pub const fn empty() -> Self {
            AlertEntry { zone_id: 0, alert_type: 0, time: 0 }
        }
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
}

// ═══════════════════════════════════════════════════════════════
// P3 WASM component binding — delegates to verified engine
// ═══════════════════════════════════════════════════════════════

use wohl_alert_bindings::exports::pulseengine::wohl_alert_dispatcher::alert::{
    Guest, DispatchAction as WitAction, DispatchResult as WitResult,
};

struct Component;

static mut TABLE: Option<engine::AlertDispatcher> = None;

fn get_table() -> &'static mut engine::AlertDispatcher {
    unsafe {
        if TABLE.is_none() {
            TABLE = Some(engine::AlertDispatcher::new());
        }
        TABLE.as_mut().unwrap()
    }
}

fn to_wit_action(action: engine::DispatchAction) -> WitAction {
    match action {
        engine::DispatchAction::Send => WitAction::Send,
        engine::DispatchAction::Deduplicated => WitAction::Deduplicated,
        engine::DispatchAction::RateLimited => WitAction::RateLimited,
    }
}

impl Guest for Component {
    #[cfg(target_arch = "wasm32")]
    async fn init() -> Result<(), String> {
        unsafe { TABLE = Some(engine::AlertDispatcher::new()); }
        Ok(())
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn init() -> Result<(), String> {
        unsafe { TABLE = Some(engine::AlertDispatcher::new()); }
        Ok(())
    }

    #[cfg(target_arch = "wasm32")]
    async fn process_alert(zone_id: u32, alert_type: u8, time: u64) -> WitResult {
        Self::do_process_alert(zone_id, alert_type, time)
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn process_alert(zone_id: u32, alert_type: u8, time: u64) -> WitResult {
        Self::do_process_alert(zone_id, alert_type, time)
    }

    #[cfg(target_arch = "wasm32")]
    async fn clear_expired(current_time: u64) {
        Self::do_clear_expired(current_time)
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn clear_expired(current_time: u64) {
        Self::do_clear_expired(current_time)
    }
}

impl Component {
    fn do_process_alert(zone_id: u32, alert_type: u8, time: u64) -> WitResult {
        let result = get_table().process_alert(zone_id, alert_type, time);
        WitResult {
            action: to_wit_action(result.action),
            queue_depth: result.queue_depth,
        }
    }

    fn do_clear_expired(current_time: u64) {
        get_table().clear_expired(current_time);
    }
}

wohl_alert_bindings::export!(Component with_types_in wohl_alert_bindings);
