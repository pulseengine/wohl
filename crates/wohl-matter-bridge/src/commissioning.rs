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
use std::sync::Arc;
use std::thread::{Builder as ThreadBuilder, JoinHandle};

use embassy_futures::select::select4;
use log::{debug, error, info, warn};

use rs_matter::crypto::{Crypto, Rng};
use rs_matter::dm::clusters::decl::boolean_state;
use rs_matter::dm::clusters::desc::{self, ClusterHandler as _};
use rs_matter::dm::devices::test::{DAC_PRIVKEY, TEST_DEV_ATT, TEST_DEV_COMM, TEST_DEV_DET};
use rs_matter::dm::endpoints;
use rs_matter::dm::networks::SysNetifs;
use rs_matter::dm::{Async, Cluster, DataModel, Dataver, DeviceType, Endpoint, Node, ReadContext};
use rs_matter::error::Error;
use rs_matter::im::{InteractionModel, InteractionModelState};
use rs_matter::pairing::DiscoveryCapabilities;
use rs_matter::pairing::qr::QrTextType;
use rs_matter::persist::DirKvBlobStore;
use rs_matter::respond::DefaultResponder;
use rs_matter::sc::pase::MAX_COMM_WINDOW_TIMEOUT_SECS;
use rs_matter::transport::MATTER_SOCKET_BIND_ADDR;
use rs_matter::transport::exchange::MatterBuffers;
use rs_matter::utils::init::InitMaybeUninit;
use rs_matter::utils::select::Coalesce;
use rs_matter::{MATTER_PORT, Matter, clusters, crypto::default_crypto, devices, root_endpoint};

use static_cell::StaticCell;

use crate::cache::{AttributeCache, AttributeKey, AttributeValue};
use crate::cluster::MatterCluster;

/// Matter Device Type Library id for a Water Leak Detector
/// (Matter 1.2+, DTL `0x0043`). This rs-matter revision does not
/// ship the sensor device-type constants, so we define the ones we
/// need here. The mandatory server cluster for this device type is
/// BooleanState (`0x0045`) — see DESIGN.md §2.
const DEV_TYPE_WATER_LEAK_DETECTOR: DeviceType = DeviceType {
    dtype: 0x0043,
    drev: 1,
};

/// The Matter endpoint id of the single demonstration water-leak
/// endpoint this slice registers. Endpoint 0 is the root node;
/// this slice uses endpoint 1 for the water-leak detector. The
/// general wohl-zone → Matter-endpoint allocation policy is the
/// next slice (DESIGN.md §7.1).
const WATER_LEAK_ENDPOINT: u16 = 1;

// BSS-allocated static singletons — see `onoff_light.rs` for the
// rationale. Each is initialized exactly once via the
// `StaticCell::uninit().init_with(...)` pattern; a second call would
// panic. This crate exposes a single `start_commissioning()` entry on
// `RsMatterBridge` that calls into this module — calling it twice on
// the same process is undefined behavior beyond a clean panic.
static MATTER: StaticCell<Matter> = StaticCell::new();
static BUFFERS: StaticCell<MatterBuffers> = StaticCell::new();
static IM_STATE: StaticCell<InteractionModelState> = StaticCell::new();

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
pub fn start(
    state_dir: PathBuf,
    cache: Arc<AttributeCache>,
) -> std::io::Result<JoinHandle<Result<(), Error>>> {
    info!(
        "[wohl-matter] spawning commissioning thread (state_dir={:?})",
        state_dir
    );
    // Stack-size budget per the upstream example. Increase if rs-matter's
    // future tree gets larger or if lower opt-levels are used.
    ThreadBuilder::new()
        .name("wohl-matter".into())
        .stack_size(550 * 1024)
        .spawn(move || run_matter(state_dir, cache))
}

/// The thread body: initialize the rs-matter stack and run forever.
///
/// Mirrors `examples/src/bin/onoff_light.rs::run` minus the application
/// clusters. Returns only on a transport / mDNS / responder / data-model
/// error — steady state is "block forever, serving controllers".
fn run_matter(state_dir: PathBuf, cache: Arc<AttributeCache>) -> Result<(), Error> {
    info!("[wohl-matter] commissioning thread up; initializing Matter device");
    info!(
        "[wohl-matter] memory: Matter (BSS)={}B, IM Buffers (BSS)={}B, IM State (BSS)={}B",
        core::mem::size_of::<Matter>(),
        core::mem::size_of::<MatterBuffers>(),
        core::mem::size_of::<InteractionModelState>()
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
    // satisfies SWREQ-MATTER-004 (fsync-durable fabric storage). The KV
    // scratch buffer now lives inside `Matter`; `matter.startup` replays
    // the persisted fabric set before any LAN socket is bound.
    let store = DirKvBlobStore::new(state_dir);
    let kv = matter.kv(store);
    matter.startup(&kv)?;
    info!(
        "[wohl-matter] persistence loaded; commissioned={}",
        matter.has_fabrics()
    );

    // ── Step 3: Construct the transport buffers & the Interaction Model
    // state (subscription table + events queue).
    let buffers = BUFFERS.init(MatterBuffers::new());
    let state = IM_STATE.init(InteractionModelState::new());

    // ── Step 4: Crypto. The PRNG is per-thread (we're on our own thread
    // here). DAC_PRIVKEY is the test-vendor private key — replace with
    // a real one when attestation certs ship.
    let crypto = default_crypto(rand::rng(), DAC_PRIVKEY);

    // ── Step 5: Build the Interaction Model over the data-model handler.
    // The root endpoint carries the commissioning clusters; the
    // water-leak endpoint exposes BooleanState served from the cache.
    let rand = crypto.rand()?;
    let im = InteractionModel::new(
        &matter,
        &crypto,
        buffers,
        bridge_handler(rand, cache),
        &kv,
        state,
    );

    // Re-hydrate the data model's persisted state and deliver the
    // `Startup` lifecycle op to all cluster handlers.
    futures_lite::future::block_on(im.startup())?;

    let responder = DefaultResponder::new(&im);
    let mut respond = pin!(responder.run::<4, 4>());
    let mut dm_job = pin!(im.run());

    // ── Step 6: UDP transport bound to MATTER_SOCKET_BIND_ADDR
    //         (the IPv6 ANY address on the IANA-assigned Matter port).
    let socket = async_io::Async::<UdpSocket>::bind(MATTER_SOCKET_BIND_ADDR)?;

    let mut mdns = pin!(run_builtin_mdns(matter, &crypto));
    let mut transport = pin!(matter.run(&crypto, &socket, &socket, &socket));

    // ── Step 7: If we have no commissioned fabric, print the QR code
    // and open a basic commissioning window. Once a controller
    // commissions us, this branch never runs again on subsequent boots
    // — `is_commissioned()` returns true.
    if !matter.has_fabrics() {
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

/// A cache-backed BooleanState cluster handler.
///
/// Implements the rs-matter generated `boolean_state::ClusterHandler`
/// trait. When a commissioned controller reads
/// `BooleanState::StateValue` on the water-leak endpoint, rs-matter
/// calls [`Self::state_value`], which reads the latest value the
/// dispatcher pushed into the [`AttributeCache`]. This is the pull
/// side of the push/pull mediation (see `cache.rs` doc).
struct CacheBooleanState {
    dataver: Dataver,
    cache: Arc<AttributeCache>,
    /// The wohl-side endpoint id whose BooleanState this handler
    /// serves. The cache is keyed by the wohl id; the controller
    /// reads by Matter endpoint id; this field bridges the two.
    wohl_endpoint_id: u32,
}

impl CacheBooleanState {
    fn new(dataver: Dataver, cache: Arc<AttributeCache>, wohl_endpoint_id: u32) -> Self {
        Self {
            dataver,
            cache,
            wohl_endpoint_id,
        }
    }

    fn adapt(self) -> boolean_state::HandlerAdaptor<Self> {
        boolean_state::HandlerAdaptor(self)
    }

    /// Pull the latest pushed value from the cache. If nothing has
    /// been published yet (no sensor event since boot), report
    /// `false` — for a WaterLeakDetector device type, false means
    /// "no leak detected", the safe default.
    ///
    /// Pure (no `ReadContext`) so it's unit-testable without the
    /// rs-matter stack.
    fn current_state(&self) -> bool {
        let key = AttributeKey::new(
            self.wohl_endpoint_id,
            MatterCluster::BooleanState.cluster_id(),
            0x0000,
        );
        matches!(self.cache.get(key), Some(AttributeValue::Bool(true)))
    }
}

impl boolean_state::ClusterHandler for CacheBooleanState {
    const CLUSTER: Cluster<'static> = boolean_state::FULL_CLUSTER;

    fn dataver(&self) -> u32 {
        self.dataver.get()
    }

    fn dataver_changed(&self) {
        self.dataver.changed();
    }

    fn state_value(&self, _ctx: impl ReadContext) -> Result<bool, Error> {
        Ok(self.current_state())
    }
}

/// The Node descriptor: the root endpoint (for commissioning) plus a
/// single Water Leak Detector device-type endpoint exposing
/// BooleanState. This slice proves the cache → handler → controller
/// read path end-to-end for one endpoint.
///
/// Next slice: the full Matter Bridge topology (aggregator +
/// dynamically-registered bridged endpoints, one per wohl
/// zone/contact/circuit) driven by the cluster mapping in
/// `cluster.rs` and the endpoint-id allocation policy (DESIGN.md §7.1).
const NODE: Node<'static> = Node {
    endpoints: &[
        root_endpoint!(eth),
        Endpoint::new(
            WATER_LEAK_ENDPOINT,
            devices!(DEV_TYPE_WATER_LEAK_DETECTOR),
            clusters!(desc::DescHandler::CLUSTER, boolean_state::FULL_CLUSTER),
        ),
    ],
};

fn bridge_handler(mut rand: impl Rng + Copy, cache: Arc<AttributeCache>) -> impl DataModel {
    (
        NODE,
        endpoints::EthSysHandlerBuilder::new()
            .netif_diag(&SysNetifs)
            .build(rand)
            // The water-leak endpoint's descriptor cluster.
            .chain(
                |e, c| e == WATER_LEAK_ENDPOINT && c == desc::DescHandler::CLUSTER.id,
                Async(desc::DescHandler::new(Dataver::new_rand(&mut rand)).adapt()),
            )
            // The water-leak endpoint's BooleanState, served from the
            // attribute cache. wohl endpoint id == Matter endpoint id
            // for this single-endpoint demonstration.
            .chain(
                |e, c| e == WATER_LEAK_ENDPOINT && c == boolean_state::FULL_CLUSTER.id,
                Async(
                    CacheBooleanState::new(
                        Dataver::new_rand(&mut rand),
                        cache,
                        WATER_LEAK_ENDPOINT as u32,
                    )
                    .adapt(),
                ),
            ),
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
    use rs_matter::transport::network::Ipv6Addr;
    use rs_matter::transport::network::mdns::builtin::{BuiltinMdns, Host};
    use rs_matter::transport::network::mdns::{
        MDNS_IPV4_BROADCAST_ADDR, MDNS_IPV6_BROADCAST_ADDR, MDNS_SOCKET_DEFAULT_BIND_ADDR,
    };
    use socket2::{Domain, Protocol, Socket, Type};

    let (ipv4_addr, ipv6_addrs, interface) = pick_network_interface()?;
    info!(
        "[wohl-matter] mDNS using interface {ipv4_addr} ({} IPv6 addr(s), index {interface})",
        ipv6_addrs.len()
    );

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

    let ipv6: Vec<Ipv6Addr> = ipv6_addrs
        .iter()
        .map(|a| Ipv6Addr::from(a.octets()))
        .collect();

    BuiltinMdns::new()
        .run(
            &socket,
            &socket,
            &Host {
                // The hostname is the operational MAC-like
                // identifier — rs-matter's tests use the hex form.
                // The hub's real serial / MAC plumbing is a later
                // slice's concern.
                hostname: "001122334455",
                ip: rs_matter::transport::network::Ipv4Addr::from(ipv4_addr.octets()),
                ipv6: &ipv6,
            },
            Some(ipv4_addr),
            Some(interface),
            matter,
            crypto,
        )
        .await
}

// Wired-first ordering, but no hard name allowlist: runners and hubs name
// their primary interface eth0, eno1, ens5, enp3s0 or wlan0 depending on
// image and kernel (#81 — a CI image with neither IPv6 nor an eth*/eno*
// name made startup fail here).
fn fallback_interface_rank(name: &str) -> u8 {
    match name {
        n if n.starts_with("eth") || n.starts_with("en") => 0,
        n if n.starts_with("wl") => 1,
        _ => 2,
    }
}

#[inline(never)]
fn pick_network_interface() -> Result<(std::net::Ipv4Addr, Vec<std::net::Ipv6Addr>, u32), Error> {
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

    let fallback_rank = fallback_interface_rank;
    let find_fallback_candidate = || {
        all.iter()
            .filter(|ia| !ia.is_loopback())
            .min_by_key(|ia| {
                // Prefer an IPv4 address row; this branch is only reached
                // when no IPv6 candidate exists.
                let v6_penalty = matches!(ia.addr, if_addrs::IfAddr::V6(_)) as u8;
                (
                    fallback_rank(&ia.name),
                    v6_penalty,
                    ia.index.unwrap_or(u32::MAX),
                )
            })
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
    };

    let candidate = find_ipv6_candidate(|ip| ip.is_unicast_link_local())
        .or_else(|| find_ipv6_candidate(|_| true))
        .or_else(|| {
            warn!("[wohl-matter] no IPv6 interface; falling back to any non-loopback interface, wired-first");
            find_fallback_candidate()
        })
        .ok_or_else(|| {
            error!("[wohl-matter] no network interface suitable for Matter mDNS broadcasting");
            ErrorCode::StdIoError
        })?;

    let (iname, ip, _ipv6, index) = candidate;
    // The responder's `Host` carries every IPv6 address of the chosen
    // interface (the upstream example does the same), so collect them all
    // rather than only the one that selected the interface.
    let ipv6s: Vec<std::net::Ipv6Addr> = all
        .iter()
        .filter(|ia| ia.name == iname)
        .filter_map(|ia| match ia.addr {
            if_addrs::IfAddr::V6(ref v6) => Some(v6.ip),
            _ => None,
        })
        .collect();
    Ok((ip, ipv6s, index))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handler(cache: Arc<AttributeCache>, wohl_endpoint_id: u32) -> CacheBooleanState {
        CacheBooleanState::new(Dataver::new(1), cache, wohl_endpoint_id)
    }

    #[test]
    fn fallback_rank_prefers_wired_then_wireless_then_anything() {
        // #81: ens5/enp3s0 (cloud/PCIe naming) must rank with eth0/eno1,
        // ahead of wlan0, and nothing may be excluded outright.
        for wired in ["eth0", "eno1", "ens5", "enp3s0"] {
            assert_eq!(fallback_interface_rank(wired), 0, "{wired}");
        }
        assert_eq!(fallback_interface_rank("wlan0"), 1);
        assert_eq!(fallback_interface_rank("wlp2s0"), 1);
        for other in ["podman0", "docker0", "tailscale0", "utun3"] {
            assert_eq!(fallback_interface_rank(other), 2, "{other}");
        }
    }

    #[test]
    fn current_state_false_when_cache_empty() {
        let cache = Arc::new(AttributeCache::new());
        let h = handler(cache, 1);
        assert!(
            !h.current_state(),
            "no published value → BooleanState false (no leak)"
        );
    }

    #[test]
    fn current_state_true_after_leak_published() {
        let cache = Arc::new(AttributeCache::new());
        // Simulate the publish path writing a leak=true into the cache
        // at wohl endpoint 1, BooleanState cluster, StateValue attr.
        cache.set(
            AttributeKey::new(1, MatterCluster::BooleanState.cluster_id(), 0x0000),
            AttributeValue::Bool(true),
        );
        let h = handler(cache, 1);
        assert!(h.current_state(), "leak published → BooleanState true");
    }

    #[test]
    fn current_state_reads_its_own_endpoint_only() {
        let cache = Arc::new(AttributeCache::new());
        // Leak published at endpoint 2, but this handler serves
        // endpoint 1 — it must NOT report the other endpoint's state.
        cache.set(
            AttributeKey::new(2, MatterCluster::BooleanState.cluster_id(), 0x0000),
            AttributeValue::Bool(true),
        );
        let h = handler(cache, 1);
        assert!(
            !h.current_state(),
            "handler for ep1 must not leak ep2's state"
        );
    }

    #[test]
    fn dev_type_water_leak_detector_is_0x0043() {
        assert_eq!(DEV_TYPE_WATER_LEAK_DETECTOR.dtype, 0x0043);
    }
}
