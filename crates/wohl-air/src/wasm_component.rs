// Wohl Air Quality Monitor — P3 WASM component.
//
// Binding glue only: the verified engine is the plain crate (#84).
//
// Built by: bazel build //:wohl-air (rules_wasm_component, wasi_version="p3")

// The verified engine comes from the plain crate (crates/wohl-air/plain),
// built as the Bazel target //:wohl-air-engine — no inline copy (#84).
use wohl_air::engine;

// ═══════════════════════════════════════════════════════════════
// P3 WASM component binding — delegates to verified engine
// ═══════════════════════════════════════════════════════════════

use wohl_air_bindings::exports::pulseengine::wohl_air_quality::air_quality::{
    Guest, AirAlertType as WitAlertType, AirAlert as WitAlert,
    AirConfig as WitConfig, AirReading as WitReading,
};

struct Component;

// Singleton monitor state. WASM components are single-threaded, but we use
// `OnceLock<Mutex<_>>` because it's the simplest way to satisfy Rust 2024's
// `static_mut_refs` lint without `static mut` / `unsafe`. The Mutex is
// uncontended in practice (single-threaded wasm32-wasip2 guest).
use std::sync::{Mutex, OnceLock};

static TABLE: OnceLock<Mutex<engine::AirMonitor>> = OnceLock::new();

fn with_table<R>(f: impl FnOnce(&mut engine::AirMonitor) -> R) -> R {
    let cell = TABLE.get_or_init(|| Mutex::new(engine::AirMonitor::new()));
    let mut guard = cell.lock().expect("AirMonitor mutex poisoned");
    f(&mut guard)
}

fn reset_table() {
    let cell = TABLE.get_or_init(|| Mutex::new(engine::AirMonitor::new()));
    *cell.lock().expect("AirMonitor mutex poisoned") = engine::AirMonitor::new();
}

fn to_wit_alert_type(t: engine::AirAlertType) -> WitAlertType {
    match t {
        engine::AirAlertType::Co2Warning => WitAlertType::Co2Warning,
        engine::AirAlertType::Co2Critical => WitAlertType::Co2Critical,
        engine::AirAlertType::Pm25Warning => WitAlertType::Pm25Warning,
        engine::AirAlertType::Pm25Critical => WitAlertType::Pm25Critical,
        engine::AirAlertType::VocWarning => WitAlertType::VocWarning,
        engine::AirAlertType::VocCritical => WitAlertType::VocCritical,
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
    async fn register_zone(config: WitConfig) -> bool {
        Self::do_register_zone(config)
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn register_zone(config: WitConfig) -> bool {
        Self::do_register_zone(config)
    }

    #[cfg(target_arch = "wasm32")]
    async fn process_reading(reading: WitReading) -> Vec<WitAlert> {
        Self::do_process_reading(reading)
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn process_reading(reading: WitReading) -> Vec<WitAlert> {
        Self::do_process_reading(reading)
    }
}

impl Component {
    fn do_register_zone(config: WitConfig) -> bool {
        with_table(|t| t.register_zone(engine::AirConfig {
            zone_id: config.zone_id,
            co2_warn: config.co2_warn,
            co2_critical: config.co2_critical,
            pm25_warn: config.pm25_warn,
            pm25_critical: config.pm25_critical,
            voc_warn: config.voc_warn,
            voc_critical: config.voc_critical,
            enabled: config.enabled,
        }))
    }

    fn do_process_reading(reading: WitReading) -> Vec<WitAlert> {
        let result = with_table(|t| t.process_reading(engine::AirReading {
            zone_id: reading.zone_id,
            co2_ppm: reading.co2_ppm,
            pm25: reading.pm25,
            voc_index: reading.voc_index,
            time: reading.time,
        }));
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

wohl_air_bindings::export!(Component with_types_in wohl_air_bindings);
