//! `RsMatterBridge` — skeleton for the live rs-matter-backed
//! [`MatterBridge`] implementation.
//!
//! This module is gated behind the `rs-matter-backend` Cargo feature.
//! In the 0.3.0-architecture deliverable it is a **typed skeleton**:
//! the struct and trait impl exist so callers can already write
//! `RsMatterBridge::new(...)` and watch the workspace compile, but
//! the methods deliberately `unimplemented!()` — the actual
//! rs-matter wiring (commissioning, fabric storage, attribute
//! publication on UDP) is the 0.3.x implementation PR.
//!
//! See:
//!   - [`SWARCH-WOHL-007`](../../../../artifacts/swarch/SWARCH-WOHL-007.yaml)
//!     for the architectural decisions this implementation realizes.
//!   - [`SWDD-MATTER-001`](../../../../artifacts/swdd/SWDD-MATTER-001.yaml)
//!     for the thread set and per-thread responsibilities.
//!   - `DESIGN.md` in this crate for the per-cluster mapping rationale,
//!     commissioning UX, multi-admin posture, attestation plan, and
//!     unit-conversion contract.
//!
//! The skeleton serves three purposes today:
//!
//! 1. Lets `wohl-hub` import a `wohl_matter_bridge::RsMatterBridge`
//!    symbol (under the same feature) so the integration boundary is
//!    real, not hypothetical.
//! 2. Pins the `MatterBridge` trait's call sites — adding a method
//!    later is a SemVer break visible here first.
//! 3. Gives the 0.3.x implementor a single file to fill in, without
//!    re-debating the architecture each session.

use crate::MatterBridge;
use crate::types::{BridgedAlert, SensorReading};

/// Configuration handed to [`RsMatterBridge::new`] at construction
/// time. Fields are placeholders; the 0.3.x impl fills them in.
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
/// In 0.3.0-architecture this is a **skeleton**: construction and the
/// `MatterBridge` impl exist, but `publish_*` panics. The 0.3.x
/// implementation PR turns each `unimplemented!()` into the actual
/// rs-matter call.
pub struct RsMatterBridge {
    _config: RsMatterConfig,
}

impl RsMatterBridge {
    /// Construct a new bridge. In the skeleton this just stashes the
    /// config; the 0.3.x impl bootstraps the rs-matter event loop and
    /// loads persisted fabrics here.
    pub fn new(config: RsMatterConfig) -> Self {
        Self { _config: config }
    }
}

impl MatterBridge for RsMatterBridge {
    fn publish_reading(&self, _reading: SensorReading) {
        unimplemented!(
            "RsMatterBridge::publish_reading is a 0.3.x scope item. \
             Use LoggingBridge for 0.2.0 / 0.3.0-architecture; switch \
             to RsMatterBridge once the live rs-matter wiring lands. \
             See SWARCH-WOHL-007 and DESIGN.md §3."
        );
    }

    fn publish_alert(&self, _alert: BridgedAlert) {
        unimplemented!(
            "RsMatterBridge::publish_alert is a 0.3.x scope item. \
             See SWARCH-WOHL-007 and DESIGN.md §3."
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
    fn bridge_constructs() {
        // Construction must work even in the skeleton; only the
        // publish_* methods are stubbed.
        let _bridge = RsMatterBridge::new(RsMatterConfig::default());
    }

    #[test]
    fn skeleton_fits_dyn_matter_bridge() {
        // Compile-time check: RsMatterBridge implements the trait so
        // wohl-hub can hold a `Box<dyn MatterBridge>` regardless of
        // which backend is compiled in.
        let _b: Box<dyn MatterBridge> = Box::new(RsMatterBridge::new(RsMatterConfig::default()));
    }
}
