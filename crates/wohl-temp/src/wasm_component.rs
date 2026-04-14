// Wohl Temperature Monitor — P3 WASM component (self-contained).
//
// This file contains:
//   1. The relay-lc WatchpointTable engine (verified, from relay-lc/plain/src/engine.rs)
//   2. The wohl-temp TemperatureMonitor engine (from plain/src/engine.rs)
//   3. The P3 async Guest trait implementation
//
// Built by: bazel build //:wohl-temp (rules_wasm_component, wasi_version="p3")

// ═══════════════════════════════════════════════════════════════
// Verified core engine — includes relay-lc + wohl-temp domain logic
// ═══════════════════════════════════════════════════════════════

mod engine {
    // ── relay-lc: verified watchpoint/limit checker engine ──────

    pub const MAX_WATCHPOINTS: usize = 128;
    pub const MAX_VIOLATIONS_PER_CYCLE: usize = 32;

    #[derive(Clone, Copy, PartialEq, Eq)]
    #[repr(u8)]
    pub enum ComparisonOp { LessThan = 0, GreaterThan = 1, LessOrEqual = 2, GreaterOrEqual = 3, Equal = 4, NotEqual = 5 }

    #[derive(Clone, Copy)]
    pub struct Watchpoint { pub sensor_id: u32, pub op: ComparisonOp, pub threshold: i64, pub enabled: bool, pub persistence: u32, pub current_count: u32 }

    #[derive(Clone, Copy)]
    pub struct Violation { pub watchpoint_id: u32, pub measured: i64, pub threshold: i64, pub op: ComparisonOp }

    #[derive(Clone, Copy)]
    pub struct SensorReading { pub sensor_id: u32, pub value: i64 }

    pub struct EvalResult { pub violations: [Violation; MAX_VIOLATIONS_PER_CYCLE], pub violation_count: u32 }

    pub struct WatchpointTable { entries: [Watchpoint; MAX_WATCHPOINTS], entry_count: u32 }

    pub fn compare(value: i64, op: ComparisonOp, threshold: i64) -> bool {
        match op {
            ComparisonOp::LessThan => value < threshold,
            ComparisonOp::GreaterThan => value > threshold,
            ComparisonOp::LessOrEqual => value <= threshold,
            ComparisonOp::GreaterOrEqual => value >= threshold,
            ComparisonOp::Equal => value == threshold,
            ComparisonOp::NotEqual => value != threshold,
        }
    }

    impl Watchpoint { pub const fn empty() -> Self { Watchpoint { sensor_id: 0, op: ComparisonOp::LessThan, threshold: 0, enabled: false, persistence: 1, current_count: 0 } } }
    impl Violation { pub const fn empty() -> Self { Violation { watchpoint_id: 0, measured: 0, threshold: 0, op: ComparisonOp::LessThan } } }

    impl WatchpointTable {
        pub fn new() -> Self { WatchpointTable { entries: [Watchpoint::empty(); MAX_WATCHPOINTS], entry_count: 0 } }

        pub fn add_watchpoint(&mut self, wp: Watchpoint) -> bool {
            if self.entry_count as usize >= MAX_WATCHPOINTS { return false; }
            self.entries[self.entry_count as usize] = wp;
            self.entry_count = self.entry_count + 1;
            true
        }

        pub fn evaluate(&mut self, reading: SensorReading) -> EvalResult {
            let mut result = EvalResult { violations: [Violation::empty(); MAX_VIOLATIONS_PER_CYCLE], violation_count: 0 };
            let count = self.entry_count;
            let mut i: u32 = 0;
            while i < count {
                if result.violation_count as usize >= MAX_VIOLATIONS_PER_CYCLE { break; }
                let idx = i as usize;
                let enabled = self.entries[idx].enabled;
                let sid = self.entries[idx].sensor_id;
                let op = self.entries[idx].op;
                let threshold = self.entries[idx].threshold;
                let persistence = self.entries[idx].persistence;
                if enabled && sid == reading.sensor_id {
                    let violated = compare(reading.value, op, threshold);
                    if violated {
                        self.entries[idx].current_count = if self.entries[idx].current_count < u32::MAX { self.entries[idx].current_count + 1 } else { u32::MAX };
                        if self.entries[idx].current_count >= persistence {
                            let vidx = result.violation_count as usize;
                            result.violations[vidx] = Violation { watchpoint_id: i, measured: reading.value, threshold, op };
                            result.violation_count = result.violation_count + 1;
                        }
                    } else {
                        self.entries[idx].current_count = 0;
                    }
                }
                i = i + 1;
            }
            result
        }
    }

    // ── wohl-temp: domain-specific temperature monitor ─────────

    pub const MAX_ZONES: usize = 32;
    pub const MAX_ALERTS_PER_READING: usize = 4;

    #[derive(Clone, Copy)]
    pub struct ZoneConfig {
        pub zone_id: u32,
        pub freeze_threshold: i32,
        pub overheat_threshold: i32,
        pub rate_threshold: i32,
        pub enabled: bool,
    }

    #[derive(Clone, Copy)]
    pub struct ZoneState {
        pub zone_id: u32,
        pub last_value: i32,
        pub last_time: u64,
        pub active: bool,
    }

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum TempAlertType { Freeze, Overheat, RapidDrop, RapidRise }

    #[derive(Clone, Copy)]
    pub struct TempAlert {
        pub zone_id: u32,
        pub alert_type: TempAlertType,
        pub value: i32,
        pub threshold: i32,
        pub time: u64,
    }

    pub struct TempResult {
        pub alerts: [TempAlert; MAX_ALERTS_PER_READING],
        pub alert_count: u32,
    }

    pub struct TemperatureMonitor {
        watchpoints: WatchpointTable,
        configs: [ZoneConfig; MAX_ZONES],
        states: [ZoneState; MAX_ZONES],
        zone_count: u32,
    }

    impl ZoneConfig {
        pub const fn empty() -> Self {
            ZoneConfig { zone_id: 0, freeze_threshold: 0, overheat_threshold: 0, rate_threshold: 0, enabled: false }
        }
    }

    impl ZoneState {
        pub const fn empty() -> Self {
            ZoneState { zone_id: 0, last_value: 0, last_time: 0, active: false }
        }
    }

    impl TempAlert {
        pub const fn empty() -> Self {
            TempAlert { zone_id: 0, alert_type: TempAlertType::Freeze, value: 0, threshold: 0, time: 0 }
        }
    }

    fn freeze_wp_id(zone_id: u32) -> u32 { zone_id * 2 }
    fn overheat_wp_id(zone_id: u32) -> u32 { zone_id * 2 + 1 }

    impl TemperatureMonitor {
        pub fn new() -> Self {
            TemperatureMonitor {
                watchpoints: WatchpointTable::new(),
                configs: [ZoneConfig::empty(); MAX_ZONES],
                states: [ZoneState::empty(); MAX_ZONES],
                zone_count: 0,
            }
        }

        pub fn register_zone(&mut self, config: ZoneConfig) -> bool {
            if self.zone_count as usize >= MAX_ZONES { return false; }

            let idx = self.zone_count as usize;
            self.configs[idx] = config;
            self.states[idx] = ZoneState {
                zone_id: config.zone_id, last_value: 0, last_time: 0, active: true,
            };

            self.watchpoints.add_watchpoint(Watchpoint {
                sensor_id: freeze_wp_id(config.zone_id),
                op: ComparisonOp::LessOrEqual,
                threshold: config.freeze_threshold as i64,
                enabled: config.enabled,
                persistence: 1,
                current_count: 0,
            });
            self.watchpoints.add_watchpoint(Watchpoint {
                sensor_id: overheat_wp_id(config.zone_id),
                op: ComparisonOp::GreaterOrEqual,
                threshold: config.overheat_threshold as i64,
                enabled: config.enabled,
                persistence: 1,
                current_count: 0,
            });

            self.zone_count = self.zone_count + 1;
            true
        }

        pub fn process_reading(&mut self, zone_id: u32, value: i32, time: u64) -> TempResult {
            let mut res = TempResult {
                alerts: [TempAlert::empty(); MAX_ALERTS_PER_READING],
                alert_count: 0,
            };

            // Phase 1: relay-lc threshold evaluation (VERIFIED)
            let freeze_result = self.watchpoints.evaluate(SensorReading {
                sensor_id: freeze_wp_id(zone_id),
                value: value as i64,
            });
            if freeze_result.violation_count > 0 && (res.alert_count as usize) < MAX_ALERTS_PER_READING {
                res.alerts[res.alert_count as usize] = TempAlert {
                    zone_id, alert_type: TempAlertType::Freeze, value,
                    threshold: freeze_result.violations[0].threshold as i32, time,
                };
                res.alert_count += 1;
            }

            let overheat_result = self.watchpoints.evaluate(SensorReading {
                sensor_id: overheat_wp_id(zone_id),
                value: value as i64,
            });
            if overheat_result.violation_count > 0 && (res.alert_count as usize) < MAX_ALERTS_PER_READING {
                res.alerts[res.alert_count as usize] = TempAlert {
                    zone_id, alert_type: TempAlertType::Overheat, value,
                    threshold: overheat_result.violations[0].threshold as i32, time,
                };
                res.alert_count += 1;
            }

            // Phase 2: rate-of-change detection (domain-specific)
            let count = self.zone_count;
            let mut i: u32 = 0;
            while i < count {
                let idx = i as usize;
                if self.states[idx].active && self.configs[idx].zone_id == zone_id && self.configs[idx].enabled {
                    if self.states[idx].last_time > 0 {
                        let last = self.states[idx].last_value;
                        let rate_thr = self.configs[idx].rate_threshold;

                        if last - value > rate_thr && (res.alert_count as usize) < MAX_ALERTS_PER_READING {
                            res.alerts[res.alert_count as usize] = TempAlert {
                                zone_id, alert_type: TempAlertType::RapidDrop, value, threshold: rate_thr, time,
                            };
                            res.alert_count += 1;
                        }

                        if value - last > rate_thr && (res.alert_count as usize) < MAX_ALERTS_PER_READING {
                            res.alerts[res.alert_count as usize] = TempAlert {
                                zone_id, alert_type: TempAlertType::RapidRise, value, threshold: rate_thr, time,
                            };
                            res.alert_count += 1;
                        }
                    }

                    self.states[idx].last_value = value;
                    self.states[idx].last_time = time;
                    break;
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

use wohl_temp_bindings::exports::pulseengine::wohl_temperature::temperature::{
    Guest, TempAlertType as WitAlertType, TempAlert as WitAlert,
};

struct Component;

static mut TABLE: Option<engine::TemperatureMonitor> = None;

fn get_table() -> &'static mut engine::TemperatureMonitor {
    unsafe {
        if TABLE.is_none() {
            TABLE = Some(engine::TemperatureMonitor::new());
        }
        TABLE.as_mut().unwrap()
    }
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
        unsafe { TABLE = Some(engine::TemperatureMonitor::new()); }
        Ok(())
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn init() -> Result<(), String> {
        unsafe { TABLE = Some(engine::TemperatureMonitor::new()); }
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
        get_table().register_zone(engine::ZoneConfig {
            zone_id,
            freeze_threshold,
            overheat_threshold,
            rate_threshold,
            enabled: true,
        })
    }

    fn do_process_reading(zone_id: u32, value: i32, time: u64) -> Vec<WitAlert> {
        let result = get_table().process_reading(zone_id, value, time);
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
