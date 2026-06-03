//! Witness MC/DC fixture — the verified-core seam's availability decision.
//!
//! Mirrors `wait_available` / `recv_from` in the composed `mcore` component
//! (crates/wohl-matter-core/compose/src/mcore.rs): the transport is ready when
//! a packet is already BUFFERED, or — short-circuit — when one is INCOMING on
//! the `on-message-in` seam. The condition is packed into one runtime arg
//! (bit0 = buffered, bit1 = incoming) so the export stays single-input.
//!
//! FINDING (see README): this is a *flat* two-condition OR. rustc (opt-level=1,
//! wasm32-unknown-unknown) lowers it to a branchless bitwise OR, so `witness`
//! recovers 0 multi-condition decisions — MC/DC here is satisfied by, and
//! equivalent to, branch coverage. Only nested mixed AND/OR decisions with
//! non-mergeable conditions (e.g. the ISO leap-year rule) carry a real MC/DC
//! truth table; the seam glue has none. That is the honest structured
//! evidence, not a coverage number.

#![no_std]

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

/// Ready if a packet is buffered, or — short-circuit — if one is incoming.
#[inline(never)]
fn seam_available(state: u32) -> bool {
    let buffered = state & 1 != 0;
    let incoming = state & 2 != 0;
    buffered || incoming
}

#[unsafe(no_mangle)]
pub extern "C" fn available(state: i32) -> i32 {
    seam_available(state as u32) as i32
}
