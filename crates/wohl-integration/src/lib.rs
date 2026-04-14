//! Wohl integration tests — end-to-end pipeline verification.
//!
//! Exercises all Wohl components together, proving the full sensor -> monitor
//! -> alert dispatcher pipeline works correctly with Relay engines underneath.

#[cfg(test)]
mod tests {
    use wohl_temp::engine::{TemperatureMonitor, ZoneConfig as TempZoneConfig, TempAlertType};
    use wohl_leak::engine::{LeakDetector, LeakAction};
    use wohl_air::engine::{AirMonitor, AirConfig, AirReading, AirAlertType};
    use wohl_door::engine::{DoorWatch, ContactConfig, DoorAlertType};
    use wohl_power::engine::{PowerMonitor, CircuitConfig, PowerAlertType};
    use wohl_alert::engine::{AlertDispatcher, DispatchAction};

    // ── Alert type encoding constants ──
    // These map component alert types to u8 values for the dispatcher.
    const ALERT_FREEZE: u8 = 1;
    const ALERT_OVERHEAT: u8 = 2;
    const ALERT_RAPID_DROP: u8 = 3;
    const ALERT_RAPID_RISE: u8 = 4;
    const ALERT_LEAK: u8 = 10;
    const ALERT_CO2_WARN: u8 = 20;
    const ALERT_CO2_CRIT: u8 = 21;
    const ALERT_PM25_WARN: u8 = 22;
    const ALERT_VOC_WARN: u8 = 24;
    const ALERT_DOOR_NIGHT: u8 = 30;
    const ALERT_DOOR_LONG: u8 = 31;
    const ALERT_POWER_OVER: u8 = 40;
    const ALERT_POWER_SPIKE: u8 = 41;

    /// Create a fully wired pipeline with standard subscriptions.
    struct Pipeline {
        temp: TemperatureMonitor,
        leak: LeakDetector,
        air: AirMonitor,
        door: DoorWatch,
        power: PowerMonitor,
        dispatcher: AlertDispatcher,
    }

    impl Pipeline {
        fn new() -> Self {
            let mut temp = TemperatureMonitor::new();
            temp.register_zone(TempZoneConfig {
                zone_id: 1,
                freeze_threshold: 0,        // 0.00 C
                overheat_threshold: 4000,    // 40.00 C
                rate_threshold: 500,         // 5.00 C/reading
                enabled: true,
            });

            let mut leak = LeakDetector::new();
            leak.register_zone(1); // kitchen
            leak.register_zone(2); // bathroom

            let mut air = AirMonitor::new();
            air.register_zone(AirConfig {
                zone_id: 1,
                co2_warn: 1000,
                co2_critical: 2000,
                pm25_warn: 250,
                pm25_critical: 500,
                voc_warn: 200,
                voc_critical: 400,
                enabled: true,
            });

            let mut door = DoorWatch::new();
            door.register_contact(ContactConfig {
                contact_id: 1,
                zone_id: 1,
                max_open_sec: 300,
                night_start_hour: 22,
                night_end_hour: 6,
                enabled: true,
            });

            let mut power = PowerMonitor::new();
            power.register_circuit(CircuitConfig {
                circuit_id: 1,
                max_watts: 30000,
                idle_watts: 100,
                spike_threshold: 10000,
                enabled: true,
            });

            let mut dispatcher = AlertDispatcher::new();
            // Subscribe all alert types for zone 1 and zone 2
            for &at in &[
                ALERT_FREEZE, ALERT_OVERHEAT, ALERT_RAPID_DROP, ALERT_RAPID_RISE,
                ALERT_LEAK, ALERT_CO2_WARN, ALERT_CO2_CRIT, ALERT_PM25_WARN,
                ALERT_VOC_WARN, ALERT_DOOR_NIGHT, ALERT_DOOR_LONG,
                ALERT_POWER_OVER, ALERT_POWER_SPIKE,
            ] {
                dispatcher.subscribe(1, at, 1);
                dispatcher.subscribe(2, at, 1);
            }

            Pipeline { temp, leak, air, door, power, dispatcher }
        }
    }

    // ══════════════════════════════════════════════════════════════
    // Test 1: Normal readings produce no alerts through the pipeline
    // ══════════════════════════════════════════════════════════════

    #[test]
    fn test_normal_readings_no_alerts() {
        let mut p = Pipeline::new();

        // Normal temperature (20.00 C)
        let temp_result = p.temp.process_reading(1, 2000, 1000);
        assert_eq!(temp_result.alert_count, 0);

        // No leak
        let leak_action = p.leak.process_event(1, false, 1000);
        assert_eq!(leak_action, LeakAction::AlreadyDry);

        // Normal air quality
        let air_result = p.air.process_reading(AirReading {
            zone_id: 1, co2_ppm: 400, pm25: 50, voc_index: 30, time: 1000,
        });
        assert_eq!(air_result.alert_count, 0);

        // Door open during day (no night alert)
        let door_result = p.door.process_event(1, true, 43200); // noon
        assert_eq!(door_result.alert_count, 0);

        // Normal power (1500W)
        let power_result = p.power.process_reading(1, 1500, 1000);
        assert_eq!(power_result.alert_count, 0);
    }

    // ══════════════════════════════════════════════════════════════
    // Test 2: Freeze temperature -> alert fires through relay-lc,
    //         then dispatched correctly through relay-to subscription
    // ══════════════════════════════════════════════════════════════

    #[test]
    fn test_freeze_alert_through_pipeline() {
        let mut p = Pipeline::new();

        // Freeze temperature (-1.00 C)
        let temp_result = p.temp.process_reading(1, -100, 1000);
        assert_eq!(temp_result.alert_count, 1);
        assert_eq!(temp_result.alerts[0].alert_type, TempAlertType::Freeze);

        // Route through dispatcher (subscribed via relay-to)
        let dispatch = p.dispatcher.process_alert(
            temp_result.alerts[0].zone_id,
            ALERT_FREEZE,
            temp_result.alerts[0].time,
        );
        assert_eq!(dispatch.action, DispatchAction::Send);
    }

    // ══════════════════════════════════════════════════════════════
    // Test 3: Water leak -> immediate alert, then dedup on repeat
    // ══════════════════════════════════════════════════════════════

    #[test]
    fn test_leak_alert_and_dedup() {
        let mut p = Pipeline::new();

        // First leak detection
        let action = p.leak.process_event(1, true, 1000);
        assert_eq!(action, LeakAction::NewLeak);

        // Route through dispatcher
        let dispatch1 = p.dispatcher.process_alert(1, ALERT_LEAK, 1000);
        assert_eq!(dispatch1.action, DispatchAction::Send);

        // Same leak reported again (still wet)
        let action2 = p.leak.process_event(1, true, 1010);
        assert_eq!(action2, LeakAction::AlreadyWet);

        // If we try to dispatch same alert type+zone within cooldown -> deduplicated
        let dispatch2 = p.dispatcher.process_alert(1, ALERT_LEAK, 1010);
        assert_eq!(dispatch2.action, DispatchAction::Deduplicated);
    }

    // ══════════════════════════════════════════════════════════════
    // Test 4: CO2 high -> alert fires through relay-lc in AirMonitor,
    //         dispatched through relay-to subscription
    // ══════════════════════════════════════════════════════════════

    #[test]
    fn test_co2_alert_through_pipeline() {
        let mut p = Pipeline::new();

        // High CO2 (1200 ppm, above 1000 warn threshold)
        let air_result = p.air.process_reading(AirReading {
            zone_id: 1, co2_ppm: 1200, pm25: 50, voc_index: 30, time: 2000,
        });
        assert!(air_result.alert_count >= 1);
        assert_eq!(air_result.alerts[0].alert_type, AirAlertType::Co2Warning);

        // Route through dispatcher
        let dispatch = p.dispatcher.process_alert(1, ALERT_CO2_WARN, 2000);
        assert_eq!(dispatch.action, DispatchAction::Send);
    }

    // ══════════════════════════════════════════════════════════════
    // Test 5: Rate limiting across all component types
    // ══════════════════════════════════════════════════════════════

    #[test]
    fn test_rate_limiting_across_pipeline() {
        let mut p = Pipeline::new();
        let base_time: u64 = 5000;

        // Fire 10 different alerts (the per-minute max) all within the same minute
        // Use different zone+type combos so none are deduplicated
        let alert_combos: [(u32, u8); 10] = [
            (1, ALERT_FREEZE),
            (1, ALERT_OVERHEAT),
            (1, ALERT_RAPID_DROP),
            (1, ALERT_LEAK),
            (2, ALERT_LEAK),
            (1, ALERT_CO2_WARN),
            (1, ALERT_CO2_CRIT),
            (1, ALERT_PM25_WARN),
            (1, ALERT_VOC_WARN),
            (1, ALERT_DOOR_NIGHT),
        ];

        for (zone, atype) in alert_combos.iter() {
            let r = p.dispatcher.process_alert(*zone, *atype, base_time);
            assert_eq!(r.action, DispatchAction::Send,
                "alert ({}, {}) should have been sent", zone, atype);
        }

        // 11th alert in same minute -> rate limited
        let r = p.dispatcher.process_alert(1, ALERT_POWER_OVER, base_time);
        assert_eq!(r.action, DispatchAction::RateLimited);

        // After the minute window resets, alerts flow again
        let r = p.dispatcher.process_alert(1, ALERT_POWER_OVER, base_time + 61);
        assert_eq!(r.action, DispatchAction::Send);
    }

    // ══════════════════════════════════════════════════════════════
    // Test 6: Door opened at night -> alert through pipeline
    // ══════════════════════════════════════════════════════════════

    #[test]
    fn test_door_night_alert_through_pipeline() {
        let mut p = Pipeline::new();

        // Open door at 23:00 (night hours: 22-06)
        // 23:00 = 82800 seconds from midnight
        let door_result = p.door.process_event(1, true, 82800);
        assert_eq!(door_result.alert_count, 1);
        assert_eq!(door_result.alerts[0].alert_type, DoorAlertType::OpenedAtNight);

        // Route through dispatcher
        let dispatch = p.dispatcher.process_alert(
            door_result.alerts[0].zone_id,
            ALERT_DOOR_NIGHT,
            door_result.alerts[0].time,
        );
        assert_eq!(dispatch.action, DispatchAction::Send);
    }

    // ══════════════════════════════════════════════════════════════
    // Test 7: Subscription filtering — unsubscribed alerts are dropped
    // ══════════════════════════════════════════════════════════════

    #[test]
    fn test_subscription_filtering() {
        let mut p = Pipeline::new();

        // Unsubscribe zone 1 from freeze alerts
        p.dispatcher.unsubscribe(1, ALERT_FREEZE);

        // Freeze detected by relay-lc (first reading, no rate-of-change possible)
        let temp_result = p.temp.process_reading(1, -100, 3000);
        assert!(temp_result.alert_count >= 1);
        assert_eq!(temp_result.alerts[0].alert_type, TempAlertType::Freeze);

        // But dispatcher drops it because not subscribed (via relay-to)
        let dispatch = p.dispatcher.process_alert(1, ALERT_FREEZE, 3000);
        assert_eq!(dispatch.action, DispatchAction::NotSubscribed);

        // Overheat still subscribed — use a fresh monitor to avoid rate-of-change
        let mut temp2 = TemperatureMonitor::new();
        temp2.register_zone(TempZoneConfig {
            zone_id: 1, freeze_threshold: 0, overheat_threshold: 4000,
            rate_threshold: 500, enabled: true,
        });
        let temp_result2 = temp2.process_reading(1, 4500, 3100);
        assert!(temp_result2.alert_count >= 1);
        assert_eq!(temp_result2.alerts[0].alert_type, TempAlertType::Overheat);
        let dispatch2 = p.dispatcher.process_alert(1, ALERT_OVERHEAT, 3100);
        assert_eq!(dispatch2.action, DispatchAction::Send);
    }

    // ══════════════════════════════════════════════════════════════
    // Test 8: Multi-component simultaneous alerts
    // ══════════════════════════════════════════════════════════════

    #[test]
    fn test_multi_component_simultaneous_alerts() {
        let mut p = Pipeline::new();
        let time: u64 = 4000;

        // Temperature freeze
        let temp_r = p.temp.process_reading(1, -500, time);
        assert!(temp_r.alert_count >= 1);

        // Water leak
        let leak_a = p.leak.process_event(1, true, time);
        assert_eq!(leak_a, LeakAction::NewLeak);

        // High CO2
        let air_r = p.air.process_reading(AirReading {
            zone_id: 1, co2_ppm: 2500, pm25: 50, voc_index: 30, time,
        });
        assert!(air_r.alert_count >= 1);

        // Power overconsumption
        let power_r = p.power.process_reading(1, 35000, time);
        assert!(power_r.alert_count >= 1);

        // All four should dispatch successfully
        let d1 = p.dispatcher.process_alert(1, ALERT_FREEZE, time);
        let d2 = p.dispatcher.process_alert(1, ALERT_LEAK, time);
        let d3 = p.dispatcher.process_alert(1, ALERT_CO2_CRIT, time);
        let d4 = p.dispatcher.process_alert(1, ALERT_POWER_OVER, time);

        assert_eq!(d1.action, DispatchAction::Send);
        assert_eq!(d2.action, DispatchAction::Send);
        assert_eq!(d3.action, DispatchAction::Send);
        assert_eq!(d4.action, DispatchAction::Send);
    }

    // ══════════════════════════════════════════════════════════════
    // Test 9: Power spike detection through the full pipeline
    // ══════════════════════════════════════════════════════════════

    #[test]
    fn test_power_spike_through_pipeline() {
        let mut p = Pipeline::new();

        // Establish baseline
        p.power.process_reading(1, 1000, 6000);

        // Sudden spike (>10000W change)
        let power_r = p.power.process_reading(1, 15000, 6100);
        let mut found_spike = false;
        for j in 0..power_r.alert_count as usize {
            if power_r.alerts[j].alert_type == PowerAlertType::Spike {
                found_spike = true;
            }
        }
        assert!(found_spike);

        // Route through dispatcher
        let dispatch = p.dispatcher.process_alert(1, ALERT_POWER_SPIKE, 6100);
        assert_eq!(dispatch.action, DispatchAction::Send);
    }

    // ══════════════════════════════════════════════════════════════
    // Test 10: Dedup works across different component sources
    // ══════════════════════════════════════════════════════════════

    #[test]
    fn test_cross_component_dedup() {
        let mut p = Pipeline::new();

        // Freeze alert dispatched
        let d1 = p.dispatcher.process_alert(1, ALERT_FREEZE, 7000);
        assert_eq!(d1.action, DispatchAction::Send);

        // Same freeze alert within cooldown -> deduplicated
        let d2 = p.dispatcher.process_alert(1, ALERT_FREEZE, 7100);
        assert_eq!(d2.action, DispatchAction::Deduplicated);

        // Different alert type same zone -> NOT deduplicated
        let d3 = p.dispatcher.process_alert(1, ALERT_LEAK, 7100);
        assert_eq!(d3.action, DispatchAction::Send);

        // Same type different zone -> NOT deduplicated
        let d4 = p.dispatcher.process_alert(2, ALERT_FREEZE, 7100);
        assert_eq!(d4.action, DispatchAction::Send);

        // After cooldown expires, same alert sends again
        p.dispatcher.clear_expired(7000 + 301);
        let d5 = p.dispatcher.process_alert(1, ALERT_FREEZE, 7000 + 301);
        assert_eq!(d5.action, DispatchAction::Send);
    }

    // ══════════════════════════════════════════════════════════════
    // Test 11: Door open-too-long timeout through pipeline
    // ══════════════════════════════════════════════════════════════

    #[test]
    fn test_door_timeout_through_pipeline() {
        let mut p = Pipeline::new();

        // Open door during the day
        let open_r = p.door.process_event(1, true, 43200); // noon
        assert_eq!(open_r.alert_count, 0); // no immediate alert

        // Check timeouts after 400 seconds (max_open_sec = 300)
        let timeout_r = p.door.check_timeouts(43200 + 400);
        assert_eq!(timeout_r.alert_count, 1);
        assert_eq!(timeout_r.alerts[0].alert_type, DoorAlertType::OpenTooLong);

        // Route through dispatcher
        let dispatch = p.dispatcher.process_alert(
            timeout_r.alerts[0].zone_id,
            ALERT_DOOR_LONG,
            43200 + 400,
        );
        assert_eq!(dispatch.action, DispatchAction::Send);
    }

    // ══════════════════════════════════════════════════════════════
    // Test 12: Thousand-reading stress test across temp + alert pipeline
    // ══════════════════════════════════════════════════════════════

    #[test]
    fn thousand_readings_pipeline() {
        // Set up full pipeline
        let mut temp = TemperatureMonitor::new();
        temp.register_zone(TempZoneConfig {
            zone_id: 1,
            freeze_threshold: 0,
            overheat_threshold: 4000,
            rate_threshold: 500,
            enabled: true,
        });
        let mut alert = AlertDispatcher::new();
        alert.subscribe(1, 0, 1); // subscribe zone 1, type 0, priority 1

        let mut total_alerts = 0u32;
        let mut total_dedups = 0u32;
        let mut _total_rate_limited = 0u32;

        for i in 0..1000u64 {
            // Alternate between normal and freezing temperatures
            let value = if i % 100 < 5 { -100 } else { 2000 };
            let result = temp.process_reading(1, value, i);
            for _j in 0..result.alert_count {
                let dispatch = alert.process_alert(1, 0, i);
                match dispatch.action {
                    DispatchAction::Send => total_alerts += 1,
                    DispatchAction::Deduplicated => total_dedups += 1,
                    DispatchAction::RateLimited => _total_rate_limited += 1,
                    DispatchAction::NotSubscribed => {},
                }
            }
        }

        // Should have some alerts, some dedups
        assert!(total_alerts > 0, "Expected some alerts");
        assert!(total_dedups > 0, "Expected some dedup");
        // Rate limiting should kick in when burst of freeze alerts happen
    }

    // ══════════════════════════════════════════════════════════════
    // Test 13: Full timeline simulation
    // ══════════════════════════════════════════════════════════════

    #[test]
    fn test_full_timeline_simulation() {
        let mut p = Pipeline::new();
        let mut sent_count: u32 = 0;
        let mut dedup_count: u32 = 0;

        // t=1000: normal temperature
        let r = p.temp.process_reading(1, 2000, 1000);
        assert_eq!(r.alert_count, 0);

        // t=1100: temperature drops to freeze
        // After reading 2000 at t=1000, reading -100 triggers both Freeze (relay-lc)
        // and RapidDrop (domain rate-of-change: 2000-(-100) = 2100 > 500 threshold)
        let r = p.temp.process_reading(1, -100, 1100);
        assert!(r.alert_count >= 1);
        assert_eq!(r.alerts[0].alert_type, TempAlertType::Freeze);
        let d = p.dispatcher.process_alert(1, ALERT_FREEZE, 1100);
        assert_eq!(d.action, DispatchAction::Send);
        sent_count += 1;

        // t=1200: leak detected in kitchen
        let a = p.leak.process_event(1, true, 1200);
        assert_eq!(a, LeakAction::NewLeak);
        let d = p.dispatcher.process_alert(1, ALERT_LEAK, 1200);
        assert_eq!(d.action, DispatchAction::Send);
        sent_count += 1;

        // t=1250: same leak again -> already wet, dispatcher dedup
        let a = p.leak.process_event(1, true, 1250);
        assert_eq!(a, LeakAction::AlreadyWet);
        let d = p.dispatcher.process_alert(1, ALERT_LEAK, 1250);
        assert_eq!(d.action, DispatchAction::Deduplicated);
        dedup_count += 1;

        // t=1300: CO2 rises
        let r = p.air.process_reading(AirReading {
            zone_id: 1, co2_ppm: 1500, pm25: 50, voc_index: 30, time: 1300,
        });
        assert!(r.alert_count >= 1);
        let d = p.dispatcher.process_alert(1, ALERT_CO2_WARN, 1300);
        assert_eq!(d.action, DispatchAction::Send);
        sent_count += 1;

        // t=1400: door opened at night (82800 = 23:00)
        let r = p.door.process_event(1, true, 82800);
        assert_eq!(r.alert_count, 1);
        let d = p.dispatcher.process_alert(1, ALERT_DOOR_NIGHT, 82800);
        assert_eq!(d.action, DispatchAction::Send);
        sent_count += 1;

        // Verify counts
        assert_eq!(sent_count, 4);
        assert_eq!(dedup_count, 1);
    }
}
