use crate::handshake::HandshakeFrame;
use serde::{Deserialize, Serialize};

/// 4-byte protocol magic prefix: "BRG1"
pub const MAGIC: [u8; 4] = *b"BRG1";

/// Maximum permitted frame length in bytes (16 megabytes) to prevent OOM DOS.
pub const MAX_FRAME_LENGTH: usize = 16 * 1024 * 1024;

/// Root frame transmitted across BridgeOS streams.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Frame {
    /// Connection setup, capability negotiation, and authentication.
    Handshake(HandshakeFrame),

    /// Keepalives and connection lifecycle signals.
    Control(ControlFrame),

    /// Application subsystem data (clipboard, file chunk, notification).
    Data(DataFrame),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlFrame {
    Ping { nonce: u64 },
    Pong { nonce: u64 },
    Disconnect { reason: DisconnectReason },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DisconnectReason {
    Graceful,
    ProtocolError,
    AuthenticationFailed,
    Timeout,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataFrame {
    /// Logical multiplex channel (e.g. 1 = Clipboard, 2 = File Transfer, 3 = Notifications, 4 = Input).
    pub channel: u16,
    /// Arbitrary binary payload or subsystem-specific serialized message.
    pub payload: Vec<u8>,
}

impl DataFrame {
    pub const CHANNEL_CLIPBOARD: u16 = 1;
    pub const CHANNEL_FILE_TRANSFER: u16 = 2;
    pub const CHANNEL_NOTIFICATIONS: u16 = 3;
    pub const CHANNEL_REMOTE_INPUT: u16 = 4;

    pub fn new(channel: u16, payload: impl Into<Vec<u8>>) -> Self {
        Self {
            channel,
            payload: payload.into(),
        }
    }
}
