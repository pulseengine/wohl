//! Wohl door-sensor firmware binary — STM32G031.
//!
//! Wakes from reset, configures clocks, GPIO, SysTick and USART1, then
//! polls the reed switch every SysTick tick (~1 ms). Every confirmed
//! debounced edge produces a CCSDS sensor packet and transmits it over
//! USART1 at 115200 8N1.
//!
//! All MCU-specific code lives in this file. The library half of the
//! crate (in `lib.rs`) has no HAL dependency and is unit-tested on the
//! host. See `boards/stm32g0/README.md` for pin mapping.
//!
//! This binary is only meaningfully built for `thumbv6m-none-eabi`.
//! Host targets emit a tiny stub so `cargo build` / `cargo clippy` work
//! without requiring `--target`.

// `no_std`/`no_main` only apply on bare-metal targets; on host targets
// the binary is a regular Rust program with `fn main()` so that
// `cargo test`, `cargo clippy`, and `cargo build` (without `--target`)
// all work without nightly or `-Zbuild-std`.
#![cfg_attr(target_os = "none", no_std)]
#![cfg_attr(target_os = "none", no_main)]
#![cfg_attr(not(target_os = "none"), allow(dead_code))]

// ── Host-target stub ───────────────────────────────────────────────
#[cfg(not(target_os = "none"))]
fn main() {}

// ── Real firmware ──────────────────────────────────────────────────
#[cfg(target_os = "none")]
mod firmware {
    use core::fmt::Write as _;

    use cortex_m_rt::entry;
    use nb::block;
    use panic_halt as _;
    use stm32g0xx_hal::{prelude::*, serial::FullConfig, stm32};

    use wohl_fw_door::ccsds::PACKET_SIZE;
    use wohl_fw_door::debounce::DoorLevel;
    use wohl_fw_door::door::DoorState;

    /// CCSDS APID for this node. One device per APID, set per build.
    /// Provisioning will turn this into a flash-resident value later.
    const DEVICE_ID: u16 = 0x012;
    /// Zone identifier (matches Wohl deployment YAMLs in `artifacts/`).
    const ZONE_ID: u16 = 0x0103;

    #[entry]
    fn main() -> ! {
        // ── Peripheral acquisition ──────────────────────────────────
        let dp = stm32::Peripherals::take().expect("device peripherals already taken");
        let cp = cortex_m::Peripherals::take().expect("core peripherals already taken");

        // Default `constrain()` leaves the chip on HSI16 (16 MHz).
        // Adequate for 115200 baud (< 0.2 % error).
        let mut rcc = dp.RCC.constrain();

        // ── GPIO ────────────────────────────────────────────────────
        let gpioa = dp.GPIOA.split(&mut rcc);
        // PA0 — reed switch, internal pull-up. High = door open.
        let reed = gpioa.pa0.into_pull_up_input();
        // PA9 / PA10 — USART1 TX / RX (AF1).
        let tx_pin = gpioa.pa9;
        let rx_pin = gpioa.pa10;

        // ── USART1 @ 115200 8N1 ────────────────────────────────────
        let usart = dp
            .USART1
            .usart(
                (tx_pin, rx_pin),
                FullConfig::default().baudrate(115200.bps()),
                &mut rcc,
            )
            .expect("USART1 init failed");
        let (mut tx, _rx) = usart.split();

        // ── SysTick-driven 1 kHz tick for the debouncer ────────────
        let mut delay = cp.SYST.delay(&mut rcc);

        // ── Door state machine ─────────────────────────────────────
        // Read initial level once at boot (pull-up has settled by now).
        let initial = DoorLevel::from_high(reed.is_high().unwrap_or(true));
        let mut state = DoorState::new(DEVICE_ID, ZONE_ID, initial);

        // Boot banner so the hub can spot a freshly-reset node.
        // Errors are ignored: a banner is best-effort, not safety.
        let _ = writeln!(
            tx,
            "wohl-fw-door boot apid=0x{:03X} zone=0x{:04X}\r",
            DEVICE_ID, ZONE_ID
        );

        loop {
            // 1 ms cadence → 50 ms debounce default = 50 samples.
            delay.delay(1u32.millis());

            let level = DoorLevel::from_high(reed.is_high().unwrap_or(true));
            if let Some(packet) = state.step(level) {
                // Push the 14 raw bytes — CCSDS is self-delimiting via
                // the length field in the header.
                for byte in &packet {
                    // `block!` busy-waits on `WouldBlock`. Hardware errors
                    // (overrun etc.) are intentionally dropped; the hub
                    // will detect a corrupted packet via CCSDS sequence.
                    let _ = block!(tx.write(*byte));
                }
                // Flush so we don't return while the holding register
                // still has bytes (matters if the loop iterates again
                // within one byte time, e.g. on a fast burst).
                let _ = block!(tx.flush());
                let _: &[u8; PACKET_SIZE] = &packet; // size sanity (compile-time).
            }
        }
    }
}
