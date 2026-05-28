//! Attribute cache for bridged Matter endpoints.
//!
//! rs-matter's `DataModel` is **pull-driven**: when a controller
//! subscribes to an attribute, rs-matter calls the application's
//! handler asking for the current value. The wohl bridge is
//! **push-driven**: the verified dispatcher hands alerts and readings
//! to [`publish_alert`] / [`publish_reading`] when they happen. The
//! cache mediates the impedance mismatch noted in `SWARCH-WOHL-007`
//! and the AADL `c_attr` comment in `spar/wohl_matter.aadl`.
//!
//! Lifecycle:
//!   1. A wohl alert / reading arrives on the dispatcher thread.
//!   2. The bridge runs the cluster mapping and the unit conversion
//!      (see [`crate::conversion`] and DESIGN.md §7.4).
//!   3. The bridge writes the converted value into this cache,
//!      keyed by `(endpoint_id, cluster_id, attribute_id)`.
//!   4. Later (next slice's work), rs-matter's `DataModelHandler`
//!      reads from this cache when a controller subscribes.
//!
//! The cache is intentionally a plain `HashMap` behind a `Mutex` —
//! lookups happen on the rs-matter thread, writes on the dispatcher
//! thread; contention is low (one writer producing at the alert
//! dispatcher's cadence, one reader at the controller's subscription
//! report cadence).
//!
//! [`publish_alert`]: crate::MatterBridge::publish_alert
//! [`publish_reading`]: crate::MatterBridge::publish_reading

use std::collections::HashMap;
use std::sync::Mutex;

/// Identifies a single attribute on a bridged Matter endpoint.
///
/// `endpoint_id` is the wohl-side identifier (a zone, contact, or
/// circuit id). The bridge's endpoint-id allocation policy maps these
/// onto Matter endpoint ids when the `DataModelHandler` is wired in
/// the next slice (see DESIGN.md §7.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AttributeKey {
    pub endpoint_id: u32,
    pub cluster_id: u32,
    pub attribute_id: u32,
}

impl AttributeKey {
    pub const fn new(endpoint_id: u32, cluster_id: u32, attribute_id: u32) -> Self {
        Self {
            endpoint_id,
            cluster_id,
            attribute_id,
        }
    }
}

/// A typed, Matter-encoded attribute value.
///
/// The variants match the wire encodings declared by each cluster in
/// the Matter App Cluster Spec — see DESIGN.md §7.4 for the
/// per-cluster conversion table. The conversion from wohl's internal
/// `i64` to one of these variants happens in [`crate::conversion`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AttributeValue {
    /// BooleanState::StateValue (`bool` on the wire). Polarity is
    /// device-type dependent — see `cluster.rs` `MatterAttribute::StateValue`
    /// doc for the rules.
    Bool(bool),
    /// TemperatureMeasurement::MeasuredValue (`int16` in 0.01 °C).
    Int16(i16),
    /// ElectricalPowerMeasurement::ActivePower (`int64` in milliwatts).
    Int64(i64),
    /// ConcentrationMeasurement-family MeasuredValue (IEEE 754
    /// `float32`). Used for CO2, PM2.5, VOC clusters.
    Float32(f32),
}

/// Thread-safe attribute cache.
///
/// Cloneable as `Arc<AttributeCache>` from the bridge. Writes go
/// through `set`; reads through `get`. Locking is per-call — the
/// cache is **not** appropriate for compound read-modify-write
/// transactions across multiple keys (don't put load-bearing
/// consistency on that).
#[derive(Debug, Default)]
pub struct AttributeCache {
    inner: Mutex<HashMap<AttributeKey, AttributeValue>>,
}

impl AttributeCache {
    /// Construct an empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Write a value into the cache, overwriting any previous value
    /// for the same key.
    pub fn set(&self, key: AttributeKey, value: AttributeValue) {
        if let Ok(mut g) = self.inner.lock() {
            g.insert(key, value);
        }
    }

    /// Read the current value for a key. Returns `None` if no value
    /// has ever been published for that endpoint+cluster+attribute.
    pub fn get(&self, key: AttributeKey) -> Option<AttributeValue> {
        self.inner.lock().ok()?.get(&key).copied()
    }

    /// Number of entries currently in the cache (mostly useful for
    /// tests and observability — not a load-bearing invariant).
    pub fn len(&self) -> usize {
        self.inner.lock().map(|g| g.len()).unwrap_or(0)
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_cache_returns_none() {
        let c = AttributeCache::new();
        assert_eq!(
            c.get(AttributeKey::new(1, 0x0402, 0x0000)),
            None,
            "no value published yet"
        );
        assert!(c.is_empty());
    }

    #[test]
    fn set_then_get_roundtrips_each_variant() {
        let c = AttributeCache::new();
        let k1 = AttributeKey::new(1, 0x0402, 0x0000);
        let k2 = AttributeKey::new(2, 0x0090, 0x0005);
        let k3 = AttributeKey::new(3, 0x0045, 0x0000);
        let k4 = AttributeKey::new(4, 0x040D, 0x0000);

        c.set(k1, AttributeValue::Int16(2150));
        c.set(k2, AttributeValue::Int64(45_000));
        c.set(k3, AttributeValue::Bool(true));
        c.set(k4, AttributeValue::Float32(412.5));

        assert_eq!(c.get(k1), Some(AttributeValue::Int16(2150)));
        assert_eq!(c.get(k2), Some(AttributeValue::Int64(45_000)));
        assert_eq!(c.get(k3), Some(AttributeValue::Bool(true)));
        assert_eq!(c.get(k4), Some(AttributeValue::Float32(412.5)));
        assert_eq!(c.len(), 4);
    }

    #[test]
    fn set_overwrites_previous_value() {
        let c = AttributeCache::new();
        let k = AttributeKey::new(1, 0x0402, 0x0000);
        c.set(k, AttributeValue::Int16(2000));
        c.set(k, AttributeValue::Int16(2200));
        assert_eq!(c.get(k), Some(AttributeValue::Int16(2200)));
        assert_eq!(c.len(), 1, "overwrite, not insert");
    }

    #[test]
    fn keys_differing_in_any_field_are_distinct() {
        let c = AttributeCache::new();
        c.set(
            AttributeKey::new(1, 0x0402, 0x0000),
            AttributeValue::Int16(1),
        );
        c.set(
            AttributeKey::new(2, 0x0402, 0x0000),
            AttributeValue::Int16(2),
        );
        c.set(
            AttributeKey::new(1, 0x0090, 0x0000),
            AttributeValue::Int64(3),
        );
        c.set(
            AttributeKey::new(1, 0x0402, 0x0001),
            AttributeValue::Int16(4),
        );
        assert_eq!(c.len(), 4);
    }
}
