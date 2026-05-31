//! Host shell component (SWARCH-WOHL-008 C4 + C4b) — provides the `matter-ports`
//! seam (modelled on spar's interface): the transport packet queues AND the
//! monotonic clock the verified core's embassy-time driver reads across the WIT
//! boundary.
//!
//! Two packet queues (channel 0 = to-device, 1 = to-controller). Single-threaded
//! wasip2, so a thread_local RefCell holds the queues and a OnceLock pins the
//! clock epoch. Uses the rule-generated bindings (`wohl_matter_transport_bindings`).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::sync::OnceLock;
use std::time::Instant;

use wohl_matter_transport_bindings::exports::wohl::matter_compose::matter_ports::Guest;

thread_local! {
    static QUEUES: RefCell<[VecDeque<Vec<u8>>; 2]> =
        RefCell::new([VecDeque::new(), VecDeque::new()]);
    // xorshift64 PRNG state for the entropy seam. A fixed seed keeps the
    // composed handshake reproducible; the point of C4c is that the bytes
    // CROSS the WIT boundary, not that they are cryptographically strong
    // (SPAKE2+ only needs distinct valid scalars to complete).
    static RNG: RefCell<u64> = const { RefCell::new(0x9E37_79B9_7F4A_7C15) };
}

static START: OnceLock<Instant> = OnceLock::new();

struct Component;

impl Guest for Component {
    fn on_message_in(channel: u8) -> Option<Vec<u8>> {
        QUEUES.with(|q| q.borrow_mut()[channel as usize].pop_front())
    }

    fn emit_message_out(channel: u8, data: Vec<u8>) {
        QUEUES.with(|q| q.borrow_mut()[channel as usize].push_back(data));
    }

    fn on_clock_in() -> u64 {
        let start = *START.get_or_init(Instant::now);
        start.elapsed().as_micros() as u64
    }

    fn on_entropy_in(len: u32) -> Vec<u8> {
        RNG.with(|s| {
            let mut x = *s.borrow();
            let mut out = Vec::with_capacity(len as usize);
            while out.len() < len as usize {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                out.extend_from_slice(&x.to_le_bytes());
            }
            out.truncate(len as usize);
            *s.borrow_mut() = x;
            out
        })
    }
}

wohl_matter_transport_bindings::export!(Component with_types_in wohl_matter_transport_bindings);
