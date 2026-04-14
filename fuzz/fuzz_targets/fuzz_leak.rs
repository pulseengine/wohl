#![no_main]
use libfuzzer_sys::fuzz_target;
use wohl_leak::engine::*;

fuzz_target!(|data: &[u8]| {
    if data.len() < 10 { return; }
    let mut det = LeakDetector::new();
    // Register a few zones
    for i in 0..core::cmp::min(data.len() / 10, MAX_ZONES) {
        let offset = i * 10;
        let zone_id = u32::from_le_bytes([data[offset], data[offset+1], data[offset+2], data[offset+3]]);
        det.register_zone(zone_id);
    }
    // Process events from remaining bytes
    let mut offset = (data.len() / 10) * 10;
    while offset + 5 <= data.len() {
        let zone_id = u32::from_le_bytes([data[offset], data[offset+1], data[offset+2], data[offset+3]]);
        let wet = data[offset + 4] > 127;
        let _ = det.process_event(zone_id, wet, offset as u64);
        offset += 5;
    }
});
