//! Verified Matter core component (SWARCH-WOHL-008 C4 + C4b).
//!
//! Imports the host seam (`matter-ports`, modelled on spar's interface) and
//! exports `runner.run`, which drives the same full SPAKE2+ PASE handshake as
//! Spike 2a/2c with every packet crossing the wac-composed WIT boundary.
//!
//! C4b: the host CLOCK now also crosses the seam. The embassy-time driver's
//! `now()` reads `on-clock-in` (provided by the host-shell component) instead
//! of std::time — so two of the three host-bound dependencies (transport +
//! clock) are genuinely composed across the WIT boundary. Entropy is the
//! remaining seam (SWV-MATTER-002 C4b).
//!
//! Uses the rule-generated bindings (`wohl_matter_core_composed_bindings`).
//! wasi p2 (sync seam funcs), busy-polled by `embassy_futures::block_on`.

use core::cell::RefCell;
use core::future::poll_fn;
use core::task::{Poll, Waker};

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

use wohl_matter_core_composed_bindings::exports::wohl::matter_compose::runner::Guest;
use wohl_matter_core_composed_bindings::wohl::matter_compose::matter_ports;

// ── embassy-time driver: time crosses the seam (C4b) ──
// now() reads host monotonic µs from the imported on-clock-in, scaled to
// embassy's selected TICK_HZ. schedule_wake is a no-op: block_on busy-polls
// and Timer::poll re-checks now() each poll.
struct HostClock;
impl embassy_time_driver::Driver for HostClock {
    fn now(&self) -> u64 {
        let micros = matter_ports::on_clock_in() as u128;
        (micros * embassy_time_driver::TICK_HZ as u128 / 1_000_000u128) as u64
    }
    fn schedule_wake(&self, _at: u64, _waker: &Waker) {}
}
embassy_time_driver::time_driver_impl!(static DRIVER: HostClock = HostClock);

// ── transport endpoint backed by the imported matter-ports seam ──
// on-message-in consumes (no peek), so wait_available buffers one packet.
struct Endpoint {
    send_channel: u8,
    recv_channel: u8,
    peer_addr: Address,
    buf: RefCell<Option<Vec<u8>>>,
}

impl Endpoint {
    fn new(send_channel: u8, recv_channel: u8, peer_addr: Address) -> Self {
        Self {
            send_channel,
            recv_channel,
            peer_addr,
            buf: RefCell::new(None),
        }
    }
}

impl NetworkSend for &Endpoint {
    async fn send_to(&mut self, data: &[u8], _addr: Address) -> Result<(), Error> {
        matter_ports::emit_message_out(self.send_channel, data); // crosses the WIT seam
        Ok(())
    }
}

impl NetworkReceive for &Endpoint {
    async fn wait_available(&mut self) -> Result<(), Error> {
        poll_fn(|_| {
            if self.buf.borrow().is_some() {
                return Poll::Ready(Ok(()));
            }
            match matter_ports::on_message_in(self.recv_channel) {
                Some(d) => {
                    *self.buf.borrow_mut() = Some(d);
                    Poll::Ready(Ok(()))
                }
                None => Poll::Pending,
            }
        })
        .await
    }

    async fn recv_from(&mut self, buffer: &mut [u8]) -> Result<(usize, Address), Error> {
        poll_fn(|_| {
            let pending = self.buf.borrow_mut().take();
            let data = match pending {
                Some(d) => Some(d),
                None => matter_ports::on_message_in(self.recv_channel),
            };
            match data {
                Some(d) => {
                    let n = d.len();
                    buffer[..n].copy_from_slice(&d);
                    Poll::Ready(Ok((n, self.peer_addr)))
                }
                None => Poll::Pending,
            }
        })
        .await
    }
}

fn addr(port: u16) -> Address {
    Address::Udp(SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), port))
}

async fn run_handshake() -> bool {
    let device_matter = Matter::new(&TEST_DEV_DET, TEST_DEV_COMM, &TEST_DEV_ATT, 0);
    let controller_matter = Matter::new(&TEST_DEV_DET, TEST_DEV_COMM, &TEST_DEV_ATT, 0);
    let crypto = test_only_crypto();

    let device_addr = addr(5540);
    let controller_addr = addr(5541);
    // channel 0 = packets to device, channel 1 = packets to controller
    let device_ep = Endpoint::new(1, 0, controller_addr);
    let controller_ep = Endpoint::new(0, 1, device_addr);

    if device_matter
        .open_basic_comm_window(MAX_COMM_WINDOW_TIMEOUT_SECS, &crypto, &())
        .is_err()
    {
        return false;
    }

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
            Either::First(_) => Err::<(), Error>(rs_matter::error::ErrorCode::Invalid.into()),
            Either::Second(r) => r,
        }
    };

    matches!(
        select(device_fut, controller_fut).await,
        Either::Second(Ok(()))
    )
}

struct Component;

impl Guest for Component {
    fn run() -> bool {
        block_on(run_handshake())
    }
}

wohl_matter_core_composed_bindings::export!(Component with_types_in wohl_matter_core_composed_bindings);
