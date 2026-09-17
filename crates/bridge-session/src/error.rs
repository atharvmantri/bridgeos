use thiserror::Error;

/// Domain errors for BridgeOS peer session orchestration.
#[derive(Debug, Error)]
pub enum SessionError {
    #[error("Transport I/O error: {0}")]
    Transport(String),

    #[error("Protocol error: {0}")]
    Protocol(String),

    #[error("Identity / crypto error: {0}")]
    Identity(#[from] bridge_identity::IdentityError),

    #[error("Core error: {0}")]
    Core(#[from] bridge_core::BridgeError),

    #[error("Clipboard error: {0}")]
    Clipboard(#[from] bridge_clipboard::ClipboardError),

    #[error("Cryptographic authentication handshake failed: {0}")]
    AuthenticationFailed(String),

    #[error("Critical security alert: Key mismatch / spoofing detected for node {node_id} (expected public key {expected}, presented {actual})")]
    KeyMismatch {
        node_id: String,
        expected: String,
        actual: String,
    },

    #[error("Peer {0} is revoked and forbidden from connecting")]
    PeerRevoked(String),

    #[error("Peer {0} is not trusted (pairing required)")]
    PeerUntrusted(String),

    #[error("Pairing ceremony rejected by user or peer")]
    PairingRejected,

    #[error("SAS PIN verification failed or timed out")]
    SasMismatch,

    #[error("Application channel {channel} is blocked because session is {state}")]
    ChannelBlocked { channel: u16, state: String },

    #[error("Session was terminated: {0}")]
    Closed(String),

    #[error("Operation timed out: {0}")]
    Timeout(String),
}
