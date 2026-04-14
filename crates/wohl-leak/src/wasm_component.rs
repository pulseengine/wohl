// Wohl Water Leak Detector — P3 WASM component (self-contained).
//
// This file contains both:
//   1. The verified core engine (from plain/src/engine.rs)
//   2. The P3 async Guest trait implementation
//
// Built by: bazel build //:wohl-leak (rules_wasm_component, wasi_version="p3")

// ═══════════════════════════════════════════════════════════════
// Verified core engine (plain Rust, identical to plain/src/engine.rs)
// ═══════════════════════════════════════════════════════════════

mod engine {
    pub const MAX_ZONES: usize = 32;

    #[derive(Clone, Copy)]
    pub struct ZoneState {
        pub zone_id: u32,
        pub wet: bool,
        pub detected_at: u64,
        pub active: bool,
    }

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum LeakAction {
        NewLeak,
        AlreadyWet,
        Cleared,
        AlreadyDry,
        Unknown,
    }

    pub struct LeakDetector {
        zones: [ZoneState; MAX_ZONES],
        zone_count: u32,
    }

    impl ZoneState {
        pub const fn empty() -> Self {
            ZoneState { zone_id: 0, wet: false, detected_at: 0, active: false }
        }
    }

    impl LeakDetector {
        pub fn new() -> Self {
            LeakDetector { zones: [ZoneState::empty(); MAX_ZONES], zone_count: 0 }
        }

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
}

// ═══════════════════════════════════════════════════════════════
// P3 WASM component binding — delegates to verified engine
// ═══════════════════════════════════════════════════════════════

use wohl_leak_bindings::exports::pulseengine::wohl_leak::leak::{
    Guest, LeakAction as WitLeakAction,
};

struct Component;

static mut TABLE: Option<engine::LeakDetector> = None;

fn get_table() -> &'static mut engine::LeakDetector {
    unsafe {
        if TABLE.is_none() {
            TABLE = Some(engine::LeakDetector::new());
        }
        TABLE.as_mut().unwrap()
    }
}

fn to_wit_action(action: engine::LeakAction) -> WitLeakAction {
    match action {
        engine::LeakAction::NewLeak => WitLeakAction::NewLeak,
        engine::LeakAction::AlreadyWet => WitLeakAction::AlreadyWet,
        engine::LeakAction::Cleared => WitLeakAction::Cleared,
        engine::LeakAction::AlreadyDry => WitLeakAction::AlreadyDry,
        engine::LeakAction::Unknown => WitLeakAction::Unknown,
    }
}

impl Guest for Component {
    #[cfg(target_arch = "wasm32")]
    async fn init() -> Result<(), String> {
        unsafe { TABLE = Some(engine::LeakDetector::new()); }
        Ok(())
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn init() -> Result<(), String> {
        unsafe { TABLE = Some(engine::LeakDetector::new()); }
        Ok(())
    }

    #[cfg(target_arch = "wasm32")]
    async fn register_zone(zone_id: u32) -> bool {
        Self::do_register_zone(zone_id)
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn register_zone(zone_id: u32) -> bool {
        Self::do_register_zone(zone_id)
    }

    #[cfg(target_arch = "wasm32")]
    async fn process_event(zone_id: u32, wet: bool, timestamp_sec: u64) -> WitLeakAction {
        Self::do_process_event(zone_id, wet, timestamp_sec)
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn process_event(zone_id: u32, wet: bool, timestamp_sec: u64) -> WitLeakAction {
        Self::do_process_event(zone_id, wet, timestamp_sec)
    }

    #[cfg(target_arch = "wasm32")]
    async fn any_wet() -> bool {
        Self::do_any_wet()
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn any_wet() -> bool {
        Self::do_any_wet()
    }
}

impl Component {
    fn do_register_zone(zone_id: u32) -> bool {
        get_table().register_zone(zone_id)
    }

    fn do_process_event(zone_id: u32, wet: bool, timestamp_sec: u64) -> WitLeakAction {
        to_wit_action(get_table().process_event(zone_id, wet, timestamp_sec))
    }

    fn do_any_wet() -> bool {
        get_table().any_wet()
    }
}

wohl_leak_bindings::export!(Component with_types_in wohl_leak_bindings);
