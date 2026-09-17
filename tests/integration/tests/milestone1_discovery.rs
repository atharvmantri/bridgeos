use bridge_core::{Capabilities, DeviceType, ProtocolVersion};
use bridge_discovery::{DiscoveryConfig, DiscoveryEvent, MdnsDiscovery, UdpConfig, UdpDiscovery};
use bridge_identity::IdentityKey;
use bridge_integration_tests::init_test_logging;
use bridge_protocol::{
    AuthResponse, AuthResult, ClientHello, ControlFrame, Frame, HandshakeFrame, ServerHello,
};
use bridge_transport::FramedStream;
use std::net::IpAddr;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::time::timeout;
use tracing::info;

#[tokio::test]
async fn test_milestone1_mdns_discovery_and_handshake() {
    init_test_logging();
    info!("Starting Milestone 1 end-to-end mDNS discovery and authenticated session test");

    let server_key = IdentityKey::generate();
    let client_key = IdentityKey::generate();
    let client_pub = client_key.public_key();

    // 1. Server binds a local TCP socket
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server_port = listener.local_addr().unwrap().port();
    info!(port = server_port, "Server TCP listener established");

    let service_type = "_bridgeos-m1._tcp.local.";

    let server_config = DiscoveryConfig {
        service_type: service_type.to_string(),
        ttl: Duration::from_secs(4),
        prune_interval: Duration::from_millis(500),
    };
    let client_config = DiscoveryConfig {
        service_type: service_type.to_string(),
        ttl: Duration::from_secs(4),
        prune_interval: Duration::from_millis(500),
    };

    // 2. Start Discovery services
    let server_discovery = MdnsDiscovery::new(server_config).unwrap();
    let mut client_discovery = MdnsDiscovery::new(client_config).unwrap();

    let mut client_sub = client_discovery.subscribe();
    client_discovery.start_discovery().unwrap();

    // 3. Server advertises service over mDNS
    let loopback: IpAddr = "127.0.0.1".parse().unwrap();
    server_discovery
        .advertise(
            server_key.node_id(),
            "BridgeOS-Server",
            DeviceType::Windows,
            server_port,
            Capabilities::all(),
            Some(vec![loopback]),
        )
        .await
        .unwrap();

    // 4. Client discovers Server via mDNS
    info!("Client awaiting peer discovery via mDNS...");
    let discovered_peer = timeout(Duration::from_secs(4), async {
        loop {
            match client_sub.recv().await {
                Ok(DiscoveryEvent::PeerDiscovered(peer) | DiscoveryEvent::PeerUpdated(peer))
                    if peer.node_id == server_key.node_id() =>
                {
                    return peer;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("client should discover server within 4 seconds");

    assert_eq!(discovered_peer.node_id, server_key.node_id());
    assert_eq!(discovered_peer.device_name, "BridgeOS-Server");
    assert_eq!(discovered_peer.device_type, DeviceType::Windows);
    assert_eq!(discovered_peer.addresses[0].port(), server_port);
    info!(peer = ?discovered_peer, "Client successfully discovered server peer via mDNS");

    // 5. Spawn server accept task
    let server_task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut framed = FramedStream::new(socket);

        // Handshake: Receive ClientHello
        let frame = framed.recv_frame().await.unwrap().expect("client hello");
        let client_hello = match frame {
            Frame::Handshake(HandshakeFrame::ClientHello(h)) => h,
            other => panic!("Expected ClientHello, got {other:?}"),
        };

        // Send ServerHello
        let server_nonce = [0x55; 32];
        let server_hello = ServerHello {
            agreed_version: ProtocolVersion::CURRENT,
            node_id: server_key.node_id(),
            device_name: "BridgeOS-Server".to_string(),
            server_nonce,
            negotiated_capabilities: client_hello.capabilities.intersect(&Capabilities::all()),
        };
        framed
            .send_frame(&Frame::Handshake(HandshakeFrame::ServerHello(server_hello)))
            .await
            .unwrap();

        // Receive AuthResponse
        let auth_frame = framed.recv_frame().await.unwrap().expect("auth resp");
        let auth_resp = match auth_frame {
            Frame::Handshake(HandshakeFrame::AuthResponse(r)) => r,
            other => panic!("Expected AuthResponse, got {other:?}"),
        };

        let verify_res = client_pub.verify_handshake_challenge(
            &client_hello.client_nonce,
            &server_nonce,
            &auth_resp.signature,
        );
        assert!(
            verify_res.is_ok(),
            "Mutual challenge verification must succeed"
        );

        // Send AuthResult::ok()
        framed
            .send_frame(&Frame::Handshake(HandshakeFrame::AuthResult(
                AuthResult::ok(),
            )))
            .await
            .unwrap();

        // Expect Ping and reply with Pong
        let ping = framed.recv_frame().await.unwrap().expect("ping");
        match ping {
            Frame::Control(ControlFrame::Ping { nonce }) => {
                framed
                    .send_frame(&Frame::Control(ControlFrame::Pong { nonce }))
                    .await
                    .unwrap();
            }
            other => panic!("Expected Ping, got {other:?}"),
        }

        // Receive Disconnect
        let disc = framed.recv_frame().await.unwrap().expect("disconnect");
        match disc {
            Frame::Control(ControlFrame::Disconnect { .. }) => {}
            other => panic!("Expected Disconnect, got {other:?}"),
        }
    });

    // 6. Client connects to discovered peer address
    let peer_addr = discovered_peer.addresses[0];
    let stream = TcpStream::connect(peer_addr).await.unwrap();
    let mut client = FramedStream::new(stream);

    let client_nonce = [0x33; 32];
    let client_hello = ClientHello {
        version: ProtocolVersion::CURRENT,
        node_id: client_key.node_id(),
        device_name: "BridgeOS-Client".to_string(),
        client_nonce,
        capabilities: Capabilities::all(),
    };
    client
        .send_frame(&Frame::Handshake(HandshakeFrame::ClientHello(client_hello)))
        .await
        .unwrap();

    let server_hello_frame = client.recv_frame().await.unwrap().expect("server hello");
    let server_hello = match server_hello_frame {
        Frame::Handshake(HandshakeFrame::ServerHello(h)) => h,
        other => panic!("Expected ServerHello, got {other:?}"),
    };

    let sig = client_key.sign_handshake_challenge(&client_nonce, &server_hello.server_nonce);
    client
        .send_frame(&Frame::Handshake(HandshakeFrame::AuthResponse(
            AuthResponse { signature: sig },
        )))
        .await
        .unwrap();

    let auth_result_frame = client.recv_frame().await.unwrap().expect("auth result");
    match auth_result_frame {
        Frame::Handshake(HandshakeFrame::AuthResult(res)) => {
            assert!(res.success);
        }
        other => panic!("Expected AuthResult, got {other:?}"),
    }

    // Ping / Pong
    client
        .send_frame(&Frame::Control(ControlFrame::Ping { nonce: 12345 }))
        .await
        .unwrap();

    let pong = client.recv_frame().await.unwrap().expect("pong");
    match pong {
        Frame::Control(ControlFrame::Pong { nonce }) => {
            assert_eq!(nonce, 12345);
        }
        other => panic!("Expected Pong, got {other:?}"),
    }

    // Disconnect
    client
        .send_frame(&Frame::Control(ControlFrame::Disconnect {
            reason: bridge_protocol::DisconnectReason::Graceful,
        }))
        .await
        .unwrap();

    server_task.await.unwrap();

    // 7. Clean up discovery
    server_discovery.stop_advertising().await.unwrap();
    client_discovery.shutdown().await.unwrap();

    info!("Milestone 1 discovery and authenticated session test passed successfully!");
}

#[tokio::test]
async fn test_milestone1_udp_beacon_discovery_and_handshake() {
    init_test_logging();
    info!("Starting Milestone 1 end-to-end UDP beacon discovery and authenticated session test");

    let server_key = IdentityKey::generate();
    let client_key = IdentityKey::generate();
    let client_pub = client_key.public_key();

    // 1. Server binds a local TCP socket
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server_tcp_port = listener.local_addr().unwrap().port();
    info!(port = server_tcp_port, "Server TCP listener established");

    let beacon_port = 43250;

    let server_config = UdpConfig {
        broadcast_port: beacon_port,
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        broadcast_targets: vec![format!("127.0.0.1:{beacon_port}").parse().unwrap()],
        beacon_interval: Duration::from_millis(100),
        ttl: Duration::from_secs(3),
        prune_interval: Duration::from_millis(200),
        query_on_start: false,
        goodbye_on_shutdown: true,
    };

    let client_config = UdpConfig {
        broadcast_port: beacon_port,
        bind_addr: format!("127.0.0.1:{beacon_port}").parse().unwrap(),
        broadcast_targets: vec![format!("127.0.0.1:{beacon_port}").parse().unwrap()],
        beacon_interval: Duration::from_millis(100),
        ttl: Duration::from_secs(3),
        prune_interval: Duration::from_millis(200),
        query_on_start: true,
        goodbye_on_shutdown: false,
    };

    let mut server_discovery = UdpDiscovery::new(server_config).unwrap();
    let mut client_discovery = UdpDiscovery::new(client_config).unwrap();

    let mut client_sub = client_discovery.subscribe();
    client_discovery.start_discovery().unwrap();

    server_discovery
        .advertise(
            server_key.node_id(),
            "BridgeOS-UdpServer",
            DeviceType::Windows,
            server_tcp_port,
            Capabilities::all(),
        )
        .await
        .unwrap();
    server_discovery.start_discovery().unwrap();

    // Client discovers Server via UDP beacon
    info!("Client awaiting peer discovery via UDP beacon...");
    let discovered_peer = timeout(Duration::from_secs(3), async {
        loop {
            match client_sub.recv().await {
                Ok(DiscoveryEvent::PeerDiscovered(peer) | DiscoveryEvent::PeerUpdated(peer))
                    if peer.node_id == server_key.node_id() =>
                {
                    return peer;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("client should discover server via UDP beacon within 3 seconds");

    assert_eq!(discovered_peer.node_id, server_key.node_id());
    assert_eq!(discovered_peer.device_name, "BridgeOS-UdpServer");
    assert_eq!(discovered_peer.addresses[0].port(), server_tcp_port);
    info!(peer = ?discovered_peer, "Client successfully discovered server peer via UDP beacon");

    // Spawn server accept task
    let server_task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut framed = FramedStream::new(socket);

        let frame = framed.recv_frame().await.unwrap().expect("client hello");
        let client_hello = match frame {
            Frame::Handshake(HandshakeFrame::ClientHello(h)) => h,
            other => panic!("Expected ClientHello, got {other:?}"),
        };

        let server_nonce = [0x77; 32];
        let server_hello = ServerHello {
            agreed_version: ProtocolVersion::CURRENT,
            node_id: server_key.node_id(),
            device_name: "BridgeOS-UdpServer".to_string(),
            server_nonce,
            negotiated_capabilities: client_hello.capabilities.intersect(&Capabilities::all()),
        };
        framed
            .send_frame(&Frame::Handshake(HandshakeFrame::ServerHello(server_hello)))
            .await
            .unwrap();

        let auth_frame = framed.recv_frame().await.unwrap().expect("auth resp");
        let auth_resp = match auth_frame {
            Frame::Handshake(HandshakeFrame::AuthResponse(r)) => r,
            other => panic!("Expected AuthResponse, got {other:?}"),
        };

        client_pub
            .verify_handshake_challenge(
                &client_hello.client_nonce,
                &server_nonce,
                &auth_resp.signature,
            )
            .expect("signature must verify");

        framed
            .send_frame(&Frame::Handshake(HandshakeFrame::AuthResult(
                AuthResult::ok(),
            )))
            .await
            .unwrap();

        let ping = framed.recv_frame().await.unwrap().expect("ping");
        match ping {
            Frame::Control(ControlFrame::Ping { nonce }) => {
                framed
                    .send_frame(&Frame::Control(ControlFrame::Pong { nonce }))
                    .await
                    .unwrap();
            }
            other => panic!("Expected Ping, got {other:?}"),
        }

        let disc = framed.recv_frame().await.unwrap().expect("disconnect");
        match disc {
            Frame::Control(ControlFrame::Disconnect { .. }) => {}
            other => panic!("Expected Disconnect, got {other:?}"),
        }
    });

    // Client connects to the discovered peer's resolved address
    let peer_addr = discovered_peer.addresses[0];
    let stream = TcpStream::connect(peer_addr).await.unwrap();
    let mut client = FramedStream::new(stream);

    let client_nonce = [0x44; 32];
    let client_hello = ClientHello {
        version: ProtocolVersion::CURRENT,
        node_id: client_key.node_id(),
        device_name: "BridgeOS-UdpClient".to_string(),
        client_nonce,
        capabilities: Capabilities::all(),
    };
    client
        .send_frame(&Frame::Handshake(HandshakeFrame::ClientHello(client_hello)))
        .await
        .unwrap();

    let server_hello_frame = client.recv_frame().await.unwrap().expect("server hello");
    let server_hello = match server_hello_frame {
        Frame::Handshake(HandshakeFrame::ServerHello(h)) => h,
        other => panic!("Expected ServerHello, got {other:?}"),
    };

    let sig = client_key.sign_handshake_challenge(&client_nonce, &server_hello.server_nonce);
    client
        .send_frame(&Frame::Handshake(HandshakeFrame::AuthResponse(
            AuthResponse { signature: sig },
        )))
        .await
        .unwrap();

    let auth_result_frame = client.recv_frame().await.unwrap().expect("auth result");
    match auth_result_frame {
        Frame::Handshake(HandshakeFrame::AuthResult(res)) => {
            assert!(res.success);
        }
        other => panic!("Expected AuthResult, got {other:?}"),
    }

    // Ping / Pong
    client
        .send_frame(&Frame::Control(ControlFrame::Ping { nonce: 9999 }))
        .await
        .unwrap();

    let pong = client.recv_frame().await.unwrap().expect("pong");
    match pong {
        Frame::Control(ControlFrame::Pong { nonce }) => {
            assert_eq!(nonce, 9999);
        }
        other => panic!("Expected Pong, got {other:?}"),
    }

    // Disconnect
    client
        .send_frame(&Frame::Control(ControlFrame::Disconnect {
            reason: bridge_protocol::DisconnectReason::Graceful,
        }))
        .await
        .unwrap();

    server_task.await.unwrap();

    // Clean up
    server_discovery.shutdown().await.unwrap();
    client_discovery.shutdown().await.unwrap();

    info!("Milestone 1 UDP discovery and authenticated session test passed successfully!");
}
