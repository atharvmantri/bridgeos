use thiserror::Error;

/// Core errors across the BridgeOS ecosystem.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum BridgeError {
    #[error("Invalid protocol version: expected major {expected}, received {received}")]
    VersionMismatch { expected: u16, received: u16 },

    #[error("Framing error: {0}")]
    Framing(String),

    #[error("Frame length {length} exceeds maximum allowable bound {max}")]
    FrameTooLarge { length: usize, max: usize },

    #[error("Invalid magic bytes in frame header")]
    InvalidMagic,

    #[error("Cryptographic identity error: {0}")]
    Identity(String),

    #[error("Authentication challenge failed")]
    AuthenticationFailed,

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Connection closed unexpectedly")]
    ConnectionClosed,

    #[error("Incompatible capabilities: {0}")]
    IncompatibleCapabilities(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, BridgeError>;
