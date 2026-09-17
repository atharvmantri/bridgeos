use bridge_clipboard::{
    ClipboardBackend, ClipboardContent, ClipboardPolicy, ClipboardSyncEngine,
    MemoryClipboardBackend,
};
use bridge_core::DeviceType;
use bridge_identity::{IdentityKey, TrustStore};
use bridge_session::{ActiveSession, AutoConfirm, SessionError, SessionState};
use std::sync::Arc;
use tokio::io::duplex;

#[tokio::test]
async fn test_untrusted_peer_handshake_and_clipboard_gating() {
    let key_a = Arc::new(IdentityKey::generate());
    let key_b = Arc::new(IdentityKey::generate());

    let trust_dir_a = tempfile::tempdir().unwrap();
    let trust_dir_b = tempfile::tempdir().unwrap();

    let trust_store_a = Arc::new(TrustStore::open(trust_dir_a.path().join("trust.db")).unwrap());
    let trust_store_b = Arc::new(TrustStore::open(trust_dir_b.path().join("trust.db")).unwrap());

    let backend_b = Arc::new(MemoryClipboardBackend::new());
    let clip_engine_b = Arc::new(ClipboardSyncEngine::new(
        key_b.node_id(),
        backend_b.clone(),
        ClipboardPolicy::default(),
    ));

    let (client_stream, server_stream) = duplex(64 * 1024);

    let key_a_clone = key_a.clone();
    let trust_store_a_clone = trust_store_a.clone();
    let client_task = tokio::spawn(async move {
        ActiveSession::client_handshake(
            client_stream,
            key_a_clone,
            "Client-A".into(),
            DeviceType::Windows,
            trust_store_a_clone,
            None,
            None,
        )
        .await
    });

    let server_session_res = ActiveSession::server_handshake(
        server_stream,
        key_b.clone(),
        "Server-B".into(),
        DeviceType::Windows,
        trust_store_b.clone(),
        Some(clip_engine_b.clone()),
        None,
    )
    .await;

    let mut client_session = client_task.await.unwrap().expect("Client handshake failed");
    let mut server_session = server_session_res.expect("Server handshake failed");

    // Both must be AuthenticatedUntrusted (not yet in TrustStore)
    assert_eq!(client_session.state, SessionState::AuthenticatedUntrusted);
    assert_eq!(server_session.state, SessionState::AuthenticatedUntrusted);
    assert!(!client_session.state.is_trusted());
    assert!(!server_session.state.is_trusted());

    // Client attempts to send clipboard frame while untrusted
    let clip_text = ClipboardContent::Text("Secret data from untrusted client".into());
    let fake_entry = bridge_clipboard::ClipboardEntry::new(key_a.node_id(), 1, clip_text, false);
    let msg = bridge_clipboard::ClipboardMessage::Sync(fake_entry);
    let df = msg.to_data_frame().unwrap();

    client_session
        .framed
        .send_frame(&bridge_protocol::Frame::Data(df))
        .await
        .unwrap();

    // Server processes next frame: should DROP it because client is untrusted!
    let processed = server_session.process_next_frame().await.unwrap();
    assert!(processed);

    // Verify Server-B clipboard was NOT modified (remains empty)
    let content = backend_b.get_content().unwrap();
    assert!(
        content.is_none(),
        "Untrusted peer was able to write clipboard!"
    );
}

#[tokio::test]
async fn test_key_mismatch_impersonation_defense() {
    let real_key_a = IdentityKey::generate();
    let imposter_key_c = IdentityKey::generate();
    let key_b = Arc::new(IdentityKey::generate());

    let trust_dir_b = tempfile::tempdir().unwrap();
    let trust_store_b = Arc::new(TrustStore::open(trust_dir_b.path().join("trust.db")).unwrap());

    // Node B already trusts Node A with real_key_a
    let trusted_peer = bridge_identity::TrustedPeer {
        node_id: real_key_a.node_id(),
        public_key: real_key_a.public_key(),
        device_name: "Real-Node-A".into(),
        device_type: DeviceType::Windows,
        first_paired_at: 1000,
        last_seen_at: 1000,
        trust_state: bridge_identity::TrustState::Trusted,
    };
    trust_store_b.save_peer(&trusted_peer).unwrap();

    let (client_stream, server_stream) = duplex(64 * 1024);

    // Imposter C claims Node A's NodeId in ClientHello, but signs challenge with imposter_key_c!
    let real_node_id = real_key_a.node_id();
    let imposter_task = tokio::spawn(async move {
        let mut framed = bridge_transport::FramedStream::new(client_stream);
        let client_nonce = rand::random::<[u8; 32]>();
        let hello = bridge_protocol::ClientHello {
            version: bridge_core::ProtocolVersion::CURRENT,
            node_id: real_node_id, // Spoofed NodeId!
            device_name: "Real-Node-A".into(),
            client_nonce,
            capabilities: bridge_core::Capabilities::all(),
        };
        framed
            .send_frame(&bridge_protocol::Frame::Handshake(
                bridge_protocol::HandshakeFrame::ClientHello(hello),
            ))
            .await
            .unwrap();

        let server_hello_frame = framed.recv_frame().await.unwrap().unwrap();
        let server_hello = match server_hello_frame {
            bridge_protocol::Frame::Handshake(bridge_protocol::HandshakeFrame::ServerHello(h)) => h,
            other => panic!("Expected ServerHello, got {other:?}"),
        };

        // Imposter signs with its own key, not real_key_a!
        let imposter_sig =
            imposter_key_c.sign_handshake_challenge(&client_nonce, &server_hello.server_nonce);
        framed
            .send_frame(&bridge_protocol::Frame::Handshake(
                bridge_protocol::HandshakeFrame::AuthResponse(bridge_protocol::AuthResponse {
                    signature: imposter_sig,
                }),
            ))
            .await
            .unwrap();

        let res = framed.recv_frame().await;
        res
    });

    let server_res = ActiveSession::server_handshake(
        server_stream,
        key_b,
        "Server-B".into(),
        DeviceType::Windows,
        trust_store_b,
        None,
        None,
    )
    .await;

    let _ = imposter_task.await;

    // Server must REJECT the connection because signature does not match spoofed NodeId!
    assert!(
        matches!(server_res, Err(SessionError::AuthenticationFailed(_))),
        "Server did not reject spoofed NodeId!"
    );
}

#[tokio::test]
async fn test_explicit_sas_pairing_flow_and_post_pairing_clipboard() {
    let key_a = Arc::new(IdentityKey::generate());
    let key_b = Arc::new(IdentityKey::generate());

    let trust_dir_a = tempfile::tempdir().unwrap();
    let trust_dir_b = tempfile::tempdir().unwrap();

    let trust_store_a = Arc::new(TrustStore::open(trust_dir_a.path().join("trust.db")).unwrap());
    let trust_store_b = Arc::new(TrustStore::open(trust_dir_b.path().join("trust.db")).unwrap());

    let backend_b = Arc::new(MemoryClipboardBackend::new());
    let clip_engine_b = Arc::new(ClipboardSyncEngine::new(
        key_b.node_id(),
        backend_b.clone(),
        ClipboardPolicy::default(),
    ));

    let (client_stream, server_stream) = duplex(64 * 1024);

    let key_a_clone = key_a.clone();
    let trust_store_a_clone = trust_store_a.clone();
    let client_task = tokio::spawn(async move {
        ActiveSession::client_handshake(
            client_stream,
            key_a_clone,
            "Client-A".into(),
            DeviceType::Windows,
            trust_store_a_clone,
            None,
            None,
        )
        .await
    });

    let mut server_session = ActiveSession::server_handshake(
        server_stream,
        key_b.clone(),
        "Server-B".into(),
        DeviceType::Windows,
        trust_store_b.clone(),
        Some(clip_engine_b.clone()),
        None,
    )
    .await
    .unwrap();

    let mut client_session = client_task.await.unwrap().unwrap();

    // Now execute pairing concurrently: Client is initiator, Server is responder
    let pair_client_task = tokio::spawn(async move {
        let sas = client_session
            .execute_pairing(true, &AutoConfirm(true))
            .await?;
        Ok::<(ActiveSession<_>, _), SessionError>((client_session, sas))
    });

    let server_sas = server_session
        .execute_pairing(false, &AutoConfirm(true))
        .await
        .expect("Server pairing failed");

    let (mut client_session, client_sas) = pair_client_task
        .await
        .unwrap()
        .expect("Client pairing failed");

    // Both MUST compute the exact same 6-digit numeric PIN!
    assert_eq!(client_sas.numeric_pin, server_sas.numeric_pin);
    assert_eq!(client_sas.formatted_pin, server_sas.formatted_pin);
    assert_eq!(client_sas.hex_fingerprint, server_sas.hex_fingerprint);

    // Both sessions must now be Trusted
    assert_eq!(client_session.state, SessionState::Trusted);
    assert_eq!(server_session.state, SessionState::Trusted);
    assert!(client_session.state.is_trusted());
    assert!(server_session.state.is_trusted());

    // Both TrustStores must have recorded the peer
    assert!(trust_store_a
        .is_trusted(&key_b.node_id(), &key_b.public_key())
        .unwrap());
    assert!(trust_store_b
        .is_trusted(&key_a.node_id(), &key_a.public_key())
        .unwrap());

    // NOW that they are trusted, test clipboard delivery!
    let clip_text = ClipboardContent::Text("Hello Trusted BridgeOS!".into());
    let entry = bridge_clipboard::ClipboardEntry::new(key_a.node_id(), 1, clip_text.clone(), false);
    let msg = bridge_clipboard::ClipboardMessage::Sync(entry);
    let df = msg.to_data_frame().unwrap();

    client_session
        .framed
        .send_frame(&bridge_protocol::Frame::Data(df))
        .await
        .unwrap();

    let processed = server_session.process_next_frame().await.unwrap();
    assert!(processed);

    // Server-B's clipboard backend MUST now contain the text!
    let content = backend_b.get_content().unwrap();
    assert_eq!(content, Some(clip_text));
}

#[tokio::test]
async fn test_pairing_user_rejection() {
    let key_a = Arc::new(IdentityKey::generate());
    let key_b = Arc::new(IdentityKey::generate());

    let trust_dir_a = tempfile::tempdir().unwrap();
    let trust_dir_b = tempfile::tempdir().unwrap();

    let trust_store_a = Arc::new(TrustStore::open(trust_dir_a.path().join("trust.db")).unwrap());
    let trust_store_b = Arc::new(TrustStore::open(trust_dir_b.path().join("trust.db")).unwrap());

    let (client_stream, server_stream) = duplex(64 * 1024);

    let key_a_clone = key_a.clone();
    let trust_store_a_clone = trust_store_a.clone();
    let client_task = tokio::spawn(async move {
        ActiveSession::client_handshake(
            client_stream,
            key_a_clone,
            "Client-A".into(),
            DeviceType::Windows,
            trust_store_a_clone,
            None,
            None,
        )
        .await
    });

    let mut server_session = ActiveSession::server_handshake(
        server_stream,
        key_b.clone(),
        "Server-B".into(),
        DeviceType::Windows,
        trust_store_b.clone(),
        None,
        None,
    )
    .await
    .unwrap();

    let mut client_session = client_task.await.unwrap().unwrap();

    // Client initiates with AutoConfirm(true), but Server REJECTS with AutoConfirm(false)
    let pair_client_task = tokio::spawn(async move {
        client_session
            .execute_pairing(true, &AutoConfirm(true))
            .await
    });

    let server_res = server_session
        .execute_pairing(false, &AutoConfirm(false))
        .await;
    assert!(matches!(server_res, Err(SessionError::PairingRejected)));

    let client_res = pair_client_task.await.unwrap();
    assert!(client_res.is_err());

    // Neither node is marked trusted in TrustStore
    assert!(!trust_store_a
        .is_trusted(&key_b.node_id(), &key_b.public_key())
        .unwrap());
    assert!(!trust_store_b
        .is_trusted(&key_a.node_id(), &key_a.public_key())
        .unwrap());
}
