// Wohl Temperature Monitor — P3 WASM component.
//
// Binding glue only: the verified engine is the plain crate (#84).
//
// Built by: bazel build //:wohl-temp (rules_wasm_component, wasi_version="p3")

// The verified engine comes from the plain crate (crates/wohl-temp/plain),
// built as the Bazel target //:wohl-temp-engine — no inline copy (#84).
use wohl_temp::engine;

// ═══════════════════════════════════════════════════════════════
// P3 WASM component binding — delegates to verified engine
// ═══════════════════════════════════════════════════════════════

use wohl_temp_bindings::exports::pulseengine::wohl_temperature::temperature::{
    Guest, TempAlertType as WitAlertType, TempAlert as WitAlert,
};

struct Component;

// Singleton monitor state. WASM components are single-threaded, but we use
// `OnceLock<Mutex<_>>` because it's the simplest way to satisfy Rust 2024's
// `static_mut_refs` lint without `static mut` / `unsafe`. The Mutex is
// uncontended in practice (single-threaded wasm32-wasip2 guest).
use std::sync::{Mutex, OnceLock};

static TABLE: OnceLock<Mutex<engine::TemperatureMonitor>> = OnceLock::new();

fn with_table<R>(f: impl FnOnce(&mut engine::TemperatureMonitor) -> R) -> R {
    let cell = TABLE.get_or_init(|| Mutex::new(engine::TemperatureMonitor::new()));
    let mut guard = cell.lock().expect("TemperatureMonitor mutex poisoned");
    f(&mut guard)
}

fn reset_table() {
    let cell = TABLE.get_or_init(|| Mutex::new(engine::TemperatureMonitor::new()));
    *cell.lock().expect("TemperatureMonitor mutex poisoned") = engine::TemperatureMonitor::new();
}

fn to_wit_alert_type(t: engine::TempAlertType) -> WitAlertType {
    match t {
        engine::TempAlertType::Freeze => WitAlertType::Freeze,
        engine::TempAlertType::Overheat => WitAlertType::Overheat,
        engine::TempAlertType::RapidDrop => WitAlertType::RapidDrop,
        engine::TempAlertType::RapidRise => WitAlertType::RapidRise,
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
    async fn register_zone(zone_id: u32, freeze_threshold: i32, overheat_threshold: i32, rate_threshold: i32) -> bool {
        Self::do_register_zone(zone_id, freeze_threshold, overheat_threshold, rate_threshold)
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn register_zone(zone_id: u32, freeze_threshold: i32, overheat_threshold: i32, rate_threshold: i32) -> bool {
        Self::do_register_zone(zone_id, freeze_threshold, overheat_threshold, rate_threshold)
    }

    #[cfg(target_arch = "wasm32")]
    async fn process_reading(zone_id: u32, value: i32, time: u64) -> Vec<WitAlert> {
        Self::do_process_reading(zone_id, value, time)
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn process_reading(zone_id: u32, value: i32, time: u64) -> Vec<WitAlert> {
        Self::do_process_reading(zone_id, value, time)
    }
}

impl Component {
    fn do_register_zone(zone_id: u32, freeze_threshold: i32, overheat_threshold: i32, rate_threshold: i32) -> bool {
        with_table(|t| t.register_zone(engine::ZoneConfig {
            zone_id,
            freeze_threshold,
            overheat_threshold,
            rate_threshold,
            enabled: true,
        }))
    }

    fn do_process_reading(zone_id: u32, value: i32, time: u64) -> Vec<WitAlert> {
        let result = with_table(|t| t.process_reading(zone_id, value, time));
        let mut out = Vec::with_capacity(result.alert_count as usize);
        for i in 0..result.alert_count as usize {
            out.push(WitAlert {
                zone_id: result.alerts[i].zone_id,
                alert_type: to_wit_alert_type(result.alerts[i].alert_type),
                value: result.alerts[i].value,
                threshold: result.alerts[i].threshold,
                time: result.alerts[i].time,
            });
        }
        out
    }
}

wohl_temp_bindings::export!(Component with_types_in wohl_temp_bindings);
