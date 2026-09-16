//! Local area network discovery abstractions (mDNS, UDP Beacons) for BridgeOS.

#![forbid(unsafe_code)]

use bridge_core::{Capabilities, DeviceType, NodeId};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::time::SystemTime;

/// Metadata published and observed during local network discovery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredPeer {
    pub node_id: NodeId,
    pub device_name: String,
    pub device_type: DeviceType,
    pub addresses: Vec<SocketAddr>,
    pub capabilities: Capabilities,
    pub last_seen: SystemTime,
}

impl DiscoveredPeer {
    pub fn new(
        node_id: NodeId,
        device_name: String,
        device_type: DeviceType,
        addresses: Vec<SocketAddr>,
        capabilities: Capabilities,
    ) -> Self {
        Self {
            node_id,
            device_name,
            device_type,
            addresses,
            capabilities,
            last_seen: SystemTime::now(),
        }
    }
}
