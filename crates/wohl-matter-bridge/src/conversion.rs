//! Unit conversions from wohl-internal values to Matter wire encoding.
//!
//! Implements the contract from DESIGN.md §7.4. Per-cluster:
//!
//! | Matter cluster                            | Wire encoding | Conversion from wohl's i64 |
//! |-------------------------------------------|---------------|-----------------------------|
//! | TemperatureMeasurement (0x0402)           | int16 centi-°C | saturating cast i64 → i16  |
//! | ElectricalPowerMeasurement (0x0090)       | int64 mW       | × 1000 saturating          |
//! | CO2 / PM2.5 / VOC ConcentrationMeasurement | float32        | `as f32` cast              |
//! | BooleanState (0x0045)                     | bool           | `value != 0`               |
//!
//! Conventions for alerts that don't carry a payload value:
//!   - BooleanState clusters (water leak, contact): the alert
//!     **firing** is the trigger; set `Bool(true)`.
//!   - Numeric clusters (Temperature, Power, Concentration): if
//!     the alert has no value, [`convert_alert`] returns `None` —
//!     the publish path skips the cache update.
//!
//! Known mismatch (DESIGN.md §7.5): VOC alerts / readings carry a
//! Sensirion-style index (0..500), not a concentration. The
//! conversion still produces an `f32`, but the value is semantically
//! wrong for the Matter VOC cluster. The 0.3.x implementor either
//! omits the VOC mapping or defines a vendor cluster; that decision
//! is deliberate and unchanged here.

use crate::cache::AttributeValue;
use crate::cluster::{MatterAttribute, MatterCluster, MatterClusterMapping};
use crate::types::{AlertKind, ReadingKind};

/// Convert a wohl sensor reading to the Matter-encoded attribute
/// value for the given mapping.
///
/// `value` is the wohl-internal `i64` in the unit implied by `kind`
/// (centi-°C for Temperature, watts for Power, ppm for CO2/PM2.5,
/// 0/1 for boolean kinds, etc. — see `types::ReadingKind` docs).
pub fn convert_reading(
    _kind: ReadingKind,
    value: i64,
    mapping: MatterClusterMapping,
) -> AttributeValue {
    convert_value(value, mapping)
}

/// Convert a wohl alert to the Matter-encoded attribute value.
///
/// Returns `None` for alerts whose target cluster expects a numeric
/// value but the alert payload is `None` — the caller skips the
/// cache update in that case (there's nothing to publish).
///
/// For boolean-target alerts (water leak, door / window), an alert
/// firing always sets the BooleanState to `true`; the absence of a
/// value field is the convention, not a failure.
pub fn convert_alert(
    _kind: AlertKind,
    value: Option<i64>,
    mapping: MatterClusterMapping,
) -> Option<AttributeValue> {
    match mapping.cluster {
        MatterCluster::BooleanState => {
            // The alert is the trigger; payload value (if any) is
            // ignored. Polarity is set at endpoint-registration time
            // per device type — see `MatterAttribute::StateValue`
            // doc.
            Some(AttributeValue::Bool(true))
        }
        _ => value.map(|v| convert_value(v, mapping)),
    }
}

/// Internal: apply the per-cluster wire encoding rules.
fn convert_value(value: i64, mapping: MatterClusterMapping) -> AttributeValue {
    match mapping.cluster {
        MatterCluster::TemperatureMeasurement => {
            // wohl: centi-°C i64 → Matter: int16 centi-°C, saturating.
            let clamped = if value > i16::MAX as i64 {
                i16::MAX
            } else if value < i16::MIN as i64 {
                i16::MIN
            } else {
                value as i16
            };
            AttributeValue::Int16(clamped)
        }
        MatterCluster::ElectricalPowerMeasurement => {
            // wohl: watts (i64) → Matter: milliwatts (int64), × 1000
            // saturating.
            AttributeValue::Int64(value.saturating_mul(1000))
        }
        MatterCluster::CarbonDioxideConcentrationMeasurement
        | MatterCluster::Pm25ConcentrationMeasurement
        | MatterCluster::TotalVolatileOrganicCompoundsConcentrationMeasurement => {
            // wohl: ppm / μg/m³ / index (i64) → Matter: float32.
            // VOC publishing index-as-concentration is the known
            // mismatch from DESIGN.md §7.5.
            AttributeValue::Float32(value as f32)
        }
        MatterCluster::BooleanState => {
            // Numeric → boolean: 0 means false, non-zero means
            // true. The polarity (which-physical-state-is-true)
            // is the endpoint's responsibility — see
            // `MatterAttribute::StateValue` doc.
            AttributeValue::Bool(value != 0)
        }
    }
}

// Re-export `MatterAttribute` for downstream conversion-table users
// that don't want to depend on `cluster` directly. (Today's only
// consumer is `rs_matter::RsMatterBridge`.)
#[allow(unused_imports)]
pub(crate) use crate::cluster::MatterAttribute as _Attr;
// Mark MatterAttribute as used so the file is structurally complete.
const _: fn() = || {
    let _ = MatterAttribute::StateValue;
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::{mapping_for_alert, mapping_for_reading};

    #[test]
    fn temperature_passes_through_as_int16() {
        let m = mapping_for_reading(ReadingKind::Temperature).unwrap();
        assert_eq!(
            convert_reading(ReadingKind::Temperature, -150, m),
            AttributeValue::Int16(-150),
            "wohl centi-°C → Matter int16 centi-°C is a passthrough"
        );
    }

    #[test]
    fn temperature_saturates_at_int16_bounds() {
        let m = mapping_for_reading(ReadingKind::Temperature).unwrap();
        assert_eq!(
            convert_reading(ReadingKind::Temperature, i64::MAX, m),
            AttributeValue::Int16(i16::MAX)
        );
        assert_eq!(
            convert_reading(ReadingKind::Temperature, i64::MIN, m),
            AttributeValue::Int16(i16::MIN)
        );
    }

    #[test]
    fn power_multiplies_watts_to_milliwatts() {
        let m = mapping_for_reading(ReadingKind::Power).unwrap();
        assert_eq!(
            convert_reading(ReadingKind::Power, 45, m),
            AttributeValue::Int64(45_000),
            "45 W → 45_000 mW"
        );
    }

    #[test]
    fn power_saturates_on_overflow() {
        let m = mapping_for_reading(ReadingKind::Power).unwrap();
        // 1e16 watts × 1000 would overflow i64; saturating_mul caps.
        let r = convert_reading(ReadingKind::Power, i64::MAX / 100, m);
        match r {
            AttributeValue::Int64(v) => assert!(v == i64::MAX, "got {v}"),
            other => panic!("expected Int64(saturated), got {other:?}"),
        }
    }

    #[test]
    fn concentration_casts_to_float32() {
        let m = mapping_for_reading(ReadingKind::Co2).unwrap();
        assert_eq!(
            convert_reading(ReadingKind::Co2, 412, m),
            AttributeValue::Float32(412.0)
        );
    }

    #[test]
    fn water_presence_reading_maps_to_bool() {
        let m = mapping_for_reading(ReadingKind::WaterPresence).unwrap();
        assert_eq!(
            convert_reading(ReadingKind::WaterPresence, 1, m),
            AttributeValue::Bool(true)
        );
        assert_eq!(
            convert_reading(ReadingKind::WaterPresence, 0, m),
            AttributeValue::Bool(false)
        );
    }

    #[test]
    fn water_leak_alert_sets_boolean_state_true_regardless_of_value() {
        let m = mapping_for_alert(AlertKind::WaterLeak).unwrap();
        // No value payload — alert firing IS the trigger.
        assert_eq!(
            convert_alert(AlertKind::WaterLeak, None, m),
            Some(AttributeValue::Bool(true))
        );
        // Even if a value were carried, it's still a trigger.
        assert_eq!(
            convert_alert(AlertKind::WaterLeak, Some(42), m),
            Some(AttributeValue::Bool(true))
        );
    }

    #[test]
    fn freeze_alert_with_value_publishes_temperature() {
        let m = mapping_for_alert(AlertKind::Freeze).unwrap();
        assert_eq!(
            convert_alert(AlertKind::Freeze, Some(-180), m),
            Some(AttributeValue::Int16(-180))
        );
    }

    #[test]
    fn freeze_alert_without_value_returns_none() {
        let m = mapping_for_alert(AlertKind::Freeze).unwrap();
        // No value carried — we can't publish a temperature reading.
        // The caller skips the cache update.
        assert_eq!(convert_alert(AlertKind::Freeze, None, m), None);
    }

    #[test]
    fn power_spike_alert_converts_watts_to_milliwatts() {
        let m = mapping_for_alert(AlertKind::PowerSpike).unwrap();
        assert_eq!(
            convert_alert(AlertKind::PowerSpike, Some(15_000), m),
            Some(AttributeValue::Int64(15_000_000))
        );
    }

    #[test]
    fn door_open_alert_sets_boolean_state_true() {
        let m = mapping_for_alert(AlertKind::DoorOpenTooLong).unwrap();
        assert_eq!(
            convert_alert(AlertKind::DoorOpenTooLong, None, m),
            Some(AttributeValue::Bool(true))
        );
    }
}
