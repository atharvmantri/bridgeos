use bridge_core::{Capabilities, ProtocolVersion};
use bridge_identity::IdentityKey;
use bridge_integration_tests::init_test_logging;
use bridge_protocol::{
    AuthResponse, AuthResult, ClientHello, ControlFrame, DataFrame, DisconnectReason, Frame,
    HandshakeFrame, ServerHello,
};
use bridge_transport::FramedStream;
use tokio::io::duplex;
use tokio::net::{TcpListener, TcpStream};
use tracing::info;

#[tokio::test]
async fn test_milestone0_in_memory_handshake_and_exchange() {
    init_test_logging();
    info!("Starting Milestone 0 in-memory handshake test");

    let client_key = IdentityKey::generate();
    let server_key = IdentityKey::generate();
    let client_pub = client_key.public_key();

    let (client_io, server_io) = duplex(64 * 1024);
    let mut client = FramedStream::new(client_io);
    let mut server = FramedStream::new(server_io);

    let client_nonce = [11u8; 32];
    let server_nonce = [22u8; 32];

    // Server background task
    let server_task = tokio::spawn(async move {
        // 1. Await ClientHello
        let frame = server.recv_frame().await.unwrap().expect("Missing frame");
        let client_hello = match frame {
            Frame::Handshake(HandshakeFrame::ClientHello(hello)) => hello,
            other => panic!("Expected ClientHello, got {other:?}"),
        };

        assert_eq!(client_hello.version, ProtocolVersion::CURRENT);
        assert_eq!(client_hello.client_nonce, client_nonce);

        // 2. Respond with ServerHello
        let agreed_caps = client_hello.capabilities.intersect(&Capabilities::all());
        let server_hello = ServerHello {
            agreed_version: ProtocolVersion::CURRENT,
            node_id: server_key.node_id(),
            device_name: "Desktop-Server".to_string(),
            server_nonce,
            negotiated_capabilities: agreed_caps,
        };
        server
            .send_frame(&Frame::Handshake(HandshakeFrame::ServerHello(server_hello)))
            .await
            .unwrap();

        // 3. Await AuthResponse
        let auth_frame = server.recv_frame().await.unwrap().expect("Missing auth");
        let auth_response = match auth_frame {
            Frame::Handshake(HandshakeFrame::AuthResponse(resp)) => resp,
            other => panic!("Expected AuthResponse, got {other:?}"),
        };

        // 4. Verify Ed25519 signature from client
        let verify_res = client_pub.verify_handshake_challenge(
            &client_nonce,
            &server_nonce,
            &auth_response.signature,
        );
        assert!(verify_res.is_ok(), "Client signature verification failed");

        // 5. Send AuthResult
        server
            .send_frame(&Frame::Handshake(HandshakeFrame::AuthResult(
                AuthResult::ok(),
            )))
            .await
            .unwrap();

        // 6. Echo application data
        let data_frame = server.recv_frame().await.unwrap().expect("Missing data");
        match data_frame {
            Frame::Data(df) => {
                assert_eq!(df.channel, DataFrame::CHANNEL_CLIPBOARD);
                assert_eq!(df.payload, b"Milestone 0 Continuity Payload");
                // Echo back confirmation
                server
                    .send_frame(&Frame::Data(DataFrame::new(
                        df.channel,
                        b"ACK: Milestone 0 Payload Received",
                    )))
                    .await
                    .unwrap();
            }
            other => panic!("Expected DataFrame, got {other:?}"),
        }

        // 7. Await graceful disconnect
        let disc_frame = server
            .recv_frame()
            .await
            .unwrap()
            .expect("Missing disconnect");
        match disc_frame {
            Frame::Control(ControlFrame::Disconnect { reason }) => {
                assert_eq!(reason, DisconnectReason::Graceful);
            }
            other => panic!("Expected Disconnect, got {other:?}"),
        }
    });

    // Client execution
    // 1. Send ClientHello
    let client_hello = ClientHello {
        version: ProtocolVersion::CURRENT,
        node_id: client_key.node_id(),
        device_name: "Desktop-Client".to_string(),
        client_nonce,
        capabilities: Capabilities::all(),
    };
    client
        .send_frame(&Frame::Handshake(HandshakeFrame::ClientHello(client_hello)))
        .await
        .unwrap();

    // 2. Await ServerHello
    let s_frame = client
        .recv_frame()
        .await
        .unwrap()
        .expect("Missing server hello");
    let server_hello = match s_frame {
        Frame::Handshake(HandshakeFrame::ServerHello(sh)) => sh,
        other => panic!("Expected ServerHello, got {other:?}"),
    };
    assert_eq!(server_hello.server_nonce, server_nonce);

    // 3. Sign challenge and send AuthResponse
    let signature = client_key.sign_handshake_challenge(&client_nonce, &server_hello.server_nonce);
    client
        .send_frame(&Frame::Handshake(HandshakeFrame::AuthResponse(
            AuthResponse { signature },
        )))
        .await
        .unwrap();

    // 4. Await AuthResult
    let res_frame = client
        .recv_frame()
        .await
        .unwrap()
        .expect("Missing auth result");
    match res_frame {
        Frame::Handshake(HandshakeFrame::AuthResult(res)) => assert!(res.success),
        other => panic!("Expected AuthResult, got {other:?}"),
    }

    // 5. Send application data
    client
        .send_frame(&Frame::Data(DataFrame::new(
            DataFrame::CHANNEL_CLIPBOARD,
            b"Milestone 0 Continuity Payload".to_vec(),
        )))
        .await
        .unwrap();

    // 6. Receive echo ACK
    let ack_frame = client.recv_frame().await.unwrap().expect("Missing ack");
    match ack_frame {
        Frame::Data(df) => {
            assert_eq!(df.payload, b"ACK: Milestone 0 Payload Received");
        }
        other => panic!("Expected DataFrame ACK, got {other:?}"),
    }

    // 7. Send graceful disconnect
    client
        .send_frame(&Frame::Control(ControlFrame::Disconnect {
            reason: DisconnectReason::Graceful,
        }))
        .await
        .unwrap();

    server_task.await.unwrap();
    info!("Milestone 0 in-memory handshake test passed successfully!");
}

#[tokio::test]
async fn test_milestone0_real_tcp_socket_handshake() {
    init_test_logging();
    info!("Starting Milestone 0 real localhost TCP handshake test");

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server_addr = listener.local_addr().unwrap();

    let client_key = IdentityKey::generate();
    let server_key = IdentityKey::generate();
    let client_pub = client_key.public_key();

    let client_nonce = [33u8; 32];
    let server_nonce = [44u8; 32];

    // Server task accepting real TCP connection
    let server_task = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut server = FramedStream::new(socket);

        // Receive ClientHello
        let frame = server.recv_frame().await.unwrap().expect("No frame");
        let hello = match frame {
            Frame::Handshake(HandshakeFrame::ClientHello(h)) => h,
            other => panic!("Expected ClientHello, got {other:?}"),
        };

        // Send ServerHello
        let server_hello = ServerHello {
            agreed_version: ProtocolVersion::CURRENT,
            node_id: server_key.node_id(),
            device_name: "TCP-Server-Node".to_string(),
            server_nonce,
            negotiated_capabilities: hello.capabilities.intersect(&Capabilities::all()),
        };
        server
            .send_frame(&Frame::Handshake(HandshakeFrame::ServerHello(server_hello)))
            .await
            .unwrap();

        // Verify AuthResponse
        let auth_frame = server.recv_frame().await.unwrap().expect("No auth");
        let auth = match auth_frame {
            Frame::Handshake(HandshakeFrame::AuthResponse(a)) => a,
            other => panic!("Expected AuthResponse, got {other:?}"),
        };

        client_pub
            .verify_handshake_challenge(&client_nonce, &server_nonce, &auth.signature)
            .expect("Signature invalid");

        server
            .send_frame(&Frame::Handshake(HandshakeFrame::AuthResult(
                AuthResult::ok(),
            )))
            .await
            .unwrap();

        // Handle Ping
        let ping_frame = server.recv_frame().await.unwrap().expect("No ping");
        match ping_frame {
            Frame::Control(ControlFrame::Ping { nonce }) => {
                server
                    .send_frame(&Frame::Control(ControlFrame::Pong { nonce }))
                    .await
                    .unwrap();
            }
            other => panic!("Expected Ping, got {other:?}"),
        }
    });

    // Client task connecting over real TCP
    let client_socket = TcpStream::connect(server_addr).await.unwrap();
    let mut client = FramedStream::new(client_socket);

    client
        .send_frame(&Frame::Handshake(HandshakeFrame::ClientHello(
            ClientHello {
                version: ProtocolVersion::CURRENT,
                node_id: client_key.node_id(),
                device_name: "TCP-Client-Node".to_string(),
                client_nonce,
                capabilities: Capabilities::all(),
            },
        )))
        .await
        .unwrap();

    let sh = match client.recv_frame().await.unwrap().expect("No SH") {
        Frame::Handshake(HandshakeFrame::ServerHello(sh)) => sh,
        other => panic!("Expected ServerHello, got {other:?}"),
    };

    let sig = client_key.sign_handshake_challenge(&client_nonce, &sh.server_nonce);
    client
        .send_frame(&Frame::Handshake(HandshakeFrame::AuthResponse(
            AuthResponse { signature: sig },
        )))
        .await
        .unwrap();

    match client.recv_frame().await.unwrap().expect("No auth res") {
        Frame::Handshake(HandshakeFrame::AuthResult(res)) => assert!(res.success),
        other => panic!("Expected AuthResult, got {other:?}"),
    }

    // Ping / Pong test
    client
        .send_frame(&Frame::Control(ControlFrame::Ping { nonce: 9999 }))
        .await
        .unwrap();

    match client.recv_frame().await.unwrap().expect("No pong") {
        Frame::Control(ControlFrame::Pong { nonce }) => assert_eq!(nonce, 9999),
        other => panic!("Expected Pong, got {other:?}"),
    }

    server_task.await.unwrap();
    info!("Milestone 0 real localhost TCP handshake test passed successfully!");
}
