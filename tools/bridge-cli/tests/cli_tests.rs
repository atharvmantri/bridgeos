use bridge_cli::client::{connect_and_handshake, run_ping, run_send_file};
use bridge_cli::node::handle_connection;
use bridge_cli::{run_identity, Cli, Commands};
use bridge_clipboard::backend::MemoryClipboardBackend;
use bridge_clipboard::ClipboardSyncEngine;
use bridge_identity::{IdentityKey, TrustStore, TrustedPeer};
use clap::Parser;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;

#[test]
fn test_cli_parsing_node() {
    let args = vec![
        "bridge-cli",
        "node",
        "--name",
        "desktop-a",
        "--port",
        "9801",
        "--receive-dir",
        "./custom_received",
        "--broadcast-port",
        "42425",
        "--no-mdns",
        "--no-udp",
    ];

    let cli = Cli::try_parse_from(args).expect("Failed to parse node command");
    match cli.command {
        Commands::Node {
            name,
            port,
            device_type,
            receive_dir,
            broadcast_port,
            no_mdns,
            no_udp,
            data_dir: _,
            memory_clipboard: _,
        } => {
            assert_eq!(name, "desktop-a");
            assert_eq!(port, 9801);
            assert_eq!(device_type, "windows");
            assert_eq!(receive_dir, PathBuf::from("./custom_received"));
            assert_eq!(broadcast_port, 42425);
            assert!(no_mdns);
            assert!(no_udp);
        }
        _ => panic!("Expected Commands::Node"),
    }
}

#[test]
fn test_cli_parsing_node_memory_clipboard() {
    let args = vec!["bridge-cli", "node", "--memory-clipboard"];
    let cli = Cli::try_parse_from(args).expect("Failed to parse node command");
    match cli.command {
        Commands::Node {
            memory_clipboard, ..
        } => {
            assert!(memory_clipboard);
        }
        _ => panic!("Expected Commands::Node"),
    }
}

#[test]
fn test_cli_parsing_ping() {
    let args = vec![
        "bridge-cli",
        "ping",
        "--peer",
        "127.0.0.1:9801",
        "--count",
        "10",
        "--name",
        "ping-bot",
    ];

    let cli = Cli::try_parse_from(args).expect("Failed to parse ping command");
    match cli.command {
        Commands::Ping { peer, count, name } => {
            assert_eq!(peer, "127.0.0.1:9801".parse::<SocketAddr>().unwrap());
            assert_eq!(count, 10);
            assert_eq!(name, "ping-bot");
        }
        _ => panic!("Expected Commands::Ping"),
    }
}

#[test]
fn test_cli_parsing_send_file() {
    let args = vec![
        "bridge-cli",
        "send-file",
        "--peer",
        "127.0.0.1:9801",
        "--file",
        "./test.txt",
        "--name",
        "sender-bot",
    ];

    let cli = Cli::try_parse_from(args).expect("Failed to parse send-file command");
    match cli.command {
        Commands::SendFile {
            peer,
            file,
            name,
            data_dir: _,
        } => {
            assert_eq!(peer, "127.0.0.1:9801".parse::<SocketAddr>().unwrap());
            assert_eq!(file, PathBuf::from("./test.txt"));
            assert_eq!(name, "sender-bot");
        }
        _ => panic!("Expected Commands::SendFile"),
    }
}

#[test]
fn test_cli_parsing_discover() {
    let args = vec![
        "bridge-cli",
        "discover",
        "--duration",
        "5",
        "--broadcast-port",
        "42424",
    ];

    let cli = Cli::try_parse_from(args).expect("Failed to parse discover command");
    match cli.command {
        Commands::Discover {
            duration,
            broadcast_port,
        } => {
            assert_eq!(duration, 5);
            assert_eq!(broadcast_port, 42424);
        }
        _ => panic!("Expected Commands::Discover"),
    }
}

#[test]
fn test_cli_parsing_identity() {
    let args = vec!["bridge-cli", "identity", "--generate"];
    let cli = Cli::try_parse_from(args).expect("Failed to parse identity command");
    match cli.command {
        Commands::Identity { generate } => {
            assert!(generate);
        }
        _ => panic!("Expected Commands::Identity"),
    }

    let key = run_identity(true).expect("run_identity should succeed");
    assert_eq!(key.public_key().to_bytes().len(), 32);
}

fn test_clip_engine(node_id: bridge_core::NodeId) -> Arc<ClipboardSyncEngine> {
    Arc::new(ClipboardSyncEngine::new(
        node_id,
        Arc::new(MemoryClipboardBackend::new()),
        bridge_clipboard::ClipboardPolicy::default(),
    ))
}

#[tokio::test]
async fn test_node_harness_ping_roundtrip() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server_identity = Arc::new(IdentityKey::generate());
    let temp_dir = std::env::temp_dir().join(format!("bridge_cli_ping_{}", rand::random::<u64>()));
    tokio::fs::create_dir_all(&temp_dir).await.unwrap();
    let receive_dir = Arc::new(temp_dir.clone());
    let server_name = Arc::new("server-node".to_string());
    let trust_store = Arc::new(TrustStore::open(temp_dir.join("trust.db")).unwrap());
    let clip_engine = test_clip_engine(server_identity.node_id());
    let active_sessions = Arc::new(Mutex::new(HashMap::new()));

    let server_task = tokio::spawn(async move {
        let (socket, remote_addr) = listener.accept().await.unwrap();
        handle_connection(
            socket,
            remote_addr,
            server_identity,
            receive_dir,
            server_name,
            trust_store,
            clip_engine,
            active_sessions,
        )
        .await
    });

    let client_res = run_ping(addr, 2, "client-node").await;
    assert!(
        client_res.is_ok(),
        "Client ping should succeed: {client_res:?}"
    );

    let server_res = server_task.await.unwrap();
    assert!(
        server_res.is_ok(),
        "Server connection handling should succeed: {server_res:?}"
    );

    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[tokio::test]
async fn test_node_harness_send_file_roundtrip() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server_identity = Arc::new(IdentityKey::generate());
    let temp_recv = std::env::temp_dir().join(format!("bridge_cli_recv_{}", rand::random::<u64>()));
    tokio::fs::create_dir_all(&temp_recv).await.unwrap();
    let receive_dir = Arc::new(temp_recv.clone());
    let server_name = Arc::new("server-node".to_string());
    let trust_store = Arc::new(TrustStore::open(temp_recv.join("trust.db")).unwrap());
    let clip_engine = test_clip_engine(server_identity.node_id());
    let active_sessions = Arc::new(Mutex::new(HashMap::new()));

    // Create client identity and register it as trusted peer on server
    let client_identity = IdentityKey::generate();
    let trusted_peer = TrustedPeer {
        node_id: client_identity.node_id(),
        public_key: client_identity.public_key(),
        device_name: "client-sender".to_string(),
        device_type: bridge_core::DeviceType::Windows,
        first_paired_at: 100,
        last_seen_at: 100,
        trust_state: bridge_identity::TrustState::Trusted,
    };
    trust_store.save_peer(&trusted_peer).unwrap();

    // Create a 150KB source file (spans 3 chunks of 64KB)
    let temp_src = std::env::temp_dir().join(format!("bridge_cli_src_{}", rand::random::<u64>()));
    tokio::fs::create_dir_all(&temp_src).await.unwrap();
    let src_file = temp_src.join("test_payload.bin");
    let test_data = vec![0x42u8; 150 * 1024];
    tokio::fs::write(&src_file, &test_data).await.unwrap();

    let server_task = tokio::spawn(async move {
        let (socket, remote_addr) = listener.accept().await.unwrap();
        handle_connection(
            socket,
            remote_addr,
            server_identity,
            receive_dir,
            server_name,
            trust_store,
            clip_engine,
            active_sessions,
        )
        .await
    });

    let client_res = run_send_file(addr, &src_file, "client-sender", Some(&client_identity)).await;
    assert!(
        client_res.is_ok(),
        "Client send-file should succeed: {client_res:?}"
    );

    let server_res = server_task.await.unwrap();
    assert!(
        server_res.is_ok(),
        "Server connection handling should succeed: {server_res:?}"
    );

    // Verify the file arrived on disk with exact byte match
    let dest_file = temp_recv.join("test_payload.bin");
    assert!(dest_file.exists(), "Received file must exist");
    let received_data = tokio::fs::read(&dest_file).await.unwrap();
    assert_eq!(
        received_data, test_data,
        "Received file content must match exactly"
    );

    let _ = tokio::fs::remove_dir_all(&temp_src).await;
    let _ = tokio::fs::remove_dir_all(&temp_recv).await;
}

#[tokio::test]
async fn test_node_handshake_authentication_negotiation() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server_identity = Arc::new(IdentityKey::generate());
    let temp_dir =
        std::env::temp_dir().join(format!("bridge_cli_handshake_{}", rand::random::<u64>()));
    tokio::fs::create_dir_all(&temp_dir).await.unwrap();
    let receive_dir = Arc::new(temp_dir.clone());
    let server_name = Arc::new("server-node".to_string());
    let trust_store = Arc::new(TrustStore::open(temp_dir.join("trust.db")).unwrap());
    let clip_engine = test_clip_engine(server_identity.node_id());
    let active_sessions = Arc::new(Mutex::new(HashMap::new()));

    let server_task = tokio::spawn(async move {
        let (socket, remote_addr) = listener.accept().await.unwrap();
        handle_connection(
            socket,
            remote_addr,
            server_identity,
            receive_dir,
            server_name,
            trust_store,
            clip_engine,
            active_sessions,
        )
        .await
    });

    let client_identity = IdentityKey::generate();
    let handshake_res = connect_and_handshake(addr, &client_identity, "test-client").await;
    assert!(
        handshake_res.is_ok(),
        "Handshake should succeed: {handshake_res:?}"
    );

    let (framed, server_hello) = handshake_res.unwrap();
    assert_eq!(server_hello.device_name, "server-node");
    assert!(server_hello
        .negotiated_capabilities
        .contains(bridge_core::Capabilities::FILE_TRANSFER));
    drop(framed);

    let _ = server_task.await;
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[tokio::test]
async fn test_node_handshake_invalid_signature_rejection() {
    use bridge_core::{Capabilities, ProtocolVersion};
    use bridge_protocol::{AuthResponse, AuthResult, ClientHello, Frame, HandshakeFrame};
    use bridge_transport::FramedStream;
    use tokio::net::TcpStream;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server_identity = Arc::new(IdentityKey::generate());
    let temp_dir =
        std::env::temp_dir().join(format!("bridge_cli_reject_{}", rand::random::<u64>()));
    tokio::fs::create_dir_all(&temp_dir).await.unwrap();
    let receive_dir = Arc::new(temp_dir.clone());
    let server_name = Arc::new("server-node".to_string());
    let trust_store = Arc::new(TrustStore::open(temp_dir.join("trust.db")).unwrap());
    let clip_engine = test_clip_engine(server_identity.node_id());
    let active_sessions = Arc::new(Mutex::new(HashMap::new()));

    let server_task = tokio::spawn(async move {
        let (socket, remote_addr) = listener.accept().await.unwrap();
        handle_connection(
            socket,
            remote_addr,
            server_identity,
            receive_dir,
            server_name,
            trust_store,
            clip_engine,
            active_sessions,
        )
        .await
    });

    let client_a = IdentityKey::generate();
    let imposter_b = IdentityKey::generate();

    let socket = TcpStream::connect(addr).await.unwrap();
    let mut framed = FramedStream::new(socket);

    // Send ClientHello declaring client_a's NodeId
    let client_nonce = rand::random::<[u8; 32]>();
    let client_hello = ClientHello {
        version: ProtocolVersion::CURRENT,
        node_id: client_a.node_id(),
        device_name: "imposter-client".to_string(),
        client_nonce,
        capabilities: Capabilities::all(),
    };
    framed
        .send_frame(&Frame::Handshake(HandshakeFrame::ClientHello(client_hello)))
        .await
        .unwrap();

    // Receive ServerHello
    let server_hello = match framed.recv_frame().await.unwrap().unwrap() {
        Frame::Handshake(HandshakeFrame::ServerHello(sh)) => sh,
        other => panic!("Expected ServerHello, got: {other:?}"),
    };

    // Sign challenge with imposter_b instead of client_a!
    let forged_signature =
        imposter_b.sign_handshake_challenge(&client_nonce, &server_hello.server_nonce);
    framed
        .send_frame(&Frame::Handshake(HandshakeFrame::AuthResponse(
            AuthResponse {
                signature: forged_signature,
            },
        )))
        .await
        .unwrap();

    // Await AuthResult: MUST BE REJECTED
    let auth_frame = framed.recv_frame().await.unwrap().unwrap();
    match auth_frame {
        Frame::Handshake(HandshakeFrame::AuthResult(AuthResult { success, reason })) => {
            assert!(
                !success,
                "Imposter challenge signature MUST fail verification"
            );
            assert!(reason.is_some(), "Failure reason must be provided");
        }
        other => panic!("Expected AuthResult frame, got: {other:?}"),
    }

    let _ = server_task.await;
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
}

#[tokio::test]
async fn test_untrusted_peer_file_transfer_blocked() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server_identity = Arc::new(IdentityKey::generate());
    let temp_recv =
        std::env::temp_dir().join(format!("bridge_cli_blocked_{}", rand::random::<u64>()));
    tokio::fs::create_dir_all(&temp_recv).await.unwrap();
    let receive_dir = Arc::new(temp_recv.clone());
    let server_name = Arc::new("server-node".to_string());
    let trust_store = Arc::new(TrustStore::open(temp_recv.join("trust.db")).unwrap());
    let clip_engine = test_clip_engine(server_identity.node_id());
    let active_sessions = Arc::new(Mutex::new(HashMap::new()));

    // Notice: Client is NOT registered in trust_store! Peer is untrusted.

    let temp_src =
        std::env::temp_dir().join(format!("bridge_cli_src_blocked_{}", rand::random::<u64>()));
    tokio::fs::create_dir_all(&temp_src).await.unwrap();
    let src_file = temp_src.join("blocked_payload.bin");
    tokio::fs::write(&src_file, b"untrusted secret payload")
        .await
        .unwrap();

    let _server_task = tokio::spawn(async move {
        let (socket, remote_addr) = listener.accept().await.unwrap();
        let _ = handle_connection(
            socket,
            remote_addr,
            server_identity,
            receive_dir,
            server_name,
            trust_store,
            clip_engine,
            active_sessions,
        )
        .await;
    });

    // Attempting to send a file from an untrusted client must fail / time out waiting for Accept
    let res = tokio::time::timeout(
        std::time::Duration::from_millis(1500),
        run_send_file(addr, &src_file, "untrusted-sender", None),
    )
    .await;

    // Either the timeout expires waiting for Accept (because server blocked/dropped the offer)
    // or run_send_file returns an Err.
    match res {
        Err(_) => {
            // Timed out because server safely blocked/ignored the transfer offer
        }
        Ok(send_res) => {
            assert!(
                send_res.is_err(),
                "Untrusted peer must NOT succeed in transferring a file: {send_res:?}"
            );
        }
    }

    let _ = tokio::fs::remove_dir_all(&temp_src).await;
    let _ = tokio::fs::remove_dir_all(&temp_recv).await;
}
