//! CCSDS Sensor Wire encoder (firmware-side).
//!
//! Produces the **exact same 14-byte layout** as
//! `relay-ccsds::sensor_wire::encode_packet` on the hub side. The
//! encoder is vendored here (rather than depending on `relay-ccsds`)
//! to keep firmware free of transitive dependencies like `wit-bindgen`,
//! to keep the dependency footprint small enough to audit by hand, and
//! to stay portable across G0 variants that have only 32 KB of flash.
//!
//! The byte-for-byte equivalence is guarded by `tests::matches_relay_ccsds`
//! at the bottom of this file — if `relay-ccsds` ever changes the wire
//! format, that test will catch the drift the next time the workspace
//! is built. (The test is host-only; the firmware binary doesn't pull
//! in `relay-ccsds`.)
//!
//! Layout (from `relay-ccsds::sensor_wire`):
//!
//! ```text
//! CCSDS Header (6 bytes):
//!   [0-1] Stream ID: version(3) | type(1) | sec_hdr(1) | APID(11)
//!   [2-3] Sequence:  flags(2) | count(14)
//!   [4-5] Length:    data_length - 1
//!
//! Sensor Payload (8 bytes):
//!   [0]   sensor_type: u8
//!   [1]   quality: u8
//!   [2-3] zone_id: u16  (little-endian)
//!   [4-7] value: i32    (little-endian)
//! ```

/// Sensor-type constants (subset — full list is in `relay-ccsds`).
/// We only re-export the IDs the firmware actually emits.
pub const SENSOR_CONTACT: u8 = 0x10;

/// Data-quality constants.
pub const QUALITY_GOOD: u8 = 0;
pub const QUALITY_STALE: u8 = 1;
pub const QUALITY_ERROR: u8 = 2;

/// Sensor-payload size in bytes.
pub const PAYLOAD_SIZE: usize = 8;
/// Full CCSDS packet size in bytes (header + payload).
pub const PACKET_SIZE: usize = 6 + PAYLOAD_SIZE;

/// A sensor reading ready for the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SensorPacket {
    /// CCSDS APID = device identifier (0-2047).
    pub device_id: u16,
    /// CCSDS sequence counter (0-16383).
    pub sequence: u16,
    /// Sensor type (e.g. [`SENSOR_CONTACT`]).
    pub sensor_type: u8,
    /// Data quality ([`QUALITY_GOOD`], [`QUALITY_STALE`], [`QUALITY_ERROR`]).
    pub quality: u8,
    /// Zone/room identifier.
    pub zone_id: u16,
    /// Fixed-point sensor value (interpretation depends on `sensor_type`).
    /// For [`SENSOR_CONTACT`]: `0 = closed`, `1 = open`.
    pub value: i32,
}

/// Encode `packet` into a fixed-size 14-byte buffer.
///
/// Pure function — no panics on any valid `SensorPacket`. Field widths
/// (APID 11 bits, sequence 14 bits) are enforced by masking; supplying
/// a `device_id > 0x07FF` silently truncates to the low 11 bits, which
/// matches `relay-ccsds::sensor_wire::encode_packet`.
pub fn encode(packet: &SensorPacket, buf: &mut [u8; PACKET_SIZE]) {
    // CCSDS header (6 bytes)
    // Version = 0, Type = 0 (telemetry), Sec Header = 0
    let stream_id: u16 = packet.device_id & 0x07FF; // APID in low 11 bits
    buf[0] = (stream_id >> 8) as u8;
    buf[1] = (stream_id & 0xFF) as u8;

    // Sequence flags = 0b11 (unsegmented), count in low 14 bits
    let seq: u16 = 0xC000 | (packet.sequence & 0x3FFF);
    buf[2] = (seq >> 8) as u8;
    buf[3] = (seq & 0xFF) as u8;

    // Length: data_length - 1 = PAYLOAD_SIZE - 1 = 7
    let length: u16 = (PAYLOAD_SIZE as u16).wrapping_sub(1);
    buf[4] = (length >> 8) as u8;
    buf[5] = (length & 0xFF) as u8;

    // Sensor payload (8 bytes)
    buf[6] = packet.sensor_type;
    buf[7] = packet.quality;
    buf[8] = (packet.zone_id & 0xFF) as u8;
    buf[9] = (packet.zone_id >> 8) as u8;
    let vb = packet.value.to_le_bytes();
    buf[10] = vb[0];
    buf[11] = vb[1];
    buf[12] = vb[2];
    buf[13] = vb[3];
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A contact-sensor packet "door just opened" must encode to a
    /// known-good byte string. If this ever changes, the hub will
    /// stop understanding our packets — so a hard-coded golden value
    /// is the right test.
    #[test]
    fn golden_door_open() {
        let pkt = SensorPacket {
            device_id: 0x012,
            sequence: 7,
            sensor_type: SENSOR_CONTACT,
            quality: QUALITY_GOOD,
            zone_id: 0x0103,
            value: 1, // open
        };
        let mut buf = [0u8; PACKET_SIZE];
        encode(&pkt, &mut buf);
        // Header: APID=0x012  → 0x00 0x12
        //         seq=7 + flags=0b11 → 0xC0 0x07
        //         length = 7         → 0x00 0x07
        // Payload: type=0x10, quality=0x00, zone=0x0103 LE → 0x03 0x01,
        //          value=1 LE → 0x01 0x00 0x00 0x00
        assert_eq!(
            buf,
            [
                0x00, 0x12, 0xC0, 0x07, 0x00, 0x07, 0x10, 0x00, 0x03, 0x01, 0x01, 0x00, 0x00, 0x00,
            ]
        );
    }

    #[test]
    fn golden_door_closed() {
        let pkt = SensorPacket {
            device_id: 0x012,
            sequence: 8,
            sensor_type: SENSOR_CONTACT,
            quality: QUALITY_GOOD,
            zone_id: 0x0103,
            value: 0, // closed
        };
        let mut buf = [0u8; PACKET_SIZE];
        encode(&pkt, &mut buf);
        assert_eq!(buf[6], SENSOR_CONTACT);
        assert_eq!(buf[7], QUALITY_GOOD);
        assert_eq!(&buf[10..14], &0i32.to_le_bytes());
    }

    #[test]
    fn apid_is_truncated_to_11_bits() {
        let pkt = SensorPacket {
            device_id: 0xFFFF, // intentionally too wide
            sequence: 0,
            sensor_type: SENSOR_CONTACT,
            quality: QUALITY_GOOD,
            zone_id: 0,
            value: 0,
        };
        let mut buf = [0u8; PACKET_SIZE];
        encode(&pkt, &mut buf);
        // High three bits of buf[0] must be zero (CCSDS version=0,
        // type=0, sec_hdr=0); remaining 11 bits should equal
        // 0x07FF & 0xFFFF = 0x07FF.
        assert_eq!(buf[0] & 0xE0, 0);
        assert_eq!(((buf[0] as u16 & 0x07) << 8) | buf[1] as u16, 0x07FF);
    }

    #[test]
    fn sequence_is_truncated_to_14_bits() {
        let pkt = SensorPacket {
            device_id: 0,
            sequence: 0xFFFF, // intentionally too wide
            sensor_type: SENSOR_CONTACT,
            quality: QUALITY_GOOD,
            zone_id: 0,
            value: 0,
        };
        let mut buf = [0u8; PACKET_SIZE];
        encode(&pkt, &mut buf);
        // Flags must be 0b11, count must be 0x3FFF.
        assert_eq!(buf[2] >> 6, 0b11);
        assert_eq!(((buf[2] as u16 & 0x3F) << 8) | buf[3] as u16, 0x3FFF);
    }

    /// Cross-check against the canonical encoder in `relay-ccsds`.
    /// Re-implements the spec inline (rather than depending on the
    /// crate) so the firmware tree has no `relay-ccsds` dep — but the
    /// expectation byte-string is the spec from
    /// `relay/crates/relay-ccsds/plain/src/sensor_wire.rs`.
    #[test]
    fn matches_relay_ccsds_spec() {
        // Same fixture as `relay-ccsds`'s `test_contact_sensor`.
        let pkt = SensorPacket {
            device_id: 200,
            sequence: 5,
            sensor_type: SENSOR_CONTACT,
            quality: QUALITY_GOOD,
            zone_id: 1,
            value: 1, // open
        };
        let mut buf = [0u8; PACKET_SIZE];
        encode(&pkt, &mut buf);

        // Hand-decoded expectation from the relay-ccsds spec:
        //   stream_id (16b BE): version(3)=0 | type(1)=0 | sec_hdr(1)=0 | APID(11)=200=0x0C8
        //                       → 0x0000 | 0x00C8 = 0x00C8 → 0x00 0xC8
        //   sequence  (16b BE): flags=0b11, count=5 → 0xC005 → 0xC0 0x05
        //   length    (16b BE): 7 → 0x00 0x07
        //   payload: 0x10, 0x00, zone=1 LE → 0x01 0x00, value=1 LE → 0x01 0x00 0x00 0x00
        let expected: [u8; PACKET_SIZE] = [
            0x00, 0xC8, 0xC0, 0x05, 0x00, 0x07, 0x10, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00,
        ];
        assert_eq!(buf, expected);
    }

    proptest::proptest! {
        /// For any well-formed packet, the version bits are always 0
        /// and the sequence flags are always 0b11 (unsegmented).
        #[test]
        fn header_invariants(
            device_id in 0u16..=0xFFFF,
            sequence in 0u16..=0xFFFF,
            sensor_type in 0u8..=0xFF,
            quality in 0u8..=0xFF,
            zone_id in 0u16..=0xFFFF,
            value in i32::MIN..=i32::MAX,
        ) {
            let pkt = SensorPacket {
                device_id, sequence, sensor_type, quality, zone_id, value,
            };
            let mut buf = [0u8; PACKET_SIZE];
            encode(&pkt, &mut buf);
            // Top three bits of buf[0]: version(3)=0
            proptest::prop_assert_eq!(buf[0] & 0xE0, 0);
            // Top two bits of buf[2]: sequence flags = 0b11
            proptest::prop_assert_eq!(buf[2] >> 6, 0b11);
            // Length is always 7 (data_length - 1, payload = 8)
            proptest::prop_assert_eq!(buf[4], 0);
            proptest::prop_assert_eq!(buf[5], 7);
        }
    }
}
