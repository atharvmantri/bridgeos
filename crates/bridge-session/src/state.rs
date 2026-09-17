use serde::{Deserialize, Serialize};

/// Current security and authorization state of a live peer session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionState {
    /// Connection is performing ClientHello / ServerHello and mutual Ed25519 challenge.
    Handshaking,

    /// Peer identity is cryptographically proven, but NOT trusted in the TrustStore.
    /// Application data channels (file transfer, clipboard) are blocked.
    AuthenticatedUntrusted,

    /// The session is actively performing an out-of-band SAS pairing ceremony.
    Pairing,

    /// Peer identity is verified and recorded as trusted in TrustStore.
    /// All negotiated application channels are open.
    Trusted,

    /// Session has been terminated or rejected.
    Terminated(String),
}

impl SessionState {
    /// Check whether the peer is fully trusted.
    pub fn is_trusted(&self) -> bool {
        matches!(self, Self::Trusted)
    }

    /// Check whether application data channels (clipboard, file transfer) may be accessed.
    pub fn can_use_application_channels(&self) -> bool {
        matches!(self, Self::Trusted)
    }

    /// Return a human-readable display label.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Handshaking => "Handshaking",
            Self::AuthenticatedUntrusted => "Authenticated (Untrusted)",
            Self::Pairing => "Pairing",
            Self::Trusted => "Trusted",
            Self::Terminated(_) => "Terminated",
        }
    }
}
