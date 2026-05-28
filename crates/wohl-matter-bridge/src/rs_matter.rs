//! `RsMatterBridge` — live rs-matter-backed [`MatterBridge`]
//! implementation.
//!
//! This module is gated behind the `rs-matter-backend` Cargo feature.
//! It pulls in `rs-matter` (project-chip/rs-matter, pinned by git
//! rev — see this crate's `Cargo.toml`) and uses its persistence
//! backend (`DirKvBlobStore`) for the fabric storage contract from
//! SWREQ-MATTER-004.
//!
//! Scope today (Track A slice 3): construction wires the real
//! `rs_matter::persist::DirKvBlobStore` at the configured state
//! directory; `publish_*` are still stubs (`unimplemented!()`). The
//! commissioning loop (mDNS announce + PASE + CASE + fabric persist
//! drive) is the next slice — it needs a dedicated OS thread with a
//! ≥550 KB stack running `block_on` against the rs-matter future
//! tree per the upstream `onoff_light` example.
//!
//! See:
//!   - [`SWARCH-WOHL-007`](../../../../artifacts/swarch/SWARCH-WOHL-007.yaml)
//!     for the architectural decisions this implementation realizes.
//!   - [`SWDD-MATTER-001`](../../../../artifacts/swdd/SWDD-MATTER-001.yaml)
//!     for the thread set and per-thread responsibilities.
//!   - `DESIGN.md` in this crate for the per-cluster mapping
//!     rationale, commissioning UX, multi-admin posture, attestation
//!     plan, and unit-conversion contract.
//!   - rs-matter upstream `examples/src/bin/onoff_light.rs` for the
//!     canonical std-thread + `block_on` construction sketch.

use std::sync::Arc;

use rs_matter::persist::DirKvBlobStore;

use crate::MatterBridge;
use crate::cache::{AttributeCache, AttributeKey};
use crate::cluster::{mapping_for_alert, mapping_for_reading};
use crate::conversion::{convert_alert, convert_reading};
use crate::types::{BridgedAlert, SensorReading};

/// Configuration handed to [`RsMatterBridge::new`] at construction
/// time.
#[derive(Debug, Clone)]
pub struct RsMatterConfig {
    /// Directory holding the persisted fabric set, ACL, setup code,
    /// and discriminator. See `SWREQ-MATTER-004` for the durability
    /// contract.
    pub state_dir: std::path::PathBuf,
    /// Vendor ID for the Matter Bridge device. `0xFFF1` is the test
    /// vendor; production must use a CSA-issued ID. See DESIGN.md §6.
    pub vendor_id: u16,
    /// Product ID for this specific Bridge SKU.
    pub product_id: u16,
    /// Human-readable label exposed to controllers.
    pub label: String,
}

impl Default for RsMatterConfig {
    fn default() -> Self {
        Self {
            state_dir: std::path::PathBuf::from("/var/lib/wohl/matter"),
            vendor_id: 0xFFF1,
            product_id: 0x8000,
            label: "Wohl Hub".to_string(),
        }
    }
}

/// Live rs-matter-backed bridge.
///
/// Owns:
///   - the rs-matter persistence backend (`DirKvBlobStore`) for
///     fabric data,
///   - an `Arc<AttributeCache>` of the latest bridged values per
///     endpoint+cluster+attribute (mediates rs-matter's pull-callback
///     `DataModel` against wohl's push-style dispatcher — see
///     `cache.rs` doc and the AADL `c_attr` comment).
///
/// `publish_reading` / `publish_alert` write into the cache. The
/// rs-matter `DataModelHandler` integration (slice 6) reads from
/// the cache when a controller subscribes; until then the cache is
/// observable from this crate's tests + the next implementor's
/// integration code.
pub struct RsMatterBridge {
    config: RsMatterConfig,
    /// Persistent fabric / ACL / setup-code storage. The
    /// commissioning thread hands a reference to
    /// `matter.load_persist(...)` at boot and stores updates through
    /// rs-matter's exchange-handler callbacks.
    kv_store: DirKvBlobStore,
    /// Current-value cache, shared with the rs-matter
    /// `DataModelHandler` once the next slice wires endpoint
    /// registration.
    cache: Arc<AttributeCache>,
}

impl RsMatterBridge {
    /// Construct a new bridge. Wires the rs-matter persistence
    /// backend at `config.state_dir`; the directory does NOT need to
    /// exist yet — `DirKvBlobStore` lazy-creates it on first write.
    /// The next slice will call `matter.load_persist(&mut self.kv_store, &mut kv_buf)`
    /// at boot to replay any prior commissioned fabric.
    pub fn new(config: RsMatterConfig) -> Self {
        let kv_store = DirKvBlobStore::new(config.state_dir.clone());
        let cache = Arc::new(AttributeCache::new());
        Self {
            config,
            kv_store,
            cache,
        }
    }

    /// Borrow the attribute cache. The next slice's
    /// `DataModelHandler` integration calls this on the commissioning
    /// thread to pull current values during controller subscriptions.
    /// Tests use it to assert the publish path landed the right
    /// value in the cache.
    pub fn cache(&self) -> &Arc<AttributeCache> {
        &self.cache
    }

    /// The state directory this bridge persists fabric data to.
    pub fn state_dir(&self) -> &std::path::Path {
        &self.config.state_dir
    }

    /// Borrow the persistence backend. The next slice's commissioning
    /// loop hands this to `rs_matter::Matter::load_persist` /
    /// `Persist::run` to drive fabric I/O.
    pub fn kv_store(&self) -> &DirKvBlobStore {
        &self.kv_store
    }

    /// Configuration the bridge was constructed with (read-only).
    pub fn config(&self) -> &RsMatterConfig {
        &self.config
    }

    /// Spawn the Matter commissioning thread.
    ///
    /// Constructs the rs-matter `Matter` instance, binds the IPv6
    /// UDP socket, opens a commissioning window if no fabric is yet
    /// persisted, and runs the rs-matter futures on a dedicated OS
    /// thread (≥550 KB stack). The thread runs for the lifetime of
    /// the process; the returned `JoinHandle` is provided for the
    /// caller's bookkeeping but is not expected to be joined under
    /// normal operation.
    ///
    /// **Single-instance constraint:** the underlying static cells
    /// (`Matter`, IM buffers, subscriptions, KV scratch) live in
    /// `static_cell::StaticCell` statics. This method can be called
    /// at most once per process — a second call panics on
    /// `StaticCell` re-init. This matches how rs-matter is
    /// architected to be embedded.
    ///
    /// On first boot (no commissioned fabric), the standard Matter
    /// QR code + manual pairing code are printed via the rs-matter
    /// helpers (Verhoeff-checksummed manual code, Base38-encoded
    /// TLV QR per Matter Core §5.1.3).
    ///
    /// This method does NOT yet wire `publish_*` to the running
    /// stack — that's the attribute-publishing slice. Today's
    /// commissioning loop will succeed (a Matter controller can
    /// commission the bridge against the test DAC / vendor 0xFFF1)
    /// but no application clusters are advertised on the endpoint
    /// set, so a commissioned controller sees only the root
    /// endpoint (Basic Information, General Commissioning,
    /// Operational Credentials).
    pub fn start_commissioning(
        &self,
    ) -> std::io::Result<std::thread::JoinHandle<Result<(), rs_matter::error::Error>>> {
        crate::commissioning::start(self.config.state_dir.clone(), Arc::clone(&self.cache))
    }
}

impl MatterBridge for RsMatterBridge {
    /// Translate a wohl sensor reading to its Matter cluster
    /// attribute and write it into the cache. Drops readings whose
    /// kind has no Matter mapping (none today; future kinds without
    /// a Matter analog would be silently dropped via the `None`
    /// branch).
    fn publish_reading(&self, reading: SensorReading) {
        let Some(mapping) = mapping_for_reading(reading.kind) else {
            return;
        };
        let value = convert_reading(reading.kind, reading.value, mapping);
        let key = AttributeKey::new(
            reading.endpoint_id,
            mapping.cluster.cluster_id(),
            mapping.attribute.attribute_id(),
        );
        self.cache.set(key, value);
    }

    /// Translate a wohl alert to its Matter cluster attribute and
    /// write it into the cache. Drops alerts:
    ///   - whose kind has no Matter mapping (today: only
    ///     `AlertKind::HealthMiss`, an internal liveness signal),
    ///   - whose target cluster expects a numeric value but the
    ///     alert payload is `None` — see `conversion::convert_alert`.
    fn publish_alert(&self, alert: BridgedAlert) {
        let Some(mapping) = mapping_for_alert(alert.kind) else {
            return;
        };
        let Some(value) = convert_alert(alert.kind, alert.value, mapping) else {
            return;
        };
        let endpoint_id = alert
            .zone_id
            .or(alert.contact_id)
            .or(alert.circuit_id)
            .unwrap_or(0);
        let key = AttributeKey::new(
            endpoint_id,
            mapping.cluster.cluster_id(),
            mapping.attribute.attribute_id(),
        );
        self.cache.set(key, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_default_uses_wohl_state_dir() {
        let c = RsMatterConfig::default();
        assert_eq!(
            c.state_dir,
            std::path::PathBuf::from("/var/lib/wohl/matter")
        );
        assert_eq!(c.vendor_id, 0xFFF1, "test vendor id by default");
        assert_eq!(c.label, "Wohl Hub");
    }

    #[test]
    fn bridge_constructs_with_rs_matter_kv_store() {
        // The construction must succeed even when the state dir does
        // not exist on disk — DirKvBlobStore lazy-creates on write.
        let cfg = RsMatterConfig {
            state_dir: std::path::PathBuf::from("/tmp/wohl-matter-test-nonexistent-path"),
            ..RsMatterConfig::default()
        };
        let bridge = RsMatterBridge::new(cfg);
        assert_eq!(
            bridge.state_dir(),
            std::path::Path::new("/tmp/wohl-matter-test-nonexistent-path"),
        );
        assert_eq!(bridge.config().vendor_id, 0xFFF1);
        // Verify the rs-matter KvBlobStore is reachable through the
        // accessor — proves the rs-matter type is wired in, not just
        // declared.
        let _: &DirKvBlobStore = bridge.kv_store();
    }

    #[test]
    fn publish_reading_populates_cache() {
        use crate::cache::AttributeValue;
        use crate::types::ReadingKind;
        let bridge = RsMatterBridge::new(RsMatterConfig::default());
        bridge.publish_reading(SensorReading {
            kind: ReadingKind::Temperature,
            endpoint_id: 7,
            value: -150,
            time: 1234,
        });
        assert_eq!(
            bridge.cache().get(AttributeKey::new(7, 0x0402, 0x0000)),
            Some(AttributeValue::Int16(-150))
        );
        assert_eq!(bridge.cache().len(), 1);
    }

    #[test]
    fn publish_alert_water_leak_sets_boolean_state_true() {
        use crate::cache::AttributeValue;
        use crate::types::AlertKind;
        let bridge = RsMatterBridge::new(RsMatterConfig::default());
        bridge.publish_alert(BridgedAlert {
            kind: AlertKind::WaterLeak,
            zone_id: Some(3),
            contact_id: None,
            circuit_id: None,
            value: None,
            time: 5000,
        });
        assert_eq!(
            bridge.cache().get(AttributeKey::new(3, 0x0045, 0x0000)),
            Some(AttributeValue::Bool(true))
        );
    }

    #[test]
    fn publish_alert_power_spike_converts_watts_to_milliwatts() {
        use crate::cache::AttributeValue;
        use crate::types::AlertKind;
        let bridge = RsMatterBridge::new(RsMatterConfig::default());
        bridge.publish_alert(BridgedAlert {
            kind: AlertKind::PowerSpike,
            zone_id: None,
            contact_id: None,
            circuit_id: Some(11),
            value: Some(15_000),
            time: 1,
        });
        assert_eq!(
            bridge.cache().get(AttributeKey::new(11, 0x0090, 0x0005)),
            Some(AttributeValue::Int64(15_000_000))
        );
    }

    #[test]
    fn publish_alert_health_miss_is_dropped() {
        use crate::types::AlertKind;
        let bridge = RsMatterBridge::new(RsMatterConfig::default());
        bridge.publish_alert(BridgedAlert {
            kind: AlertKind::HealthMiss,
            zone_id: None,
            contact_id: None,
            circuit_id: None,
            value: Some(1),
            time: 10,
        });
        assert!(
            bridge.cache().is_empty(),
            "HealthMiss must not appear on Matter"
        );
    }

    #[test]
    fn publish_alert_freeze_without_value_skips_cache() {
        use crate::types::AlertKind;
        let bridge = RsMatterBridge::new(RsMatterConfig::default());
        bridge.publish_alert(BridgedAlert {
            kind: AlertKind::Freeze,
            zone_id: Some(2),
            contact_id: None,
            circuit_id: None,
            value: None,
            time: 0,
        });
        // No value → no Matter Report can be emitted → cache stays
        // empty rather than publishing a stale or fabricated number.
        assert!(bridge.cache().is_empty());
    }

    #[test]
    fn cache_is_shared_through_arc() {
        // The cache accessor returns an Arc so the future
        // commissioning-thread integration can clone a handle
        // without taking a long-lived borrow on the bridge.
        let bridge = RsMatterBridge::new(RsMatterConfig::default());
        let cache_clone = bridge.cache().clone();
        assert_eq!(cache_clone.len(), 0);
        // A write through the bridge is visible through the
        // independently-held handle.
        bridge.publish_reading(SensorReading {
            kind: crate::types::ReadingKind::Co2,
            endpoint_id: 99,
            value: 412,
            time: 0,
        });
        assert_eq!(cache_clone.len(), 1);
    }

    #[test]
    fn skeleton_fits_dyn_matter_bridge() {
        // Compile-time check: RsMatterBridge implements the trait so
        // wohl-hub can hold a `Box<dyn MatterBridge>` regardless of
        // which backend is compiled in.
        let _b: Box<dyn MatterBridge> = Box::new(RsMatterBridge::new(RsMatterConfig::default()));
    }
}
