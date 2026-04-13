//! Wohl Power Meter — uses Relay Limit Checker for overconsumption thresholds.
//!
//! Architecture:
//!   - relay-lc::WatchpointTable handles overconsumption threshold (VERIFIED)
//!   - This module adds: spike detection (rate-of-change), domain types

use relay_lc::engine::{
    ComparisonOp, SensorReading, Watchpoint, WatchpointTable,
};

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

        // Overconsumption watchpoint via relay-lc (VERIFIED)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config(id: u32) -> CircuitConfig {
        CircuitConfig { circuit_id: id, max_watts: 30000, idle_watts: 100, spike_threshold: 10000, enabled: true }
    }

    #[test] fn test_empty() { let mut m = PowerMonitor::new(); let r = m.process_reading(1, 1000, 100); assert_eq!(r.alert_count, 0); }

    #[test] fn test_overconsumption_via_relay_lc() {
        let mut m = PowerMonitor::new(); m.register_circuit(make_config(1));
        let r = m.process_reading(1, 35000, 100);
        assert!(r.alert_count >= 1);
        assert_eq!(r.alerts[0].alert_type, PowerAlertType::OverConsumption);
    }

    #[test] fn test_spike() {
        let mut m = PowerMonitor::new(); m.register_circuit(make_config(1));
        m.process_reading(1, 1000, 100);
        let r = m.process_reading(1, 15000, 200);
        let mut found = false;
        for j in 0..r.alert_count as usize { if r.alerts[j].alert_type == PowerAlertType::Spike { found = true; } }
        assert!(found);
    }

    #[test] fn test_normal() { let mut m = PowerMonitor::new(); m.register_circuit(make_config(1)); assert_eq!(m.process_reading(1, 1500, 100).alert_count, 0); }
    #[test] fn test_unknown() { let mut m = PowerMonitor::new(); assert_eq!(m.process_reading(99, 50000, 100).alert_count, 0); }
    #[test] fn test_idle() { let mut m = PowerMonitor::new(); m.register_circuit(make_config(1)); assert!(m.check_idle(1, 50)); assert!(!m.check_idle(1, 200)); }
}
