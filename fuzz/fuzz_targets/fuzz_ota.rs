#![no_main]
use libfuzzer_sys::fuzz_target;
use wohl_ota::engine::*;

// Fuzz the dual-bank OTA state machine: arbitrary start slot + manifest, then a
// stream of chunk writes and a finish, driving the download path. Goal: no panic
// on any input, and the active slot is never changed by the download path alone
// (a swap is a separate, explicit step) — the OTA-P01 shape.
fuzz_target!(|data: &[u8]| {
    if data.len() < 9 {
        return;
    }
    let start = if data[0] & 1 == 0 { Slot::A } else { Slot::B };
    let mut core = OtaCore::new(start);
    let active_before = core.active_slot();

    let manifest = OtaManifest {
        version: u32::from_le_bytes([data[1], data[2], data[3], data[4]]),
        size_bytes: u32::from_le_bytes([data[5], data[6], data[7], data[8]]),
        sha256: [0u8; 32],
        signature: [0u8; 64],
    };
    let _ = core.start_download(manifest);

    let mut offset = 9;
    while offset + 4 <= data.len() {
        let chunk = u32::from_le_bytes([data[offset], data[offset + 1], data[offset + 2], data[offset + 3]]);
        let _ = core.write_chunk(chunk);
        offset += 4;
    }
    let _ = core.finish_download();

    // OTA-P01: the download path never flips the active slot (only an explicit
    // swap does). Kani proves this bounded; fuzz guards it on arbitrary streams.
    assert_eq!(core.active_slot(), active_before);
});
