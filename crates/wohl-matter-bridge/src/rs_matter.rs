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

use rs_matter::persist::DirKvBlobStore;

use crate::MatterBridge;
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
/// In the current slice, construction wires the concrete
/// `DirKvBlobStore` against the configured state directory but does
/// not yet start the Matter runtime. The `MatterBridge` trait impl's
/// `publish_*` methods still `unimplemented!()` because the runtime
/// they need (rs-matter event loop on a dedicated OS thread) is
/// constructed in the next slice.
pub struct RsMatterBridge {
    config: RsMatterConfig,
    /// Persistent fabric / ACL / setup-code storage. The next slice
    /// hands a reference to `matter.load_persist(...)` at boot and
    /// stores updates through rs-matter's exchange-handler callbacks.
    kv_store: DirKvBlobStore,
}

impl RsMatterBridge {
    /// Construct a new bridge. Wires the rs-matter persistence
    /// backend at `config.state_dir`; the directory does NOT need to
    /// exist yet — `DirKvBlobStore` lazy-creates it on first write.
    /// The next slice will call `matter.load_persist(&mut self.kv_store, &mut kv_buf)`
    /// at boot to replay any prior commissioned fabric.
    pub fn new(config: RsMatterConfig) -> Self {
        let kv_store = DirKvBlobStore::new(config.state_dir.clone());
        Self { config, kv_store }
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
}

impl MatterBridge for RsMatterBridge {
    fn publish_reading(&self, _reading: SensorReading) {
        unimplemented!(
            "RsMatterBridge::publish_reading needs the commissioning \
             loop (mDNS + PASE + CASE) running first. That's the next \
             slice. Use LoggingBridge for now; switch to \
             RsMatterBridge once the live runtime starts. See \
             SWARCH-WOHL-007 and DESIGN.md §3."
        );
    }

    fn publish_alert(&self, _alert: BridgedAlert) {
        unimplemented!(
            "RsMatterBridge::publish_alert needs the commissioning \
             loop running first. See SWARCH-WOHL-007 and DESIGN.md §3."
        );
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
    fn skeleton_fits_dyn_matter_bridge() {
        // Compile-time check: RsMatterBridge implements the trait so
        // wohl-hub can hold a `Box<dyn MatterBridge>` regardless of
        // which backend is compiled in.
        let _b: Box<dyn MatterBridge> = Box::new(RsMatterBridge::new(RsMatterConfig::default()));
    }
}
