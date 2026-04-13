//! Wohl Air Quality Monitor — uses Relay Limit Checker for threshold evaluation.
//!
//! Architecture:
//!   - relay-lc::WatchpointTable handles CO2/PM2.5/VOC thresholds (VERIFIED)
//!   - This module adds: domain type translation, multi-metric mapping

use relay_lc::engine::{
    ComparisonOp, SensorReading, Watchpoint, WatchpointTable,
};

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

// Watchpoint ID encoding: zone_id * 6 + metric_offset
// 0=co2_warn, 1=co2_critical, 2=pm25_warn, 3=pm25_critical, 4=voc_warn, 5=voc_critical
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

    /// Register a zone. Creates 6 relay-lc watchpoints (warn+critical for each metric).
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

    /// Process an air quality reading. All threshold checks go through relay-lc.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config(zone_id: u32) -> AirConfig {
        AirConfig { zone_id, co2_warn: 1000, co2_critical: 2000, pm25_warn: 250, pm25_critical: 500, voc_warn: 200, voc_critical: 400, enabled: true }
    }

    #[test]
    fn test_empty() { let mut m = AirMonitor::new(); let r = m.process_reading(AirReading { zone_id: 1, co2_ppm: 400, pm25: 50, voc_index: 30, time: 100 }); assert_eq!(r.alert_count, 0); }

    #[test]
    fn test_co2_warning() {
        let mut m = AirMonitor::new(); m.register_zone(make_config(1));
        let r = m.process_reading(AirReading { zone_id: 1, co2_ppm: 1200, pm25: 50, voc_index: 30, time: 100 });
        assert!(r.alert_count >= 1);
        assert_eq!(r.alerts[0].alert_type, AirAlertType::Co2Warning);
    }

    #[test]
    fn test_co2_critical() {
        let mut m = AirMonitor::new(); m.register_zone(make_config(1));
        let r = m.process_reading(AirReading { zone_id: 1, co2_ppm: 2500, pm25: 50, voc_index: 30, time: 100 });
        // Should trigger both co2_warn AND co2_critical
        assert!(r.alert_count >= 2);
    }

    #[test]
    fn test_pm25_alert() {
        let mut m = AirMonitor::new(); m.register_zone(make_config(1));
        let r = m.process_reading(AirReading { zone_id: 1, co2_ppm: 400, pm25: 300, voc_index: 30, time: 100 });
        let mut found = false;
        for j in 0..r.alert_count as usize { if r.alerts[j].alert_type == AirAlertType::Pm25Warning { found = true; } }
        assert!(found);
    }

    #[test]
    fn test_voc_alert() {
        let mut m = AirMonitor::new(); m.register_zone(make_config(1));
        let r = m.process_reading(AirReading { zone_id: 1, co2_ppm: 400, pm25: 50, voc_index: 250, time: 100 });
        let mut found = false;
        for j in 0..r.alert_count as usize { if r.alerts[j].alert_type == AirAlertType::VocWarning { found = true; } }
        assert!(found);
    }

    #[test]
    fn test_normal_no_alert() {
        let mut m = AirMonitor::new(); m.register_zone(make_config(1));
        let r = m.process_reading(AirReading { zone_id: 1, co2_ppm: 400, pm25: 50, voc_index: 30, time: 100 });
        assert_eq!(r.alert_count, 0);
    }
}

// ── Kani bounded model checking harnesses ────────────────────

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// AIR-P03: alert_count never exceeds MAX_ALERTS_PER_READING
    #[kani::proof]
    fn verify_alert_count_bounded() {
        let mut m = AirMonitor::new();
        let config = AirConfig {
            zone_id: 1,
            co2_warn: kani::any(),
            co2_critical: kani::any(),
            pm25_warn: kani::any(),
            pm25_critical: kani::any(),
            voc_warn: kani::any(),
            voc_critical: kani::any(),
            enabled: true,
        };
        m.register_zone(config);
        let reading = AirReading {
            zone_id: 1,
            co2_ppm: kani::any(),
            pm25: kani::any(),
            voc_index: kani::any(),
            time: kani::any(),
        };
        let r = m.process_reading(reading);
        assert!(r.alert_count as usize <= MAX_ALERTS_PER_READING);
    }

    /// AIR-P04: values below all thresholds produce 0 alerts
    #[kani::proof]
    fn verify_normal_no_alerts() {
        let mut m = AirMonitor::new();
        let co2_warn: u32 = kani::any();
        let pm25_warn: u32 = kani::any();
        let voc_warn: u32 = kani::any();
        kani::assume(co2_warn > 1 && pm25_warn > 1 && voc_warn > 1);
        let config = AirConfig {
            zone_id: 1,
            co2_warn,
            co2_critical: u32::MAX,
            pm25_warn,
            pm25_critical: u32::MAX,
            voc_warn,
            voc_critical: u32::MAX,
            enabled: true,
        };
        m.register_zone(config);
        let co2: u32 = kani::any();
        let pm25: u32 = kani::any();
        let voc: u32 = kani::any();
        // All values strictly below their warn thresholds
        kani::assume(co2 < co2_warn && pm25 < pm25_warn && voc < voc_warn);
        let r = m.process_reading(AirReading {
            zone_id: 1, co2_ppm: co2, pm25, voc_index: voc, time: 100,
        });
        assert_eq!(r.alert_count, 0);
    }

    /// No panics for any combination of symbolic inputs
    #[kani::proof]
    fn verify_no_panic() {
        let mut m = AirMonitor::new();
        let zone_id: u32 = kani::any();
        kani::assume(zone_id < 100);
        let config = AirConfig {
            zone_id,
            co2_warn: kani::any(),
            co2_critical: kani::any(),
            pm25_warn: kani::any(),
            pm25_critical: kani::any(),
            voc_warn: kani::any(),
            voc_critical: kani::any(),
            enabled: kani::any(),
        };
        m.register_zone(config);
        let reading = AirReading {
            zone_id,
            co2_ppm: kani::any(),
            pm25: kani::any(),
            voc_index: kani::any(),
            time: kani::any(),
        };
        let _ = m.process_reading(reading);
    }
}
