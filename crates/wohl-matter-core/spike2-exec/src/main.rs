//! Spike 2a — prove rs-matter's PASE handshake EXECUTES under wasmtime (wasip2).
//!
//! Mirrors rs-matter's own `tests/pase.rs` (a full SPAKE2+ PASE handshake
//! between an in-process initiator and a SecureChannel responder), but with two
//! changes that make it wasm32-wasip2-buildable:
//!
//!   1. The localhost UDP socket pair (async-io / `os`) is replaced with an
//!      in-memory loopback `NetworkSend`/`NetworkReceive` pipe — pure Rust, no
//!      sockets, no `os` feature.
//!   2. An `embassy-time` driver is supplied here (the `os` feature normally
//!      provides `embassy-time/std`). rs-matter calls `Instant::now()` /
//!      `Timer::after()` internally — including inside the PASE handshake — so a
//!      driver is mandatory. `embassy_futures::block_on` busy-polls with a
//!      no-op waker and `Timer::poll` re-checks `now()` every poll, so a driver
//!      with a real `now()` (wasi monotonic clock via std) + a no-op
//!      `schedule_wake` is sufficient: real time advances during the busy-loop.
//!
//! Success criterion: the initiator's `PaseInitiator::initiate(...)` returns
//! `Ok` — which requires the complete pake1/pake2/pake3 round-trip through the
//! responder — and we print `PASE-RUNS-OK`. That is the executable proof for
//! SWARCH-WOHL-008 that the verified Matter protocol+crypto core RUNS in wasm.

use core::cell::RefCell;
use core::future::poll_fn;
use core::task::{Poll, Waker};
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::Instant as StdInstant;

use embassy_futures::block_on;
use embassy_futures::select::{select, Either};

use rs_matter::crypto::test_only_crypto;
use rs_matter::dm::devices::test::{TEST_DEV_ATT, TEST_DEV_COMM, TEST_DEV_DET};
use rs_matter::error::Error;
use rs_matter::respond::Responder;
use rs_matter::sc::pase::{PaseInitiator, MAX_COMM_WINDOW_TIMEOUT_SECS};
use rs_matter::sc::SecureChannel;
use rs_matter::transport::exchange::Exchange;
use rs_matter::transport::network::{
    Address, IpAddr, Ipv6Addr, NetworkReceive, NetworkSend, NoNetwork, SocketAddr,
};
use rs_matter::utils::select::Coalesce;
use rs_matter::Matter;

// ───────────────────────── embassy-time driver (wasip2) ─────────────────────
//
// `now()` reads wasm32-wasip2's monotonic clock through `std::time::Instant`
// (backed by wasi:clocks/monotonic-clock). Scaled to embassy's selected
// `TICK_HZ` so it is correct regardless of which tick-rate feature is active.

struct WasiDriver;

static START: OnceLock<StdInstant> = OnceLock::new();

impl embassy_time_driver::Driver for WasiDriver {
    fn now(&self) -> u64 {
        let start = *START.get_or_init(StdInstant::now);
        let ns = start.elapsed().as_nanos();
        (ns * embassy_time_driver::TICK_HZ as u128 / 1_000_000_000u128) as u64
    }

    fn schedule_wake(&self, _at: u64, _waker: &Waker) {
        // No-op: block_on busy-polls and Timer re-checks now() each poll, so
        // timers elapse against the real wasi clock without explicit wakeups.
    }
}

embassy_time_driver::time_driver_impl!(static DRIVER: WasiDriver = WasiDriver);

// ───────────────────────── in-memory loopback transport ─────────────────────
//
// One packet queue per direction. `send_to` pushes (bytes, my_addr) into the
// PEER's inbox; `recv_from` pops from MY inbox and reports the sender address.
// Interior mutability (RefCell) lets the traits be implemented on `&Endpoint`,
// matching rs-matter's `matter.run(&sock, &sock, ...)` shared-ref usage. !Send
// is fine — block_on is single-threaded.

type Inbox = Rc<RefCell<VecDeque<(Vec<u8>, Address)>>>;

struct Endpoint {
    inbox: Inbox,
    peer: Inbox,
    my_addr: Address,
}

impl NetworkSend for &Endpoint {
    async fn send_to(&mut self, data: &[u8], _addr: Address) -> Result<(), Error> {
        self.peer.borrow_mut().push_back((data.to_vec(), self.my_addr));
        Ok(())
    }
}

impl NetworkReceive for &Endpoint {
    async fn wait_available(&mut self) -> Result<(), Error> {
        poll_fn(|_| {
            if self.inbox.borrow().is_empty() {
                Poll::Pending
            } else {
                Poll::Ready(Ok(()))
            }
        })
        .await
    }

    async fn recv_from(&mut self, buffer: &mut [u8]) -> Result<(usize, Address), Error> {
        poll_fn(|_| {
            let mut q = self.inbox.borrow_mut();
            if let Some((data, addr)) = q.pop_front() {
                let n = data.len();
                buffer[..n].copy_from_slice(&data);
                Poll::Ready(Ok((n, addr)))
            } else {
                Poll::Pending
            }
        })
        .await
    }
}

fn addr(port: u16) -> Address {
    Address::Udp(SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), port))
}

fn main() {
    let code = block_on(run());
    std::process::exit(code);
}

async fn run() -> i32 {
    let device_matter = Matter::new(&TEST_DEV_DET, TEST_DEV_COMM, &TEST_DEV_ATT, 0);
    let controller_matter = Matter::new(&TEST_DEV_DET, TEST_DEV_COMM, &TEST_DEV_ATT, 0);
    let crypto = test_only_crypto();

    let device_inbox: Inbox = Rc::new(RefCell::new(VecDeque::new()));
    let controller_inbox: Inbox = Rc::new(RefCell::new(VecDeque::new()));
    let device_addr = addr(5540);
    let controller_addr = addr(5541);

    let device_ep = Endpoint {
        inbox: device_inbox.clone(),
        peer: controller_inbox.clone(),
        my_addr: device_addr,
    };
    let controller_ep = Endpoint {
        inbox: controller_inbox.clone(),
        peer: device_inbox.clone(),
        my_addr: controller_addr,
    };

    // Open the commissioning window so the device accepts PBKDFParamRequest.
    if let Err(e) = device_matter.open_basic_comm_window(MAX_COMM_WINDOW_TIMEOUT_SECS, &crypto, &()) {
        eprintln!("open_basic_comm_window failed: {e:?}");
        return 3;
    }

    // Device: transport loop + SecureChannel responder.
    let sc = SecureChannel::new(&crypto, &());
    let responder = Responder::new("device", sc, &device_matter, 0);
    let device_fut = async {
        select(
            device_matter.run(&crypto, &device_ep, &device_ep, NoNetwork),
            responder.run::<4>(),
        )
        .coalesce()
        .await
    };

    // Controller: transport loop + PASE initiator. The handshake completing Ok
    // requires the full exchange to have round-tripped through the responder.
    let controller_fut = async {
        let transport =
            controller_matter.run(&crypto, &controller_ep, &controller_ep, NoNetwork);
        let handshake = async {
            let mut exchange =
                Exchange::initiate_unsecured(&controller_matter, &crypto, device_addr).await?;
            PaseInitiator::initiate(&mut exchange, &crypto, 20202021).await?;
            Ok::<(), Error>(())
        };
        match select(transport, handshake).await {
            Either::First(r) => {
                eprintln!("controller transport exited before handshake: {r:?}");
                Err::<(), Error>(rs_matter::error::ErrorCode::Invalid.into())
            }
            Either::Second(r) => r,
        }
    };

    match select(device_fut, controller_fut).await {
        Either::First(r) => {
            eprintln!("device side exited before controller finished: {r:?}");
            2
        }
        Either::Second(Ok(())) => {
            println!("PASE-RUNS-OK: full SPAKE2+ handshake completed under wasmtime (wasip2)");
            0
        }
        Either::Second(Err(e)) => {
            eprintln!("PASE-FAILED: {e:?}");
            1
        }
    }
}
