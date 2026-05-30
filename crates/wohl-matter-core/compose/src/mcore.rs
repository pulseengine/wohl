//! Verified Matter core component (bazel / rules_wasm_component landing of the
//! locally-proven spike2c-compose `mcore`; SWARCH-WOHL-008 C4).
//!
//! Imports the `wire` seam and exports `runner.run`, which drives the same full
//! SPAKE2+ PASE handshake as Spike 2a/2c with every packet crossing the
//! wac-composed WIT boundary into the transport shell. Returns true on success.
//! A CI wasmtime step invokes `run` on the composed graph.
//!
//! Uses the rule-generated bindings (`wohl_matter_core_composed_bindings`),
//! not `wit_bindgen::generate!` — matching the existing components' convention.
//! wasi p2 (sync seam funcs), so the cross-component calls are ordinary
//! synchronous imports from inside rs-matter's async `poll_fn`, busy-polled by
//! `embassy_futures::block_on`.

use core::future::poll_fn;
use core::task::{Poll, Waker};
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

use wohl_matter_core_composed_bindings::exports::wohl::matter_compose::runner::Guest;
use wohl_matter_core_composed_bindings::wohl::matter_compose::wire;

// ── embassy-time driver (wasip2; see Spike 2a) ──
struct WasiDriver;
static START: OnceLock<StdInstant> = OnceLock::new();
impl embassy_time_driver::Driver for WasiDriver {
    fn now(&self) -> u64 {
        let start = *START.get_or_init(StdInstant::now);
        (start.elapsed().as_nanos() * embassy_time_driver::TICK_HZ as u128 / 1_000_000_000u128)
            as u64
    }
    fn schedule_wake(&self, _at: u64, _waker: &Waker) {}
}
embassy_time_driver::time_driver_impl!(static DRIVER: WasiDriver = WasiDriver);

// ── transport endpoint backed by the imported `wire` seam ──
struct Endpoint {
    send_channel: u8,
    recv_channel: u8,
    peer_addr: Address,
}

impl NetworkSend for &Endpoint {
    async fn send_to(&mut self, data: &[u8], _addr: Address) -> Result<(), Error> {
        wire::push(self.send_channel, data); // crosses the WIT boundary
        Ok(())
    }
}

impl NetworkReceive for &Endpoint {
    async fn wait_available(&mut self) -> Result<(), Error> {
        poll_fn(|_| {
            if wire::peek(self.recv_channel) {
                Poll::Ready(Ok(()))
            } else {
                Poll::Pending
            }
        })
        .await
    }

    async fn recv_from(&mut self, buffer: &mut [u8]) -> Result<(usize, Address), Error> {
        poll_fn(|_| match wire::pop(self.recv_channel) {
            Some(data) => {
                let n = data.len();
                buffer[..n].copy_from_slice(&data);
                Poll::Ready(Ok((n, self.peer_addr)))
            }
            None => Poll::Pending,
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
    let device_ep = Endpoint {
        send_channel: 1,
        recv_channel: 0,
        peer_addr: controller_addr,
    };
    let controller_ep = Endpoint {
        send_channel: 0,
        recv_channel: 1,
        peer_addr: device_addr,
    };

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
