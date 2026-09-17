use thiserror::Error;

/// Domain errors for the BridgeOS clipboard synchronization engine.
#[derive(Debug, Error)]
pub enum ClipboardError {
    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Deserialization error: {0}")]
    Deserialization(String),

    #[error("Payload exceeds maximum permitted size: {size} bytes (max: {max} bytes)")]
    OversizedPayload { size: usize, max: usize },

    #[error("Content integrity mismatch: expected Blake3 {expected}, calculated {actual}")]
    HashMismatch { expected: String, actual: String },

    #[error("Unsupported or disabled clipboard format: {0}")]
    UnsupportedFormat(String),

    #[error("Sensitive content blocked by privacy policy")]
    SensitiveContentBlocked,

    #[error("Stale or duplicate sequence number: received {received}, current {current}")]
    StaleSequence { current: u64, received: u64 },

    #[error("Clipboard backend failure: {0}")]
    BackendError(String),

    #[error("Channel mismatch: expected channel {expected}, got {actual}")]
    ChannelMismatch { expected: u16, actual: u16 },

    #[error("Core error: {0}")]
    Core(#[from] bridge_core::BridgeError),
}
