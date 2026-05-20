//! High-level door state machine.
//!
//! Composes [`debounce`](crate::debounce) and [`ccsds`](crate::ccsds):
//! every confirmed debounced edge produces one CCSDS packet with a
//! monotonically increasing sequence counter. Wrap-around at 14 bits
//! is intentional — the hub re-syncs through CCSDS sequence-flag rules.

use crate::ccsds::{PACKET_SIZE, QUALITY_GOOD, SENSOR_CONTACT, SensorPacket, encode};
use crate::debounce::{DEFAULT_STABLE_TICKS, Debouncer, DoorLevel, Edge};

/// Identity + persistence used by the door firmware. Fields are
/// `Copy` so they live in a single `&mut State` without allocation.
#[derive(Clone, Copy, Debug)]
pub struct DoorState {
    /// CCSDS APID for this node (0-2047).
    pub device_id: u16,
    /// Zone/room identifier (free-form within the deployment).
    pub zone_id: u16,
    /// Next sequence number to emit. Wraps at 2^14.
    pub next_sequence: u16,
    /// Debouncer with the project-default stable window.
    pub debouncer: Debouncer<DEFAULT_STABLE_TICKS>,
}

impl DoorState {
    /// Construct fresh state. `initial_level` is the GPIO read once
    /// at boot, after pull-up has settled (typically a `for _ in 0..1000`
    /// nop loop, or simply reading after clock setup).
    pub const fn new(device_id: u16, zone_id: u16, initial_level: DoorLevel) -> Self {
        Self {
            device_id,
            zone_id,
            next_sequence: 0,
            debouncer: Debouncer::new(initial_level),
        }
    }

    /// Feed a sample; if a confirmed edge fires, encode and return a
    /// packet ready for UART TX. Otherwise returns `None`.
    pub fn step(&mut self, sample: DoorLevel) -> Option<[u8; PACKET_SIZE]> {
        let edge = self.debouncer.update(sample)?;
        let value = match edge {
            Edge::Opened => DoorLevel::Open.as_value(),
            Edge::Closed => DoorLevel::Closed.as_value(),
        };
        let packet = SensorPacket {
            device_id: self.device_id,
            sequence: self.next_sequence & 0x3FFF,
            sensor_type: SENSOR_CONTACT,
            quality: QUALITY_GOOD,
            zone_id: self.zone_id,
            value,
        };
        // Wrap explicitly at 14 bits to make the bound obvious in `&mut`.
        self.next_sequence = self.next_sequence.wrapping_add(1) & 0x3FFF;
        let mut buf = [0u8; PACKET_SIZE];
        encode(&packet, &mut buf);
        Some(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ccsds::SENSOR_CONTACT;

    fn drain<F: FnMut(&mut DoorState) -> Option<[u8; PACKET_SIZE]>>(
        state: &mut DoorState,
        mut step: F,
        n: usize,
    ) -> Vec<[u8; PACKET_SIZE]> {
        let mut out = Vec::new();
        for _ in 0..n {
            if let Some(buf) = step(state) {
                out.push(buf);
            }
        }
        out
    }

    #[test]
    fn open_event_emits_one_packet() {
        let mut s = DoorState::new(0x42, 0x0103, DoorLevel::Closed);
        let pkts = drain(&mut s, |s| s.step(DoorLevel::Open), 60);
        assert_eq!(pkts.len(), 1);
        let p = &pkts[0];
        // Sensor type byte at offset 6 must be SENSOR_CONTACT.
        assert_eq!(p[6], SENSOR_CONTACT);
        // value at [10..14] must be 1 (open).
        assert_eq!(&p[10..14], &1i32.to_le_bytes());
        // Sequence count starts at 0.
        assert_eq!(((p[2] as u16 & 0x3F) << 8) | p[3] as u16, 0);
    }

    #[test]
    fn close_event_after_open_increments_sequence() {
        let mut s = DoorState::new(0x42, 0x0103, DoorLevel::Closed);
        let opens = drain(&mut s, |s| s.step(DoorLevel::Open), 60);
        let closes = drain(&mut s, |s| s.step(DoorLevel::Closed), 60);
        assert_eq!(opens.len(), 1);
        assert_eq!(closes.len(), 1);
        // Second packet has sequence = 1.
        let p = &closes[0];
        assert_eq!(((p[2] as u16 & 0x3F) << 8) | p[3] as u16, 1);
        // value at [10..14] must be 0 (closed).
        assert_eq!(&p[10..14], &0i32.to_le_bytes());
    }

    #[test]
    fn stable_input_emits_nothing() {
        let mut s = DoorState::new(0x42, 0x0103, DoorLevel::Closed);
        let pkts = drain(&mut s, |s| s.step(DoorLevel::Closed), 10_000);
        assert!(pkts.is_empty());
    }

    #[test]
    fn sequence_wraps_at_14_bits() {
        let mut s = DoorState::new(0x42, 0x0103, DoorLevel::Closed);
        // Force the sequence counter to wrap by setting it just below.
        s.next_sequence = 0x3FFE;
        // Sample 50 ticks of "open" → emit one packet with seq=0x3FFE.
        let pkts = drain(&mut s, |s| s.step(DoorLevel::Open), 60);
        assert_eq!(pkts.len(), 1);
        let p = &pkts[0];
        assert_eq!(((p[2] as u16 & 0x3F) << 8) | p[3] as u16, 0x3FFE);
        // Now state.next_sequence == 0x3FFF.
        assert_eq!(s.next_sequence, 0x3FFF);
        // Sample 50 ticks of "closed" → emit one packet with seq=0x3FFF;
        // counter then wraps to 0.
        let pkts = drain(&mut s, |s| s.step(DoorLevel::Closed), 60);
        assert_eq!(pkts.len(), 1);
        let p = &pkts[0];
        assert_eq!(((p[2] as u16 & 0x3F) << 8) | p[3] as u16, 0x3FFF);
        assert_eq!(s.next_sequence, 0);
    }
}
