//! Wohl Temperature Monitor — plain Rust (generated from Verus source).
//! Source of truth: ../src/core.rs. Do not edit manually.

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

#[derive(Clone, Copy)]
pub struct TempResult {
    pub alerts: [TempAlert; MAX_ALERTS_PER_READING],
    pub alert_count: u32,
}

pub struct TemperatureMonitor {
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

impl TempResult {
    pub const fn empty() -> Self {
        TempResult { alerts: [TempAlert::empty(); MAX_ALERTS_PER_READING], alert_count: 0 }
    }
}

impl TemperatureMonitor {
    pub fn new() -> Self {
        TemperatureMonitor {
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
            zone_id: config.zone_id,
            last_value: 0,
            last_time: 0,
            active: true,
        };
        self.zone_count = self.zone_count + 1;
        true
    }

    pub fn process_reading(&mut self, zone_id: u32, value: i32, time: u64) -> TempResult {
        let mut res = TempResult::empty();
        let count = self.zone_count;
        let mut i: u32 = 0;
        while i < count {
            let idx = i as usize;
            if self.states[idx].active
                && self.configs[idx].enabled
                && self.configs[idx].zone_id == zone_id
            {
                // Freeze detection
                if value <= self.configs[idx].freeze_threshold {
                    if (res.alert_count as usize) < MAX_ALERTS_PER_READING {
                        res.alerts[res.alert_count as usize] = TempAlert {
                            zone_id, alert_type: TempAlertType::Freeze, value,
                            threshold: self.configs[idx].freeze_threshold, time,
                        };
                        res.alert_count += 1;
                    }
                }

                // Overheat detection
                if value >= self.configs[idx].overheat_threshold {
                    if (res.alert_count as usize) < MAX_ALERTS_PER_READING {
                        res.alerts[res.alert_count as usize] = TempAlert {
                            zone_id, alert_type: TempAlertType::Overheat, value,
                            threshold: self.configs[idx].overheat_threshold, time,
                        };
                        res.alert_count += 1;
                    }
                }

                // Rate detection (only with prior reading)
                if self.states[idx].last_time > 0 {
                    let last = self.states[idx].last_value;
                    let rate_thr = self.configs[idx].rate_threshold;

                    if last - value > rate_thr {
                        if (res.alert_count as usize) < MAX_ALERTS_PER_READING {
                            res.alerts[res.alert_count as usize] = TempAlert {
                                zone_id, alert_type: TempAlertType::RapidDrop, value,
                                threshold: rate_thr, time,
                            };
                            res.alert_count += 1;
                        }
                    }

                    if value - last > rate_thr {
                        if (res.alert_count as usize) < MAX_ALERTS_PER_READING {
                            res.alerts[res.alert_count as usize] = TempAlert {
                                zone_id, alert_type: TempAlertType::RapidRise, value,
                                threshold: rate_thr, time,
                            };
                            res.alert_count += 1;
                        }
                    }
                }

                // Update state
                self.states[idx].last_value = value;
                self.states[idx].last_time = time;

                return res;
            }
            i += 1;
        }

        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config(zone_id: u32) -> ZoneConfig {
        ZoneConfig {
            zone_id,
            freeze_threshold: 0,
            overheat_threshold: 4000,
            rate_threshold: 500,
            enabled: true,
        }
    }

    #[test]
    fn test_empty_monitor() {
        let mut m = TemperatureMonitor::new();
        let r = m.process_reading(1, 2000, 100);
        assert_eq!(r.alert_count, 0);
    }

    #[test]
    fn test_freeze_detection() {
        let mut m = TemperatureMonitor::new();
        m.register_zone(make_config(1));
        let r = m.process_reading(1, -100, 100);
        assert_eq!(r.alert_count, 1);
        assert_eq!(r.alerts[0].alert_type, TempAlertType::Freeze);
    }

    #[test]
    fn test_overheat_detection() {
        let mut m = TemperatureMonitor::new();
        m.register_zone(make_config(1));
        let r = m.process_reading(1, 4500, 100);
        assert_eq!(r.alert_count, 1);
        assert_eq!(r.alerts[0].alert_type, TempAlertType::Overheat);
    }

    #[test]
    fn test_normal_range_no_alert() {
        let mut m = TemperatureMonitor::new();
        m.register_zone(make_config(1));
        let r = m.process_reading(1, 2150, 100);
        assert_eq!(r.alert_count, 0);
    }

    #[test]
    fn test_rapid_drop() {
        let mut m = TemperatureMonitor::new();
        m.register_zone(make_config(1));
        m.process_reading(1, 2500, 100);
        let r = m.process_reading(1, 1500, 200);
        assert!(r.alert_count >= 1);
        let mut found = false;
        let mut j = 0u32;
        while j < r.alert_count { if r.alerts[j as usize].alert_type == TempAlertType::RapidDrop { found = true; } j += 1; }
        assert!(found);
    }

    #[test]
    fn test_rapid_rise() {
        let mut m = TemperatureMonitor::new();
        m.register_zone(make_config(1));
        m.process_reading(1, 1500, 100);
        let r = m.process_reading(1, 2500, 200);
        assert!(r.alert_count >= 1);
        let mut found = false;
        let mut j = 0u32;
        while j < r.alert_count { if r.alerts[j as usize].alert_type == TempAlertType::RapidRise { found = true; } j += 1; }
        assert!(found);
    }

    #[test]
    fn test_unknown_zone() {
        let mut m = TemperatureMonitor::new();
        m.register_zone(make_config(1));
        let r = m.process_reading(99, 2000, 100);
        assert_eq!(r.alert_count, 0);
    }
}
