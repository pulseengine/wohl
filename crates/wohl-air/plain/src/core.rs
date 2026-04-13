//! Wohl Air Quality Monitor — plain Rust (generated from Verus source).
//! Source of truth: ../src/core.rs. Do not edit manually.

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

#[derive(Clone, Copy)]
pub struct AirResult {
    pub alerts: [AirAlert; MAX_ALERTS_PER_READING],
    pub alert_count: u32,
}

pub struct AirMonitor {
    configs: [AirConfig; MAX_ZONES],
    zone_count: u32,
}

impl AirConfig {
    pub const fn empty() -> Self {
        AirConfig { zone_id: 0, co2_warn: 0, co2_critical: 0, pm25_warn: 0, pm25_critical: 0, voc_warn: 0, voc_critical: 0, enabled: false }
    }
}

impl AirAlert {
    pub const fn empty() -> Self {
        AirAlert { zone_id: 0, alert_type: AirAlertType::Co2Warning, value: 0, threshold: 0, time: 0 }
    }
}

impl AirResult {
    pub const fn empty() -> Self {
        AirResult { alerts: [AirAlert::empty(); MAX_ALERTS_PER_READING], alert_count: 0 }
    }
}

impl AirMonitor {
    pub fn new() -> Self {
        AirMonitor { configs: [AirConfig::empty(); MAX_ZONES], zone_count: 0 }
    }

    pub fn register_zone(&mut self, config: AirConfig) -> bool {
        if self.zone_count as usize >= MAX_ZONES { return false; }
        let idx = self.zone_count as usize;
        self.configs[idx] = config;
        self.zone_count += 1;
        true
    }

    pub fn process_reading(&mut self, reading: AirReading) -> AirResult {
        let mut res = AirResult::empty();
        let count = self.zone_count;
        let mut i: u32 = 0;
        while i < count {
            let idx = i as usize;
            if self.configs[idx].enabled && self.configs[idx].zone_id == reading.zone_id {

                // CO2 checks
                if reading.co2_ppm >= self.configs[idx].co2_critical {
                    if (res.alert_count as usize) < MAX_ALERTS_PER_READING {
                        res.alerts[res.alert_count as usize] = AirAlert {
                            zone_id: reading.zone_id, alert_type: AirAlertType::Co2Critical,
                            value: reading.co2_ppm, threshold: self.configs[idx].co2_critical, time: reading.time,
                        };
                        res.alert_count += 1;
                    }
                } else if reading.co2_ppm >= self.configs[idx].co2_warn {
                    if (res.alert_count as usize) < MAX_ALERTS_PER_READING {
                        res.alerts[res.alert_count as usize] = AirAlert {
                            zone_id: reading.zone_id, alert_type: AirAlertType::Co2Warning,
                            value: reading.co2_ppm, threshold: self.configs[idx].co2_warn, time: reading.time,
                        };
                        res.alert_count += 1;
                    }
                }

                // PM2.5 checks
                if reading.pm25 >= self.configs[idx].pm25_critical {
                    if (res.alert_count as usize) < MAX_ALERTS_PER_READING {
                        res.alerts[res.alert_count as usize] = AirAlert {
                            zone_id: reading.zone_id, alert_type: AirAlertType::Pm25Critical,
                            value: reading.pm25, threshold: self.configs[idx].pm25_critical, time: reading.time,
                        };
                        res.alert_count += 1;
                    }
                } else if reading.pm25 >= self.configs[idx].pm25_warn {
                    if (res.alert_count as usize) < MAX_ALERTS_PER_READING {
                        res.alerts[res.alert_count as usize] = AirAlert {
                            zone_id: reading.zone_id, alert_type: AirAlertType::Pm25Warning,
                            value: reading.pm25, threshold: self.configs[idx].pm25_warn, time: reading.time,
                        };
                        res.alert_count += 1;
                    }
                }

                // VOC checks
                if reading.voc_index >= self.configs[idx].voc_critical {
                    if (res.alert_count as usize) < MAX_ALERTS_PER_READING {
                        res.alerts[res.alert_count as usize] = AirAlert {
                            zone_id: reading.zone_id, alert_type: AirAlertType::VocCritical,
                            value: reading.voc_index, threshold: self.configs[idx].voc_critical, time: reading.time,
                        };
                        res.alert_count += 1;
                    }
                } else if reading.voc_index >= self.configs[idx].voc_warn {
                    if (res.alert_count as usize) < MAX_ALERTS_PER_READING {
                        res.alerts[res.alert_count as usize] = AirAlert {
                            zone_id: reading.zone_id, alert_type: AirAlertType::VocWarning,
                            value: reading.voc_index, threshold: self.configs[idx].voc_warn, time: reading.time,
                        };
                        res.alert_count += 1;
                    }
                }

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

    fn make_config(zone_id: u32) -> AirConfig {
        AirConfig {
            zone_id, co2_warn: 1000, co2_critical: 2000,
            pm25_warn: 350, pm25_critical: 750,
            voc_warn: 200, voc_critical: 400, enabled: true,
        }
    }

    fn make_reading(zone_id: u32, co2: u32, pm25: u32, voc: u32) -> AirReading {
        AirReading { zone_id, co2_ppm: co2, pm25, voc_index: voc, time: 100 }
    }

    #[test] fn test_empty() { let mut m = AirMonitor::new(); let r = m.process_reading(make_reading(1, 500, 100, 50)); assert_eq!(r.alert_count, 0); }
    #[test] fn test_co2_warning() { let mut m = AirMonitor::new(); m.register_zone(make_config(1)); let r = m.process_reading(make_reading(1, 1200, 100, 50)); assert_eq!(r.alert_count, 1); assert_eq!(r.alerts[0].alert_type, AirAlertType::Co2Warning); }
    #[test] fn test_co2_critical() { let mut m = AirMonitor::new(); m.register_zone(make_config(1)); let r = m.process_reading(make_reading(1, 2500, 100, 50)); assert_eq!(r.alert_count, 1); assert_eq!(r.alerts[0].alert_type, AirAlertType::Co2Critical); }
    #[test] fn test_pm25_alert() { let mut m = AirMonitor::new(); m.register_zone(make_config(1)); let r = m.process_reading(make_reading(1, 400, 800, 50)); assert_eq!(r.alert_count, 1); assert_eq!(r.alerts[0].alert_type, AirAlertType::Pm25Critical); }
    #[test] fn test_voc_alert() { let mut m = AirMonitor::new(); m.register_zone(make_config(1)); let r = m.process_reading(make_reading(1, 400, 100, 250)); assert_eq!(r.alert_count, 1); assert_eq!(r.alerts[0].alert_type, AirAlertType::VocWarning); }
    #[test] fn test_normal() { let mut m = AirMonitor::new(); m.register_zone(make_config(1)); let r = m.process_reading(make_reading(1, 400, 100, 50)); assert_eq!(r.alert_count, 0); }
}
