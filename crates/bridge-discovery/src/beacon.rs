use crate::error::{DiscoveryError, Result};
use bridge_core::{Capabilities, DeviceType, NodeId, ProtocolVersion};
use serde::{Deserialize, Serialize};

/// 4-byte protocol magic prefix for BridgeOS discovery beacons: "BRGB" (BridgeOS Beacon)
pub const BEACON_MAGIC: [u8; 4] = *b"BRGB";

/// Protocol version of the beacon envelope.
pub const BEACON_VERSION: u8 = 1;

/// Maximum permitted beacon packet size (1400 bytes) to prevent IP fragmentation and memory abuse.
pub const MAX_BEACON_SIZE: usize = 1400;

/// Maximum permitted device name byte length in beacon announcements.
pub const MAX_DEVICE_NAME_LEN: usize = 64;

/// Messages exchanged via UDP broadcast for peer discovery and presence management.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BeaconMessage {
    /// Periodic announcement or immediate response to network entry / query probe.
    Announcement {
        version: ProtocolVersion,
        node_id: NodeId,
        device_name: String,
        device_type: DeviceType,
        port: u16,
        capabilities: Capabilities,
        seq: u64,
    },
    /// Explicit goodbye announcement sent upon node shutdown.
    Goodbye { node_id: NodeId, seq: u64 },
    /// Query probe sent by a joining node to solicit immediate announcements from peers.
    Query { sender_id: NodeId, seq: u64 },
}

impl BeaconMessage {
    /// Returns the node identifier associated with this beacon message.
    pub fn node_id(&self) -> NodeId {
        match self {
            Self::Announcement { node_id, .. } | Self::Goodbye { node_id, .. } => *node_id,
            Self::Query { sender_id, .. } => *sender_id,
        }
    }
}

/// Encodes a beacon message into a bounded byte datagram prefixed with `BEACON_MAGIC` and `BEACON_VERSION`.
pub fn encode_beacon(message: &BeaconMessage) -> Result<Vec<u8>> {
    let payload = postcard::to_allocvec(message)
        .map_err(|e| DiscoveryError::Internal(format!("failed to serialize beacon: {e}")))?;

    let total_len = 4 + 1 + payload.len();
    if total_len > MAX_BEACON_SIZE {
        return Err(DiscoveryError::InvalidPacket(format!(
            "beacon packet size {total_len} exceeds maximum allowed {MAX_BEACON_SIZE}"
        )));
    }

    let mut buf = Vec::with_capacity(total_len);
    buf.extend_from_slice(&BEACON_MAGIC);
    buf.push(BEACON_VERSION);
    buf.extend_from_slice(&payload);
    Ok(buf)
}

/// Decodes and validates a raw UDP datagram into a verified `BeaconMessage`.
pub fn decode_beacon(bytes: &[u8]) -> Result<BeaconMessage> {
    if bytes.len() < 5 {
        return Err(DiscoveryError::InvalidPacket(format!(
            "datagram length {} is too short for a valid beacon",
            bytes.len()
        )));
    }

    if bytes.len() > MAX_BEACON_SIZE {
        return Err(DiscoveryError::InvalidPacket(format!(
            "datagram length {} exceeds maximum allowed {}",
            bytes.len(),
            MAX_BEACON_SIZE
        )));
    }

    if bytes[0..4] != BEACON_MAGIC {
        return Err(DiscoveryError::InvalidPacket(format!(
            "invalid magic header {:?}, expected {:?}",
            &bytes[0..4],
            BEACON_MAGIC
        )));
    }

    if bytes[4] != BEACON_VERSION {
        return Err(DiscoveryError::InvalidPacket(format!(
            "unsupported beacon version {}, expected {}",
            bytes[4], BEACON_VERSION
        )));
    }

    let mut msg: BeaconMessage = postcard::from_bytes(&bytes[5..])
        .map_err(|e| DiscoveryError::InvalidPacket(format!("failed to deserialize beacon: {e}")))?;

    // Validate and sanitize fields
    if let BeaconMessage::Announcement {
        ref mut device_name,
        port,
        ..
    } = msg
    {
        if port == 0 {
            return Err(DiscoveryError::InvalidPacket(
                "announcement port cannot be 0".to_string(),
            ));
        }

        if device_name.len() > MAX_DEVICE_NAME_LEN {
            // Safely truncate to max boundary
            let mut end = MAX_DEVICE_NAME_LEN;
            while !device_name.is_char_boundary(end) && end > 0 {
                end -= 1;
            }
            device_name.truncate(end);
        }
    }

    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_beacon_announcement_encode_decode_roundtrip() {
        let node_id = NodeId::from_bytes([0x42; 32]);
        let msg = BeaconMessage::Announcement {
            version: ProtocolVersion::CURRENT,
            node_id,
            device_name: "TestDevice".to_string(),
            device_type: DeviceType::Windows,
            port: 8888,
            capabilities: Capabilities::all(),
            seq: 101,
        };

        let encoded = encode_beacon(&msg).expect("encode beacon");
        assert_eq!(&encoded[0..4], &BEACON_MAGIC);
        assert_eq!(encoded[4], BEACON_VERSION);

        let decoded = decode_beacon(&encoded).expect("decode beacon");
        assert_eq!(decoded, msg);
        assert_eq!(decoded.node_id(), node_id);
    }

    #[test]
    fn test_beacon_goodbye_roundtrip() {
        let node_id = NodeId::from_bytes([0x99; 32]);
        let msg = BeaconMessage::Goodbye { node_id, seq: 42 };

        let encoded = encode_beacon(&msg).expect("encode goodbye");
        let decoded = decode_beacon(&encoded).expect("decode goodbye");
        assert_eq!(decoded, msg);
        assert_eq!(decoded.node_id(), node_id);
    }

    #[test]
    fn test_beacon_query_roundtrip() {
        let sender_id = NodeId::from_bytes([0x12; 32]);
        let msg = BeaconMessage::Query { sender_id, seq: 1 };

        let encoded = encode_beacon(&msg).expect("encode query");
        let decoded = decode_beacon(&encoded).expect("decode query");
        assert_eq!(decoded, msg);
        assert_eq!(decoded.node_id(), sender_id);
    }

    #[test]
    fn test_decode_invalid_magic() {
        let mut buf = vec![b'X', b'X', b'X', b'X', BEACON_VERSION];
        buf.extend_from_slice(&[0u8; 10]);
        let err = decode_beacon(&buf).unwrap_err();
        assert!(matches!(err, DiscoveryError::InvalidPacket(_)));
    }

    #[test]
    fn test_decode_invalid_version() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&BEACON_MAGIC);
        buf.push(99); // unsupported version
        buf.extend_from_slice(&[0u8; 10]);
        let err = decode_beacon(&buf).unwrap_err();
        assert!(matches!(err, DiscoveryError::InvalidPacket(_)));
    }

    #[test]
    fn test_decode_too_short() {
        let buf = *b"BR";
        let err = decode_beacon(&buf).unwrap_err();
        assert!(matches!(err, DiscoveryError::InvalidPacket(_)));
    }

    #[test]
    fn test_decode_device_name_truncation() {
        let node_id = NodeId::from_bytes([0x55; 32]);
        let long_name = "A".repeat(128);
        let msg = BeaconMessage::Announcement {
            version: ProtocolVersion::CURRENT,
            node_id,
            device_name: long_name,
            device_type: DeviceType::Android,
            port: 7000,
            capabilities: Capabilities::default(),
            seq: 1,
        };

        let encoded = encode_beacon(&msg).expect("encode beacon");
        let decoded = decode_beacon(&encoded).expect("decode beacon");
        if let BeaconMessage::Announcement { device_name, .. } = decoded {
            assert_eq!(device_name.len(), MAX_DEVICE_NAME_LEN);
        } else {
            panic!("expected announcement");
        }
    }

    #[test]
    fn test_decode_zero_port_rejected() {
        let node_id = NodeId::from_bytes([0x55; 32]);
        let msg = BeaconMessage::Announcement {
            version: ProtocolVersion::CURRENT,
            node_id,
            device_name: "ZeroPort".to_string(),
            device_type: DeviceType::Linux,
            port: 0,
            capabilities: Capabilities::default(),
            seq: 1,
        };

        let encoded = encode_beacon(&msg).expect("encode beacon");
        let err = decode_beacon(&encoded).unwrap_err();
        assert!(matches!(err, DiscoveryError::InvalidPacket(_)));
    }
}
