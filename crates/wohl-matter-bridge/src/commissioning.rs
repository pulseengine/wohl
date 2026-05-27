//! Matter commissioning loop on the wohl hub.
//!
//! Adapts the canonical `examples/src/bin/onoff_light.rs` from rs-matter
//! upstream, stripping the application clusters (we only need the root
//! endpoint for commissioning — the Matter Bridge device type and
//! cluster fan-out is the next slice's work).
//!
//! Lifecycle:
//!
//! 1. [`start`] spawns a dedicated OS thread with a ≥550 KB stack (the
//!    `onoff_light` example's empirically-found minimum for rs-matter
//!    running at `opt-level = 3`; at lower opt levels the figure can be
//!    several MB). The thread takes ownership of the BSS-allocated
//!    `Matter` instance and runs the rs-matter futures via
//!    `futures_lite::future::block_on`.
//!
//! 2. The thread function [`run_matter`] constructs the Matter device
//!    (`Matter::init`), loads any persisted fabric via `load_persist`,
//!    binds the IPv6 UDP socket, opens a basic commissioning window if
//!    not commissioned, and `select`s over the transport future, mDNS
//!    future, responder future, and data-model background job.
//!
//! 3. The thread runs until any of those futures resolves with an error
//!    (or forever, in the steady-state case where commissioning
//!    succeeded and the hub is responding to controllers).
//!
//! Single-instance constraint: `Matter`, IM buffers, subscriptions, and
//! the KV scratch buffer live in `static_cell::StaticCell` statics —
//! only ONE [`start`] call per process succeeds; subsequent calls panic
//! on the `init_with` re-entry. This matches how rs-matter is designed
//! to be embedded; running multiple Matter devices in one process is
//! out of scope for the wohl bridge.
//!
//! Platform: Linux + macOS via the rs-matter `builtin` mDNS responder
//! (no D-Bus / Avahi / Bonjour dependency). The responder enumerates
//! interfaces with `if-addrs` and joins IPv6 + IPv4 multicast groups
//! via `socket2`.

use std::net::UdpSocket;
use std::path::PathBuf;
use std::pin::pin;
use std::thread::{Builder as ThreadBuilder, JoinHandle};

use embassy_futures::select::select4;
use log::{debug, error, info, warn};

use rs_matter::crypto::Crypto;
use rs_matter::dm::IMBuffer;
use rs_matter::dm::clusters::net_comm::SharedNetworks;
use rs_matter::dm::devices::test::{DAC_PRIVKEY, TEST_DEV_ATT, TEST_DEV_COMM, TEST_DEV_DET};
use rs_matter::dm::endpoints;
use rs_matter::dm::events::NoEvents;
use rs_matter::dm::networks::SysNetifs;
use rs_matter::dm::networks::eth::EthNetwork;
use rs_matter::dm::subscriptions::Subscriptions;
use rs_matter::dm::{DataModel, DataModelHandler, Node};
use rs_matter::error::Error;
use rs_matter::pairing::DiscoveryCapabilities;
use rs_matter::pairing::qr::QrTextType;
use rs_matter::persist::{DirKvBlobStore, SharedKvBlobStore};
use rs_matter::respond::DefaultResponder;
use rs_matter::sc::pase::MAX_COMM_WINDOW_TIMEOUT_SECS;
use rs_matter::transport::MATTER_SOCKET_BIND_ADDR;
use rs_matter::utils::init::InitMaybeUninit;
use rs_matter::utils::select::Coalesce;
use rs_matter::utils::storage::pooled::PooledBuffers;
use rs_matter::{MATTER_PORT, Matter, crypto::default_crypto, root_endpoint};

use static_cell::StaticCell;

// BSS-allocated static singletons — see `onoff_light.rs` for the
// rationale. Each is initialized exactly once via the
// `StaticCell::uninit().init_with(...)` pattern; a second call would
// panic. This crate exposes a single `start_commissioning()` entry on
// `RsMatterBridge` that calls into this module — calling it twice on
// the same process is undefined behavior beyond a clean panic.
static MATTER: StaticCell<Matter> = StaticCell::new();
static BUFFERS: StaticCell<PooledBuffers<10, IMBuffer>> = StaticCell::new();
static SUBSCRIPTIONS: StaticCell<Subscriptions> = StaticCell::new();
static KV_BUF: StaticCell<[u8; 4096]> = StaticCell::new();

/// Spawn the Matter commissioning thread.
///
/// `state_dir` is where the rs-matter `DirKvBlobStore` keeps fabric
/// data; SWREQ-MATTER-004 requires fsync-durable persistence at this
/// path. The directory does not need to exist yet — `DirKvBlobStore`
/// creates it lazily on first write.
///
/// Returns a `JoinHandle` for the dedicated thread; today nothing
/// joins it — the thread runs for the lifetime of the process. A
/// future slice may add a shutdown signal.
pub fn start(state_dir: PathBuf) -> std::io::Result<JoinHandle<Result<(), Error>>> {
    info!(
        "[wohl-matter] spawning commissioning thread (state_dir={:?})",
        state_dir
    );
    // Stack-size budget per the upstream example. Increase if rs-matter's
    // future tree gets larger or if lower opt-levels are used.
    ThreadBuilder::new()
        .name("wohl-matter".into())
        .stack_size(550 * 1024)
        .spawn(move || run_matter(state_dir))
}

/// The thread body: initialize the rs-matter stack and run forever.
///
/// Mirrors `examples/src/bin/onoff_light.rs::run` minus the application
/// clusters. Returns only on a transport / mDNS / responder / data-model
/// error — steady state is "block forever, serving controllers".
fn run_matter(state_dir: PathBuf) -> Result<(), Error> {
    info!("[wohl-matter] commissioning thread up; initializing Matter device");
    info!(
        "[wohl-matter] memory: Matter (BSS)={}B, IM Buffers (BSS)={}B, Subscriptions (BSS)={}B",
        core::mem::size_of::<Matter>(),
        core::mem::size_of::<PooledBuffers<10, IMBuffer>>(),
        core::mem::size_of::<Subscriptions>()
    );

    // ── Step 1: Build the Matter instance using the *test* attestation
    // & basic-info objects. These ship with rs-matter and embed the
    // Test Vendor 1 (0xFFF1) CSA-reserved test allocation. SWARCH-WOHL-007
    // says production builds will replace these with real DAC/CD/PAI
    // chain — that's an attestation-cert PR, not commissioning scope.
    let matter = MATTER.uninit().init_with(Matter::init(
        &TEST_DEV_DET,
        TEST_DEV_COMM,
        &TEST_DEV_ATT,
        MATTER_PORT,
    ));

    // ── Step 2: Persistence. The DirKvBlobStore at the configured path
    // satisfies SWREQ-MATTER-004 (fsync-durable fabric storage). On
    // first boot the load is a no-op; on subsequent boots it replays
    // the persisted fabric set before any LAN socket is bound.
    let kv_buf = KV_BUF.uninit().init_zeroed().as_mut_slice();
    let mut kv = DirKvBlobStore::new(state_dir);
    futures_lite::future::block_on(matter.load_persist(&mut kv, kv_buf))?;
    info!(
        "[wohl-matter] persistence loaded; commissioned={}",
        matter.is_commissioned()
    );

    // ── Step 3: Construct the transport buffers & subscription table.
    let buffers = BUFFERS.uninit().init_with(PooledBuffers::init(0));
    let subscriptions = SUBSCRIPTIONS.uninit().init_with(Subscriptions::init());

    // ── Step 4: Crypto. The PRNG is per-thread (we're on our own thread
    // here). DAC_PRIVKEY is the test-vendor private key — replace with
    // a real one when attestation certs ship.
    let crypto = default_crypto(rand::thread_rng(), DAC_PRIVKEY);

    // ── Step 5: Build the data-model handler with NO application clusters.
    // The root endpoint is the only one we register; commissioning + the
    // Operational Credentials / Basic Information / General Commissioning
    // clusters live there.
    let rand = crypto.rand()?;
    let events = NoEvents::new();
    let dm = DataModel::new(
        matter,
        &crypto,
        buffers,
        subscriptions,
        &events,
        commissioning_only_handler(rand),
        SharedKvBlobStore::new(kv, kv_buf),
        SharedNetworks::new(EthNetwork::new_default()),
    );

    let responder = DefaultResponder::new(&dm);
    let mut respond = pin!(responder.run::<4, 4>());
    let mut dm_job = pin!(dm.run());

    // ── Step 6: UDP transport bound to MATTER_SOCKET_BIND_ADDR
    //         (the IPv6 ANY address on the IANA-assigned Matter port).
    let socket = async_io::Async::<UdpSocket>::bind(MATTER_SOCKET_BIND_ADDR)?;

    let mut mdns = pin!(run_builtin_mdns(matter, &crypto));
    let mut transport = pin!(matter.run(&crypto, &socket, &socket, &socket));

    // ── Step 7: If we have no commissioned fabric, print the QR code
    // and open a basic commissioning window. Once a controller
    // commissions us, this branch never runs again on subsequent boots
    // — `is_commissioned()` returns true.
    if !matter.is_commissioned() {
        warn!("[wohl-matter] no commissioned fabric — opening commissioning window");
        matter.print_standard_qr_text(DiscoveryCapabilities::IP)?;
        matter.print_standard_qr_code(QrTextType::Unicode, DiscoveryCapabilities::IP)?;
        matter.open_basic_comm_window(MAX_COMM_WINDOW_TIMEOUT_SECS, &crypto, &())?;
    } else {
        info!("[wohl-matter] fabric already commissioned — ready to serve controllers");
    }

    // ── Step 8: Run all four futures concurrently until one returns.
    // `coalesce()` turns the `Either4` result into the inner `Result`.
    let all = select4(&mut transport, &mut mdns, &mut respond, &mut dm_job).coalesce();
    info!("[wohl-matter] entering matter event loop");
    futures_lite::future::block_on(all)
}

/// The Node descriptor for the wohl bridge with only the root endpoint
/// — no application clusters yet. The root endpoint carries the
/// Operational Credentials, Basic Information, and General
/// Commissioning clusters that are required for commissioning.
///
/// Next slice replaces this with the Matter Bridge device type +
/// per-sensor bridged endpoints driven by the cluster mapping in
/// `cluster.rs`.
const NODE: Node<'static> = Node {
    endpoints: &[root_endpoint!(eth)],
};

fn commissioning_only_handler(rand: impl rand::RngCore + Copy) -> impl DataModelHandler {
    (
        NODE,
        endpoints::EthSysHandlerBuilder::new()
            .netif_diag(&SysNetifs)
            .build(rand),
    )
}

// ── Built-in mDNS responder ────────────────────────────────────────────
//
// Verbatim from the upstream `examples/src/common/mdns.rs` modulo
// formatting; we ship a built-in responder rather than D-Bus / Bonjour
// because the hub is a self-contained Linux service that shouldn't
// depend on the host's mDNS daemon.

async fn run_builtin_mdns<C: rs_matter::crypto::Crypto>(
    matter: &Matter<'_>,
    crypto: C,
) -> Result<(), Error> {
    use rs_matter::transport::network::mdns::builtin::{BuiltinMdnsResponder, Host};
    use rs_matter::transport::network::mdns::{
        MDNS_IPV4_BROADCAST_ADDR, MDNS_IPV6_BROADCAST_ADDR, MDNS_SOCKET_DEFAULT_BIND_ADDR,
    };
    use rs_matter::transport::network::{Ipv4Addr, Ipv6Addr};
    use socket2::{Domain, Protocol, Socket, Type};

    let (ipv4_addr, ipv6_addr, interface) = pick_network_interface()?;
    info!("[wohl-matter] mDNS using interface {ipv4_addr}/{ipv6_addr} (index {interface})");

    // IPv6 dual-stack UDP socket with multicast joining.
    let socket = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_reuse_address(true)?;
    socket.set_only_v6(false)?;
    socket.bind(&MDNS_SOCKET_DEFAULT_BIND_ADDR.into())?;
    let socket = async_io::Async::<UdpSocket>::new_nonblocking(socket.into())?;
    socket
        .get_ref()
        .join_multicast_v6(&MDNS_IPV6_BROADCAST_ADDR, interface)?;
    socket
        .get_ref()
        .join_multicast_v4(&MDNS_IPV4_BROADCAST_ADDR, &ipv4_addr)?;

    BuiltinMdnsResponder::new()
        .run(
            &socket,
            &socket,
            &Host {
                // The hostname is the operational MAC-like
                // identifier — rs-matter's tests use the hex form.
                // The hub's real serial / MAC plumbing is a later
                // slice's concern.
                hostname: "001122334455",
                ip: Ipv4Addr::from(ipv4_addr.octets()),
                ipv6: Ipv6Addr::from(ipv6_addr.octets()),
            },
            Some(ipv4_addr),
            Some(interface),
            matter,
            crypto,
        )
        .await
}

#[inline(never)]
fn pick_network_interface() -> Result<(std::net::Ipv4Addr, std::net::Ipv6Addr, u32), Error> {
    use rs_matter::error::ErrorCode;

    let all = if_addrs::get_if_addrs().map_err(|_| ErrorCode::StdIoError)?;
    debug!("[wohl-matter] interfaces: {:?}", all);

    let find_ipv6_candidate = |ipv6_filter: fn(std::net::Ipv6Addr) -> bool| {
        all.iter()
            .filter(|ia| !ia.is_loopback())
            .filter_map(|ia| match ia.addr {
                if_addrs::IfAddr::V6(ref v6) if ipv6_filter(v6.ip) => {
                    Some((ia.name.clone(), v6.ip, ia.index.unwrap_or(0)))
                }
                _ => None,
            })
            .find_map(|(iname, ipv6, index)| {
                all.iter()
                    .filter(|ia2| ia2.name == iname)
                    .find_map(|ia2| match ia2.addr {
                        if_addrs::IfAddr::V4(ref v4) => Some((iname.clone(), v4.ip, ipv6, index)),
                        _ => None,
                    })
            })
    };

    let find_fallback_candidate = || {
        all.iter()
            .filter(|ia| !ia.is_loopback())
            .filter(|ia| ia.name.starts_with("eth") || ia.name.starts_with("eno"))
            .map(|ia| match ia.addr {
                if_addrs::IfAddr::V4(ref v4) => (
                    ia.name.clone(),
                    v4.ip,
                    std::net::Ipv6Addr::UNSPECIFIED,
                    ia.index.unwrap_or(0),
                ),
                if_addrs::IfAddr::V6(ref v6) => (
                    ia.name.clone(),
                    std::net::Ipv4Addr::UNSPECIFIED,
                    v6.ip,
                    ia.index.unwrap_or(0),
                ),
            })
            .next()
    };

    let candidate = find_ipv6_candidate(|ip| ip.is_unicast_link_local())
        .or_else(|| find_ipv6_candidate(|_| true))
        .or_else(|| {
            warn!("[wohl-matter] no IPv6 interface; falling back to ethN/enoN with IPv4 only");
            find_fallback_candidate()
        })
        .ok_or_else(|| {
            error!("[wohl-matter] no network interface suitable for Matter mDNS broadcasting");
            ErrorCode::StdIoError
        })?;

    let (_iname, ip, ipv6, index) = candidate;
    Ok((ip, ipv6, index))
}
