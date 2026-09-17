use bridge_core::NodeId;
use thiserror::Error;

/// Specific error types for identity, pairing, and trust operations.
#[derive(Debug, Error)]
pub enum IdentityError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("Filesystem I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Core bridge error: {0}")]
    Core(#[from] bridge_core::BridgeError),

    #[error("Key mismatch / impersonation detected for peer {node_id}: expected {expected}, presented {presented}")]
    KeyMismatch {
        node_id: NodeId,
        expected: String,
        presented: String,
    },

    #[error("Peer {0} is revoked and barred from connecting")]
    PeerRevoked(NodeId),

    #[error("Peer {0} is unknown and has not been paired")]
    PeerUntrusted(NodeId),

    #[error("Pairing failed: {0}")]
    PairingFailed(String),

    #[error("Verification value (SAS) mismatch during pairing ceremony")]
    SasMismatch,

    #[error("Pairing session expired or timed out")]
    PairingTimeout,

    #[error("Invalid secret key file: {0}")]
    InvalidSecretKeyFile(String),

    #[error("Cryptographic operation failed: {0}")]
    Crypto(String),
}

pub type Result<T> = std::result::Result<T, IdentityError>;
