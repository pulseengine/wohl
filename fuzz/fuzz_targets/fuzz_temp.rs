#![no_main]
use libfuzzer_sys::fuzz_target;
use wohl_temp::engine::*;

fuzz_target!(|data: &[u8]| {
    if data.len() < 16 { return; }
    let mut mon = TemperatureMonitor::new();

    // Parse zone config from first 16 bytes
    let freeze = i32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    let overheat = i32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    let rate = i32::from_le_bytes([data[8], data[9], data[10], data[11]]);
    let zone_id = u32::from_le_bytes([data[12], data[13], data[14], data[15]]);

    mon.register_zone(ZoneConfig {
        zone_id,
        freeze_threshold: freeze,
        overheat_threshold: overheat,
        rate_threshold: rate,
        enabled: true,
    });

    // Process readings from remaining bytes (6 bytes each: 4 for value, 2 for time offset)
    let mut offset = 16;
    let mut time: u64 = 0;
    while offset + 6 <= data.len() {
        let value = i32::from_le_bytes([data[offset], data[offset+1], data[offset+2], data[offset+3]]);
        let dt = u16::from_le_bytes([data[offset+4], data[offset+5]]) as u64;
        time = time.wrapping_add(dt);
        let result = mon.process_reading(zone_id, value, time);
        assert!(result.alert_count as usize <= MAX_ALERTS_PER_READING);
        offset += 6;
    }
});
