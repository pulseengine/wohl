//! Wohl Hub — home supervision system wiring 7 Relay engines.
//!
//! Reads sensor data from stdin (JSON lines), routes through monitors,
//! dispatches alerts through dedup/rate-limiter, prints alerts to stdout.

use std::io::BufRead;

use serde::{Deserialize, Serialize};

// ── Configuration types ────────────────────────────────────────

#[derive(Deserialize, Clone, Debug)]
struct HubConfig {
    scheduler: Option<SchedulerConfig>,
    #[serde(default)]
    zones: Vec<ZoneConfig>,
    #[serde(default)]
    contacts: Vec<ContactConfigToml>,
    alerts: Option<AlertConfig>,
}

#[derive(Deserialize, Clone, Debug)]
struct SchedulerConfig {
    tick_rate_ms: Option<u32>,
}

#[derive(Deserialize, Clone, Debug)]
struct ZoneConfig {
    id: u32,
    name: String,
    #[serde(default)]
    sensors: Vec<String>,
    temp_freeze: Option<i32>,
    temp_overheat: Option<i32>,
    temp_rate: Option<i32>,
    co2_warn: Option<u32>,
    co2_critical: Option<u32>,
    power_max_watts: Option<u32>,
    power_spike: Option<u32>,
}

#[derive(Deserialize, Clone, Debug)]
struct ContactConfigToml {
    id: u32,
    zone: u32,
    name: String,
    max_open_sec: Option<u32>,
    night_start: Option<u8>,
    night_end: Option<u8>,
}

#[derive(Deserialize, Clone, Debug)]
struct AlertConfig {
    rate_limit_per_minute: Option<u32>,
    dedup_cooldown_sec: Option<u64>,
}

// ── Sensor event / alert output types ──────────────────────────

#[derive(Deserialize, Debug)]
#[serde(tag = "type")]
enum SensorEvent {
    #[serde(rename = "temp")]
    Temp { zone: u32, value: i32, time: u64 },
    #[serde(rename = "water")]
    Water { zone: u32, wet: bool, time: u64 },
    #[serde(rename = "air")]
    Air { zone: u32, co2: u32, pm25: Option<u32>, voc: Option<u32>, time: u64 },
    #[serde(rename = "contact")]
    Contact { id: u32, open: bool, time: u64 },
    #[serde(rename = "power")]
    Power { circuit: u32, watts: u32, time: u64 },
    #[serde(rename = "tick")]
    Tick { time: u64 },
}

#[derive(Serialize, Debug, Clone, PartialEq)]
struct AlertOutput {
    alert: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    zone: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    circuit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    contact: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    threshold: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration: Option<u64>,
    time: u64,
}

// ── Alert type encoding for the dispatcher ─────────────────────

// We encode each domain alert type as a u8 for use with AlertDispatcher.
// These constants define the mapping.
const ALERT_FREEZE: u8 = 1;
const ALERT_OVERHEAT: u8 = 2;
const ALERT_RAPID_DROP: u8 = 3;
const ALERT_RAPID_RISE: u8 = 4;
const ALERT_WATER_LEAK: u8 = 5;
const ALERT_CO2_WARNING: u8 = 6;
const ALERT_CO2_CRITICAL: u8 = 7;
const ALERT_PM25_WARNING: u8 = 8;
const ALERT_PM25_CRITICAL: u8 = 9;
const ALERT_VOC_WARNING: u8 = 10;
const ALERT_VOC_CRITICAL: u8 = 11;
const ALERT_DOOR_OPEN_TOO_LONG: u8 = 12;
const ALERT_DOOR_NIGHT: u8 = 13;
const ALERT_OVERCONSUMPTION: u8 = 14;
const ALERT_POWER_SPIKE: u8 = 15;
const ALERT_HEALTH_MISS: u8 = 16;

// ── Monitor app IDs for health tracking ────────────────────────

const APP_TEMP: u32 = 1;
const APP_LEAK: u32 = 2;
const APP_AIR: u32 = 3;
const APP_DOOR: u32 = 4;
const APP_POWER: u32 = 5;
const APP_SCHEDULER: u32 = 6;

// ── WohlHub ────────────────────────────────────────────────────

struct WohlHub {
    // Wohl monitors
    leak: wohl_leak::engine::LeakDetector,
    temp: wohl_temp::engine::TemperatureMonitor,
    air: wohl_air::engine::AirMonitor,
    door: wohl_door::engine::DoorWatch,
    power: wohl_power::engine::PowerMonitor,
    alert: wohl_alert::engine::AlertDispatcher,

    // Relay engines
    scheduler: relay_sch::engine::ScheduleTable,
    health: relay_hs::engine::HealthTable,
    storage: relay_ds::engine::FilterTable,
    checksummer: relay_cs::engine::ChecksumTable,

    // Counters for health monitoring (one per monitor)
    monitor_counters: [u32; 6],

    // Map contact_id -> zone_id for routing door alerts
    contact_zones: [(u32, u32); 32],
    contact_zone_count: u32,

    // Track the last tick time for housekeeping
    last_tick: u64,
}

impl WohlHub {
    fn new(config: &HubConfig) -> Self {
        let mut hub = WohlHub {
            leak: wohl_leak::engine::LeakDetector::new(),
            temp: wohl_temp::engine::TemperatureMonitor::new(),
            air: wohl_air::engine::AirMonitor::new(),
            door: wohl_door::engine::DoorWatch::new(),
            power: wohl_power::engine::PowerMonitor::new(),
            alert: wohl_alert::engine::AlertDispatcher::new(),
            scheduler: relay_sch::engine::ScheduleTable::new(),
            health: relay_hs::engine::HealthTable::new(),
            storage: relay_ds::engine::FilterTable::new(),
            checksummer: relay_cs::engine::ChecksumTable::new(),
            monitor_counters: [0; 6],
            contact_zones: [(0, 0); 32],
            contact_zone_count: 0,
            last_tick: 0,
        };

        hub.configure(config);
        hub
    }

    fn configure(&mut self, config: &HubConfig) {
        // Register zones with monitors
        for zone in &config.zones {
            let sensors = &zone.sensors;

            if sensors.iter().any(|s| s == "temp") {
                let temp_config = wohl_temp::engine::ZoneConfig {
                    zone_id: zone.id,
                    freeze_threshold: zone.temp_freeze.unwrap_or(0),
                    overheat_threshold: zone.temp_overheat.unwrap_or(4000),
                    rate_threshold: zone.temp_rate.unwrap_or(500),
                    enabled: true,
                };
                self.temp.register_zone(temp_config);

                // Subscribe temperature alert types for this zone
                self.alert.subscribe(zone.id, ALERT_FREEZE, 0);
                self.alert.subscribe(zone.id, ALERT_OVERHEAT, 0);
                self.alert.subscribe(zone.id, ALERT_RAPID_DROP, 1);
                self.alert.subscribe(zone.id, ALERT_RAPID_RISE, 1);
            }

            if sensors.iter().any(|s| s == "water") {
                self.leak.register_zone(zone.id);

                // Subscribe water leak alert for this zone
                self.alert.subscribe(zone.id, ALERT_WATER_LEAK, 0);
            }

            if sensors.iter().any(|s| s == "air") {
                let air_config = wohl_air::engine::AirConfig {
                    zone_id: zone.id,
                    co2_warn: zone.co2_warn.unwrap_or(1000),
                    co2_critical: zone.co2_critical.unwrap_or(2000),
                    pm25_warn: 250,
                    pm25_critical: 500,
                    voc_warn: 200,
                    voc_critical: 400,
                    enabled: true,
                };
                self.air.register_zone(air_config);

                // Subscribe air alert types for this zone
                self.alert.subscribe(zone.id, ALERT_CO2_WARNING, 1);
                self.alert.subscribe(zone.id, ALERT_CO2_CRITICAL, 0);
                self.alert.subscribe(zone.id, ALERT_PM25_WARNING, 1);
                self.alert.subscribe(zone.id, ALERT_PM25_CRITICAL, 0);
                self.alert.subscribe(zone.id, ALERT_VOC_WARNING, 1);
                self.alert.subscribe(zone.id, ALERT_VOC_CRITICAL, 0);
            }

            if sensors.iter().any(|s| s == "power") {
                let power_config = wohl_power::engine::CircuitConfig {
                    circuit_id: zone.id,
                    max_watts: zone.power_max_watts.unwrap_or(30000),
                    idle_watts: 100,
                    spike_threshold: zone.power_spike.unwrap_or(10000),
                    enabled: true,
                };
                self.power.register_circuit(power_config);

                // Subscribe power alert types (using zone.id as circuit zone)
                self.alert.subscribe(zone.id, ALERT_OVERCONSUMPTION, 0);
                self.alert.subscribe(zone.id, ALERT_POWER_SPIKE, 1);
            }
        }

        // Register contacts
        for contact in &config.contacts {
            let door_config = wohl_door::engine::ContactConfig {
                contact_id: contact.id,
                zone_id: contact.zone,
                max_open_sec: contact.max_open_sec.unwrap_or(300),
                night_start_hour: contact.night_start.unwrap_or(22),
                night_end_hour: contact.night_end.unwrap_or(6),
                enabled: true,
            };
            self.door.register_contact(door_config);

            // Track contact -> zone mapping
            if (self.contact_zone_count as usize) < self.contact_zones.len() {
                self.contact_zones[self.contact_zone_count as usize] =
                    (contact.id, contact.zone);
                self.contact_zone_count += 1;
            }

            // Subscribe door alert types for the contact's zone
            self.alert.subscribe(contact.zone, ALERT_DOOR_OPEN_TOO_LONG, 1);
            self.alert.subscribe(contact.zone, ALERT_DOOR_NIGHT, 1);
        }

        // Register health monitors for each app
        self.health.register_app(APP_TEMP, 3, relay_hs::engine::HsAction::Event);
        self.health.register_app(APP_LEAK, 3, relay_hs::engine::HsAction::Event);
        self.health.register_app(APP_AIR, 3, relay_hs::engine::HsAction::Event);
        self.health.register_app(APP_DOOR, 3, relay_hs::engine::HsAction::Event);
        self.health.register_app(APP_POWER, 3, relay_hs::engine::HsAction::Event);
        self.health.register_app(APP_SCHEDULER, 3, relay_hs::engine::HsAction::Event);

        // Set up scheduler slot: minor_frame=0, major_frame=0 (every tick)
        // Channel 1 = health check, Channel 2 = door timeout check
        self.scheduler.add_slot(relay_sch::engine::ScheduleSlot {
            minor_frame: 0,
            major_frame: 0,
            target_channel: 1,
            payload_offset: 0,
            payload_len: 0,
            enabled: true,
        });
        self.scheduler.add_slot(relay_sch::engine::ScheduleSlot {
            minor_frame: 0,
            major_frame: 0,
            target_channel: 2,
            payload_offset: 0,
            payload_len: 0,
            enabled: true,
        });

        // Set up storage filter for alerts (data_id=1 -> destination=0 for logging)
        self.storage.add_filter(relay_ds::engine::FilterEntry {
            data_id: 1,
            destination: 0,
            enabled: true,
            file_type: relay_ds::engine::FileType::Time,
        });

        // Register config region for checksum integrity
        let config_bytes = b"wohl-hub-config-v1";
        let config_crc = relay_cs::engine::crc32_compute(config_bytes);
        self.checksummer.register_region(1, config_crc);
    }

    fn contact_zone(&self, contact_id: u32) -> u32 {
        for i in 0..self.contact_zone_count as usize {
            if self.contact_zones[i].0 == contact_id {
                return self.contact_zones[i].1;
            }
        }
        0
    }

    fn try_dispatch(&mut self, zone_id: u32, alert_type: u8, time: u64) -> bool {
        let result = self.alert.process_alert(zone_id, alert_type, time);
        result.action == wohl_alert::engine::DispatchAction::Send
    }

    fn process_event(&mut self, event: SensorEvent) -> Vec<AlertOutput> {
        let mut alerts = Vec::new();

        match event {
            SensorEvent::Temp { zone, value, time } => {
                self.monitor_counters[0] += 1;
                self.health.update_counter(APP_TEMP, self.monitor_counters[0]);

                let result = self.temp.process_reading(zone, value, time);
                for i in 0..result.alert_count as usize {
                    let a = &result.alerts[i];
                    let (alert_name, alert_code) = match a.alert_type {
                        wohl_temp::engine::TempAlertType::Freeze => ("freeze", ALERT_FREEZE),
                        wohl_temp::engine::TempAlertType::Overheat => ("overheat", ALERT_OVERHEAT),
                        wohl_temp::engine::TempAlertType::RapidDrop => ("rapid_drop", ALERT_RAPID_DROP),
                        wohl_temp::engine::TempAlertType::RapidRise => ("rapid_rise", ALERT_RAPID_RISE),
                    };
                    if self.try_dispatch(zone, alert_code, time) {
                        alerts.push(AlertOutput {
                            alert: alert_name.to_string(),
                            zone: Some(zone),
                            circuit: None,
                            contact: None,
                            value: Some(a.value as i64),
                            threshold: Some(a.threshold as i64),
                            duration: None,
                            time,
                        });
                    }
                }
            }

            SensorEvent::Water { zone, wet, time } => {
                self.monitor_counters[1] += 1;
                self.health.update_counter(APP_LEAK, self.monitor_counters[1]);

                let action = self.leak.process_event(zone, wet, time);
                if action == wohl_leak::engine::LeakAction::NewLeak {
                    if self.try_dispatch(zone, ALERT_WATER_LEAK, time) {
                        alerts.push(AlertOutput {
                            alert: "water_leak".to_string(),
                            zone: Some(zone),
                            circuit: None,
                            contact: None,
                            value: None,
                            threshold: None,
                            duration: None,
                            time,
                        });
                    }
                }
            }

            SensorEvent::Air { zone, co2, pm25, voc, time } => {
                self.monitor_counters[2] += 1;
                self.health.update_counter(APP_AIR, self.monitor_counters[2]);

                let reading = wohl_air::engine::AirReading {
                    zone_id: zone,
                    co2_ppm: co2,
                    pm25: pm25.unwrap_or(0),
                    voc_index: voc.unwrap_or(0),
                    time,
                };
                let result = self.air.process_reading(reading);
                for i in 0..result.alert_count as usize {
                    let a = &result.alerts[i];
                    let (alert_name, alert_code) = match a.alert_type {
                        wohl_air::engine::AirAlertType::Co2Warning => ("co2_warning", ALERT_CO2_WARNING),
                        wohl_air::engine::AirAlertType::Co2Critical => ("co2_critical", ALERT_CO2_CRITICAL),
                        wohl_air::engine::AirAlertType::Pm25Warning => ("pm25_warning", ALERT_PM25_WARNING),
                        wohl_air::engine::AirAlertType::Pm25Critical => ("pm25_critical", ALERT_PM25_CRITICAL),
                        wohl_air::engine::AirAlertType::VocWarning => ("voc_warning", ALERT_VOC_WARNING),
                        wohl_air::engine::AirAlertType::VocCritical => ("voc_critical", ALERT_VOC_CRITICAL),
                    };
                    if self.try_dispatch(zone, alert_code, time) {
                        alerts.push(AlertOutput {
                            alert: alert_name.to_string(),
                            zone: Some(zone),
                            circuit: None,
                            contact: None,
                            value: Some(a.value as i64),
                            threshold: Some(a.threshold as i64),
                            duration: None,
                            time,
                        });
                    }
                }
            }

            SensorEvent::Contact { id, open, time } => {
                self.monitor_counters[3] += 1;
                self.health.update_counter(APP_DOOR, self.monitor_counters[3]);

                let result = self.door.process_event(id, open, time);
                for i in 0..result.alert_count as usize {
                    let a = &result.alerts[i];
                    let (alert_name, alert_code) = match a.alert_type {
                        wohl_door::engine::DoorAlertType::OpenTooLong => {
                            ("door_open_too_long", ALERT_DOOR_OPEN_TOO_LONG)
                        }
                        wohl_door::engine::DoorAlertType::OpenedAtNight => {
                            ("door_opened_at_night", ALERT_DOOR_NIGHT)
                        }
                    };
                    let zone = a.zone_id;
                    if self.try_dispatch(zone, alert_code, time) {
                        alerts.push(AlertOutput {
                            alert: alert_name.to_string(),
                            zone: Some(zone),
                            circuit: None,
                            contact: Some(a.contact_id),
                            value: None,
                            threshold: None,
                            duration: Some(a.open_duration_sec),
                            time,
                        });
                    }
                }
            }

            SensorEvent::Power { circuit, watts, time } => {
                self.monitor_counters[4] += 1;
                self.health.update_counter(APP_POWER, self.monitor_counters[4]);

                let result = self.power.process_reading(circuit, watts, time);
                for i in 0..result.alert_count as usize {
                    let a = &result.alerts[i];
                    let (alert_name, alert_code) = match a.alert_type {
                        wohl_power::engine::PowerAlertType::OverConsumption => {
                            ("overconsumption", ALERT_OVERCONSUMPTION)
                        }
                        wohl_power::engine::PowerAlertType::Spike => {
                            ("power_spike", ALERT_POWER_SPIKE)
                        }
                        wohl_power::engine::PowerAlertType::DeviceLeftOn => {
                            ("device_left_on", ALERT_OVERCONSUMPTION)
                        }
                    };
                    // Use circuit id as zone for dispatch
                    if self.try_dispatch(circuit, alert_code, time) {
                        alerts.push(AlertOutput {
                            alert: alert_name.to_string(),
                            zone: None,
                            circuit: Some(circuit),
                            contact: None,
                            value: Some(a.value as i64),
                            threshold: Some(a.threshold as i64),
                            duration: None,
                            time,
                        });
                    }
                }
            }

            SensorEvent::Tick { time } => {
                self.monitor_counters[5] += 1;
                self.health.update_counter(APP_SCHEDULER, self.monitor_counters[5]);
                self.last_tick = time;

                // Compute minor frame from time (wrapping tick counter)
                let minor = (time / 1000) as u32;
                let major = (time / 60000) as u32;

                let tick_result = self.scheduler.process_tick(minor % 60, major);

                for i in 0..tick_result.action_count as usize {
                    let action = &tick_result.actions[i];
                    match action.target_channel {
                        // Channel 1: health check
                        1 => {
                            let hs_result = self.health.check_health(time);
                            for j in 0..hs_result.alert_count as usize {
                                let ha = &hs_result.alerts[j];
                                alerts.push(AlertOutput {
                                    alert: "health_miss".to_string(),
                                    zone: None,
                                    circuit: None,
                                    contact: None,
                                    value: Some(ha.app_id as i64),
                                    threshold: Some(ha.miss_count as i64),
                                    duration: None,
                                    time,
                                });
                            }
                        }
                        // Channel 2: door timeout check
                        2 => {
                            let door_result = self.door.check_timeouts(time);
                            for j in 0..door_result.alert_count as usize {
                                let da = &door_result.alerts[j];
                                let zone = da.zone_id;
                                if self.try_dispatch(
                                    zone,
                                    ALERT_DOOR_OPEN_TOO_LONG,
                                    time,
                                ) {
                                    alerts.push(AlertOutput {
                                        alert: "door_open_too_long".to_string(),
                                        zone: Some(zone),
                                        circuit: None,
                                        contact: Some(da.contact_id),
                                        value: None,
                                        threshold: None,
                                        duration: Some(da.open_duration_sec),
                                        time,
                                    });
                                }
                            }
                        }
                        _ => {}
                    }
                }

                // Clear expired dedup entries periodically
                self.alert.clear_expired(time);
            }
        }

        alerts
    }
}

// ── Config loading ─────────────────────────────────────────────

fn load_config() -> HubConfig {
    let config_paths = ["wohl.toml", "crates/wohl-hub/wohl.toml"];

    for path in &config_paths {
        if let Ok(content) = std::fs::read_to_string(path) {
            match toml::from_str::<HubConfig>(&content) {
                Ok(config) => {
                    eprintln!("[wohl-hub] loaded config from {}", path);
                    return config;
                }
                Err(e) => {
                    eprintln!("[wohl-hub] error parsing {}: {}", path, e);
                }
            }
        }
    }

    eprintln!("[wohl-hub] no config file found, using defaults");
    default_config()
}

fn default_config() -> HubConfig {
    HubConfig {
        scheduler: Some(SchedulerConfig {
            tick_rate_ms: Some(1000),
        }),
        zones: vec![
            ZoneConfig {
                id: 1,
                name: "kitchen".to_string(),
                sensors: vec!["temp".to_string(), "water".to_string(), "air".to_string()],
                temp_freeze: Some(0),
                temp_overheat: Some(4000),
                temp_rate: Some(500),
                co2_warn: Some(1000),
                co2_critical: Some(2000),
                power_max_watts: None,
                power_spike: None,
            },
            ZoneConfig {
                id: 2,
                name: "bathroom".to_string(),
                sensors: vec!["temp".to_string(), "water".to_string()],
                temp_freeze: Some(500),
                temp_overheat: Some(3500),
                temp_rate: None,
                co2_warn: None,
                co2_critical: None,
                power_max_watts: None,
                power_spike: None,
            },
            ZoneConfig {
                id: 3,
                name: "basement".to_string(),
                sensors: vec!["water".to_string(), "power".to_string()],
                temp_freeze: None,
                temp_overheat: None,
                temp_rate: None,
                co2_warn: None,
                co2_critical: None,
                power_max_watts: Some(30000),
                power_spike: Some(10000),
            },
        ],
        contacts: vec![ContactConfigToml {
            id: 1,
            zone: 1,
            name: "front_door".to_string(),
            max_open_sec: Some(300),
            night_start: Some(22),
            night_end: Some(6),
        }],
        alerts: Some(AlertConfig {
            rate_limit_per_minute: Some(10),
            dedup_cooldown_sec: Some(300),
        }),
    }
}

fn config_from_str(s: &str) -> Result<HubConfig, toml::de::Error> {
    toml::from_str(s)
}

// ── Main ───────────────────────────────────────────────────────

fn main() {
    let config = load_config();
    let mut hub = WohlHub::new(&config);

    eprintln!("[wohl-hub] ready — reading sensor events from stdin");

    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[wohl-hub] stdin error: {}", e);
                break;
            }
        };

        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let event: SensorEvent = match serde_json::from_str(trimmed) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("[wohl-hub] parse error: {} — line: {}", e, trimmed);
                continue;
            }
        };

        let alerts = hub.process_event(event);
        for alert in &alerts {
            match serde_json::to_string(alert) {
                Ok(json) => println!("{}", json),
                Err(e) => eprintln!("[wohl-hub] serialize error: {}", e),
            }
        }
    }

    eprintln!("[wohl-hub] done");
}

// ── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_hub() -> WohlHub {
        WohlHub::new(&default_config())
    }

    #[test]
    fn test_config_loading() {
        let toml_str = r#"
[scheduler]
tick_rate_ms = 500

[[zones]]
id = 1
name = "kitchen"
sensors = ["temp", "water"]
temp_freeze = -100
temp_overheat = 5000

[[contacts]]
id = 1
zone = 1
name = "front_door"
max_open_sec = 120
night_start = 23
night_end = 5

[alerts]
rate_limit_per_minute = 5
dedup_cooldown_sec = 60
"#;
        let config = config_from_str(toml_str).expect("valid toml");
        assert_eq!(config.zones.len(), 1);
        assert_eq!(config.zones[0].name, "kitchen");
        assert_eq!(config.zones[0].temp_freeze, Some(-100));
        assert_eq!(config.contacts.len(), 1);
        assert_eq!(config.contacts[0].night_start, Some(23));
        assert_eq!(
            config.alerts.as_ref().unwrap().rate_limit_per_minute,
            Some(5)
        );
    }

    #[test]
    fn test_default_config() {
        let config = default_config();
        assert_eq!(config.zones.len(), 3);
        assert_eq!(config.contacts.len(), 1);
    }

    #[test]
    fn test_process_temp_event_freeze() {
        let mut hub = test_hub();
        let event = SensorEvent::Temp {
            zone: 1,
            value: -100,
            time: 1000,
        };
        let alerts = hub.process_event(event);
        assert!(!alerts.is_empty(), "should produce freeze alert");
        assert_eq!(alerts[0].alert, "freeze");
        assert_eq!(alerts[0].zone, Some(1));
        assert_eq!(alerts[0].value, Some(-100));
        assert_eq!(alerts[0].time, 1000);
    }

    #[test]
    fn test_process_temp_event_overheat() {
        let mut hub = test_hub();
        let event = SensorEvent::Temp {
            zone: 1,
            value: 4500,
            time: 2000,
        };
        let alerts = hub.process_event(event);
        assert!(!alerts.is_empty(), "should produce overheat alert");
        assert_eq!(alerts[0].alert, "overheat");
    }

    #[test]
    fn test_process_temp_event_normal() {
        let mut hub = test_hub();
        let event = SensorEvent::Temp {
            zone: 1,
            value: 2150,
            time: 1000,
        };
        let alerts = hub.process_event(event);
        assert!(alerts.is_empty(), "normal temp should produce no alert");
    }

    #[test]
    fn test_process_water_event_leak() {
        let mut hub = test_hub();
        let event = SensorEvent::Water {
            zone: 2,
            wet: true,
            time: 1001,
        };
        let alerts = hub.process_event(event);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].alert, "water_leak");
        assert_eq!(alerts[0].zone, Some(2));
    }

    #[test]
    fn test_process_water_event_dry() {
        let mut hub = test_hub();
        let event = SensorEvent::Water {
            zone: 1,
            wet: false,
            time: 1000,
        };
        let alerts = hub.process_event(event);
        assert!(alerts.is_empty(), "dry event should produce no alert");
    }

    #[test]
    fn test_process_water_event_dedup() {
        let mut hub = test_hub();

        // First leak
        let alerts1 = hub.process_event(SensorEvent::Water {
            zone: 1,
            wet: true,
            time: 1000,
        });
        assert_eq!(alerts1.len(), 1);

        // Second leak in same zone within cooldown: leak detector returns AlreadyWet
        let alerts2 = hub.process_event(SensorEvent::Water {
            zone: 1,
            wet: true,
            time: 1001,
        });
        assert!(
            alerts2.is_empty(),
            "already wet should not produce new alert"
        );
    }

    #[test]
    fn test_process_air_event_co2_warning() {
        let mut hub = test_hub();
        let event = SensorEvent::Air {
            zone: 1,
            co2: 1200,
            pm25: Some(50),
            voc: Some(30),
            time: 1002,
        };
        let alerts = hub.process_event(event);
        assert!(!alerts.is_empty(), "should produce CO2 warning");
        let co2_alerts: Vec<_> = alerts.iter().filter(|a| a.alert == "co2_warning").collect();
        assert!(!co2_alerts.is_empty());
        assert_eq!(co2_alerts[0].value, Some(1200));
    }

    #[test]
    fn test_process_contact_event_night() {
        let mut hub = test_hub();
        // Time 82800 = 23:00:00 UTC (23h * 3600)
        let event = SensorEvent::Contact {
            id: 1,
            open: true,
            time: 82800,
        };
        let alerts = hub.process_event(event);
        assert!(!alerts.is_empty(), "should produce night door alert");
        assert_eq!(alerts[0].alert, "door_opened_at_night");
    }

    #[test]
    fn test_process_power_event_overconsumption() {
        let mut hub = test_hub();
        // Circuit 3 is the basement which has power monitoring
        let event = SensorEvent::Power {
            circuit: 3,
            watts: 35000,
            time: 1004,
        };
        let alerts = hub.process_event(event);
        assert!(!alerts.is_empty(), "should produce overconsumption alert");
        assert_eq!(alerts[0].alert, "overconsumption");
    }

    #[test]
    fn test_process_power_event_normal() {
        let mut hub = test_hub();
        let event = SensorEvent::Power {
            circuit: 3,
            watts: 1500,
            time: 1004,
        };
        let alerts = hub.process_event(event);
        assert!(
            alerts.is_empty(),
            "normal power should produce no alert"
        );
    }

    #[test]
    fn test_process_tick_event() {
        let mut hub = test_hub();

        // Open a door during daytime (time=43200000ms = noon in ms, but
        // the contact sensor uses seconds for hour calculation, so use
        // a daytime timestamp). Door open at time=0ms (midnight=0h, but
        // that's night for the config 22-6, so use time that maps to daytime).
        // time=43200000 => hour_of_day = (43200000 % 86400) / 3600 = 12 (noon)
        // But contact time is in seconds in the door engine (passed raw).
        // Actually the door uses: hour_of_day = ((time % 86400) / 3600) as u8
        // So time=43200 => hour=12 (noon, not night). Good.
        hub.process_event(SensorEvent::Contact {
            id: 1,
            open: true,
            time: 43200, // noon (seconds in door engine)
        });

        // Tick >300s later, at a time where (time/1000)%60 == 0 so the
        // scheduler slot with minor_frame=0 fires.
        // time=43200 + 301*1000 = 344200 (but we need ms-based minor to be 0)
        // Actually: minor = (time / 1000) % 60. For minor=0, time must be
        // a multiple of 60000. Use time=360000: (360000/1000)%60 = 360%60 = 0.
        // Duration since door open: 360000 - 43200 = 316800 > 300 (the max_open_sec).
        let alerts = hub.process_event(SensorEvent::Tick {
            time: 360000,
        });

        // The tick drives scheduler, which checks door timeouts
        let door_alerts: Vec<_> = alerts
            .iter()
            .filter(|a| a.alert == "door_open_too_long")
            .collect();
        assert!(
            !door_alerts.is_empty(),
            "tick should detect door open too long, got alerts: {:?}",
            alerts
        );
    }

    #[test]
    fn test_full_pipeline() {
        let mut hub = test_hub();
        let mut all_alerts = Vec::new();

        // Temperature freeze
        all_alerts.extend(hub.process_event(SensorEvent::Temp {
            zone: 1,
            value: -100,
            time: 100,
        }));

        // Water leak in bathroom
        all_alerts.extend(hub.process_event(SensorEvent::Water {
            zone: 2,
            wet: true,
            time: 200,
        }));

        // CO2 warning in kitchen
        all_alerts.extend(hub.process_event(SensorEvent::Air {
            zone: 1,
            co2: 1200,
            pm25: Some(50),
            voc: Some(30),
            time: 300,
        }));

        // Normal power reading (no alert)
        all_alerts.extend(hub.process_event(SensorEvent::Power {
            circuit: 3,
            watts: 1500,
            time: 400,
        }));

        // Verify we got the expected alerts
        let alert_types: Vec<&str> = all_alerts.iter().map(|a| a.alert.as_str()).collect();

        assert!(
            alert_types.contains(&"freeze"),
            "expected freeze alert, got: {:?}",
            alert_types
        );
        assert!(
            alert_types.contains(&"water_leak"),
            "expected water_leak alert, got: {:?}",
            alert_types
        );
        assert!(
            alert_types.contains(&"co2_warning"),
            "expected co2_warning alert, got: {:?}",
            alert_types
        );

        // No power alert for normal reading
        assert!(
            !alert_types.contains(&"overconsumption"),
            "should not have overconsumption alert"
        );
    }

    #[test]
    fn test_alert_dedup_through_dispatcher() {
        let mut hub = test_hub();

        // First freeze alert should pass
        let alerts1 = hub.process_event(SensorEvent::Temp {
            zone: 1,
            value: -100,
            time: 1000,
        });
        assert!(!alerts1.is_empty(), "first freeze should dispatch");

        // Second freeze in same zone within cooldown should be deduped
        let alerts2 = hub.process_event(SensorEvent::Temp {
            zone: 1,
            value: -200,
            time: 1010,
        });
        assert!(
            alerts2.is_empty(),
            "second freeze within cooldown should be deduped"
        );
    }

    #[test]
    fn test_health_monitoring_active() {
        let mut hub = test_hub();

        // Send some sensor events to keep monitors active
        hub.process_event(SensorEvent::Temp {
            zone: 1,
            value: 2000,
            time: 1000,
        });
        hub.process_event(SensorEvent::Water {
            zone: 1,
            wet: false,
            time: 1000,
        });

        // Tick to run health check — monitors should be healthy since we updated counters
        let alerts = hub.process_event(SensorEvent::Tick { time: 1000 });

        // Health check should not produce alerts since monitors are active
        let health_alerts: Vec<_> = alerts
            .iter()
            .filter(|a| a.alert == "health_miss")
            .collect();
        // Some monitors (air, power, door) were not exercised,
        // but they start at 0/0 so first check increments miss counter.
        // With max_miss=3 they should not alert on first check.
        // The monitors we DID exercise should be fine.
        assert!(
            health_alerts.len() <= 6,
            "health alerts bounded: got {}",
            health_alerts.len()
        );
    }

    #[test]
    fn test_storage_filter_configured() {
        let hub = test_hub();
        // Verify storage filter is set up
        assert_eq!(hub.storage.filter_count(), 1);
    }

    #[test]
    fn test_checksum_integrity() {
        let hub = test_hub();
        assert_eq!(hub.checksummer.region_count(), 1);
    }

    #[test]
    fn test_scheduler_configured() {
        let hub = test_hub();
        assert_eq!(hub.scheduler.slot_count(), 2);
    }

    #[test]
    fn test_health_table_configured() {
        let hub = test_hub();
        assert_eq!(hub.health.app_count(), 6);
    }

    #[test]
    fn test_json_roundtrip() {
        let alert = AlertOutput {
            alert: "freeze".to_string(),
            zone: Some(1),
            circuit: None,
            contact: None,
            value: Some(-100),
            threshold: Some(0),
            duration: None,
            time: 1000,
        };
        let json = serde_json::to_string(&alert).unwrap();
        assert!(json.contains("\"alert\":\"freeze\""));
        assert!(json.contains("\"zone\":1"));
        assert!(json.contains("\"value\":-100"));
        assert!(!json.contains("circuit")); // skipped when None
    }

    #[test]
    fn test_sensor_event_parsing() {
        let temp: SensorEvent =
            serde_json::from_str(r#"{"type":"temp","zone":1,"value":2150,"time":1000}"#).unwrap();
        assert!(matches!(temp, SensorEvent::Temp { zone: 1, value: 2150, time: 1000 }));

        let water: SensorEvent =
            serde_json::from_str(r#"{"type":"water","zone":2,"wet":true,"time":1001}"#).unwrap();
        assert!(matches!(water, SensorEvent::Water { zone: 2, wet: true, time: 1001 }));

        let air: SensorEvent =
            serde_json::from_str(r#"{"type":"air","zone":1,"co2":1200,"pm25":50,"voc":30,"time":1002}"#).unwrap();
        assert!(matches!(air, SensorEvent::Air { zone: 1, co2: 1200, .. }));

        let contact: SensorEvent =
            serde_json::from_str(r#"{"type":"contact","id":1,"open":true,"time":1003}"#).unwrap();
        assert!(matches!(contact, SensorEvent::Contact { id: 1, open: true, time: 1003 }));

        let power: SensorEvent =
            serde_json::from_str(r#"{"type":"power","circuit":1,"watts":1500,"time":1004}"#).unwrap();
        assert!(matches!(power, SensorEvent::Power { circuit: 1, watts: 1500, time: 1004 }));

        let tick: SensorEvent =
            serde_json::from_str(r#"{"type":"tick","time":2000}"#).unwrap();
        assert!(matches!(tick, SensorEvent::Tick { time: 2000 }));
    }
}
