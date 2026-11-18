// Wohl Power Meter — P3 WASM component.
//
// Binding glue only: the verified engine is the plain crate (#84).
//
// Built by: bazel build //:wohl-power (rules_wasm_component, wasi_version="p3")

// The verified engine comes from the plain crate (crates/wohl-power/plain),
// built as the Bazel target //:wohl-power-engine — no inline copy (#84).
use wohl_power::engine;

// ═══════════════════════════════════════════════════════════════
// P3 WASM component binding — delegates to verified engine
// ═══════════════════════════════════════════════════════════════

use wohl_power_bindings::exports::pulseengine::wohl_power_meter::power::{
    Guest, PowerAlertType as WitAlertType, PowerAlert as WitAlert,
};

struct Component;

// Singleton monitor state. WASM components are single-threaded, but we use
// `OnceLock<Mutex<_>>` because it's the simplest way to satisfy Rust 2024's
// `static_mut_refs` lint without `static mut` / `unsafe`. The Mutex is
// uncontended in practice (single-threaded wasm32-wasip2 guest).
use std::sync::{Mutex, OnceLock};

static TABLE: OnceLock<Mutex<engine::PowerMonitor>> = OnceLock::new();

fn with_table<R>(f: impl FnOnce(&mut engine::PowerMonitor) -> R) -> R {
    let cell = TABLE.get_or_init(|| Mutex::new(engine::PowerMonitor::new()));
    let mut guard = cell.lock().expect("PowerMonitor mutex poisoned");
    f(&mut guard)
}

fn reset_table() {
    let cell = TABLE.get_or_init(|| Mutex::new(engine::PowerMonitor::new()));
    *cell.lock().expect("PowerMonitor mutex poisoned") = engine::PowerMonitor::new();
}

fn to_wit_alert_type(t: engine::PowerAlertType) -> WitAlertType {
    match t {
        engine::PowerAlertType::OverConsumption => WitAlertType::OverConsumption,
        engine::PowerAlertType::Spike => WitAlertType::Spike,
        engine::PowerAlertType::DeviceLeftOn => WitAlertType::DeviceLeftOn,
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
    async fn register_circuit(circuit_id: u32, max_watts: u32, idle_watts: u32, spike_threshold: u32) -> bool {
        Self::do_register_circuit(circuit_id, max_watts, idle_watts, spike_threshold)
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn register_circuit(circuit_id: u32, max_watts: u32, idle_watts: u32, spike_threshold: u32) -> bool {
        Self::do_register_circuit(circuit_id, max_watts, idle_watts, spike_threshold)
    }

    #[cfg(target_arch = "wasm32")]
    async fn process_reading(circuit_id: u32, watts: u32, time: u64) -> Vec<WitAlert> {
        Self::do_process_reading(circuit_id, watts, time)
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn process_reading(circuit_id: u32, watts: u32, time: u64) -> Vec<WitAlert> {
        Self::do_process_reading(circuit_id, watts, time)
    }

    #[cfg(target_arch = "wasm32")]
    async fn check_idle(circuit_id: u32, current_watts: u32) -> bool {
        Self::do_check_idle(circuit_id, current_watts)
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn check_idle(circuit_id: u32, current_watts: u32) -> bool {
        Self::do_check_idle(circuit_id, current_watts)
    }
}

impl Component {
    fn do_register_circuit(circuit_id: u32, max_watts: u32, idle_watts: u32, spike_threshold: u32) -> bool {
        with_table(|t| t.register_circuit(engine::CircuitConfig {
            circuit_id,
            max_watts,
            idle_watts,
            spike_threshold,
            enabled: true,
        }))
    }

    fn do_process_reading(circuit_id: u32, watts: u32, time: u64) -> Vec<WitAlert> {
        let result = with_table(|t| t.process_reading(circuit_id, watts, time));
        let mut out = Vec::with_capacity(result.alert_count as usize);
        for i in 0..result.alert_count as usize {
            out.push(WitAlert {
                circuit_id: result.alerts[i].circuit_id,
                alert_type: to_wit_alert_type(result.alerts[i].alert_type),
                value: result.alerts[i].value,
                threshold: result.alerts[i].threshold,
                time: result.alerts[i].time,
            });
        }
        out
    }

    fn do_check_idle(circuit_id: u32, current_watts: u32) -> bool {
        with_table(|t| t.check_idle(circuit_id, current_watts))
    }
}

wohl_power_bindings::export!(Component with_types_in wohl_power_bindings);
