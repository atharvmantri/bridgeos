use thiserror::Error;

/// Errors arising during local network peer discovery and announcement.
#[derive(Error, Debug)]
pub enum DiscoveryError {
    #[error("mDNS daemon error: {0}")]
    Mdns(String),

    #[error("Missing or invalid TXT record property: {0}")]
    InvalidTxtRecord(String),

    #[error("Invalid node identifier: {0}")]
    InvalidNodeId(String),

    #[error("Internal discovery error: {0}")]
    Internal(String),

    #[error("Discovery service is already active")]
    AlreadyRunning,

    #[error("Discovery service is not running")]
    NotRunning,
}

pub type Result<T> = std::result::Result<T, DiscoveryError>;
