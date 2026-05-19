//! Wohl door/window sensor firmware — pure-logic library.
//!
//! This crate is split in two:
//!
//! - **`lib.rs`** (this file) — pure-Rust, `no_std`, `no_alloc` modules:
//!     - [`ccsds`] — 14-byte CCSDS sensor wire encoder (byte-identical
//!       to `relay-ccsds::sensor_wire::encode_packet`).
//!     - [`debounce`] — generic edge-debouncer for the reed switch.
//!     - [`door`] — high-level state machine that turns debounced edges
//!       into [`ccsds::SensorPacket`]s with a monotonic sequence counter.
//!
//!   These modules have no MCU-specific code, build on any target, and
//!   are unit-tested on the host (`cargo test -p wohl-fw-door`).
//! - **`main.rs`** — the actual firmware binary. Wires GPIO + SysTick +
//!   USART1 of an STM32G031 into the modules above. Gated behind
//!   `#[cfg(target_os = "none")]` so `cargo test` on the host skips it.
//!
//! The crate is `no_std` / `no_alloc` end-to-end (see Wohl `CLAUDE.md`,
//! "Rules"). The library never panics on valid input; encoder writes
//! into a caller-provided fixed-size buffer.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]

pub mod ccsds;
pub mod debounce;
pub mod door;
