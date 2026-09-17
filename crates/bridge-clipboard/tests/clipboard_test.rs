use bridge_clipboard::*;
use bridge_core::NodeId;
use bridge_protocol::DataFrame;
use std::sync::Arc;

#[tokio::test]
async fn test_clipboard_content_hashing_and_wire_roundtrip() {
    let node_id = NodeId::from_bytes([0x11; 32]);
    let text_content = ClipboardContent::Text("Hello BridgeOS Clipboard!".to_string());
    assert_eq!(text_content.format(), ClipboardFormat::Text);
    assert_eq!(text_content.byte_len(), 25);

    let hash = text_content.compute_hash();
    assert_ne!(hash, [0u8; 32]);

    let entry = ClipboardEntry::new(node_id, 1, text_content.clone(), false);
    assert_eq!(entry.metadata.origin, node_id);
    assert_eq!(entry.metadata.sequence, 1);
    assert_eq!(entry.metadata.content_hash, hash);
    assert!(entry.validate_integrity().is_ok());

    // Serialize to DataFrame
    let message = ClipboardMessage::Sync(entry);
    let frame = message.to_data_frame().expect("Frame encoding failed");
    assert_eq!(frame.channel, DataFrame::CHANNEL_CLIPBOARD);

    // Decode from DataFrame
    let decoded_msg = ClipboardMessage::from_data_frame(&frame).expect("Frame decoding failed");
    match decoded_msg {
        ClipboardMessage::Sync(decoded_entry) => {
            assert_eq!(decoded_entry.metadata.origin, node_id);
            assert_eq!(decoded_entry.metadata.sequence, 1);
            assert_eq!(decoded_entry.content, text_content);
            assert!(decoded_entry.validate_integrity().is_ok());
        }
        _ => panic!("Expected Sync message"),
    }
}

#[tokio::test]
async fn test_echo_guard_deduplication_and_sequence_tracking() {
    let mut guard = EchoGuard::new(4);
    let node_a = NodeId::from_bytes([0x01; 32]);

    let hash_1 = [0xAA; 32];
    let hash_2 = [0xBB; 32];
    let hash_3 = [0xCC; 32];

    // Outbound send records hash
    guard.record_sent(hash_1);
    assert!(guard.should_suppress(&hash_1));
    assert!(!guard.should_suppress(&hash_2));

    // Receiving an already recorded hash is suppressed
    assert!(!guard.record_received(node_a, 1, hash_1));

    // Receiving a new hash with sequence 1 succeeds
    assert!(guard.record_received(node_a, 1, hash_2));
    assert!(guard.should_suppress(&hash_2));
    assert_eq!(guard.last_sequence(&node_a), Some(1));

    // Receiving stale sequence (seq 1 again or seq 0) is rejected
    assert!(!guard.record_received(node_a, 1, hash_3));
    assert!(!guard.record_received(node_a, 0, hash_3));

    // Receiving higher sequence (seq 2) succeeds
    assert!(guard.record_received(node_a, 2, hash_3));
    assert_eq!(guard.last_sequence(&node_a), Some(2));

    // Capacity eviction in ring buffer (capacity was 4)
    let h4 = [0x44; 32];
    let h5 = [0x55; 32];
    let h6 = [0x66; 32];
    guard.record_sent(h4);
    guard.record_sent(h5);
    guard.record_sent(h6);

    // Oldest hash (hash_1) should be evicted from the bounded set
    assert!(!guard.should_suppress(&hash_1));
    // Newest hash should be present
    assert!(guard.should_suppress(&h6));
}

#[tokio::test]
async fn test_clipboard_policy_enforcement() {
    let policy = ClipboardPolicy {
        max_text_bytes: 100,
        max_image_bytes: 500,
        allow_images: false,
        allow_html: true,
        filter_sensitive: true,
        enabled: true,
    };

    // 1. Normal text passes
    let normal_text = ClipboardContent::Text("Short text".into());
    assert!(policy.validate(&normal_text, false).is_ok());

    // 2. Sensitive text is blocked
    assert!(matches!(
        policy.validate(&normal_text, true),
        Err(ClipboardError::SensitiveContentBlocked)
    ));

    // 3. Oversized text is blocked
    let long_text = ClipboardContent::Text("A".repeat(101));
    assert!(matches!(
        policy.validate(&long_text, false),
        Err(ClipboardError::OversizedPayload {
            size: 101,
            max: 100
        })
    ));

    // 4. Image is blocked when disallowed
    let img = ClipboardContent::Image {
        format: ImageFormat::Png,
        data: vec![0u8; 50],
        width: 10,
        height: 10,
    };
    assert!(matches!(
        policy.validate(&img, false),
        Err(ClipboardError::UnsupportedFormat(_))
    ));
}

#[tokio::test]
async fn test_end_to_end_bidirectional_sync_and_loopback_suppression() {
    let node_a_id = NodeId::from_bytes([0x0A; 32]);
    let node_b_id = NodeId::from_bytes([0x0B; 32]);

    let backend_a = Arc::new(MemoryClipboardBackend::new());
    let backend_b = Arc::new(MemoryClipboardBackend::new());

    let engine_a = Arc::new(ClipboardSyncEngine::new(
        node_a_id,
        backend_a.clone(),
        ClipboardPolicy::default(),
    ));
    let engine_b = Arc::new(ClipboardSyncEngine::new(
        node_b_id,
        backend_b.clone(),
        ClipboardPolicy::default(),
    ));

    let mut events_b = engine_b.subscribe();

    // Node A copies some text
    let test_str = "Copied on Node A".to_string();
    let frame_opt = engine_a
        .handle_local_change(ClipboardContent::Text(test_str.clone()), false)
        .expect("Node A failed to package local change");

    let frame = frame_opt.expect("Expected outbound frame from Node A");
    assert_eq!(frame.channel, DataFrame::CHANNEL_CLIPBOARD);

    // Node B receives frame from Node A
    let event = engine_b
        .handle_incoming_frame(&frame)
        .expect("Node B failed to handle incoming frame");

    match event {
        Some(ClipboardSyncEvent::RemoteApplied {
            origin,
            sequence,
            format,
            ..
        }) => {
            assert_eq!(origin, node_a_id);
            assert_eq!(sequence, 1);
            assert_eq!(format, ClipboardFormat::Text);
        }
        _ => panic!("Expected RemoteApplied event on Node B"),
    }

    // Verify Node B's local clipboard was updated
    let content_b = backend_b
        .get_content()
        .expect("Failed to get Node B content");
    assert_eq!(content_b, Some(ClipboardContent::Text(test_str.clone())));

    // CRITICAL LOOPBACK TEST:
    // Now simulate Node B's local clipboard watcher firing with the exact same content
    // that was just applied from Node A.
    let loopback_frame = engine_b
        .handle_local_change(ClipboardContent::Text(test_str.clone()), false)
        .expect("Node B check failed");

    // Must be suppressed! (None returned, preventing infinite echo loop)
    assert!(
        loopback_frame.is_none(),
        "Loopback echo was NOT suppressed by EchoGuard!"
    );

    // Verify an EchoSuppressed event was received by subscribers on Node B
    let received_evt = events_b.recv().await.expect("Failed to receive event");
    assert!(matches!(
        received_evt,
        ClipboardSyncEvent::RemoteApplied { .. }
    ));
    let second_evt = events_b
        .recv()
        .await
        .expect("Failed to receive second event");
    assert!(matches!(
        second_evt,
        ClipboardSyncEvent::EchoSuppressed { .. }
    ));
}

#[tokio::test]
async fn test_corrupted_hash_rejection() {
    let node_a = NodeId::from_bytes([0x01; 32]);
    let backend = Arc::new(MemoryClipboardBackend::new());
    let engine = ClipboardSyncEngine::new(node_a, backend, ClipboardPolicy::default());

    // Create entry with falsified hash
    let mut entry = ClipboardEntry::new(
        NodeId::from_bytes([0x02; 32]),
        1,
        ClipboardContent::Text("Valid content".into()),
        false,
    );
    entry.metadata.content_hash = [0xFF; 32]; // Tampered hash

    let msg = ClipboardMessage::Sync(entry);
    let frame = msg.to_data_frame().unwrap();

    let res = engine.handle_incoming_frame(&frame);
    assert!(matches!(res, Err(ClipboardError::HashMismatch { .. })));
}

#[tokio::test]
async fn test_image_clipboard_sync_roundtrip() {
    let node_a = NodeId::from_bytes([0x01; 32]);
    let node_b = NodeId::from_bytes([0x02; 32]);

    let backend_b = Arc::new(MemoryClipboardBackend::new());
    let engine_b = ClipboardSyncEngine::new(node_b, backend_b.clone(), ClipboardPolicy::default());

    let fake_png = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x01];
    let image_content = ClipboardContent::Image {
        format: ImageFormat::Png,
        data: fake_png.clone(),
        width: 1920,
        height: 1080,
    };

    let entry = ClipboardEntry::new(node_a, 42, image_content.clone(), false);
    let msg = ClipboardMessage::Sync(entry);
    let frame = msg.to_data_frame().unwrap();

    let evt = engine_b.handle_incoming_frame(&frame).unwrap();
    assert!(matches!(
        evt,
        Some(ClipboardSyncEvent::RemoteApplied {
            format: ClipboardFormat::Image(ImageFormat::Png),
            ..
        })
    ));

    let received = backend_b.get_content().unwrap().unwrap();
    assert_eq!(received, image_content);
}
