use bridge_core::{DeviceType, NodeId};
use bridge_identity::{
    IdentityError, IdentityKey, IdentityStorage, PairingSession, SasVerification, TrustState,
    TrustStore, TrustedPeer,
};

#[test]
fn test_trust_store_in_memory_crud() {
    let store = TrustStore::open_in_memory().expect("Failed to create in-memory store");

    let key = IdentityKey::generate();
    let node_id = key.node_id();
    let pubkey = key.public_key();

    let peer = TrustedPeer {
        node_id,
        public_key: pubkey,
        device_name: "Phone-Pixel".to_string(),
        device_type: DeviceType::Android,
        first_paired_at: 1000,
        last_seen_at: 1050,
        trust_state: TrustState::Trusted,
    };

    store.save_peer(&peer).expect("save_peer should succeed");

    let retrieved = store
        .get_peer(&node_id)
        .expect("get_peer should succeed")
        .expect("peer should be found");

    assert_eq!(retrieved.node_id, node_id);
    assert_eq!(retrieved.public_key, pubkey);
    assert_eq!(retrieved.device_name, "Phone-Pixel");
    assert_eq!(retrieved.device_type, DeviceType::Android);
    assert_eq!(retrieved.first_paired_at, 1000);
    assert_eq!(retrieved.last_seen_at, 1050);
    assert_eq!(retrieved.trust_state, TrustState::Trusted);

    // Update last seen
    store
        .update_last_seen(&node_id, 2000)
        .expect("update_last_seen should succeed");
    let updated = store.get_peer(&node_id).unwrap().unwrap();
    assert_eq!(updated.last_seen_at, 2000);

    // List peers
    let peers = store.list_peers().expect("list_peers should succeed");
    assert_eq!(peers.len(), 1);

    let trusted = store
        .list_trusted_peers()
        .expect("list_trusted_peers should succeed");
    assert_eq!(trusted.len(), 1);

    // Delete peer
    let deleted = store
        .delete_peer(&node_id)
        .expect("delete_peer should succeed");
    assert!(deleted);
    assert!(store.get_peer(&node_id).unwrap().is_none());
}

#[test]
fn test_trust_store_disk_persistence_across_restarts() {
    let temp_dir =
        std::env::temp_dir().join(format!("bridge_trust_test_{}", rand::random::<u64>()));
    let db_path = temp_dir.join("trust.db");

    let key1 = IdentityKey::generate();
    let peer1 = TrustedPeer {
        node_id: key1.node_id(),
        public_key: key1.public_key(),
        device_name: "Desktop-Win".to_string(),
        device_type: DeviceType::Windows,
        first_paired_at: 500,
        last_seen_at: 550,
        trust_state: TrustState::Trusted,
    };

    let key2 = IdentityKey::generate();
    let peer2 = TrustedPeer {
        node_id: key2.node_id(),
        public_key: key2.public_key(),
        device_name: "Laptop-Mac".to_string(),
        device_type: DeviceType::MacOS,
        first_paired_at: 600,
        last_seen_at: 600,
        trust_state: TrustState::Pending,
    };

    // 1. Initial write
    {
        let store = TrustStore::open(&db_path).expect("open store should succeed");
        store.save_peer(&peer1).unwrap();
        store.save_peer(&peer2).unwrap();
    }

    // 2. Reopen after restart
    {
        let store = TrustStore::open(&db_path).expect("reopen store should succeed");

        let p1 = store
            .get_peer(&key1.node_id())
            .unwrap()
            .expect("p1 must exist");
        assert_eq!(p1.device_name, "Desktop-Win");
        assert_eq!(p1.trust_state, TrustState::Trusted);

        let p2 = store
            .get_peer(&key2.node_id())
            .unwrap()
            .expect("p2 must exist");
        assert_eq!(p2.device_name, "Laptop-Mac");
        assert_eq!(p2.trust_state, TrustState::Pending);

        let trusted_only = store.list_trusted_peers().unwrap();
        assert_eq!(trusted_only.len(), 1);
        assert_eq!(trusted_only[0].node_id, key1.node_id());
    }

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_is_trusted_validates_public_key_and_detects_impersonation() {
    let store = TrustStore::open_in_memory().unwrap();

    let genuine_key = IdentityKey::generate();
    let imposter_key = IdentityKey::generate();
    let unknown_key = IdentityKey::generate();

    let peer = TrustedPeer {
        node_id: genuine_key.node_id(),
        public_key: genuine_key.public_key(),
        device_name: "Genuine-Peer".to_string(),
        device_type: DeviceType::Linux,
        first_paired_at: 100,
        last_seen_at: 100,
        trust_state: TrustState::Trusted,
    };
    store.save_peer(&peer).unwrap();

    // 1. Legitimate peer with matching key
    let trusted = store
        .is_trusted(&genuine_key.node_id(), &genuine_key.public_key())
        .expect("is_trusted query should succeed");
    assert!(trusted, "Genuine peer with matching key must be trusted");

    // 2. Unknown peer
    let unknown_trusted = store
        .is_trusted(&unknown_key.node_id(), &unknown_key.public_key())
        .expect("unknown query should succeed");
    assert!(!unknown_trusted, "Unknown peer must not be trusted");

    // 3. Impersonation attack: Attacker presents Genuine's NodeId, but signs with Imposter's key!
    let impersonation_result = store.is_trusted(&genuine_key.node_id(), &imposter_key.public_key());
    match impersonation_result {
        Err(IdentityError::KeyMismatch { node_id, .. }) => {
            assert_eq!(node_id, genuine_key.node_id());
        }
        other => panic!("Expected KeyMismatch error on impersonation, got: {other:?}"),
    }
}

#[test]
fn test_revocation_lifecycle() {
    let store = TrustStore::open_in_memory().unwrap();
    let key = IdentityKey::generate();

    let peer = TrustedPeer {
        node_id: key.node_id(),
        public_key: key.public_key(),
        device_name: "Revocable-Device".to_string(),
        device_type: DeviceType::Android,
        first_paired_at: 100,
        last_seen_at: 100,
        trust_state: TrustState::Trusted,
    };
    store.save_peer(&peer).unwrap();

    assert!(store.is_trusted(&key.node_id(), &key.public_key()).unwrap());

    // Revoke
    store.revoke_peer(&key.node_id()).unwrap();

    let p = store.get_peer(&key.node_id()).unwrap().unwrap();
    assert_eq!(p.trust_state, TrustState::Revoked);

    let verify_res = store.is_trusted(&key.node_id(), &key.public_key());
    match verify_res {
        Err(IdentityError::PeerRevoked(id)) => assert_eq!(id, key.node_id()),
        other => panic!("Expected PeerRevoked, got: {other:?}"),
    }
}

#[test]
fn test_pairing_session_mutual_sas_derivation_and_finalization() {
    let alice_key = IdentityKey::generate();
    let bob_key = IdentityKey::generate();

    // 1. Alice initiates pairing with Bob
    let (alice_session, req) =
        PairingSession::new_initiator(alice_key, "Alice-Laptop", DeviceType::Windows);

    // 2. Bob receives Alice's request and responds
    let (mut bob_session, resp, bob_sas) =
        PairingSession::new_responder(bob_key, "Bob-Phone", DeviceType::Android, &req)
            .expect("Bob should accept request");

    // 3. Alice receives Bob's response
    let mut alice_session = alice_session;
    let alice_sas = alice_session
        .initiator_receive_response(&resp)
        .expect("Alice should accept response");

    // 4. CRITICAL: Both Alice and Bob must derive the EXACT SAME SAS PIN & FINGERPRINT
    assert_eq!(alice_sas.numeric_pin, bob_sas.numeric_pin);
    assert_eq!(alice_sas.formatted_pin, bob_sas.formatted_pin);
    assert_eq!(alice_sas.hex_fingerprint, bob_sas.hex_fingerprint);
    assert_eq!(alice_sas.formatted_pin.len(), 7); // "XXX-XXX" format

    // 5. Humans verify SAS on both screens and confirm
    let alice_confirm = alice_session.confirm().expect("Alice confirmation");
    let bob_confirm = bob_session.confirm().expect("Bob confirmation");

    // 6. Mutual finalization
    let bob_trusted_by_alice = alice_session
        .finalize(&bob_confirm)
        .expect("Alice finalize Bob");
    let alice_trusted_by_bob = bob_session
        .finalize(&alice_confirm)
        .expect("Bob finalize Alice");

    assert_eq!(bob_trusted_by_alice.trust_state, TrustState::Trusted);
    assert_eq!(bob_trusted_by_alice.device_name, "Bob-Phone");

    assert_eq!(alice_trusted_by_bob.trust_state, TrustState::Trusted);
    assert_eq!(alice_trusted_by_bob.device_name, "Alice-Laptop");
}

#[test]
fn test_pairing_symmetry_regardless_of_initiator() {
    let node_a = NodeId::from_bytes([0x11; 32]);
    let node_b = NodeId::from_bytes([0xee; 32]);
    let nonce_a = [0xaa; 32];
    let nonce_b = [0xbb; 32];

    let sas1 = SasVerification::derive(&node_a, &node_b, &nonce_a, &nonce_b);
    let sas2 = SasVerification::derive(&node_b, &node_a, &nonce_b, &nonce_a);

    assert_eq!(
        sas1, sas2,
        "SAS must be symmetric regardless of who initiated"
    );
}

#[test]
fn test_pairing_user_rejection() {
    let alice_key = IdentityKey::generate();
    let bob_key = IdentityKey::generate();

    let (alice_session, req) =
        PairingSession::new_initiator(alice_key, "Alice-Laptop", DeviceType::Windows);
    let (bob_session, resp, _) =
        PairingSession::new_responder(bob_key, "Bob-Phone", DeviceType::Android, &req).unwrap();

    let mut alice_session = alice_session;
    let _ = alice_session.initiator_receive_response(&resp).unwrap();

    // Bob rejects (user tapped 'Reject' because SAS didn't match or suspicious device)
    let bob_rejection = bridge_identity::PairingConfirm {
        node_id: bob_session.remote_peer().unwrap().node_id,
        confirmed: false,
        signature: vec![],
    };

    let finalize_res = alice_session.finalize(&bob_rejection);
    assert!(finalize_res.is_err(), "Finalize must fail if peer rejected");
}

#[test]
fn test_identity_storage_save_and_load() {
    let temp_dir =
        std::env::temp_dir().join(format!("bridge_id_storage_{}", rand::random::<u64>()));
    let keyfile = temp_dir.join("device.key");

    // 1. First call generates and saves
    let key1 = IdentityStorage::load_or_generate(&keyfile).unwrap();
    assert!(keyfile.exists());

    // 2. Second call loads exact same key
    let key2 = IdentityStorage::load_or_generate(&keyfile).unwrap();
    assert_eq!(key1.node_id(), key2.node_id());
    assert_eq!(key1.to_bytes(), key2.to_bytes());

    let _ = std::fs::remove_dir_all(&temp_dir);
}
