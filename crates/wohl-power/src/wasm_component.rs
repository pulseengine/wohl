// Wohl Power Meter — P3 WASM component (self-contained).
//
// This file contains:
//   1. The relay-lc WatchpointTable engine (verified, from relay-lc/plain/src/engine.rs)
//   2. The wohl-power PowerMonitor engine (from plain/src/engine.rs)
//   3. The P3 async Guest trait implementation
//
// Built by: bazel build //:wohl-power (rules_wasm_component, wasi_version="p3")

// ═══════════════════════════════════════════════════════════════
// Verified core engine — includes relay-lc + wohl-power domain logic
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

    // ── wohl-power: domain-specific power monitor ──────────────

    pub const MAX_CIRCUITS: usize = 16;
    pub const MAX_ALERTS_PER_READING: usize = 4;

    #[derive(Clone, Copy)]
    pub struct CircuitConfig { pub circuit_id: u32, pub max_watts: u32, pub idle_watts: u32, pub spike_threshold: u32, pub enabled: bool }

    #[derive(Clone, Copy)]
    pub struct CircuitState { pub circuit_id: u32, pub last_watts: u32, pub last_time: u64, pub active: bool }

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum PowerAlertType { OverConsumption, Spike, DeviceLeftOn }

    #[derive(Clone, Copy)]
    pub struct PowerAlert { pub circuit_id: u32, pub alert_type: PowerAlertType, pub value: u32, pub threshold: u32, pub time: u64 }

    pub struct PowerResult { pub alerts: [PowerAlert; MAX_ALERTS_PER_READING], pub alert_count: u32 }

    pub struct PowerMonitor {
        watchpoints: WatchpointTable,
        configs: [CircuitConfig; MAX_CIRCUITS],
        states: [CircuitState; MAX_CIRCUITS],
        circuit_count: u32,
    }

    impl CircuitConfig { pub const fn empty() -> Self { CircuitConfig { circuit_id: 0, max_watts: 0, idle_watts: 0, spike_threshold: 0, enabled: false } } }
    impl CircuitState { pub const fn empty() -> Self { CircuitState { circuit_id: 0, last_watts: 0, last_time: 0, active: false } } }
    impl PowerAlert { pub const fn empty() -> Self { PowerAlert { circuit_id: 0, alert_type: PowerAlertType::OverConsumption, value: 0, threshold: 0, time: 0 } } }

    impl PowerMonitor {
        pub fn new() -> Self {
            PowerMonitor {
                watchpoints: WatchpointTable::new(),
                configs: [CircuitConfig::empty(); MAX_CIRCUITS],
                states: [CircuitState::empty(); MAX_CIRCUITS],
                circuit_count: 0,
            }
        }

        pub fn register_circuit(&mut self, config: CircuitConfig) -> bool {
            if self.circuit_count as usize >= MAX_CIRCUITS { return false; }
            let idx = self.circuit_count as usize;
            self.configs[idx] = config;
            self.states[idx] = CircuitState { circuit_id: config.circuit_id, last_watts: 0, last_time: 0, active: true };

            self.watchpoints.add_watchpoint(Watchpoint {
                sensor_id: config.circuit_id,
                op: ComparisonOp::GreaterThan,
                threshold: config.max_watts as i64,
                enabled: config.enabled,
                persistence: 1,
                current_count: 0,
            });

            self.circuit_count += 1;
            true
        }

        pub fn process_reading(&mut self, circuit_id: u32, watts: u32, time: u64) -> PowerResult {
            let mut res = PowerResult { alerts: [PowerAlert::empty(); MAX_ALERTS_PER_READING], alert_count: 0 };

            // Phase 1: overconsumption via relay-lc (VERIFIED)
            let oc_result = self.watchpoints.evaluate(SensorReading { sensor_id: circuit_id, value: watts as i64 });
            if oc_result.violation_count > 0 && (res.alert_count as usize) < MAX_ALERTS_PER_READING {
                res.alerts[res.alert_count as usize] = PowerAlert {
                    circuit_id, alert_type: PowerAlertType::OverConsumption,
                    value: watts, threshold: oc_result.violations[0].threshold as u32, time,
                };
                res.alert_count += 1;
            }

            // Phase 2: spike detection (domain-specific rate-of-change)
            let count = self.circuit_count;
            let mut i: u32 = 0;
            while i < count {
                let idx = i as usize;
                if self.states[idx].active && self.configs[idx].circuit_id == circuit_id && self.configs[idx].enabled {
                    if self.states[idx].last_time > 0 {
                        let last = self.states[idx].last_watts;
                        let diff = if watts > last { watts - last } else { last - watts };
                        if diff > self.configs[idx].spike_threshold && (res.alert_count as usize) < MAX_ALERTS_PER_READING {
                            res.alerts[res.alert_count as usize] = PowerAlert {
                                circuit_id, alert_type: PowerAlertType::Spike,
                                value: watts, threshold: self.configs[idx].spike_threshold, time,
                            };
                            res.alert_count += 1;
                        }
                    }
                    self.states[idx].last_watts = watts;
                    self.states[idx].last_time = time;
                    break;
                }
                i += 1;
            }

            res
        }

        pub fn check_idle(&self, circuit_id: u32, current_watts: u32) -> bool {
            let mut i: u32 = 0;
            while i < self.circuit_count {
                let idx = i as usize;
                if self.configs[idx].circuit_id == circuit_id { return current_watts <= self.configs[idx].idle_watts; }
                i += 1;
            }
            false
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// P3 WASM component binding — delegates to verified engine
// ═══════════════════════════════════════════════════════════════

use wohl_power_bindings::exports::pulseengine::wohl_power_meter::power::{
    Guest, PowerAlertType as WitAlertType, PowerAlert as WitAlert,
};

struct Component;

static mut TABLE: Option<engine::PowerMonitor> = None;

fn get_table() -> &'static mut engine::PowerMonitor {
    unsafe {
        if TABLE.is_none() {
            TABLE = Some(engine::PowerMonitor::new());
        }
        TABLE.as_mut().unwrap()
    }
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
        unsafe { TABLE = Some(engine::PowerMonitor::new()); }
        Ok(())
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn init() -> Result<(), String> {
        unsafe { TABLE = Some(engine::PowerMonitor::new()); }
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
        get_table().register_circuit(engine::CircuitConfig {
            circuit_id,
            max_watts,
            idle_watts,
            spike_threshold,
            enabled: true,
        })
    }

    fn do_process_reading(circuit_id: u32, watts: u32, time: u64) -> Vec<WitAlert> {
        let result = get_table().process_reading(circuit_id, watts, time);
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
        get_table().check_idle(circuit_id, current_watts)
    }
}

wohl_power_bindings::export!(Component with_types_in wohl_power_bindings);
