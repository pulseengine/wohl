//! Wohl Matter Bridge — scaffold for the hub-side Matter integration.
//!
//! Per [SWARCH-WOHL-006], Wohl sensors stay lean and verified, emitting CCSDS
//! packets to the hub. The hub translates that traffic into Matter bridged
//! endpoints. This crate defines the *boundary* between Wohl's alert /
//! sensor-reading domain types and the Matter cluster/attribute model.
//!
//! In 0.2.0 this crate contains:
//!   - the [`MatterBridge`] trait wohl-hub calls,
//!   - minimal data types ([`SensorReading`], [`BridgedAlert`]) decoupled
//!     from wohl-hub's internal alert struct,
//!   - a typed [`MatterClusterMapping`] table mapping each alert / reading
//!     kind to a concrete Matter cluster + attribute,
//!   - [`LoggingBridge`], a stderr-logging stub impl used by
//!     `wohl-hub --matter`.
//!
//! The live rs-matter integration (commissioning, fabrics, mDNS, attestation,
//! attribute publication on UDP) is the 0.3.0 scope. See `DESIGN.md`.
//!
//! This crate is `no_std`-compatible at the trait + mapping layer (no alloc,
//! no std), but the [`LoggingBridge`] needs `std` for I/O. We allow `std`
//! here because the bridge always runs on the hub (Pi / STM32H7-class),
//! never on a sensor node. The verified sensor line stays untouched.
//!
//! [SWARCH-WOHL-006]: ../../../artifacts/swarch/SWARCH-WOHL-006.yaml

#![forbid(unsafe_code)]

pub mod cluster;
pub mod logging;
pub mod types;

#[cfg(feature = "rs-matter-backend")]
pub mod rs_matter;

pub use cluster::{MatterAttribute, MatterCluster, MatterClusterMapping, matter_cluster_for};
pub use logging::LoggingBridge;
pub use types::{AlertKind, BridgedAlert, ReadingKind, SensorReading};

#[cfg(feature = "rs-matter-backend")]
pub use rs_matter::{RsMatterBridge, RsMatterConfig};

/// The boundary between wohl-hub's alert / reading loop and any Matter
/// implementation. wohl-hub holds a `Box<dyn MatterBridge>` (or similar)
/// and calls these methods alongside its existing stdout JSON output.
///
/// Implementations:
///   - [`LoggingBridge`] (0.2.0) — stderr stub used to validate wiring.
///   - `RsMatterBridge` (planned, 0.3.0) — the real rs-matter-backed impl,
///     publishing attribute updates to the Matter fabric.
///
/// The trait is intentionally narrow. Anything stateful (commissioning,
/// fabric membership, attribute caches) lives behind the implementor.
pub trait MatterBridge: Send + Sync + 'static {
    /// A periodic sensor reading arrived. The bridge translates it to the
    /// matching Matter cluster attribute and publishes the update.
    fn publish_reading(&self, reading: SensorReading);

    /// A monitor produced an alert that survived dedup + rate-limit. The
    /// bridge translates it to the matching Matter cluster attribute
    /// (typically BooleanState-like) and publishes the update.
    fn publish_alert(&self, alert: BridgedAlert);
}
