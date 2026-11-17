// Wohl Water Leak Detector — P3 WASM component.
//
// Binding glue only: the verified engine is the plain crate (#84).
//
// Built by: bazel build //:wohl-leak (rules_wasm_component, wasi_version="p3")

// The verified engine comes from the plain crate (crates/wohl-leak/plain),
// built as the Bazel target //:wohl-leak-engine — no inline copy (#84).
use wohl_leak::engine;

// ═══════════════════════════════════════════════════════════════
// P3 WASM component binding — delegates to verified engine
// ═══════════════════════════════════════════════════════════════

use wohl_leak_bindings::exports::pulseengine::wohl_leak::leak::{
    Guest, LeakAction as WitLeakAction,
};

struct Component;

// Singleton detector state. WASM components are single-threaded, but we use
// `OnceLock<Mutex<_>>` because it's the simplest way to satisfy Rust 2024's
// `static_mut_refs` lint without `static mut` / `unsafe`. The Mutex is
// uncontended in practice (single-threaded wasm32-wasip2 guest).
use std::sync::{Mutex, OnceLock};

static TABLE: OnceLock<Mutex<engine::LeakDetector>> = OnceLock::new();

fn with_table<R>(f: impl FnOnce(&mut engine::LeakDetector) -> R) -> R {
    let cell = TABLE.get_or_init(|| Mutex::new(engine::LeakDetector::new()));
    let mut guard = cell.lock().expect("LeakDetector mutex poisoned");
    f(&mut guard)
}

fn reset_table() {
    let cell = TABLE.get_or_init(|| Mutex::new(engine::LeakDetector::new()));
    *cell.lock().expect("LeakDetector mutex poisoned") = engine::LeakDetector::new();
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
        reset_table();
        Ok(())
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn init() -> Result<(), String> {
        reset_table();
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
        with_table(|t| t.register_zone(zone_id))
    }

    fn do_process_event(zone_id: u32, wet: bool, timestamp_sec: u64) -> WitLeakAction {
        with_table(|t| to_wit_action(t.process_event(zone_id, wet, timestamp_sec)))
    }

    fn do_any_wet() -> bool {
        with_table(|t| t.any_wet())
    }
}

wohl_leak_bindings::export!(Component with_types_in wohl_leak_bindings);
