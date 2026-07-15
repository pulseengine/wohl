#![no_main]
use libfuzzer_sys::fuzz_target;
use wohl_power::engine::*;

// Fuzz the power meter: arbitrary circuit limits + a stream of wattage readings
// (exercises over-consumption + spike detection). Invariant: alert_count never
// exceeds the fixed result buffer.
fuzz_target!(|data: &[u8]| {
    if data.len() < 16 {
        return;
    }
    let mut mon = PowerMonitor::new();
    let circuit_id = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    mon.register_circuit(CircuitConfig {
        circuit_id,
        max_watts: u32::from_le_bytes([data[4], data[5], data[6], data[7]]),
        idle_watts: u32::from_le_bytes([data[8], data[9], data[10], data[11]]),
        spike_threshold: u32::from_le_bytes([data[12], data[13], data[14], data[15]]),
        enabled: true,
    });

    let mut offset = 16;
    let mut time: u64 = 0;
    while offset + 6 <= data.len() {
        let watts = u32::from_le_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]]);
        let dt = u16::from_le_bytes([data[offset + 4], data[offset + 5]]) as u64;
        time = time.wrapping_add(dt);
        let r = mon.process_reading(circuit_id, watts, time);
        assert!(r.alert_count as usize <= MAX_ALERTS_PER_READING);
        let _ = mon.check_idle(circuit_id, watts);
        offset += 6;
    }
});
