//! Local area network discovery abstractions (mDNS, UDP Beacons) for BridgeOS.

#![forbid(unsafe_code)]

pub mod beacon;
pub mod error;
pub mod peer;
pub mod service;
pub mod udp;
pub mod unified;

pub use beacon::{
    decode_beacon, encode_beacon, BeaconMessage, BEACON_MAGIC, BEACON_VERSION, MAX_BEACON_SIZE,
};
pub use error::{DiscoveryError, Result};
pub use peer::{DiscoveredPeer, PeerDirectory};
pub use service::*;
pub use udp::{UdpConfig, UdpDiscovery, DEFAULT_UDP_BROADCAST_PORT};
pub use unified::{UnifiedDiscovery, UnifiedDiscoveryConfig, UnifiedDiscoveryMode};
