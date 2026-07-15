#![no_main]
use libfuzzer_sys::fuzz_target;
use wohl_air::engine::*;

// Fuzz the air-quality monitor: arbitrary thresholds + a stream of CO2/PM2.5/VOC
// readings. Invariant: alert_count never exceeds the fixed result buffer.
fuzz_target!(|data: &[u8]| {
    if data.len() < 28 {
        return;
    }
    let mut mon = AirMonitor::new();
    let zone_id = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    mon.register_zone(AirConfig {
        zone_id,
        co2_warn: u32::from_le_bytes([data[4], data[5], data[6], data[7]]),
        co2_critical: u32::from_le_bytes([data[8], data[9], data[10], data[11]]),
        pm25_warn: u32::from_le_bytes([data[12], data[13], data[14], data[15]]),
        pm25_critical: u32::from_le_bytes([data[16], data[17], data[18], data[19]]),
        voc_warn: u32::from_le_bytes([data[20], data[21], data[22], data[23]]),
        voc_critical: u32::from_le_bytes([data[24], data[25], data[26], data[27]]),
        enabled: true,
    });

    let mut offset = 28;
    let mut time: u64 = 0;
    while offset + 14 <= data.len() {
        let co2 = u32::from_le_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]]);
        let pm25 = u32::from_le_bytes([data[offset + 4], data[offset + 5], data[offset + 6], data[offset + 7]]);
        let voc = u32::from_le_bytes([data[offset + 8], data[offset + 9], data[offset + 10], data[offset + 11]]);
        let dt = u16::from_le_bytes([data[offset + 12], data[offset + 13]]) as u64;
        time = time.wrapping_add(dt);
        let r = mon.process_reading(AirReading { zone_id, co2_ppm: co2, pm25, voc_index: voc, time });
        assert!(r.alert_count as usize <= MAX_ALERTS_PER_READING);
        offset += 14;
    }
});
