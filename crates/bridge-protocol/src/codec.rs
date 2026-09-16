use crate::frame::{Frame, MAGIC, MAX_FRAME_LENGTH};
use bridge_core::{BridgeError, Result};

/// Encodes a `Frame` into the standard wire format:
/// `[MAGIC: 4 bytes] [LENGTH: 4 bytes big-endian] [PAYLOAD: N bytes]`
pub fn encode_frame(frame: &Frame) -> Result<Vec<u8>> {
    let payload = postcard::to_allocvec(frame)
        .map_err(|e| BridgeError::Serialization(format!("Postcard serialization error: {e}")))?;

    let len = payload.len();
    if len > MAX_FRAME_LENGTH {
        return Err(BridgeError::FrameTooLarge {
            length: len,
            max: MAX_FRAME_LENGTH,
        });
    }

    let len_u32 = u32::try_from(len).map_err(|_| BridgeError::FrameTooLarge {
        length: len,
        max: MAX_FRAME_LENGTH,
    })?;

    let mut buf = Vec::with_capacity(4 + 4 + len);
    buf.extend_from_slice(&MAGIC);
    buf.extend_from_slice(&len_u32.to_be_bytes());
    buf.extend_from_slice(&payload);
    Ok(buf)
}

/// Decodes a raw slice containing `[MAGIC] [LENGTH] [PAYLOAD]` into a `Frame`.
pub fn decode_frame(bytes: &[u8]) -> Result<Frame> {
    if bytes.len() < 8 {
        return Err(BridgeError::Framing(format!(
            "Buffer too short for header: got {} bytes, expected at least 8",
            bytes.len()
        )));
    }

    if bytes[0..4] != MAGIC {
        return Err(BridgeError::InvalidMagic);
    }

    let len = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    if len > MAX_FRAME_LENGTH {
        return Err(BridgeError::FrameTooLarge {
            length: len,
            max: MAX_FRAME_LENGTH,
        });
    }

    if bytes.len() < 8 + len {
        return Err(BridgeError::Framing(format!(
            "Incomplete frame: expected {} bytes total, have {}",
            8 + len,
            bytes.len()
        )));
    }

    let payload = &bytes[8..8 + len];
    decode_payload(payload)
}

/// Decodes an already extracted frame payload slice.
pub fn decode_payload(payload: &[u8]) -> Result<Frame> {
    postcard::from_bytes(payload)
        .map_err(|e| BridgeError::Serialization(format!("Postcard deserialization error: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::ControlFrame;
    use crate::handshake::{ClientHello, HandshakeFrame};
    use bridge_core::{Capabilities, NodeId, ProtocolVersion};

    #[test]
    fn test_encode_decode_roundtrip() {
        let frame = Frame::Control(ControlFrame::Ping { nonce: 987_654_321 });
        let encoded = encode_frame(&frame).expect("Failed to encode");

        assert_eq!(&encoded[0..4], &MAGIC);
        let decoded = decode_frame(&encoded).expect("Failed to decode");
        assert_eq!(frame, decoded);
    }

    #[test]
    fn test_client_hello_roundtrip() {
        let hello = ClientHello {
            version: ProtocolVersion::CURRENT,
            node_id: NodeId::from_bytes([7u8; 32]),
            device_name: "Desktop-Win11".to_string(),
            client_nonce: [12u8; 32],
            capabilities: Capabilities::all(),
        };

        let frame = Frame::Handshake(HandshakeFrame::ClientHello(hello));
        let encoded = encode_frame(&frame).expect("Failed to encode");
        let decoded = decode_frame(&encoded).expect("Failed to decode");
        assert_eq!(frame, decoded);
    }

    #[test]
    fn test_invalid_magic_rejected() {
        let mut encoded = encode_frame(&Frame::Control(ControlFrame::Pong { nonce: 1 })).unwrap();
        encoded[0] = b'X'; // Corrupt magic

        let err = decode_frame(&encoded).unwrap_err();
        assert_eq!(err, BridgeError::InvalidMagic);
    }

    #[test]
    fn test_incomplete_frame_rejected() {
        let encoded = encode_frame(&Frame::Control(ControlFrame::Pong { nonce: 1 })).unwrap();
        // Truncate frame
        let truncated = &encoded[..encoded.len() - 2];
        let err = decode_frame(truncated).unwrap_err();
        match err {
            BridgeError::Framing(_) => (),
            other => panic!("Expected Framing error, got {other:?}"),
        }
    }
}
