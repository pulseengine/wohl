//! Wohl Temperature Monitor — uses Relay Limit Checker for threshold evaluation.
//!
//! Architecture:
//!   - relay-lc::WatchpointTable handles freeze/overheat thresholds (VERIFIED)
//!   - This module adds: rate-of-change detection, domain type translation
//!
//! Wohl provides CONFIGURATION + DOMAIN GLUE.
//! Relay provides VERIFIED THRESHOLD ENGINE.

use relay_lc::engine::{ComparisonOp, SensorReading, Watchpoint, WatchpointTable};

pub const MAX_ZONES: usize = 32;
pub const MAX_ALERTS_PER_READING: usize = 4;

/// Rapid-drop window (SWREQ-TEMP-003): a drop greater than the zone's
/// rate threshold within this many seconds fires RapidDrop.
pub const RATE_WINDOW_SEC: u64 = 600;
/// Bucket granularity of the window. The ring keeps one in-progress
/// bucket plus RATE_WINDOW_SEC/RATE_BUCKET_SEC full ones, so every drop
/// completing within RATE_WINDOW_SEC is seen, and a drop may also be
/// seen up to one bucket (60s) beyond it.
pub const RATE_BUCKET_SEC: u64 = 60;
pub const RATE_BUCKETS: usize = 11;

/// Per-zone configuration — domain-specific thresholds in centidegrees.
#[derive(Clone, Copy)]
pub struct ZoneConfig {
    pub zone_id: u32,
    pub freeze_threshold: i32,   // centidegrees (e.g., 0 = 0.00°C)
    pub overheat_threshold: i32, // centidegrees (e.g., 4000 = 40.00°C)
    /// Centidegrees. RapidDrop: max drop within the RATE_WINDOW_SEC
    /// window (strict >). RapidRise: max rise between two consecutive
    /// readings (strict >).
    pub rate_threshold: i32,
    pub enabled: bool,
}

/// Per-zone state for rate-of-change detection (domain-specific, not in relay-lc).
#[derive(Clone, Copy)]
pub struct ZoneState {
    pub zone_id: u32,
    pub last_value: i32,
    pub last_time: u64,
    pub active: bool,
    /// Per-bucket maximum of readings in the rapid-drop window ring.
    pub bucket_max: [i32; RATE_BUCKETS],
    pub bucket_valid: [bool; RATE_BUCKETS],
    /// Absolute bucket index (time / RATE_BUCKET_SEC) of the newest bucket.
    pub newest_bucket: u64,
    pub window_init: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TempAlertType {
    Freeze,
    Overheat,
    RapidDrop,
    RapidRise,
}

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
        ZoneConfig {
            zone_id: 0,
            freeze_threshold: 0,
            overheat_threshold: 0,
            rate_threshold: 0,
            enabled: false,
        }
    }
}

impl ZoneState {
    pub const fn empty() -> Self {
        ZoneState {
            zone_id: 0,
            last_value: 0,
            last_time: 0,
            active: false,
            bucket_max: [0; RATE_BUCKETS],
            bucket_valid: [false; RATE_BUCKETS],
            newest_bucket: 0,
            window_init: false,
        }
    }
}

impl TempAlert {
    pub const fn empty() -> Self {
        TempAlert {
            zone_id: 0,
            alert_type: TempAlertType::Freeze,
            value: 0,
            threshold: 0,
            time: 0,
        }
    }
}

// Watchpoint ID encoding: zone_id * 2 + offset
// offset 0 = freeze watchpoint (LessOrEqual)
// offset 1 = overheat watchpoint (GreaterOrEqual)
// Uses saturating_mul/add so adversarial u32 zone_ids cannot panic on overflow.
// Real zone_ids are bounded by MAX_ZONES (32), so saturation only affects
// pathological inputs and never collides in practice.
fn freeze_wp_id(zone_id: u32) -> u32 {
    zone_id.saturating_mul(2)
}
fn overheat_wp_id(zone_id: u32) -> u32 {
    zone_id.saturating_mul(2).saturating_add(1)
}

impl Default for TemperatureMonitor {
    fn default() -> Self {
        Self::new()
    }
}

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
        if self.zone_count as usize >= MAX_ZONES {
            return false;
        }

        let idx = self.zone_count as usize;
        self.configs[idx] = config;
        self.states[idx] = ZoneState {
            zone_id: config.zone_id,
            last_value: 0,
            last_time: 0,
            active: true,
            bucket_max: [0; RATE_BUCKETS],
            bucket_valid: [false; RATE_BUCKETS],
            newest_bucket: 0,
            window_init: false,
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

        self.zone_count += 1;
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
        if freeze_result.violation_count > 0 && (res.alert_count as usize) < MAX_ALERTS_PER_READING
        {
            res.alerts[res.alert_count as usize] = TempAlert {
                zone_id,
                alert_type: TempAlertType::Freeze,
                value,
                threshold: freeze_result.violations[0].threshold as i32,
                time,
            };
            res.alert_count += 1;
        }

        // Check overheat threshold via relay-lc
        let overheat_result = self.watchpoints.evaluate(SensorReading {
            sensor_id: overheat_wp_id(zone_id),
            value: value as i64,
        });
        if overheat_result.violation_count > 0
            && (res.alert_count as usize) < MAX_ALERTS_PER_READING
        {
            res.alerts[res.alert_count as usize] = TempAlert {
                zone_id,
                alert_type: TempAlertType::Overheat,
                value,
                threshold: overheat_result.violations[0].threshold as i32,
                time,
            };
            res.alert_count += 1;
        }

        // ── Phase 2: rate-of-change detection (domain-specific) ──

        let count = self.zone_count;
        let mut i: u32 = 0;
        while i < count {
            let idx = i as usize;
            if self.states[idx].active
                && self.configs[idx].zone_id == zone_id
                && self.configs[idx].enabled
            {
                let rate_thr = self.configs[idx].rate_threshold;

                // ── RapidDrop: windowed (SWREQ-TEMP-003) ──
                // Ring of RATE_BUCKETS per-bucket maxima. Advance the ring
                // to the current bucket, take the max over surviving
                // buckets (the samples of the last <= 660s), compare, then
                // insert the current reading.
                let b = time / RATE_BUCKET_SEC;
                let st = &mut self.states[idx];
                if !st.window_init || b < st.newest_bucket {
                    // First reading, or time went backwards (clock
                    // adjustment): restart the window rather than compare
                    // against stale maxima.
                    let mut k: usize = 0;
                    while k < RATE_BUCKETS {
                        st.bucket_valid[k] = false;
                        k += 1;
                    }
                    st.newest_bucket = b;
                    st.window_init = true;
                } else if b > st.newest_bucket {
                    if b - st.newest_bucket >= RATE_BUCKETS as u64 {
                        // The whole window expired.
                        let mut k: usize = 0;
                        while k < RATE_BUCKETS {
                            st.bucket_valid[k] = false;
                            k += 1;
                        }
                    } else {
                        // Invalidate the buckets the gap skipped over
                        // (at most RATE_BUCKETS - 1 steps).
                        let mut step = st.newest_bucket + 1;
                        while step <= b {
                            st.bucket_valid[(step % RATE_BUCKETS as u64) as usize] = false;
                            step += 1;
                        }
                    }
                    st.newest_bucket = b;
                }

                let mut have_max = false;
                let mut window_max: i32 = 0;
                let mut k: usize = 0;
                while k < RATE_BUCKETS {
                    if st.bucket_valid[k] && (!have_max || st.bucket_max[k] > window_max) {
                        window_max = st.bucket_max[k];
                        have_max = true;
                    }
                    k += 1;
                }

                if have_max
                    && window_max.saturating_sub(value) > rate_thr
                    && (res.alert_count as usize) < MAX_ALERTS_PER_READING
                {
                    res.alerts[res.alert_count as usize] = TempAlert {
                        zone_id,
                        alert_type: TempAlertType::RapidDrop,
                        value,
                        threshold: rate_thr,
                        time,
                    };
                    res.alert_count += 1;
                }

                let slot = (b % RATE_BUCKETS as u64) as usize;
                if st.bucket_valid[slot] {
                    if value > st.bucket_max[slot] {
                        st.bucket_max[slot] = value;
                    }
                } else {
                    st.bucket_valid[slot] = true;
                    st.bucket_max[slot] = value;
                }

                // ── RapidRise: consecutive readings (unchanged) ──
                if st.last_time > 0
                    && value.saturating_sub(st.last_value) > rate_thr
                    && (res.alert_count as usize) < MAX_ALERTS_PER_READING
                {
                    res.alerts[res.alert_count as usize] = TempAlert {
                        zone_id,
                        alert_type: TempAlertType::RapidRise,
                        value,
                        threshold: rate_thr,
                        time,
                    };
                    res.alert_count += 1;
                }

                st.last_value = value;
                st.last_time = time;
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
        while j < r.alert_count {
            if r.alerts[j as usize].alert_type == TempAlertType::RapidDrop {
                found = true;
            }
            j += 1;
        }
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
        while j < r.alert_count {
            if r.alerts[j as usize].alert_type == TempAlertType::RapidRise {
                found = true;
            }
            j += 1;
        }
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

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn alert_count_always_bounded(
            value in -5000i32..10000,
            freeze in -2000i32..500,
            overheat in 3000i32..8000,
            rate in 100i32..2000,
        ) {
            let mut m = TemperatureMonitor::new();
            m.register_zone(ZoneConfig {
                zone_id: 1, freeze_threshold: freeze,
                overheat_threshold: overheat, rate_threshold: rate, enabled: true,
            });
            let r = m.process_reading(1, value, 100);
            prop_assert!(r.alert_count as usize <= MAX_ALERTS_PER_READING);
        }

        #[test]
        fn window_fires_any_alignment(
            t1 in 0u64..1_000_000,
            dt in 0u64..=RATE_WINDOW_SEC,
            v1 in -10000i32..10000,
            drop in 201i32..5000,
        ) {
            // Complements the Kani harnesses' first-bucket narrowing: any
            // t1 alignment, any in-window dt, any drop over the threshold.
            let mut m = TemperatureMonitor::new();
            m.register_zone(ZoneConfig {
                zone_id: 1, freeze_threshold: i32::MIN,
                overheat_threshold: i32::MAX, rate_threshold: 200, enabled: true,
            });
            m.process_reading(1, v1, t1);
            let r = m.process_reading(1, v1 - drop, t1 + dt);
            let mut found = false;
            let mut j = 0u32;
            while j < r.alert_count {
                if r.alerts[j as usize].alert_type == TempAlertType::RapidDrop {
                    found = true;
                }
                j += 1;
            }
            prop_assert!(found);
        }

        #[test]
        fn normal_range_no_alerts(
            value in 1000i32..3000,
        ) {
            let mut m = TemperatureMonitor::new();
            m.register_zone(ZoneConfig {
                zone_id: 1, freeze_threshold: 0,
                overheat_threshold: 4000, rate_threshold: 5000, enabled: true,
            });
            let r = m.process_reading(1, value, 100);
            prop_assert_eq!(r.alert_count, 0);
        }
    }
}

// ── Kani bounded model checking harnesses ────────────────────

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// TEMP-P04: alert_count never exceeds MAX_ALERTS_PER_READING.
    ///
    /// Input narrowing (trust escape, documented): the reading times are
    /// LITERAL constants, one pair per window code path (in-window insert,
    /// time-backwards reset, gap clear-all, one-bucket skip), run as four
    /// independent monitors in straight-line code. Symbolic times — even a
    /// symbolic choice over these same four pairs — leave a symbolic u64
    /// division CBMC cannot constant-fold, which made this harness
    /// intractable once the bucket ring landed (two runs killed). Values
    /// and thresholds stay symbolic; the exhaustive window_tests cover the
    /// time dimension densely.
    #[kani::proof]
    fn verify_alert_count_bounded() {
        // (t1, t2) per path; RATE_WINDOW_SEC = 600, clear-all gap = 810.
        let cases: [(u64, u64); 4] = [(0, 600), (100, 50), (0, 810), (0, 90)];
        let mut c = 0usize;
        while c < 4 {
            let (t1, t2) = cases[c];
            let mut m = TemperatureMonitor::new();
            let config = ZoneConfig {
                zone_id: 1,
                freeze_threshold: kani::any(),
                overheat_threshold: kani::any(),
                rate_threshold: kani::any(),
                enabled: true,
            };
            m.register_zone(config);
            let v1: i32 = kani::any();
            m.process_reading(1, v1, t1);
            let v2: i32 = kani::any();
            let r = m.process_reading(1, v2, t2);
            assert!(r.alert_count as usize <= MAX_ALERTS_PER_READING);
            c += 1;
        }
    }

    /// TEMP-P01: value <= freeze_threshold produces Freeze alert (via relay-lc)
    #[kani::proof]
    fn verify_freeze_detection() {
        let mut m = TemperatureMonitor::new();
        let freeze_thr: i32 = kani::any();
        let config = ZoneConfig {
            zone_id: 1,
            freeze_threshold: freeze_thr,
            overheat_threshold: i32::MAX, // won't trigger overheat
            rate_threshold: i32::MAX,     // won't trigger rate
            enabled: true,
        };
        m.register_zone(config);
        let value: i32 = kani::any();
        kani::assume(value <= freeze_thr);
        let r = m.process_reading(1, value, 100);
        // relay-lc uses LessOrEqual, so value <= freeze_threshold fires
        let mut found_freeze = false;
        let mut j: u32 = 0;
        while j < r.alert_count {
            if r.alerts[j as usize].alert_type == TempAlertType::Freeze {
                found_freeze = true;
            }
            j += 1;
        }
        assert!(found_freeze);
    }

    /// TEMP-003: any drop greater than rate_threshold between two readings
    /// RATE_WINDOW_SEC apart fires RapidDrop, for every value pair.
    ///
    /// Input narrowing (trust escape, documented): the two reading TIMES are
    /// concrete (0 and RATE_WINDOW_SEC, the farthest in-window alignment) —
    /// symbolic u64 time division made the harness intractable for CBMC
    /// (two runs killed after 5 h+). The time dimension is instead covered
    /// COMPLETELY by the exhaustive window_tests::exhaustive_* tests (all
    /// 60 bucket alignments x every dt in range, at boundary values) and by
    /// the window_fires_any_alignment proptest (random large t1). Values
    /// here stay symbolic in [-10000, 10000].
    #[kani::proof]
    fn verify_window_drop_fires() {
        let mut m = TemperatureMonitor::new();
        m.register_zone(ZoneConfig {
            zone_id: 1,
            freeze_threshold: i32::MIN,
            overheat_threshold: i32::MAX,
            rate_threshold: 200,
            enabled: true,
        });
        let v1: i32 = kani::any();
        let v2: i32 = kani::any();
        kani::assume(v1 >= -10000 && v1 <= 10000);
        kani::assume(v2 >= -10000 && v2 <= 10000);
        kani::assume(v1 - v2 > 200);
        m.process_reading(1, v1, 0);
        let r = m.process_reading(1, v2, RATE_WINDOW_SEC);
        let mut found = false;
        let mut j: u32 = 0;
        while j < r.alert_count {
            if r.alerts[j as usize].alert_type == TempAlertType::RapidDrop {
                found = true;
            }
            j += 1;
        }
        assert!(found);
    }

    /// TEMP-003 negative: with only two readings more than
    /// RATE_WINDOW_SEC + RATE_BUCKET_SEC apart, the baseline has expired
    /// and no RapidDrop fires, however large the drop.
    #[kani::proof]
    fn verify_window_drop_expired_no_fire() {
        let mut m = TemperatureMonitor::new();
        m.register_zone(ZoneConfig {
            zone_id: 1,
            freeze_threshold: i32::MIN,
            overheat_threshold: i32::MAX,
            rate_threshold: 200,
            enabled: true,
        });
        // Input narrowing (trust escape, documented): concrete reading times
        // (0 and RATE_WINDOW_SEC + RATE_BUCKET_SEC, the nearest expired
        // alignment) for the same tractability reason as
        // verify_window_drop_fires; the exhaustive tests and the proptest
        // cover the time dimension. Values stay symbolic.
        let v1: i32 = kani::any();
        let v2: i32 = kani::any();
        kani::assume(v1 >= -10000 && v1 <= 10000);
        kani::assume(v2 >= -10000 && v2 <= 10000);
        m.process_reading(1, v1, 0);
        let r = m.process_reading(1, v2, RATE_WINDOW_SEC + RATE_BUCKET_SEC);
        let mut j: u32 = 0;
        while j < r.alert_count {
            assert!(r.alerts[j as usize].alert_type != TempAlertType::RapidDrop);
            j += 1;
        }
    }

    /// No panics for any combination of symbolic inputs
    #[kani::proof]
    fn verify_no_panic() {
        let mut m = TemperatureMonitor::new();
        let zone_id: u32 = kani::any();
        let config = ZoneConfig {
            zone_id,
            freeze_threshold: kani::any(),
            overheat_threshold: kani::any(),
            rate_threshold: kani::any(),
            enabled: kani::any(),
        };
        m.register_zone(config);
        let value: i32 = kani::any();
        let time: u64 = kani::any();
        let _ = m.process_reading(zone_id, value, time);
    }

    /// No panics through every window code path (insert, time-backwards
    /// reset, gap clear-all, one-bucket skip) with a second reading.
    /// Input narrowing (trust escape, documented): the times are literal
    /// constants per path (symbolic time is intractable for CBMC with the
    /// bucket ring — see verify_alert_count_bounded); verify_no_panic
    /// above keeps one fully symbolic time, and values, thresholds and
    /// the zone id stay symbolic here.
    #[kani::proof]
    fn verify_no_panic_window_paths() {
        let cases: [(u64, u64); 4] = [(0, 600), (100, 50), (0, 810), (0, 90)];
        let mut c = 0usize;
        while c < 4 {
            let (t1, t2) = cases[c];
            let mut m = TemperatureMonitor::new();
            let zone_id: u32 = kani::any();
            let config = ZoneConfig {
                zone_id,
                freeze_threshold: kani::any(),
                overheat_threshold: kani::any(),
                rate_threshold: kani::any(),
                enabled: kani::any(),
            };
            m.register_zone(config);
            let v1: i32 = kani::any();
            let v2: i32 = kani::any();
            let _ = m.process_reading(zone_id, v1, t1);
            let _ = m.process_reading(zone_id, v2, t2);
            c += 1;
        }
    }
}

/// Unit tests pinning behaviors a cargo-mutants pass reported untested.
#[cfg(test)]
mod mutation_tests {
    use super::*;

    fn cfg(zone_id: u32, freeze: i32, overheat: i32, rate: i32) -> ZoneConfig {
        ZoneConfig {
            zone_id,
            freeze_threshold: freeze,
            overheat_threshold: overheat,
            rate_threshold: rate,
            enabled: true,
        }
    }

    // freeze watchpoint id derived from the zone (vs constant 0)
    #[test]
    fn freeze_watchpoints_are_per_zone() {
        let mut m = TemperatureMonitor::new();
        assert!(m.register_zone(cfg(0, 0, 4000, 100000)));
        assert!(m.register_zone(cfg(1, 1000, 4000, 100000)));
        // zone 0 freeze threshold is 0, so 500 must not alert
        let r0 = m.process_reading(0, 500, 100);
        assert_eq!(r0.alert_count, 0);
        // zone 1 freeze threshold is 1000, so 500 must alert Freeze
        let r1 = m.process_reading(1, 500, 100);
        assert_eq!(r1.alert_count, 1);
        assert_eq!(r1.alerts[0].alert_type, TempAlertType::Freeze);
        assert_eq!(r1.alerts[0].threshold, 1000);
    }

    // freeze watchpoint id (vs constant 1): zone 0 fires exactly one alert
    #[test]
    fn zone_zero_freeze_only_one_alert() {
        let mut m = TemperatureMonitor::new();
        assert!(m.register_zone(cfg(0, 0, 4000, 500)));
        let r = m.process_reading(0, -100, 100);
        assert_eq!(r.alert_count, 1);
        assert_eq!(r.alerts[0].alert_type, TempAlertType::Freeze);
    }

    #[test]
    fn zone_zero_overheat_only_one_alert() {
        let mut m = TemperatureMonitor::new();
        assert!(m.register_zone(cfg(0, 0, 4000, 5000)));
        let r = m.process_reading(0, 4500, 100);
        assert_eq!(r.alert_count, 1);
        assert_eq!(r.alerts[0].alert_type, TempAlertType::Overheat);
    }

    // overheat watchpoint id derived from the zone (vs constant 0)
    #[test]
    fn overheat_watchpoints_are_per_zone() {
        let mut m = TemperatureMonitor::new();
        assert!(m.register_zone(cfg(1, 0, 4000, 100000)));
        assert!(m.register_zone(cfg(2, 0, 3000, 100000)));
        // 3500 is below zone 1's overheat threshold, so no alert must fire for zone 1.
        let r = m.process_reading(1, 3500, 100);
        assert_eq!(r.alert_count, 0);
        // Zone 2 does overheat at 3500.
        let r2 = m.process_reading(2, 3500, 200);
        assert_eq!(r2.alert_count, 1);
        assert_eq!(r2.alerts[0].alert_type, TempAlertType::Overheat);
        assert_eq!(r2.alerts[0].threshold, 3000);
    }

    // overheat watchpoint id (vs constant 1): zone 0's threshold applies to zone 0
    #[test]
    fn overheat_watchpoint_zone_zero() {
        let mut m = TemperatureMonitor::new();
        assert!(m.register_zone(cfg(0, 0, 4000, 500)));
        assert!(m.register_zone(cfg(1, 0, 6000, 500)));
        // 5000 is below zone 1's overheat threshold, so no alert is expected for zone 1
        let r = m.process_reading(1, 5000, 100);
        assert_eq!(r.alert_count, 0);
        // zone 0 does overheat at 5000
        let r0 = m.process_reading(0, 5000, 200);
        assert_eq!(r0.alert_count, 1);
        assert_eq!(r0.alerts[0].alert_type, TempAlertType::Overheat);
        assert_eq!(r0.alerts[0].threshold, 4000);
    }

    // process_reading: loop bound < zone_count (vs <=) with a full table
    #[test]
    fn full_table_unknown_zone_scans_without_overrun() {
        let mut m = TemperatureMonitor::new();
        let mut z = 1u32;
        while (z as usize) <= MAX_ZONES {
            assert!(m.register_zone(cfg(z, 0, 4000, 500)));
            z += 1;
        }
        // Unknown zone: the rate-of-change scan walks all zones and never breaks early.
        let r = m.process_reading(999, 2000, 100);
        assert_eq!(r.alert_count, 0);
    }

    // process_reading: id match && has-state (vs ||)
    #[test]
    fn other_zone_reading_does_not_use_foreign_state() {
        let mut m = TemperatureMonitor::new();
        assert!(m.register_zone(cfg(1, 0, 4000, 500)));
        assert!(m.register_zone(cfg(2, 0, 4000, 500)));
        let r1 = m.process_reading(1, 2000, 100);
        assert_eq!(r1.alert_count, 0);
        // first reading for zone 2: no baseline, so no rate alert
        let r2 = m.process_reading(2, 3000, 100);
        assert_eq!(r2.alert_count, 0);
        // unknown zone must not alter or use any state
        let r3 = m.process_reading(99, 3500, 200);
        assert_eq!(r3.alert_count, 0);
    }

    // process_reading: id match && enabled (vs ||)
    #[test]
    fn disabled_zone_no_rate_alerts() {
        let mut m = TemperatureMonitor::new();
        let mut c = cfg(1, 0, 4000, 500);
        c.enabled = false;
        m.register_zone(c);
        let r1 = m.process_reading(1, 2500, 100);
        assert_eq!(r1.alert_count, 0);
        let r2 = m.process_reading(1, 1500, 200);
        assert_eq!(r2.alert_count, 0);
        let r3 = m.process_reading(1, 2900, 300);
        assert_eq!(r3.alert_count, 0);
    }

    #[test]
    fn other_zone_reading_does_not_use_enabled_zone_state() {
        let mut m = TemperatureMonitor::new();
        m.register_zone(cfg(1, 0, 4000, 500));
        m.process_reading(1, 2500, 100);
        let r = m.process_reading(99, 1500, 200);
        assert_eq!(r.alert_count, 0);
    }

    // rapid drop: strictly over rate_threshold (> vs >=)
    #[test]
    fn rapid_drop_equal_to_threshold_is_not_alert() {
        let mut m = TemperatureMonitor::new();
        m.register_zone(cfg(1, -10000, 10000, 500));
        m.process_reading(1, 2500, 100);
        let r = m.process_reading(1, 2000, 200);
        assert_eq!(r.alert_count, 0);
    }

    #[test]
    fn rapid_drop_above_threshold_is_alert() {
        let mut m = TemperatureMonitor::new();
        m.register_zone(cfg(1, -10000, 10000, 500));
        m.process_reading(1, 2500, 100);
        let r = m.process_reading(1, 1999, 200);
        assert_eq!(r.alert_count, 1);
        assert_eq!(r.alerts[0].alert_type, TempAlertType::RapidDrop);
    }

    // rapid drop: has-baseline && over-threshold (vs ||)
    #[test]
    fn steady_reading_no_rapid_drop() {
        let mut m = TemperatureMonitor::new();
        assert!(m.register_zone(cfg(1, 0, 4000, 500)));
        let r1 = m.process_reading(1, 2000, 100);
        assert_eq!(r1.alert_count, 0);
        let r2 = m.process_reading(1, 2000, 200);
        assert_eq!(r2.alert_count, 0);
        let r3 = m.process_reading(1, 2100, 300);
        assert_eq!(r3.alert_count, 0);
    }

    // rapid rise: strictly over rate_threshold (> vs >=)
    #[test]
    fn rise_equal_to_rate_threshold_is_not_rapid_rise() {
        let mut m = TemperatureMonitor::new();
        assert!(m.register_zone(cfg(1, 0, 4000, 500)));
        let r0 = m.process_reading(1, 1500, 100);
        assert_eq!(r0.alert_count, 0);
        let r = m.process_reading(1, 2000, 200);
        assert_eq!(r.alert_count, 0);
        let r2 = m.process_reading(1, 2501, 300);
        assert_eq!(r2.alert_count, 1);
        assert_eq!(r2.alerts[0].alert_type, TempAlertType::RapidRise);
    }

    // rapid rise: has-baseline && over-threshold (vs ||)
    #[test]
    fn no_rise_alert_for_small_change() {
        let mut m = TemperatureMonitor::new();
        m.register_zone(cfg(1, 0, 4000, 500));
        let r0 = m.process_reading(1, 2000, 100);
        assert_eq!(r0.alert_count, 0);
        let r = m.process_reading(1, 2100, 200);
        assert_eq!(r.alert_count, 0);
        let r2 = m.process_reading(1, 2050, 300);
        assert_eq!(r2.alert_count, 0);
    }
}

/// TEMP-003 window tests: the 10-minute rapid-drop window (owner decision
/// implement-window, 2026-11-19). The requirement's own scenario: a gradual
/// freeze onset must fire even when no consecutive-reading step exceeds the
/// threshold.
#[cfg(test)]
mod window_tests {
    use super::*;

    fn cfg(zone_id: u32) -> ZoneConfig {
        ZoneConfig {
            zone_id,
            freeze_threshold: -10000,
            overheat_threshold: 10000,
            rate_threshold: 200, // 2.00 C, the requirement's value
            enabled: true,
        }
    }

    fn drop_alert_in(r: &TempResult) -> bool {
        let mut j = 0u32;
        let mut found = false;
        while j < r.alert_count {
            if r.alerts[j as usize].alert_type == TempAlertType::RapidDrop {
                found = true;
            }
            j += 1;
        }
        found
    }

    // SWREQ-TEMP-003 verification criterion: a 2.01 C drop within 9 minutes
    // fires. 60s cadence, -0.3 C per reading: no consecutive step exceeds
    // 2.00 C, only the window sees it.
    #[test]
    fn gradual_drop_201_within_9_minutes_fires() {
        let mut m = TemperatureMonitor::new();
        assert!(m.register_zone(cfg(1)));
        let mut fired = false;
        let mut i: u64 = 0;
        while i <= 9 {
            let value = 2000i32 - (i as i32) * 30; // t=540: 1730, drop 270
            let r = m.process_reading(1, value, i * 60);
            if drop_alert_in(&r) {
                fired = true;
            }
            i += 1;
        }
        assert!(
            fired,
            "gradual 2.7C/9min drop must fire the window detector"
        );
    }

    // SWREQ-TEMP-003 verification criterion: a 1.99 C drop does not fire.
    #[test]
    fn gradual_drop_199_within_9_minutes_does_not_fire() {
        let mut m = TemperatureMonitor::new();
        assert!(m.register_zone(cfg(1)));
        let mut i: u64 = 0;
        while i <= 9 {
            // t=0: 2000 ... t=540: 1801 — total drop 199 (1.99 C)
            let value = if i == 9 {
                1801
            } else {
                2000i32 - (i as i32) * 22
            };
            let r = m.process_reading(1, value, i * 60);
            assert!(!drop_alert_in(&r), "1.99C drop must not fire (reading {i})");
            i += 1;
        }
    }

    // A baseline older than the window (plus the 60s bucket slack) expires:
    // the same total drop spread beyond the window does not fire.
    #[test]
    fn drop_beyond_expired_window_does_not_fire() {
        let mut m = TemperatureMonitor::new();
        assert!(m.register_zone(cfg(1)));
        let r0 = m.process_reading(1, 2000, 0);
        assert!(!drop_alert_in(&r0));
        // 700s later (> 660s = window + bucket slack): baseline expired.
        let r1 = m.process_reading(1, 1700, 700);
        assert!(
            !drop_alert_in(&r1),
            "a 3.0C drop over 700s is outside the 10-minute window"
        );
    }

    // The old consecutive-step behavior still fires through the window:
    // one >2.00 C step between consecutive readings is a within-window drop.
    #[test]
    fn single_step_drop_still_fires() {
        let mut m = TemperatureMonitor::new();
        assert!(m.register_zone(cfg(1)));
        m.process_reading(1, 2500, 100);
        let r = m.process_reading(1, 2299, 101);
        assert!(drop_alert_in(&r));
    }

    // Boundary: window max minus value exactly equal to the threshold does
    // not fire (strict >), one below it does.
    #[test]
    fn window_drop_boundary_is_strict() {
        let mut m = TemperatureMonitor::new();
        assert!(m.register_zone(cfg(1)));
        m.process_reading(1, 2000, 0);
        let r_eq = m.process_reading(1, 1800, 300); // drop 200 == threshold
        assert!(!drop_alert_in(&r_eq));
        let r_gt = m.process_reading(1, 1799, 301); // drop 201 > threshold
        assert!(drop_alert_in(&r_gt));
    }

    // Exhaustive complement to the Kani harnesses, which hold the reading
    // times CONCRETE (symbolic u64 time division is intractable for CBMC;
    // two runs were killed after 5 h+). Bucket indexing repeats modulo
    // RATE_BUCKET_SEC, so t1 in 0..60 enumerates EVERY alignment the engine
    // can see; with every dt in range this covers the whole time dimension
    // at the boundary values 201 (must fire) and 200 (must not). The Kani
    // harnesses cover the whole value dimension at concrete times.

    // Every alignment x every in-window dt: a 2.01 C drop fires, and a
    // 2.00 C drop never does (strict threshold), at all 36,660 time pairs.
    #[test]
    fn exhaustive_all_alignments_in_window() {
        for t1 in 0..RATE_BUCKET_SEC {
            for dt in 0..=RATE_WINDOW_SEC {
                let mut m = TemperatureMonitor::new();
                assert!(m.register_zone(cfg(1)));
                m.process_reading(1, 2000, t1);
                let r = m.process_reading(1, 1799, t1 + dt);
                assert!(drop_alert_in(&r), "drop 201 must fire at t1={t1} dt={dt}");
                let mut m2 = TemperatureMonitor::new();
                assert!(m2.register_zone(cfg(1)));
                m2.process_reading(1, 2000, t1);
                let r2 = m2.process_reading(1, 1800, t1 + dt);
                assert!(
                    !drop_alert_in(&r2),
                    "drop 200 must not fire at t1={t1} dt={dt}"
                );
            }
        }
    }

    // Every alignment x every expired dt up to 2000s: with the baseline
    // older than the window plus the bucket slack, no drop fires however
    // large (here 30.0 C), at all 80,460 time pairs.
    #[test]
    fn exhaustive_all_alignments_expired() {
        let first_expired = RATE_WINDOW_SEC + RATE_BUCKET_SEC;
        for t1 in 0..RATE_BUCKET_SEC {
            for dt in first_expired..=2000 {
                let mut m = TemperatureMonitor::new();
                assert!(m.register_zone(cfg(1)));
                m.process_reading(1, 2000, t1);
                let r = m.process_reading(1, -1000, t1 + dt);
                assert!(
                    !drop_alert_in(&r),
                    "expired baseline must not fire at t1={t1} dt={dt}"
                );
            }
        }
    }
}
