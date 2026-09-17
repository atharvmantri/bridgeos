use bridge_core::{Capabilities, DeviceType, NodeId};
use bridge_discovery::{
    DiscoveryEvent, UdpConfig, UdpDiscovery, UnifiedDiscovery, UnifiedDiscoveryConfig,
    UnifiedDiscoveryMode,
};
use std::time::Duration;
use tokio::time::timeout;

#[tokio::test]
async fn test_udp_live_discovery_and_goodbye() {
    let test_port = 43110;

    let node_a_id = NodeId::from_bytes([0xAA; 32]);
    let _node_b_id = NodeId::from_bytes([0xBB; 32]);

    // Node B listens on test_port
    let config_b = UdpConfig {
        broadcast_port: test_port,
        bind_addr: format!("127.0.0.1:{test_port}").parse().unwrap(),
        broadcast_targets: vec![format!("127.0.0.1:{test_port}").parse().unwrap()],
        beacon_interval: Duration::from_millis(100),
        ttl: Duration::from_millis(800),
        prune_interval: Duration::from_millis(100),
        query_on_start: false,
        goodbye_on_shutdown: true,
    };

    let mut discovery_b = UdpDiscovery::new(config_b).expect("create node B discovery");
    let mut rx_b = discovery_b.subscribe();
    discovery_b
        .start_discovery()
        .expect("start node B discovery");

    // Node A sends beacons targeting test_port
    let config_a = UdpConfig {
        broadcast_port: test_port,
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        broadcast_targets: vec![format!("127.0.0.1:{test_port}").parse().unwrap()],
        beacon_interval: Duration::from_millis(100),
        ttl: Duration::from_millis(800),
        prune_interval: Duration::from_millis(100),
        query_on_start: false,
        goodbye_on_shutdown: true,
    };

    let mut discovery_a = UdpDiscovery::new(config_a).expect("create node A discovery");
    let mut caps_a = Capabilities::default();
    caps_a.insert(Capabilities::FILE_TRANSFER);
    caps_a.insert(Capabilities::CLIPBOARD_TEXT);

    discovery_a
        .advertise(node_a_id, "NodeA-PC", DeviceType::Windows, 9191, caps_a)
        .await
        .expect("node A advertise");
    discovery_a
        .start_discovery()
        .expect("start node A discovery");

    // 1. Verify Node B discovers Node A within 2 seconds
    let discovered = timeout(Duration::from_secs(2), async {
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
    .await
    .expect("node B should discover node A via UDP beacon");

    assert_eq!(discovered.node_id, node_a_id);
    assert_eq!(discovered.device_name, "NodeA-PC");
    assert_eq!(discovered.device_type, DeviceType::Windows);
    assert_eq!(discovered.addresses.len(), 1);
    assert_eq!(discovered.addresses[0].port(), 9191);
    assert!(discovered
        .capabilities
        .contains(Capabilities::FILE_TRANSFER));
    assert!(discovered
        .capabilities
        .contains(Capabilities::CLIPBOARD_TEXT));
    assert!(discovery_b.directory().contains(&node_a_id));

    // 2. Test explicit goodbye departure
    discovery_a
        .shutdown()
        .await
        .expect("shutdown A with goodbye");

    let departed = timeout(Duration::from_secs(2), async {
        loop {
            match rx_b.recv().await {
                Ok(DiscoveryEvent::PeerLost(id)) if id == node_a_id => return true,
                _ => {}
            }
        }
    })
    .await
    .expect("node B should receive PeerLost event on Goodbye");

    assert!(departed);
    assert!(!discovery_b.directory().contains(&node_a_id));

    discovery_b.shutdown().await.expect("shutdown B");
}

#[tokio::test]
async fn test_udp_discovery_ttl_expiry() {
    let test_port = 43120;
    let node_x_id = NodeId::from_bytes([0x77; 32]);

    let config = UdpConfig {
        broadcast_port: test_port,
        bind_addr: format!("127.0.0.1:{test_port}").parse().unwrap(),
        broadcast_targets: vec![format!("127.0.0.1:{test_port}").parse().unwrap()],
        beacon_interval: Duration::from_millis(50),
        ttl: Duration::from_millis(300),
        prune_interval: Duration::from_millis(100),
        query_on_start: false,
        goodbye_on_shutdown: false,
    };

    let mut discovery = UdpDiscovery::new(config).expect("create discovery");
    let mut rx = discovery.subscribe();
    discovery.start_discovery().expect("start discovery");

    // Manually inject peer to test TTL expiration in UdpDiscovery directory
    let peer = bridge_discovery::DiscoveredPeer::new(
        node_x_id,
        "ExpiringPeer".into(),
        DeviceType::Android,
        vec!["127.0.0.1:8080".parse().unwrap()],
        Capabilities::default(),
    );
    discovery.directory().insert_or_update(peer);
    assert!(discovery.directory().contains(&node_x_id));

    // Wait for TTL expiration
    let expired = timeout(Duration::from_secs(2), async {
        loop {
            match rx.recv().await {
                Ok(DiscoveryEvent::PeerExpired(id)) if id == node_x_id => return true,
                _ => {}
            }
        }
    })
    .await
    .expect("peer should expire after TTL");

    assert!(expired);
    assert!(!discovery.directory().contains(&node_x_id));

    discovery.shutdown().await.expect("shutdown");
}

#[tokio::test]
async fn test_udp_query_probe_immediate_response() {
    let test_port = 43130;
    let node_server_id = NodeId::from_bytes([0x55; 32]);
    let node_client_id = NodeId::from_bytes([0x66; 32]);

    // Server listens on test_port and advertises
    let config_server = UdpConfig {
        broadcast_port: test_port,
        bind_addr: format!("127.0.0.1:{test_port}").parse().unwrap(),
        broadcast_targets: vec![format!("127.0.0.1:{test_port}").parse().unwrap()],
        // Slow beacon interval: 10 seconds!
        beacon_interval: Duration::from_secs(10),
        ttl: Duration::from_secs(15),
        prune_interval: Duration::from_secs(1),
        query_on_start: false,
        goodbye_on_shutdown: false,
    };

    let mut server = UdpDiscovery::new(config_server).expect("create server");
    server
        .advertise(
            node_server_id,
            "SlowBeaconServer",
            DeviceType::Linux,
            7777,
            Capabilities::all(),
        )
        .await
        .expect("server advertise");
    server.start_discovery().expect("start server");

    // Client starts with query_on_start = true, binding to an ephemeral port and targeting server port
    let client_port = 43131;
    let config_client = UdpConfig {
        broadcast_port: client_port,
        bind_addr: format!("127.0.0.1:{client_port}").parse().unwrap(),
        broadcast_targets: vec![format!("127.0.0.1:{test_port}").parse().unwrap()],
        beacon_interval: Duration::from_secs(10),
        ttl: Duration::from_secs(15),
        prune_interval: Duration::from_secs(1),
        query_on_start: true, // Query probe!
        goodbye_on_shutdown: false,
    };

    let mut client = UdpDiscovery::new(config_client).expect("create client");
    client
        .advertise(
            node_client_id,
            "QueryingClient",
            DeviceType::Windows,
            8888,
            Capabilities::default(),
        )
        .await
        .expect("client advertise");

    let mut client_rx = client.subscribe();
    client.start_discovery().expect("start client");

    // Client should discover Server quickly via the Query probe response (way before 10s beacon interval)
    let discovered = timeout(Duration::from_secs(2), async {
        loop {
            match client_rx.recv().await {
                Ok(DiscoveryEvent::PeerDiscovered(peer)) if peer.node_id == node_server_id => {
                    return peer;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("client should immediately discover server via Query probe response");

    assert_eq!(discovered.node_id, node_server_id);
    assert_eq!(discovered.device_name, "SlowBeaconServer");
    assert_eq!(discovered.addresses[0].port(), 7777);

    server.shutdown().await.expect("shutdown server");
    client.shutdown().await.expect("shutdown client");
}

#[tokio::test]
async fn test_unified_discovery_udp_mode() {
    let test_port = 43140;
    let node_id = NodeId::from_bytes([0x88; 32]);

    let udp_config = UdpConfig {
        broadcast_port: test_port,
        bind_addr: format!("127.0.0.1:{test_port}").parse().unwrap(),
        broadcast_targets: vec![format!("127.0.0.1:{test_port}").parse().unwrap()],
        beacon_interval: Duration::from_millis(100),
        ttl: Duration::from_millis(500),
        prune_interval: Duration::from_millis(100),
        query_on_start: false,
        goodbye_on_shutdown: false,
    };

    let unified_config = UnifiedDiscoveryConfig {
        mode: UnifiedDiscoveryMode::UdpOnly,
        udp: udp_config,
        ..UnifiedDiscoveryConfig::default()
    };

    let mut unified = UnifiedDiscovery::new(unified_config).expect("create unified");
    unified
        .advertise(
            node_id,
            "UnifiedPeer",
            DeviceType::Android,
            5555,
            Capabilities::all(),
            None,
        )
        .await
        .expect("advertise");

    unified.start_discovery().expect("start discovery");
    assert!(unified.directory().is_empty()); // Hasn't discovered any other peer yet
    unified.stop_advertising().await.expect("stop advertising");
    unified.shutdown().await.expect("shutdown");
}
