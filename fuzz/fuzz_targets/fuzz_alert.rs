#![no_main]
use libfuzzer_sys::fuzz_target;
use wohl_alert::engine::*;

// Fuzz the alert dispatcher: subscribe a spread of (zone, type) keys, then drive
// process_alert with arbitrary keys at wrapping timestamps (exercises dedup +
// rate-limit). Invariant: the recent-window depth never exceeds its bound.
fuzz_target!(|data: &[u8]| {
    if data.len() < 4 {
        return;
    }
    let mut d = AlertDispatcher::new();
    for z in 0u32..4 {
        for t in 0u8..4 {
            d.subscribe(z, t, 1);
        }
    }

    let mut offset = 0;
    let mut time: u64 = 0;
    while offset + 4 <= data.len() {
        let zone = (data[offset] % 4) as u32;
        let atype = data[offset + 1] % 4;
        let dt = u16::from_le_bytes([data[offset + 2], data[offset + 3]]) as u64;
        time = time.wrapping_add(dt);
        let r = d.process_alert(zone, atype, time);
        assert!(r.queue_depth as usize <= MAX_RECENT_ALERTS);
        offset += 4;
    }
});
