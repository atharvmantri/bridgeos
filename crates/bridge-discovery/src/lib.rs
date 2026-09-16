//! Local area network discovery abstractions (mDNS, UDP Beacons) for BridgeOS.

#![forbid(unsafe_code)]

pub mod error;
pub mod peer;
pub mod service;

pub use error::{DiscoveryError, Result};
pub use peer::{DiscoveredPeer, PeerDirectory};
pub use service::*;
