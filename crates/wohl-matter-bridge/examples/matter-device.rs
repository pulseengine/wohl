//! Run the wohl Matter bridge as a real commissionable Matter device, for
//! INTEROP testing against an independent controller (chip-tool).
//!
//! This is the falsification test for the verified-core / commissioning work:
//! the CI `wasmtime --invoke 'run()'` gate proves rs-matter agrees with
//! rs-matter; this proves an INDEPENDENT implementation (the official CHIP
//! reference controller) can commission us and read an attribute. If chip-tool
//! cannot, we've found a real spec-divergence bug that CI never would.
//!
//! Build: `cargo build -p wohl-matter-bridge --features rs-matter-backend \
//!            --example matter-device`
//! The device opens a commissioning window with the standard CHIP test
//! credentials (passcode 20202021, discriminator 3840 via TEST_DEV_COMM) and
//! exposes a WaterLeakDetector (device type 0x0043) on endpoint 1 with
//! BooleanState (0x0045) StateValue, served from the attribute cache.

use std::path::PathBuf;

use wohl_matter_bridge::{RsMatterConfig, RsMatterBridge};

fn main() -> std::io::Result<()> {
    let state_dir = std::env::var("WOHL_MATTER_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("wohl-matter-interop"));
    std::fs::create_dir_all(&state_dir)?;
    eprintln!("[matter-device] state_dir = {}", state_dir.display());

    let bridge = RsMatterBridge::new(RsMatterConfig {
        state_dir,
        ..Default::default()
    });

    eprintln!(
        "[matter-device] starting commissionable device — passcode 20202021, discriminator 3840, \
         WaterLeakDetector BooleanState on endpoint 1"
    );
    let handle = bridge.start_commissioning()?;

    match handle.join() {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => {
            eprintln!("[matter-device] device exited with error: {e:?}");
            std::process::exit(1);
        }
        Err(_) => {
            eprintln!("[matter-device] device thread panicked");
            std::process::exit(2);
        }
    }
}
