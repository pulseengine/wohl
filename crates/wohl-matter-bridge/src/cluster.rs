//! Matter cluster mapping table.
//!
//! Maps each wohl [`AlertKind`] / [`ReadingKind`] to the specific Matter
//! cluster + attribute that should reflect the event.
//!
//! Encoded as enums (not strings) so that:
//!   - The mapping is exhaustive at compile time — adding a new alert kind
//!     forces an explicit decision here (the match in
//!     [`matter_cluster_for`] is non-`_`-defaulted).
//!   - Downstream code (the future rs-matter impl) gets a typed contract.
//!
//! Cluster IDs and attribute references follow the Matter Application
//! Cluster Specification 1.3 (latest at time of writing). Source for each
//! mapping decision is documented inline.
//!
//! ## On device types vs clusters
//!
//! Matter draws a sharp line between **clusters** (server-side functional
//! units identified by cluster id, e.g. BooleanState = 0x0045) and
//! **device types** (named compositions of mandatory + optional clusters,
//! identified by a separate Device Type ID, e.g. WaterLeakDetector = 0x0043).
//! This enum tracks **clusters only**. Water-leak support is a *device type*
//! that uses BooleanState (0x0045) as its mandatory server cluster; the
//! "WaterLeakDetector" name is intentionally NOT enumerated here — see
//! DESIGN.md §2 for the rationale.

use crate::types::{AlertKind, ReadingKind};

/// A Matter cluster, identified by its application cluster id (hex).
///
/// Only the clusters Wohl bridges today are enumerated. Adding a new
/// sensor type is a deliberate change here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MatterCluster {
    /// Boolean State (0x0045) — Matter 1.0 generic "is this thing
    /// triggered?" cluster. Used for door/window contact and for
    /// water-leak (the WaterLeakDetector device type uses BooleanState
    /// as its mandatory server cluster; there is no separate
    /// "WaterLeakDetector cluster" in the Matter spec — see DESIGN.md
    /// §2 for the corrected understanding).
    BooleanState,
    /// Temperature Measurement (0x0402) — measured temperature in
    /// centi-degrees Celsius (signed int16).
    TemperatureMeasurement,
    /// Carbon Dioxide Concentration Measurement (0x040D).
    CarbonDioxideConcentrationMeasurement,
    /// PM2.5 Concentration Measurement (0x042A).
    Pm25ConcentrationMeasurement,
    /// Total Volatile Organic Compounds Concentration Measurement (0x042C).
    TotalVolatileOrganicCompoundsConcentrationMeasurement,
    /// Electrical Power Measurement (0x0090) — Matter 1.3+. ActivePower
    /// attribute carries the live wattage.
    ElectricalPowerMeasurement,
}

impl MatterCluster {
    /// Numeric Matter cluster id (the value used on the wire).
    pub const fn cluster_id(self) -> u32 {
        match self {
            Self::BooleanState => 0x0045,
            Self::TemperatureMeasurement => 0x0402,
            Self::CarbonDioxideConcentrationMeasurement => 0x040D,
            Self::Pm25ConcentrationMeasurement => 0x042A,
            Self::TotalVolatileOrganicCompoundsConcentrationMeasurement => 0x042C,
            Self::ElectricalPowerMeasurement => 0x0090,
        }
    }
}

/// A specific Matter attribute within a cluster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MatterAttribute {
    /// BooleanState::StateValue (0x0000). Polarity is **device-type
    /// dependent** and must be applied at publish time, not stored
    /// as a trait invariant. On a ContactSensor device type (door /
    /// window), `true` means contact closed (door shut) and `false`
    /// means open. On a WaterLeakDetector device type — Matter 1.2+
    /// — `true` means leak detected. The bridge implementor reads
    /// the wohl `AlertKind` / `ReadingKind` to decide which polarity
    /// to encode; the device-type descriptor on the endpoint
    /// disambiguates for the controller. The cluster + attribute id
    /// are the same in both cases (0x0045 / 0x0000).
    StateValue,
    /// MeasuredValue (0x0000) — the generic measurement attribute, used
    /// by Temperature, CO2, PM2.5, VOC clusters. **Wire encoding is
    /// cluster-dependent**: TemperatureMeasurement encodes int16 in
    /// 0.01 °C; the concentration-measurement clusters (CO2, PM2.5,
    /// VOC) encode IEEE 754 float32 with a separate MeasurementUnit
    /// (0x0008) attribute declaring the unit. The bridge publish path
    /// applies the per-cluster encoding — see DESIGN.md §7.4.
    MeasuredValue,
    /// ElectricalPowerMeasurement::ActivePower (0x0005) —
    /// instantaneous active power. **Matter wire encoding is
    /// milliwatts (int64).** wohl's internal `Reading::Power.value`
    /// is watts, so the bridge implementor must scale `× 1000`
    /// before publishing. See DESIGN.md §7.4 for the unit-conversion
    /// contract.
    ActivePower,
}

impl MatterAttribute {
    /// Numeric Matter attribute id.
    pub const fn attribute_id(self) -> u32 {
        match self {
            Self::StateValue => 0x0000,
            Self::MeasuredValue => 0x0000,
            Self::ActivePower => 0x0005,
        }
    }
}

/// A resolved (cluster, attribute) pair, plus a short rationale tag for
/// the implementor reading the mapping table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatterClusterMapping {
    pub cluster: MatterCluster,
    pub attribute: MatterAttribute,
}

impl MatterClusterMapping {
    pub const fn new(cluster: MatterCluster, attribute: MatterAttribute) -> Self {
        Self { cluster, attribute }
    }
}

/// Map a wohl alert tag (as emitted by wohl-hub today) to the Matter
/// cluster + attribute that should reflect it.
///
/// Returns `None` for tags that don't have a meaningful Matter analog —
/// e.g. `"health_miss"`, which is an internal-health signal and is not
/// surfaced to Matter controllers.
///
/// This wraps [`AlertKind::from_tag`] + [`mapping_for_alert`] for callers
/// that hold the raw string.
pub fn matter_cluster_for(alert_kind: &str) -> Option<MatterClusterMapping> {
    AlertKind::from_tag(alert_kind).and_then(mapping_for_alert)
}

/// Typed counterpart of [`matter_cluster_for`] — exhaustive over
/// [`AlertKind`].
pub const fn mapping_for_alert(kind: AlertKind) -> Option<MatterClusterMapping> {
    use MatterAttribute::*;
    use MatterCluster::*;

    Some(match kind {
        // Temperature monitor — the alert latch reflects the same
        // physical attribute Matter exposes (MeasuredValue). Controllers
        // already alarm on out-of-band values, so we surface the reading
        // and let the value speak. See DESIGN.md "Why no separate
        // freeze-alert cluster".
        AlertKind::Freeze | AlertKind::Overheat | AlertKind::RapidDrop | AlertKind::RapidRise => {
            MatterClusterMapping::new(TemperatureMeasurement, MeasuredValue)
        }

        // Water leak. The endpoint is declared with the WaterLeakDetector
        // *device type* (DTL 0x0043) which mandates BooleanState (0x0045)
        // as its server cluster. There is no separate "WaterLeakDetector
        // cluster"; an earlier draft incorrectly listed cluster id 0x0048
        // (which is the Smoke/CO Alarm cluster). See DESIGN.md §2 and
        // SWDD-MATTER-001 for the corrected design.
        AlertKind::WaterLeak => MatterClusterMapping::new(BooleanState, StateValue),

        // Air-quality clusters all share the MeasuredValue attribute id
        // (0x0000) within their respective concentration-measurement
        // clusters. We surface MeasuredValue; controllers compare it
        // against their own thresholds. Wire encoding is IEEE 754
        // float32 for concentration MeasuredValue — see DESIGN.md §7.4.
        AlertKind::Co2Warning | AlertKind::Co2Critical => {
            MatterClusterMapping::new(CarbonDioxideConcentrationMeasurement, MeasuredValue)
        }
        AlertKind::Pm25Warning | AlertKind::Pm25Critical => {
            MatterClusterMapping::new(Pm25ConcentrationMeasurement, MeasuredValue)
        }
        AlertKind::VocWarning | AlertKind::VocCritical => MatterClusterMapping::new(
            TotalVolatileOrganicCompoundsConcentrationMeasurement,
            MeasuredValue,
        ),

        // Door / window contact events surface as BooleanState::StateValue
        // toggling on a ContactSensor device-type endpoint. The two alert
        // flavors (open-too-long, opened-at-night) both point at the same
        // attribute; the rich detail lives in the hub's notification
        // path, not Matter.
        AlertKind::DoorOpenTooLong | AlertKind::DoorOpenedAtNight => {
            MatterClusterMapping::new(BooleanState, StateValue)
        }

        // Power. ElectricalPowerMeasurement::ActivePower is the live
        // wattage. Both overconsumption and spike alerts publish the
        // wattage that triggered the alert.
        AlertKind::Overconsumption | AlertKind::PowerSpike | AlertKind::DeviceLeftOn => {
            MatterClusterMapping::new(ElectricalPowerMeasurement, ActivePower)
        }

        // Internal: not bridged to Matter.
        AlertKind::HealthMiss => return None,
    })
}

/// Map a periodic sensor reading to its Matter cluster + attribute.
/// Returns `None` for reading kinds without a Matter analog.
pub const fn mapping_for_reading(kind: ReadingKind) -> Option<MatterClusterMapping> {
    use MatterAttribute::*;
    use MatterCluster::*;

    Some(match kind {
        ReadingKind::Temperature => {
            MatterClusterMapping::new(TemperatureMeasurement, MeasuredValue)
        }
        ReadingKind::Co2 => {
            MatterClusterMapping::new(CarbonDioxideConcentrationMeasurement, MeasuredValue)
        }
        ReadingKind::Pm25 => MatterClusterMapping::new(Pm25ConcentrationMeasurement, MeasuredValue),
        ReadingKind::Voc => MatterClusterMapping::new(
            TotalVolatileOrganicCompoundsConcentrationMeasurement,
            MeasuredValue,
        ),
        ReadingKind::Power => MatterClusterMapping::new(ElectricalPowerMeasurement, ActivePower),
        ReadingKind::Contact | ReadingKind::WaterPresence => {
            MatterClusterMapping::new(BooleanState, StateValue)
        }
    })
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alert_kind_roundtrip_through_string() {
        for kind in [
            AlertKind::Freeze,
            AlertKind::Overheat,
            AlertKind::RapidDrop,
            AlertKind::RapidRise,
            AlertKind::WaterLeak,
            AlertKind::Co2Warning,
            AlertKind::Co2Critical,
            AlertKind::Pm25Warning,
            AlertKind::Pm25Critical,
            AlertKind::VocWarning,
            AlertKind::VocCritical,
            AlertKind::DoorOpenTooLong,
            AlertKind::DoorOpenedAtNight,
            AlertKind::Overconsumption,
            AlertKind::PowerSpike,
            AlertKind::DeviceLeftOn,
            AlertKind::HealthMiss,
        ] {
            let s = kind.as_tag();
            assert_eq!(
                AlertKind::from_tag(s),
                Some(kind),
                "roundtrip failed for {:?}",
                kind
            );
        }
    }

    #[test]
    fn unknown_alert_string_returns_none() {
        assert!(matter_cluster_for("not_a_real_alert").is_none());
        assert!(matter_cluster_for("").is_none());
    }

    #[test]
    fn health_miss_is_not_bridged_to_matter() {
        // The mapping must explicitly return None for internal health
        // signals; controllers shouldn't see them.
        assert!(matter_cluster_for("health_miss").is_none());
        assert!(mapping_for_alert(AlertKind::HealthMiss).is_none());
    }

    #[test]
    fn temperature_alerts_publish_to_temperature_measurement() {
        for tag in ["freeze", "overheat", "rapid_drop", "rapid_rise"] {
            let m = matter_cluster_for(tag).unwrap_or_else(|| panic!("no mapping for {}", tag));
            assert_eq!(m.cluster, MatterCluster::TemperatureMeasurement);
            assert_eq!(m.attribute, MatterAttribute::MeasuredValue);
            assert_eq!(m.cluster.cluster_id(), 0x0402);
            assert_eq!(m.attribute.attribute_id(), 0x0000);
        }
    }

    #[test]
    fn water_leak_maps_to_boolean_state_on_water_leak_detector_device_type() {
        // Matter 1.2 water-leak is a DEVICE TYPE (DTL 0x0043), not a
        // separate cluster. The mandatory server cluster on that
        // device type is BooleanState (0x0045). Cluster id 0x0048 —
        // which an earlier draft incorrectly assigned to water-leak —
        // is actually the Smoke/CO Alarm cluster.
        let m = matter_cluster_for("water_leak").unwrap();
        assert_eq!(m.cluster, MatterCluster::BooleanState);
        assert_eq!(m.attribute, MatterAttribute::StateValue);
        assert_eq!(m.cluster.cluster_id(), 0x0045);
    }

    #[test]
    fn co2_alerts_publish_to_co2_concentration() {
        for tag in ["co2_warning", "co2_critical"] {
            let m = matter_cluster_for(tag).unwrap();
            assert_eq!(
                m.cluster,
                MatterCluster::CarbonDioxideConcentrationMeasurement
            );
            assert_eq!(m.cluster.cluster_id(), 0x040D);
        }
    }

    #[test]
    fn pm25_alerts_publish_to_pm25_concentration() {
        for tag in ["pm25_warning", "pm25_critical"] {
            let m = matter_cluster_for(tag).unwrap();
            assert_eq!(m.cluster, MatterCluster::Pm25ConcentrationMeasurement);
            assert_eq!(m.cluster.cluster_id(), 0x042A);
        }
    }

    #[test]
    fn voc_alerts_publish_to_voc_concentration() {
        for tag in ["voc_warning", "voc_critical"] {
            let m = matter_cluster_for(tag).unwrap();
            assert_eq!(
                m.cluster,
                MatterCluster::TotalVolatileOrganicCompoundsConcentrationMeasurement
            );
            assert_eq!(m.cluster.cluster_id(), 0x042C);
        }
    }

    #[test]
    fn door_alerts_publish_to_boolean_state() {
        for tag in ["door_open_too_long", "door_opened_at_night"] {
            let m = matter_cluster_for(tag).unwrap();
            assert_eq!(m.cluster, MatterCluster::BooleanState);
            assert_eq!(m.attribute, MatterAttribute::StateValue);
        }
    }

    #[test]
    fn power_alerts_publish_to_active_power() {
        for tag in ["overconsumption", "power_spike", "device_left_on"] {
            let m = matter_cluster_for(tag).unwrap();
            assert_eq!(m.cluster, MatterCluster::ElectricalPowerMeasurement);
            assert_eq!(m.attribute, MatterAttribute::ActivePower);
            assert_eq!(m.cluster.cluster_id(), 0x0090);
            assert_eq!(m.attribute.attribute_id(), 0x0005);
        }
    }

    #[test]
    fn cluster_ids_are_stable_hex_values() {
        // Pin specific cluster IDs to catch accidental edits — these are
        // the Matter Application Cluster Specification 1.3 values.
        assert_eq!(MatterCluster::BooleanState.cluster_id(), 0x0045);
        assert_eq!(MatterCluster::TemperatureMeasurement.cluster_id(), 0x0402);
        assert_eq!(
            MatterCluster::CarbonDioxideConcentrationMeasurement.cluster_id(),
            0x040D
        );
        assert_eq!(
            MatterCluster::Pm25ConcentrationMeasurement.cluster_id(),
            0x042A
        );
        assert_eq!(
            MatterCluster::TotalVolatileOrganicCompoundsConcentrationMeasurement.cluster_id(),
            0x042C
        );
        assert_eq!(
            MatterCluster::ElectricalPowerMeasurement.cluster_id(),
            0x0090
        );
    }

    #[test]
    fn reading_mappings_cover_all_kinds() {
        // Every ReadingKind must have a non-None mapping. If a future
        // reading is added without a Matter analog, change this test
        // and document the gap.
        for k in [
            ReadingKind::Temperature,
            ReadingKind::Co2,
            ReadingKind::Pm25,
            ReadingKind::Voc,
            ReadingKind::Power,
            ReadingKind::Contact,
            ReadingKind::WaterPresence,
        ] {
            assert!(
                mapping_for_reading(k).is_some(),
                "no Matter mapping for reading kind {:?}",
                k
            );
        }
    }

    #[test]
    fn reading_temperature_matches_alert_temperature() {
        // Sanity: a Temp reading and a freeze alert publish to the
        // same Matter attribute — they describe the same physical
        // value.
        let r = mapping_for_reading(ReadingKind::Temperature).unwrap();
        let a = mapping_for_alert(AlertKind::Freeze).unwrap();
        assert_eq!(r, a);
    }
}
