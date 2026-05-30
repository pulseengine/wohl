//! Host transport shell component (bazel / rules_wasm_component landing of
//! the locally-proven spike2c-compose `transport` crate; SWARCH-WOHL-008 C4).
//!
//! Exports the `wire` seam: two packet queues (channel 0 = to-device,
//! 1 = to-controller). The verified Matter core pushes/pops across the
//! wac-composed WIT boundary; every PASE packet traverses this component.
//! Single-threaded wasip2, so a thread_local RefCell holds the state.
//!
//! Unlike the spike crate (which used `wit_bindgen::generate!`), this uses the
//! bindings the rule generates (`wohl_matter_transport_bindings`), matching the
//! convention of the existing monitor components.

use std::cell::RefCell;
use std::collections::VecDeque;

use wohl_matter_transport_bindings::exports::wohl::matter_compose::wire::Guest;

thread_local! {
    static QUEUES: RefCell<[VecDeque<Vec<u8>>; 2]> =
        RefCell::new([VecDeque::new(), VecDeque::new()]);
}

struct Component;

impl Guest for Component {
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

wohl_matter_transport_bindings::export!(Component with_types_in wohl_matter_transport_bindings);
