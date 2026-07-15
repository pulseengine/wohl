#![no_main]
use libfuzzer_sys::fuzz_target;
use wohl_door::engine::*;

// Fuzz the door watch: arbitrary night window + a stream of open/close events at
// wrapping timestamps, plus timeout checks. Invariant: alert_count never exceeds
// the fixed result buffer.
fuzz_target!(|data: &[u8]| {
    if data.len() < 10 {
        return;
    }
    let mut w = DoorWatch::new();
    let contact_id = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    let zone_id = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    w.register_contact(ContactConfig {
        contact_id,
        zone_id,
        max_open_sec: 300,
        night_start_hour: data[8] % 24,
        night_end_hour: data[9] % 24,
        enabled: true,
    });

    let mut offset = 10;
    let mut time: u64 = 0;
    while offset + 3 <= data.len() {
        let open = data[offset] > 127;
        let dt = u16::from_le_bytes([data[offset + 1], data[offset + 2]]) as u64;
        time = time.wrapping_add(dt);
        let r = w.process_event(contact_id, open, time);
        assert!(r.alert_count as usize <= MAX_ALERTS_PER_CHECK);
        let t = w.check_timeouts(time);
        assert!(t.alert_count as usize <= MAX_ALERTS_PER_CHECK);
        offset += 3;
    }
});
