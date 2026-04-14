// Wohl Door Watch — P3 WASM component (self-contained).
//
// This file contains both:
//   1. The verified core engine (from plain/src/engine.rs)
//   2. The P3 async Guest trait implementation
//
// Built by: bazel build //:wohl-door (rules_wasm_component, wasi_version="p3")

// ═══════════════════════════════════════════════════════════════
// Verified core engine (plain Rust, identical to plain/src/engine.rs)
// ═══════════════════════════════════════════════════════════════

mod engine {
    pub const MAX_CONTACTS: usize = 32;
    pub const MAX_ALERTS_PER_CHECK: usize = 4;

    #[derive(Clone, Copy)]
    pub struct ContactConfig {
        pub contact_id: u32,
        pub zone_id: u32,
        pub max_open_sec: u32,
        pub night_start_hour: u8,
        pub night_end_hour: u8,
        pub enabled: bool,
    }

    #[derive(Clone, Copy)]
    pub struct ContactState {
        pub contact_id: u32,
        pub open: bool,
        pub opened_at: u64,
        pub active: bool,
    }

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum DoorAlertType {
        OpenTooLong,
        OpenedAtNight,
    }

    #[derive(Clone, Copy)]
    pub struct DoorAlert {
        pub contact_id: u32,
        pub zone_id: u32,
        pub alert_type: DoorAlertType,
        pub open_duration_sec: u64,
        pub time: u64,
    }

    #[derive(Clone, Copy)]
    pub struct DoorResult {
        pub alerts: [DoorAlert; MAX_ALERTS_PER_CHECK],
        pub alert_count: u32,
    }

    pub struct DoorWatch {
        configs: [ContactConfig; MAX_CONTACTS],
        states: [ContactState; MAX_CONTACTS],
        contact_count: u32,
    }

    impl ContactConfig {
        pub const fn empty() -> Self {
            ContactConfig { contact_id: 0, zone_id: 0, max_open_sec: 0, night_start_hour: 0, night_end_hour: 0, enabled: false }
        }
    }

    impl ContactState {
        pub const fn empty() -> Self {
            ContactState { contact_id: 0, open: false, opened_at: 0, active: false }
        }
    }

    impl DoorAlert {
        pub const fn empty() -> Self {
            DoorAlert { contact_id: 0, zone_id: 0, alert_type: DoorAlertType::OpenTooLong, open_duration_sec: 0, time: 0 }
        }
    }

    impl DoorResult {
        pub const fn empty() -> Self {
            DoorResult { alerts: [DoorAlert::empty(); MAX_ALERTS_PER_CHECK], alert_count: 0 }
        }
    }

    impl DoorWatch {
        pub fn new() -> Self {
            DoorWatch {
                configs: [ContactConfig::empty(); MAX_CONTACTS],
                states: [ContactState::empty(); MAX_CONTACTS],
                contact_count: 0,
            }
        }

        pub fn register_contact(&mut self, config: ContactConfig) -> bool {
            if self.contact_count as usize >= MAX_CONTACTS { return false; }
            let idx = self.contact_count as usize;
            self.configs[idx] = config;
            self.states[idx] = ContactState {
                contact_id: config.contact_id,
                open: false,
                opened_at: 0,
                active: true,
            };
            self.contact_count += 1;
            true
        }

        fn is_night_hour(hour: u8, night_start: u8, night_end: u8) -> bool {
            if night_start <= night_end {
                hour >= night_start && hour < night_end
            } else {
                hour >= night_start || hour < night_end
            }
        }

        pub fn process_event(&mut self, contact_id: u32, open: bool, time: u64) -> DoorResult {
            let mut res = DoorResult::empty();
            let count = self.contact_count;
            let mut i: u32 = 0;
            while i < count {
                let idx = i as usize;
                if self.states[idx].active
                    && self.configs[idx].enabled
                    && self.configs[idx].contact_id == contact_id
                {
                    if open {
                        self.states[idx].open = true;
                        self.states[idx].opened_at = time;

                        let hour_of_day: u8 = ((time % 86400) / 3600) as u8;
                        if Self::is_night_hour(
                            hour_of_day,
                            self.configs[idx].night_start_hour,
                            self.configs[idx].night_end_hour,
                        ) {
                            if (res.alert_count as usize) < MAX_ALERTS_PER_CHECK {
                                res.alerts[res.alert_count as usize] = DoorAlert {
                                    contact_id,
                                    zone_id: self.configs[idx].zone_id,
                                    alert_type: DoorAlertType::OpenedAtNight,
                                    open_duration_sec: 0,
                                    time,
                                };
                                res.alert_count += 1;
                            }
                        }
                    } else {
                        self.states[idx].open = false;
                    }

                    return res;
                }
                i += 1;
            }

            res
        }

        pub fn check_timeouts(&mut self, current_time: u64) -> DoorResult {
            let mut res = DoorResult::empty();
            let count = self.contact_count;
            let mut i: u32 = 0;
            while i < count {
                let idx = i as usize;
                if self.states[idx].active
                    && self.configs[idx].enabled
                    && self.states[idx].open
                {
                    let opened_at = self.states[idx].opened_at;
                    if current_time >= opened_at {
                        let duration = current_time - opened_at;
                        if duration > self.configs[idx].max_open_sec as u64 {
                            if (res.alert_count as usize) < MAX_ALERTS_PER_CHECK {
                                res.alerts[res.alert_count as usize] = DoorAlert {
                                    contact_id: self.configs[idx].contact_id,
                                    zone_id: self.configs[idx].zone_id,
                                    alert_type: DoorAlertType::OpenTooLong,
                                    open_duration_sec: duration,
                                    time: current_time,
                                };
                                res.alert_count += 1;
                            }
                        }
                    }
                }
                i += 1;
            }

            res
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// P3 WASM component binding — delegates to verified engine
// ═══════════════════════════════════════════════════════════════

use wohl_door_bindings::exports::pulseengine::wohl_door_watch::door_watch::{
    Guest, DoorAlertType as WitAlertType, DoorAlert as WitAlert,
};

struct Component;

static mut TABLE: Option<engine::DoorWatch> = None;

fn get_table() -> &'static mut engine::DoorWatch {
    unsafe {
        if TABLE.is_none() {
            TABLE = Some(engine::DoorWatch::new());
        }
        TABLE.as_mut().unwrap()
    }
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
        unsafe { TABLE = Some(engine::DoorWatch::new()); }
        Ok(())
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn init() -> Result<(), String> {
        unsafe { TABLE = Some(engine::DoorWatch::new()); }
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
        get_table().register_contact(engine::ContactConfig {
            contact_id,
            zone_id,
            max_open_sec,
            night_start_hour,
            night_end_hour,
            enabled: true,
        })
    }

    fn do_process_event(contact_id: u32, open: bool, time: u64) -> Vec<WitAlert> {
        door_result_to_vec(get_table().process_event(contact_id, open, time))
    }

    fn do_check_timeouts(current_time: u64) -> Vec<WitAlert> {
        door_result_to_vec(get_table().check_timeouts(current_time))
    }
}

wohl_door_bindings::export!(Component with_types_in wohl_door_bindings);
