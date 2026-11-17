// Wohl Door Watch — P3 WASM component.
//
// Binding glue only: the verified engine is the plain crate (#84).
//
// Built by: bazel build //:wohl-door (rules_wasm_component, wasi_version="p3")

// The verified engine comes from the plain crate (crates/wohl-door/plain),
// built as the Bazel target //:wohl-door-engine — no inline copy (#84).
use wohl_door::engine;

// ═══════════════════════════════════════════════════════════════
// P3 WASM component binding — delegates to verified engine
// ═══════════════════════════════════════════════════════════════

use wohl_door_bindings::exports::pulseengine::wohl_door_watch::door_watch::{
    Guest, DoorAlertType as WitAlertType, DoorAlert as WitAlert,
};

struct Component;

// Singleton watch state. WASM components are single-threaded, but we use
// `OnceLock<Mutex<_>>` because it's the simplest way to satisfy Rust 2024's
// `static_mut_refs` lint without `static mut` / `unsafe`. The Mutex is
// uncontended in practice (single-threaded wasm32-wasip2 guest).
use std::sync::{Mutex, OnceLock};

static TABLE: OnceLock<Mutex<engine::DoorWatch>> = OnceLock::new();

fn with_table<R>(f: impl FnOnce(&mut engine::DoorWatch) -> R) -> R {
    let cell = TABLE.get_or_init(|| Mutex::new(engine::DoorWatch::new()));
    let mut guard = cell.lock().expect("DoorWatch mutex poisoned");
    f(&mut guard)
}

fn reset_table() {
    let cell = TABLE.get_or_init(|| Mutex::new(engine::DoorWatch::new()));
    *cell.lock().expect("DoorWatch mutex poisoned") = engine::DoorWatch::new();
}

fn to_wit_alert_type(t: engine::DoorAlertType) -> WitAlertType {
    match t {
        engine::DoorAlertType::OpenTooLong => WitAlertType::OpenTooLong,
        engine::DoorAlertType::OpenedAtNight => WitAlertType::OpenedAtNight,
    }
}

fn door_result_to_vec(res: engine::DoorResult) -> Vec<WitAlert> {
    let mut out = Vec::with_capacity(res.alert_count as usize);
    for i in 0..res.alert_count as usize {
        out.push(WitAlert {
            contact_id: res.alerts[i].contact_id,
            zone_id: res.alerts[i].zone_id,
            alert_type: to_wit_alert_type(res.alerts[i].alert_type),
            open_duration_sec: res.alerts[i].open_duration_sec,
            time: res.alerts[i].time,
        });
    }
    out
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
    async fn register_contact(contact_id: u32, zone_id: u32, max_open_sec: u32, night_start_hour: u8, night_end_hour: u8) -> bool {
        Self::do_register_contact(contact_id, zone_id, max_open_sec, night_start_hour, night_end_hour)
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn register_contact(contact_id: u32, zone_id: u32, max_open_sec: u32, night_start_hour: u8, night_end_hour: u8) -> bool {
        Self::do_register_contact(contact_id, zone_id, max_open_sec, night_start_hour, night_end_hour)
    }

    #[cfg(target_arch = "wasm32")]
    async fn process_event(contact_id: u32, open: bool, time: u64) -> Vec<WitAlert> {
        Self::do_process_event(contact_id, open, time)
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn process_event(contact_id: u32, open: bool, time: u64) -> Vec<WitAlert> {
        Self::do_process_event(contact_id, open, time)
    }

    #[cfg(target_arch = "wasm32")]
    async fn check_timeouts(current_time: u64) -> Vec<WitAlert> {
        Self::do_check_timeouts(current_time)
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn check_timeouts(current_time: u64) -> Vec<WitAlert> {
        Self::do_check_timeouts(current_time)
    }
}

impl Component {
    fn do_register_contact(contact_id: u32, zone_id: u32, max_open_sec: u32, night_start_hour: u8, night_end_hour: u8) -> bool {
        with_table(|t| t.register_contact(engine::ContactConfig {
            contact_id,
            zone_id,
            max_open_sec,
            night_start_hour,
            night_end_hour,
            enabled: true,
        }))
    }

    fn do_process_event(contact_id: u32, open: bool, time: u64) -> Vec<WitAlert> {
        door_result_to_vec(with_table(|t| t.process_event(contact_id, open, time)))
    }

    fn do_check_timeouts(current_time: u64) -> Vec<WitAlert> {
        door_result_to_vec(with_table(|t| t.check_timeouts(current_time)))
    }
}

wohl_door_bindings::export!(Component with_types_in wohl_door_bindings);
