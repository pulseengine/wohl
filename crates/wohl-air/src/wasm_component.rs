// Wohl Air Quality Monitor — P3 WASM component (self-contained).
//
// This file contains:
//   1. The relay-lc WatchpointTable engine (verified, from relay-lc/plain/src/engine.rs)
//   2. The wohl-air AirMonitor engine (from plain/src/engine.rs)
//   3. The P3 async Guest trait implementation
//
// Built by: bazel build //:wohl-air (rules_wasm_component, wasi_version="p3")

// ═══════════════════════════════════════════════════════════════
// Verified core engine — includes relay-lc + wohl-air domain logic
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

    // ── wohl-air: domain-specific air quality monitor ──────────

    pub const MAX_ZONES: usize = 16;
    pub const MAX_ALERTS_PER_READING: usize = 6;

    #[derive(Clone, Copy)]
    pub struct AirConfig {
        pub zone_id: u32,
        pub co2_warn: u32,
        pub co2_critical: u32,
        pub pm25_warn: u32,
        pub pm25_critical: u32,
        pub voc_warn: u32,
        pub voc_critical: u32,
        pub enabled: bool,
    }

    #[derive(Clone, Copy)]
    pub struct AirReading {
        pub zone_id: u32,
        pub co2_ppm: u32,
        pub pm25: u32,
        pub voc_index: u32,
        pub time: u64,
    }

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum AirAlertType { Co2Warning, Co2Critical, Pm25Warning, Pm25Critical, VocWarning, VocCritical }

    #[derive(Clone, Copy)]
    pub struct AirAlert {
        pub zone_id: u32,
        pub alert_type: AirAlertType,
        pub value: u32,
        pub threshold: u32,
        pub time: u64,
    }

    pub struct AirResult {
        pub alerts: [AirAlert; MAX_ALERTS_PER_READING],
        pub alert_count: u32,
    }

    impl AirAlert {
        pub const fn empty() -> Self {
            AirAlert { zone_id: 0, alert_type: AirAlertType::Co2Warning, value: 0, threshold: 0, time: 0 }
        }
    }

    const ALERT_TYPES: [AirAlertType; 6] = [
        AirAlertType::Co2Warning, AirAlertType::Co2Critical,
        AirAlertType::Pm25Warning, AirAlertType::Pm25Critical,
        AirAlertType::VocWarning, AirAlertType::VocCritical,
    ];

    pub struct AirMonitor {
        watchpoints: WatchpointTable,
        zone_count: u32,
    }

    impl AirConfig {
        pub const fn empty() -> Self {
            AirConfig { zone_id: 0, co2_warn: 0, co2_critical: 0, pm25_warn: 0, pm25_critical: 0, voc_warn: 0, voc_critical: 0, enabled: false }
        }
    }

    impl AirMonitor {
        pub fn new() -> Self {
            AirMonitor { watchpoints: WatchpointTable::new(), zone_count: 0 }
        }

        pub fn register_zone(&mut self, config: AirConfig) -> bool {
            if self.zone_count as usize >= MAX_ZONES { return false; }

            let base = config.zone_id * 6;
            let thresholds = [
                config.co2_warn, config.co2_critical,
                config.pm25_warn, config.pm25_critical,
                config.voc_warn, config.voc_critical,
            ];

            for i in 0..6 {
                self.watchpoints.add_watchpoint(Watchpoint {
                    sensor_id: base + i as u32,
                    op: ComparisonOp::GreaterOrEqual,
                    threshold: thresholds[i] as i64,
                    enabled: config.enabled,
                    persistence: 1,
                    current_count: 0,
                });
            }

            self.zone_count += 1;
            true
        }

        pub fn process_reading(&mut self, reading: AirReading) -> AirResult {
            let mut res = AirResult { alerts: [AirAlert::empty(); MAX_ALERTS_PER_READING], alert_count: 0 };
            let base = reading.zone_id * 6;
            let values = [reading.co2_ppm, reading.co2_ppm, reading.pm25, reading.pm25, reading.voc_index, reading.voc_index];

            for i in 0..6 {
                if res.alert_count as usize >= MAX_ALERTS_PER_READING { break; }
                let result = self.watchpoints.evaluate(SensorReading {
                    sensor_id: base + i as u32,
                    value: values[i] as i64,
                });
                if result.violation_count > 0 {
                    res.alerts[res.alert_count as usize] = AirAlert {
                        zone_id: reading.zone_id,
                        alert_type: ALERT_TYPES[i],
                        value: values[i],
                        threshold: result.violations[0].threshold as u32,
                        time: reading.time,
                    };
                    res.alert_count += 1;
                }
            }

            res
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// P3 WASM component binding — delegates to verified engine
// ═══════════════════════════════════════════════════════════════

use wohl_air_bindings::exports::pulseengine::wohl_air_quality::air_quality::{
    Guest, AirAlertType as WitAlertType, AirAlert as WitAlert,
    AirConfig as WitConfig, AirReading as WitReading,
};

struct Component;

static mut TABLE: Option<engine::AirMonitor> = None;

fn get_table() -> &'static mut engine::AirMonitor {
    unsafe {
        if TABLE.is_none() {
            TABLE = Some(engine::AirMonitor::new());
        }
        TABLE.as_mut().unwrap()
    }
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
        unsafe { TABLE = Some(engine::AirMonitor::new()); }
        Ok(())
    }
    #[cfg(not(target_arch = "wasm32"))]
    fn init() -> Result<(), String> {
        unsafe { TABLE = Some(engine::AirMonitor::new()); }
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
        get_table().register_zone(engine::AirConfig {
            zone_id: config.zone_id,
            co2_warn: config.co2_warn,
            co2_critical: config.co2_critical,
            pm25_warn: config.pm25_warn,
            pm25_critical: config.pm25_critical,
            voc_warn: config.voc_warn,
            voc_critical: config.voc_critical,
            enabled: config.enabled,
        })
    }

    fn do_process_reading(reading: WitReading) -> Vec<WitAlert> {
        let result = get_table().process_reading(engine::AirReading {
            zone_id: reading.zone_id,
            co2_ppm: reading.co2_ppm,
            pm25: reading.pm25,
            voc_index: reading.voc_index,
            time: reading.time,
        });
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
