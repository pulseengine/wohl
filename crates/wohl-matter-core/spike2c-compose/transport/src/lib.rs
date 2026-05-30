//! Host transport shell component (Spike 2c) — provides the `wire` seam.
//!
//! Owns two packet queues (channel 0 = to-device, 1 = to-controller). The
//! verified Matter core pushes/pops across the wac-composed WIT boundary;
//! every PASE packet traverses this component. Single-threaded wasip2, so a
//! plain RefCell holds the state.

wit_bindgen::generate!({ world: "transport", path: "../wit" });

use std::cell::RefCell;
use std::collections::VecDeque;

thread_local! {
    static QUEUES: RefCell<[VecDeque<Vec<u8>>; 2]> =
        RefCell::new([VecDeque::new(), VecDeque::new()]);
}

struct Component;

impl exports::wohl::matter_compose::wire::Guest for Component {
    fn push(channel: u8, data: Vec<u8>) {
        QUEUES.with(|q| q.borrow_mut()[channel as usize].push_back(data));
    }
    fn pop(channel: u8) -> Option<Vec<u8>> {
        QUEUES.with(|q| q.borrow_mut()[channel as usize].pop_front())
    }
    fn peek(channel: u8) -> bool {
        QUEUES.with(|q| !q.borrow()[channel as usize].is_empty())
    }
}

export!(Component);
