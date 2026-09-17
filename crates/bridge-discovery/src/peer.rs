use bridge_core::{Capabilities, DeviceType, NodeId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};

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

    /// Checks if this peer has expired given a maximum age duration.
    pub fn is_expired(&self, max_age: Duration) -> bool {
        match SystemTime::now().duration_since(self.last_seen) {
            Ok(elapsed) => elapsed > max_age,
            Err(_) => false,
        }
    }

    /// Updates the last seen timestamp of this peer to now.
    pub fn touch(&mut self) {
        self.last_seen = SystemTime::now();
    }
}

/// Thread-safe local directory tracking active network peers and freshness.
#[derive(Debug, Clone, Default)]
pub struct PeerDirectory {
    peers: Arc<RwLock<HashMap<NodeId, DiscoveredPeer>>>,
}

impl PeerDirectory {
    pub fn new() -> Self {
        Self {
            peers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Inserts a new peer or updates an existing peer's addresses, capabilities, and last_seen.
    /// Returns `(peer, is_new)`.
    pub fn insert_or_update(&self, mut peer: DiscoveredPeer) -> (DiscoveredPeer, bool) {
        let mut map = self
            .peers
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(existing) = map.get_mut(&peer.node_id) {
            existing.device_name = peer.device_name;
            existing.device_type = peer.device_type;
            existing.capabilities = peer.capabilities;
            for addr in peer.addresses {
                if !existing.addresses.contains(&addr) {
                    existing.addresses.push(addr);
                }
            }
            existing.last_seen = SystemTime::now();
            (existing.clone(), false)
        } else {
            peer.last_seen = SystemTime::now();
            let cloned = peer.clone();
            map.insert(peer.node_id, peer);
            (cloned, true)
        }
    }

    /// Retrieves a copy of a peer by `NodeId`.
    pub fn get(&self, node_id: &NodeId) -> Option<DiscoveredPeer> {
        let map = self
            .peers
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.get(node_id).cloned()
    }

    /// Removes a peer by `NodeId`.
    pub fn remove(&self, node_id: &NodeId) -> Option<DiscoveredPeer> {
        let mut map = self
            .peers
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.remove(node_id)
    }

    /// Returns a list of all currently tracked peers.
    pub fn list(&self) -> Vec<DiscoveredPeer> {
        let map = self
            .peers
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.values().cloned().collect()
    }

    /// Checks if a peer with `NodeId` is present in the directory.
    pub fn contains(&self, node_id: &NodeId) -> bool {
        let map = self
            .peers
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.contains_key(node_id)
    }

    /// Returns the number of peers currently in the directory.
    pub fn len(&self) -> usize {
        let map = self
            .peers
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.len()
    }

    /// Returns `true` if the directory is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Prunes peers whose `last_seen` timestamp is older than `max_age`.
    /// Returns a vector of the pruned peers.
    pub fn prune_expired(&self, max_age: Duration) -> Vec<DiscoveredPeer> {
        let now = SystemTime::now();
        let mut map = self
            .peers
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut expired = Vec::new();

        map.retain(|_id, peer| {
            let is_expired = match now.duration_since(peer.last_seen) {
                Ok(elapsed) => elapsed > max_age,
                Err(_) => false,
            };
            if is_expired {
                expired.push(peer.clone());
                false
            } else {
                true
            }
        });

        expired
    }

    /// Removes all peers from the directory.
    pub fn clear(&self) {
        let mut map = self
            .peers
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_peer_directory_crud() {
        let directory = PeerDirectory::new();
        assert!(directory.is_empty());

        let node_id = NodeId::from_bytes([1u8; 32]);
        let peer = DiscoveredPeer::new(
            node_id,
            "Laptop-Node".into(),
            DeviceType::Windows,
            vec!["192.168.1.100:8443".parse().unwrap()],
            Capabilities::all(),
        );

        let (inserted, is_new) = directory.insert_or_update(peer.clone());
        assert!(is_new);
        assert_eq!(inserted.node_id, node_id);
        assert_eq!(directory.len(), 1);
        assert!(directory.contains(&node_id));

        // Update existing peer
        let mut updated_peer = peer.clone();
        updated_peer.device_name = "Laptop-Renamed".into();
        let (updated, is_new_2) = directory.insert_or_update(updated_peer);
        assert!(!is_new_2);
        assert_eq!(updated.device_name, "Laptop-Renamed");
        assert_eq!(directory.len(), 1);

        // Fetch peer
        let fetched = directory.get(&node_id).expect("peer should exist");
        assert_eq!(fetched.device_name, "Laptop-Renamed");

        // List
        let list = directory.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].node_id, node_id);

        // Remove
        let removed = directory.remove(&node_id).expect("peer should be removed");
        assert_eq!(removed.node_id, node_id);
        assert!(directory.is_empty());
        assert!(!directory.contains(&node_id));
    }

    #[test]
    fn test_peer_directory_prune_expired() {
        let directory = PeerDirectory::new();
        let node_id = NodeId::from_bytes([2u8; 32]);
        let mut peer = DiscoveredPeer::new(
            node_id,
            "Phone-Node".into(),
            DeviceType::Android,
            vec!["192.168.1.105:8443".parse().unwrap()],
            Capabilities::default(),
        );

        // Backdate last_seen by 10 seconds
        peer.last_seen = SystemTime::now() - Duration::from_secs(10);
        {
            let mut map = directory.peers.write().unwrap();
            map.insert(node_id, peer);
        }

        assert_eq!(directory.len(), 1);

        // Pruning with max_age = 5s should prune this peer
        let pruned = directory.prune_expired(Duration::from_secs(5));
        assert_eq!(pruned.len(), 1);
        assert_eq!(pruned[0].node_id, node_id);
        assert!(directory.is_empty());
    }
}
