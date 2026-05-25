//! Bridge-facing event types.
//!
//! Mirrors wohl-hub's `SensorEvent` / `AlertOutput` shape but with a stable,
//! Matter-oriented vocabulary. Kept deliberately minimal — the bridge only
//! needs enough to pick a cluster and emit a value.

/// What kind of alert fired. Matches the 1:1 wohl alert names used in
/// wohl-hub's `AlertOutput.alert` string (e.g. `"freeze"`, `"water_leak"`).
///
/// Encoded as an enum so the cluster mapping is exhaustive at compile time
/// (no stringly-typed lookups). The `from_str` helper does the conversion
/// from wohl-hub's existing string tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AlertKind {
    Freeze,
    Overheat,
    RapidDrop,
    RapidRise,
    WaterLeak,
    Co2Warning,
    Co2Critical,
    Pm25Warning,
    Pm25Critical,
    VocWarning,
    VocCritical,
    DoorOpenTooLong,
    DoorOpenedAtNight,
    Overconsumption,
    PowerSpike,
    DeviceLeftOn,
    HealthMiss,
}

impl AlertKind {
    /// Parse from wohl-hub's `AlertOutput.alert` string tag. Unknown tags
    /// return `None` — the bridge skips them rather than crash.
    pub fn from_tag(s: &str) -> Option<Self> {
        Some(match s {
            "freeze" => Self::Freeze,
            "overheat" => Self::Overheat,
            "rapid_drop" => Self::RapidDrop,
            "rapid_rise" => Self::RapidRise,
            "water_leak" => Self::WaterLeak,
            "co2_warning" => Self::Co2Warning,
            "co2_critical" => Self::Co2Critical,
            "pm25_warning" => Self::Pm25Warning,
            "pm25_critical" => Self::Pm25Critical,
            "voc_warning" => Self::VocWarning,
            "voc_critical" => Self::VocCritical,
            "door_open_too_long" => Self::DoorOpenTooLong,
            "door_opened_at_night" => Self::DoorOpenedAtNight,
            "overconsumption" => Self::Overconsumption,
            "power_spike" => Self::PowerSpike,
            "device_left_on" => Self::DeviceLeftOn,
            "health_miss" => Self::HealthMiss,
            _ => return None,
        })
    }

    /// Human-readable tag, the inverse of `from_tag`.
    pub fn as_tag(self) -> &'static str {
        match self {
            Self::Freeze => "freeze",
            Self::Overheat => "overheat",
            Self::RapidDrop => "rapid_drop",
            Self::RapidRise => "rapid_rise",
            Self::WaterLeak => "water_leak",
            Self::Co2Warning => "co2_warning",
            Self::Co2Critical => "co2_critical",
            Self::Pm25Warning => "pm25_warning",
            Self::Pm25Critical => "pm25_critical",
            Self::VocWarning => "voc_warning",
            Self::VocCritical => "voc_critical",
            Self::DoorOpenTooLong => "door_open_too_long",
            Self::DoorOpenedAtNight => "door_opened_at_night",
            Self::Overconsumption => "overconsumption",
            Self::PowerSpike => "power_spike",
            Self::DeviceLeftOn => "device_left_on",
            Self::HealthMiss => "health_miss",
        }
    }
}

/// What kind of periodic reading the sensor produced. Distinguishes the
/// channel from the value's interpretation (the value lives in
/// [`SensorReading::value`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReadingKind {
    /// Temperature, centi-degrees Celsius (matches Wohl wire format).
    Temperature,
    /// CO₂ concentration, ppm.
    Co2,
    /// PM2.5 concentration, μg/m³.
    Pm25,
    /// Volatile organic compounds, index value.
    Voc,
    /// Active power, watts.
    Power,
    /// Contact state (open=1 / closed=0). Treated as a reading rather than
    /// an alert when the hub forwards bare state changes.
    Contact,
    /// Wet/dry leak state.
    WaterPresence,
}

/// An alert flagged by a monitor that survived dedup + rate-limit and is
/// ready to be reflected on the Matter fabric.
#[derive(Debug, Clone)]
pub struct BridgedAlert {
    pub kind: AlertKind,
    /// Wohl zone id (matches `wohl-hub`'s zone numbering). `None` for
    /// non-zone-scoped alerts (e.g. health-miss).
    pub zone_id: Option<u32>,
    /// Contact id for door/window alerts.
    pub contact_id: Option<u32>,
    /// Circuit id for power alerts.
    pub circuit_id: Option<u32>,
    /// Reading value at the time the alert fired, if applicable
    /// (e.g. the temperature in centi-degrees for a freeze alert).
    pub value: Option<i64>,
    /// Hub wall-clock timestamp, seconds since UNIX epoch.
    pub time: u64,
}

/// A periodic sensor reading — the steady-state stream of values for
/// each bridged endpoint.
#[derive(Debug, Clone)]
pub struct SensorReading {
    pub kind: ReadingKind,
    /// Endpoint id within the bridge. Maps to a wohl zone, contact, or
    /// circuit — the implementor decides how to flatten the three
    /// id spaces onto Matter endpoint ids.
    pub endpoint_id: u32,
    /// The value, in the unit implied by `kind` (centi-degrees for
    /// Temperature, ppm for Co2, etc.).
    pub value: i64,
    /// Hub wall-clock timestamp, seconds since UNIX epoch.
    pub time: u64,
}
