//! [`LoggingBridge`] — stub [`MatterBridge`] that logs what it *would*
//! publish to a real Matter fabric.
//!
//! Used by `wohl-hub --matter` in 0.2.0 to validate the wiring end-to-end
//! without pulling in rs-matter. The output format is deliberately
//! human-readable (single line per event, on stderr) — it is not a stable
//! contract.

use std::io::{self, Write};
use std::sync::Mutex;

use crate::MatterBridge;
use crate::cluster::{mapping_for_alert, mapping_for_reading};
use crate::types::{BridgedAlert, SensorReading};

/// Logging stub. Each call writes a single line to a configurable sink
/// (stderr by default) describing the would-be Matter publish.
pub struct LoggingBridge {
    sink: Mutex<Box<dyn Write + Send>>,
}

impl LoggingBridge {
    /// Default constructor: log to stderr.
    pub fn to_stderr() -> Self {
        Self {
            sink: Mutex::new(Box::new(io::stderr())),
        }
    }

    /// Inject an alternate sink — used by tests to capture output.
    pub fn with_sink<W: Write + Send + 'static>(sink: W) -> Self {
        Self {
            sink: Mutex::new(Box::new(sink)),
        }
    }

    fn write_line(&self, line: &str) {
        if let Ok(mut s) = self.sink.lock() {
            let _ = writeln!(s, "{}", line);
        }
    }
}

impl Default for LoggingBridge {
    fn default() -> Self {
        Self::to_stderr()
    }
}

impl MatterBridge for LoggingBridge {
    fn publish_reading(&self, reading: SensorReading) {
        let Some(m) = mapping_for_reading(reading.kind) else {
            return;
        };
        self.write_line(&format!(
            "[matter-bridge] reading endpoint={} cluster=0x{:04X} attr=0x{:04X} value={} t={}",
            reading.endpoint_id,
            m.cluster.cluster_id(),
            m.attribute.attribute_id(),
            reading.value,
            reading.time,
        ));
    }

    fn publish_alert(&self, alert: BridgedAlert) {
        let Some(m) = mapping_for_alert(alert.kind) else {
            // Unmapped (e.g. HealthMiss). Drop silently — see DESIGN.md.
            return;
        };
        let endpoint = alert
            .zone_id
            .or(alert.contact_id)
            .or(alert.circuit_id)
            .unwrap_or(0);
        let value = alert
            .value
            .map(|v| v.to_string())
            .unwrap_or_else(|| "-".to_string());
        self.write_line(&format!(
            "[matter-bridge] alert kind={} endpoint={} cluster=0x{:04X} attr=0x{:04X} value={} t={}",
            alert.kind.as_tag(),
            endpoint,
            m.cluster.cluster_id(),
            m.attribute.attribute_id(),
            value,
            alert.time,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AlertKind, ReadingKind};
    use std::sync::{Arc, Mutex as StdMutex};

    /// Test sink: keeps written bytes in a shared buffer.
    #[derive(Clone)]
    struct VecSink(Arc<StdMutex<Vec<u8>>>);

    impl Write for VecSink {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn make_bridge() -> (LoggingBridge, Arc<StdMutex<Vec<u8>>>) {
        let buf = Arc::new(StdMutex::new(Vec::new()));
        let sink = VecSink(buf.clone());
        (LoggingBridge::with_sink(sink), buf)
    }

    fn captured(buf: &Arc<StdMutex<Vec<u8>>>) -> String {
        String::from_utf8(buf.lock().unwrap().clone()).unwrap()
    }

    #[test]
    fn logs_temperature_reading_with_correct_cluster_and_attribute() {
        let (bridge, buf) = make_bridge();
        bridge.publish_reading(SensorReading {
            kind: ReadingKind::Temperature,
            endpoint_id: 7,
            value: -100,
            time: 1234,
        });
        let out = captured(&buf);
        assert!(out.contains("endpoint=7"), "got: {}", out);
        assert!(out.contains("cluster=0x0402"), "got: {}", out);
        assert!(out.contains("attr=0x0000"), "got: {}", out);
        assert!(out.contains("value=-100"), "got: {}", out);
        assert!(out.contains("t=1234"), "got: {}", out);
    }

    #[test]
    fn logs_freeze_alert_with_temperature_cluster() {
        let (bridge, buf) = make_bridge();
        bridge.publish_alert(BridgedAlert {
            kind: AlertKind::Freeze,
            zone_id: Some(3),
            contact_id: None,
            circuit_id: None,
            value: Some(-150),
            time: 5000,
        });
        let out = captured(&buf);
        assert!(out.contains("kind=freeze"));
        assert!(out.contains("endpoint=3"));
        assert!(out.contains("cluster=0x0402"));
        assert!(out.contains("value=-150"));
    }

    #[test]
    fn logs_water_leak_alert_with_boolean_state() {
        let (bridge, buf) = make_bridge();
        bridge.publish_alert(BridgedAlert {
            kind: AlertKind::WaterLeak,
            zone_id: Some(2),
            contact_id: None,
            circuit_id: None,
            value: None,
            time: 99,
        });
        let out = captured(&buf);
        assert!(out.contains("kind=water_leak"));
        assert!(out.contains("cluster=0x0045"));
        assert!(out.contains("value=-")); // None rendered as "-"
    }

    #[test]
    fn health_miss_alert_is_dropped_silently() {
        let (bridge, buf) = make_bridge();
        bridge.publish_alert(BridgedAlert {
            kind: AlertKind::HealthMiss,
            zone_id: None,
            contact_id: None,
            circuit_id: None,
            value: Some(1),
            time: 10,
        });
        assert!(captured(&buf).is_empty(), "health-miss must not be bridged");
    }

    #[test]
    fn endpoint_falls_back_through_zone_contact_circuit() {
        let (bridge, buf) = make_bridge();
        bridge.publish_alert(BridgedAlert {
            kind: AlertKind::PowerSpike,
            zone_id: None,
            contact_id: None,
            circuit_id: Some(11),
            value: Some(15000),
            time: 1,
        });
        let out = captured(&buf);
        assert!(out.contains("endpoint=11"), "got: {}", out);
    }

    #[test]
    fn trait_object_is_constructible() {
        // Compile-time check: LoggingBridge fits the dyn-compatible trait.
        let _b: Box<dyn MatterBridge> = Box::new(LoggingBridge::to_stderr());
    }
}
