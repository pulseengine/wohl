//! Wohl Power Meter — plain Rust (generated from Verus source).
//! Source of truth: ../src/core.rs. Do not edit manually.

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

pub struct PowerMonitor { configs: [CircuitConfig; MAX_CIRCUITS], states: [CircuitState; MAX_CIRCUITS], circuit_count: u32 }

impl CircuitConfig {
    pub const fn empty() -> Self { CircuitConfig { circuit_id: 0, max_watts: 0, idle_watts: 0, spike_threshold: 0, enabled: false } }
}

impl CircuitState {
    pub const fn empty() -> Self { CircuitState { circuit_id: 0, last_watts: 0, last_time: 0, active: false } }
}

impl PowerAlert {
    pub const fn empty() -> Self { PowerAlert { circuit_id: 0, alert_type: PowerAlertType::OverConsumption, value: 0, threshold: 0, time: 0 } }
}

impl PowerResult {
    pub const fn empty() -> Self { PowerResult { alerts: [PowerAlert::empty(); MAX_ALERTS_PER_READING], alert_count: 0 } }
}

impl PowerMonitor {
    pub fn new() -> Self {
        PowerMonitor {
            configs: [CircuitConfig::empty(); MAX_CIRCUITS],
            states: [CircuitState::empty(); MAX_CIRCUITS],
            circuit_count: 0,
        }
    }

    pub fn register_circuit(&mut self, config: CircuitConfig) -> bool {
        if self.circuit_count as usize >= MAX_CIRCUITS { return false; }
        let idx = self.circuit_count as usize;
        self.configs[idx] = config;
        self.states[idx] = CircuitState {
            circuit_id: config.circuit_id,
            last_watts: 0,
            last_time: 0,
            active: true,
        };
        self.circuit_count = self.circuit_count + 1;
        true
    }

    pub fn process_reading(&mut self, circuit_id: u32, watts: u32, time: u64) -> PowerResult {
        let mut result = PowerResult::empty();
        let count = self.circuit_count;
        let mut i: u32 = 0;
        while i < count {
            let idx = i as usize;
            if self.states[idx].active && self.configs[idx].circuit_id == circuit_id && self.configs[idx].enabled {
                // Over-consumption check
                if watts > self.configs[idx].max_watts {
                    if (result.alert_count as usize) < MAX_ALERTS_PER_READING {
                        let aidx = result.alert_count as usize;
                        result.alerts[aidx] = PowerAlert {
                            circuit_id,
                            alert_type: PowerAlertType::OverConsumption,
                            value: watts,
                            threshold: self.configs[idx].max_watts,
                            time,
                        };
                        result.alert_count = result.alert_count + 1;
                    }
                }

                // Spike check
                let last = self.states[idx].last_watts;
                let diff = if watts >= last { watts - last } else { last - watts };
                if diff > self.configs[idx].spike_threshold && self.states[idx].last_time > 0 {
                    if (result.alert_count as usize) < MAX_ALERTS_PER_READING {
                        let aidx = result.alert_count as usize;
                        result.alerts[aidx] = PowerAlert {
                            circuit_id,
                            alert_type: PowerAlertType::Spike,
                            value: diff,
                            threshold: self.configs[idx].spike_threshold,
                            time,
                        };
                        result.alert_count = result.alert_count + 1;
                    }
                }

                // Update state
                self.states[idx].last_watts = watts;
                self.states[idx].last_time = time;

                return result;
            }
            i = i + 1;
        }

        result
    }

    pub fn check_idle(&self, circuit_id: u32, current_watts: u32) -> bool {
        let count = self.circuit_count;
        let mut i: u32 = 0;
        while i < count {
            let idx = i as usize;
            if self.configs[idx].enabled && self.configs[idx].circuit_id == circuit_id {
                return current_watts <= self.configs[idx].idle_watts;
            }
            i = i + 1;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config(id: u32, max_w: u32, idle_w: u32, spike: u32) -> CircuitConfig {
        CircuitConfig { circuit_id: id, max_watts: max_w, idle_watts: idle_w, spike_threshold: spike, enabled: true }
    }

    #[test] fn test_empty_monitor() { let mut mon = PowerMonitor::new(); let r = mon.process_reading(1, 100, 1000); assert_eq!(r.alert_count, 0); }
    #[test] fn test_over_consumption() { let mut mon = PowerMonitor::new(); mon.register_circuit(make_config(1, 1000, 50, 500)); let r = mon.process_reading(1, 1500, 1000); assert_eq!(r.alert_count, 1); assert_eq!(r.alerts[0].alert_type, PowerAlertType::OverConsumption); }
    #[test] fn test_spike_detection() { let mut mon = PowerMonitor::new(); mon.register_circuit(make_config(1, 5000, 50, 200)); mon.process_reading(1, 100, 1000); let r = mon.process_reading(1, 500, 1001); let mut found = false; let mut j: u32 = 0; while j < r.alert_count { if r.alerts[j as usize].alert_type == PowerAlertType::Spike { found = true; } j = j + 1; } assert!(found); }
    #[test] fn test_normal_no_alert() { let mut mon = PowerMonitor::new(); mon.register_circuit(make_config(1, 1000, 50, 500)); let r = mon.process_reading(1, 500, 1000); assert_eq!(r.alert_count, 0); }
    #[test] fn test_unknown_circuit() { let mut mon = PowerMonitor::new(); mon.register_circuit(make_config(1, 1000, 50, 500)); let r = mon.process_reading(99, 500, 1000); assert_eq!(r.alert_count, 0); }
    #[test] fn test_idle_check() { let mut mon = PowerMonitor::new(); mon.register_circuit(make_config(1, 1000, 50, 500)); assert!(mon.check_idle(1, 50)); assert!(mon.check_idle(1, 30)); assert!(!mon.check_idle(1, 51)); }
}
