//! Wohl Temperature Monitor — uses Relay Limit Checker for threshold evaluation.
//!
//! Architecture:
//!   - relay-lc::WatchpointTable handles freeze/overheat thresholds (VERIFIED)
//!   - This module adds: rate-of-change detection, domain type translation
//!
//! Wohl provides CONFIGURATION + DOMAIN GLUE.
//! Relay provides VERIFIED THRESHOLD ENGINE.

use relay_lc::engine::{
    ComparisonOp, SensorReading, Watchpoint, WatchpointTable,
};

pub const MAX_ZONES: usize = 32;
pub const MAX_ALERTS_PER_READING: usize = 4;

/// Per-zone configuration — domain-specific thresholds in centidegrees.
#[derive(Clone, Copy)]
pub struct ZoneConfig {
    pub zone_id: u32,
    pub freeze_threshold: i32,    // centidegrees (e.g., 0 = 0.00°C)
    pub overheat_threshold: i32,  // centidegrees (e.g., 4000 = 40.00°C)
    pub rate_threshold: i32,      // max change per reading in centidegrees
    pub enabled: bool,
}

/// Per-zone state for rate-of-change detection (domain-specific, not in relay-lc).
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

/// Temperature Monitor — thin domain wrapper around relay-lc.
pub struct TemperatureMonitor {
    /// Relay's verified watchpoint table handles freeze/overheat thresholds.
    watchpoints: WatchpointTable,
    /// Domain-specific: zone configs for rate detection.
    configs: [ZoneConfig; MAX_ZONES],
    /// Domain-specific: per-zone state for rate-of-change.
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

// Watchpoint ID encoding: zone_id * 2 + offset
// offset 0 = freeze watchpoint (LessOrEqual)
// offset 1 = overheat watchpoint (GreaterOrEqual)
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

    /// Register a zone. Creates relay-lc watchpoints for freeze and overheat.
    pub fn register_zone(&mut self, config: ZoneConfig) -> bool {
        if self.zone_count as usize >= MAX_ZONES { return false; }

        let idx = self.zone_count as usize;
        self.configs[idx] = config;
        self.states[idx] = ZoneState {
            zone_id: config.zone_id, last_value: 0, last_time: 0, active: true,
        };

        // Create relay-lc watchpoints — VERIFIED threshold evaluation
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

    /// Process a temperature reading.
    /// Threshold checks (freeze/overheat) → relay-lc (VERIFIED).
    /// Rate-of-change checks → domain-specific (this module).
    pub fn process_reading(&mut self, zone_id: u32, value: i32, time: u64) -> TempResult {
        let mut res = TempResult {
            alerts: [TempAlert::empty(); MAX_ALERTS_PER_READING],
            alert_count: 0,
        };

        // ── Phase 1: relay-lc threshold evaluation (VERIFIED) ──

        // Check freeze threshold via relay-lc
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

        // Check overheat threshold via relay-lc
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

        // ── Phase 2: rate-of-change detection (domain-specific) ──

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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config(zone_id: u32) -> ZoneConfig {
        ZoneConfig { zone_id, freeze_threshold: 0, overheat_threshold: 4000, rate_threshold: 500, enabled: true }
    }

    #[test]
    fn test_empty_monitor() {
        let mut m = TemperatureMonitor::new();
        let r = m.process_reading(1, 2000, 100);
        assert_eq!(r.alert_count, 0);
    }

    #[test]
    fn test_freeze_via_relay_lc() {
        let mut m = TemperatureMonitor::new();
        m.register_zone(make_config(1));
        let r = m.process_reading(1, -100, 100);
        assert_eq!(r.alert_count, 1);
        assert_eq!(r.alerts[0].alert_type, TempAlertType::Freeze);
    }

    #[test]
    fn test_overheat_via_relay_lc() {
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
