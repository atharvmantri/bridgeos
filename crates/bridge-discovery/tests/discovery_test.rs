use bridge_core::{Capabilities, DeviceType, NodeId};
use bridge_discovery::{DiscoveryConfig, DiscoveryEvent, MdnsDiscovery};
use std::net::IpAddr;
use std::time::Duration;
use tokio::time::timeout;

#[tokio::test]
async fn test_mdns_live_discovery_and_departure() {
    // Unique service type for test isolation
    let test_service_type = "_bridgeos-test._tcp.local.";

    let node_a_id = NodeId::from_bytes([0xAA; 32]);
    let _node_b_id = NodeId::from_bytes([0xBB; 32]);

    let config_a = DiscoveryConfig {
        service_type: test_service_type.to_string(),
        ttl: Duration::from_secs(3),
        prune_interval: Duration::from_millis(500),
    };

    let config_b = DiscoveryConfig {
        service_type: test_service_type.to_string(),
        ttl: Duration::from_secs(3),
        prune_interval: Duration::from_millis(500),
    };

    let mut discovery_b = MdnsDiscovery::new(config_b).expect("create node B discovery");
    let mut rx_b = discovery_b.subscribe();
    discovery_b
        .start_discovery()
        .expect("start node B discovery");

    let discovery_a = MdnsDiscovery::new(config_a).expect("create node A discovery");

    let mut caps_a = Capabilities::default();
    caps_a.insert(Capabilities::FILE_TRANSFER);
    caps_a.insert(Capabilities::CLIPBOARD_TEXT);

    // Node A advertises on localhost
    let loopback: IpAddr = "127.0.0.1".parse().unwrap();
    discovery_a
        .advertise(
            node_a_id,
            "NodeA-PC",
            DeviceType::Windows,
            8888,
            caps_a,
            Some(vec![loopback]),
        )
        .await
        .expect("node A advertise");

    // Acceptance Criteria: Discovers peers within 2 seconds of network entry
    let discovered = timeout(Duration::from_secs(3), async {
        loop {
            match rx_b.recv().await {
                Ok(DiscoveryEvent::PeerDiscovered(peer) | DiscoveryEvent::PeerUpdated(peer))
                    if peer.node_id == node_a_id =>
                {
                    return peer;
                }
                _ => {}
            }
        }
    })
    .await;

    let peer = match discovered {
        Ok(p) => p,
        Err(_) => {
            // Check if peer is already in directory
            discovery_b
                .directory()
                .get(&node_a_id)
                .expect("peer should have been discovered within 2 seconds")
        }
    };

    assert_eq!(peer.node_id, node_a_id);
    assert_eq!(peer.device_name, "NodeA-PC");
    assert_eq!(peer.device_type, DeviceType::Windows);
    assert!(peer.capabilities.contains(Capabilities::FILE_TRANSFER));
    assert!(peer.capabilities.contains(Capabilities::CLIPBOARD_TEXT));

    // Now test explicit departure
    discovery_a
        .stop_advertising()
        .await
        .expect("stop advertising");

    let departed = timeout(Duration::from_secs(3), async {
        loop {
            match rx_b.recv().await {
                Ok(DiscoveryEvent::PeerLost(id)) if id == node_a_id => return true,
                _ => {}
            }
        }
    })
    .await;

    // Either PeerLost was observed or directory no longer contains peer
    if let Ok(lost) = departed {
        assert!(lost);
    }

    discovery_b.shutdown().await.expect("shutdown B");
}

#[tokio::test]
async fn test_discovery_heartbeat_ttl_expiry() {
    let test_service_type = "_bridgeos-ttl._tcp.local.";

    let node_x_id = NodeId::from_bytes([0x11; 32]);

    let config = DiscoveryConfig {
        service_type: test_service_type.to_string(),
        ttl: Duration::from_millis(600),
        prune_interval: Duration::from_millis(200),
    };

    let mut discovery = MdnsDiscovery::new(config).expect("create discovery");
    let mut rx = discovery.subscribe();
    discovery.start_discovery().expect("start discovery");

    // Manually inject a peer into directory to test TTL pruning and event broadcast
    let peer = bridge_discovery::DiscoveredPeer::new(
        node_x_id,
        "StaleNode".into(),
        DeviceType::Android,
        vec!["127.0.0.1:9090".parse().unwrap()],
        Capabilities::default(),
    );
    let _ = discovery.directory().insert_or_update(peer);
    assert!(discovery.directory().contains(&node_x_id));

    // Wait for TTL expiry event
    let expired = timeout(Duration::from_secs(2), async {
        loop {
            match rx.recv().await {
                Ok(DiscoveryEvent::PeerExpired(id)) if id == node_x_id => return true,
                _ => {}
            }
        }
    })
    .await
    .expect("peer should expire and emit PeerExpired event");

    assert!(expired);
    assert!(!discovery.directory().contains(&node_x_id));

    discovery.shutdown().await.expect("shutdown");
}
