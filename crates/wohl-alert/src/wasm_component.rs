// Wohl Alert Dispatcher — P3 WASM component.
//
// Binding glue only: the verified engine is the plain crate (#84).
//
// Built by: bazel build //:wohl-alert (rules_wasm_component, wasi_version="p3")

// The verified engine comes from the plain crate (crates/wohl-alert/plain),
// built as the Bazel target //:wohl-alert-engine — no inline copy (#84).
use wohl_alert::engine;

// ═══════════════════════════════════════════════════════════════
// P3 WASM component binding — delegates to verified engine
// ═══════════════════════════════════════════════════════════════

use wohl_alert_bindings::exports::pulseengine::wohl_alert_dispatcher::alert::{
    Guest, DispatchAction as WitAction, DispatchResult as WitResult,
};

struct Component;

// Singleton dispatcher state. WASM components are single-threaded, but we use
// `OnceLock<Mutex<_>>` because it's the simplest way to satisfy Rust 2024's
// `static_mut_refs` lint without `static mut` / `unsafe`. The Mutex is
// uncontended in practice (single-threaded wasm32-wasip2 guest).
use std::sync::{Mutex, OnceLock};

static TABLE: OnceLock<Mutex<engine::AlertDispatcher>> = OnceLock::new();

fn with_table<R>(f: impl FnOnce(&mut engine::AlertDispatcher) -> R) -> R {
    let cell = TABLE.get_or_init(|| Mutex::new(engine::AlertDispatcher::new()));
    let mut guard = cell.lock().expect("AlertDispatcher mutex poisoned");
    f(&mut guard)
}

fn reset_table() {
    let cell = TABLE.get_or_init(|| Mutex::new(engine::AlertDispatcher::new()));
    *cell.lock().expect("AlertDispatcher mutex poisoned") = engine::AlertDispatcher::new();
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
        reset_table();
        Ok(())
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn init() -> Result<(), String> {
        reset_table();
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
        let result = with_table(|t| t.process_alert(zone_id, alert_type, time));
        WitResult {
            action: to_wit_action(result.action),
            queue_depth: result.queue_depth,
        }
    }

    fn do_clear_expired(current_time: u64) {
        with_table(|t| t.clear_expired(current_time));
    }
}

wohl_alert_bindings::export!(Component with_types_in wohl_alert_bindings);
