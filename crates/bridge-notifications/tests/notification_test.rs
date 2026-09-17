use bridge_core::NodeId;
use bridge_notifications::{
    NotificationAction, NotificationMessage, NotificationPolicy, NotificationSyncEngine,
    NotificationSyncEvent, NotificationUrgency,
};

fn make_engine(id_byte: u8) -> NotificationSyncEngine {
    let node_id = NodeId::from_bytes([id_byte; 32]);
    NotificationSyncEngine::new(node_id, NotificationPolicy::default())
}

// ─── Wire round-trip ────────────────────────────────────────────────────────

#[test]
fn test_post_message_wire_roundtrip() {
    let origin = NodeId::from_bytes([0x01; 32]);
    let entry = bridge_notifications::NotificationEntry::new(
        origin,
        1,
        "notif:1",
        "com.whatsapp",
        "WhatsApp",
        "Alice: Hey!",
        "How are you?",
        NotificationUrgency::Normal,
    );
    let msg = NotificationMessage::Post(entry.clone());
    let frame = msg.to_data_frame().expect("serialization failed");
    let decoded = NotificationMessage::from_data_frame(&frame).expect("deserialization failed");
    assert_eq!(msg, decoded);
}

#[test]
fn test_dismiss_message_wire_roundtrip() {
    let origin = NodeId::from_bytes([0x01; 32]);
    let msg = NotificationMessage::Dismiss {
        origin,
        notification_id: "notif:42".to_string(),
        sequence: 7,
    };
    let frame = msg.to_data_frame().unwrap();
    let decoded = NotificationMessage::from_data_frame(&frame).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn test_action_invoked_wire_roundtrip() {
    let origin = NodeId::from_bytes([0x02; 32]);
    let msg = NotificationMessage::ActionInvoked {
        origin,
        notification_id: "notif:10".to_string(),
        sequence: 1,
        action: NotificationAction::Reply {
            text: "On my way!".to_string(),
        },
    };
    let frame = msg.to_data_frame().unwrap();
    let decoded = NotificationMessage::from_data_frame(&frame).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn test_clear_all_wire_roundtrip() {
    let origin = NodeId::from_bytes([0x03; 32]);
    let msg = NotificationMessage::ClearAll {
        origin,
        sequence: 99,
    };
    let frame = msg.to_data_frame().unwrap();
    let decoded = NotificationMessage::from_data_frame(&frame).unwrap();
    assert_eq!(msg, decoded);
}

#[test]
fn test_wrong_channel_rejected() {
    use bridge_protocol::DataFrame;
    let frame = DataFrame::new(1, b"not-a-notification".to_vec()); // channel 1 = clipboard
    let result = NotificationMessage::from_data_frame(&frame);
    assert!(result.is_err());
}

// ─── Engine: inbound post ────────────────────────────────────────────────────

#[tokio::test]
async fn test_engine_receives_notification() {
    let engine = make_engine(0x10);
    let mut events = engine.subscribe();

    let remote_origin = NodeId::from_bytes([0x20; 32]);
    let entry = bridge_notifications::NotificationEntry::new(
        remote_origin,
        1,
        "msg:1",
        "com.telegram",
        "Telegram",
        "Bob: Hello",
        "Are you free?",
        NotificationUrgency::Normal,
    );
    let msg = NotificationMessage::Post(entry.clone());
    let frame = msg.to_data_frame().unwrap();
    engine.handle_incoming_frame(&frame).await.unwrap();

    let event = events.try_recv().unwrap();
    assert!(matches!(event, NotificationSyncEvent::Received(_)));
}

#[tokio::test]
async fn test_engine_deduplication_suppresses_identical() {
    let engine = make_engine(0x11);
    let mut events = engine.subscribe();

    let remote_origin = NodeId::from_bytes([0x21; 32]);
    let entry = bridge_notifications::NotificationEntry::new(
        remote_origin,
        1,
        "msg:dup",
        "com.signal",
        "Signal",
        "New message",
        "Hey there",
        NotificationUrgency::Normal,
    );
    let msg = NotificationMessage::Post(entry.clone());
    let frame = msg.to_data_frame().unwrap();

    // First delivery should succeed.
    engine.handle_incoming_frame(&frame).await.unwrap();
    let event1 = events.try_recv().unwrap();
    assert!(matches!(event1, NotificationSyncEvent::Received(_)));

    // Identical second delivery should be silently dropped.
    engine.handle_incoming_frame(&frame).await.unwrap();
    assert!(events.try_recv().is_err(), "duplicate must be suppressed");
}

#[tokio::test]
async fn test_engine_updated_content_is_delivered() {
    let engine = make_engine(0x12);
    let mut events = engine.subscribe();

    let remote_origin = NodeId::from_bytes([0x22; 32]);
    let entry_v1 = bridge_notifications::NotificationEntry::new(
        remote_origin,
        1,
        "msg:update",
        "com.mail",
        "Mail",
        "1 new email",
        "From: Alice",
        NotificationUrgency::Normal,
    );
    let entry_v2 = bridge_notifications::NotificationEntry::new(
        remote_origin,
        2,
        "msg:update",
        "com.mail",
        "Mail",
        "3 new emails",
        "From: Alice, Bob, Carol",
        NotificationUrgency::Normal,
    );

    let f1 = NotificationMessage::Post(entry_v1).to_data_frame().unwrap();
    let f2 = NotificationMessage::Post(entry_v2).to_data_frame().unwrap();

    engine.handle_incoming_frame(&f1).await.unwrap();
    events.try_recv().unwrap(); // consume first event

    // Updated content (different text) must be delivered.
    engine.handle_incoming_frame(&f2).await.unwrap();
    let event2 = events.try_recv();
    assert!(event2.is_ok(), "updated notification must be delivered");
}

// ─── Engine: dismiss ─────────────────────────────────────────────────────────

#[tokio::test]
async fn test_engine_dismiss_emits_event() {
    let engine = make_engine(0x13);
    let mut events = engine.subscribe();

    let remote_origin = NodeId::from_bytes([0x23; 32]);
    let dismiss_msg = NotificationMessage::Dismiss {
        origin: remote_origin,
        notification_id: "notif:abc".to_string(),
        sequence: 1,
    };
    let frame = dismiss_msg.to_data_frame().unwrap();
    engine.handle_incoming_frame(&frame).await.unwrap();

    let event = events.try_recv().unwrap();
    assert!(matches!(event, NotificationSyncEvent::Dismissed { .. }));
}

#[tokio::test]
async fn test_engine_dismissed_notification_not_redelivered() {
    let engine = make_engine(0x14);
    let mut events = engine.subscribe();

    let remote_origin = NodeId::from_bytes([0x24; 32]);
    let post_entry = bridge_notifications::NotificationEntry::new(
        remote_origin,
        1,
        "notif:once",
        "com.app",
        "App",
        "Title",
        "Body",
        NotificationUrgency::Normal,
    );

    let post_frame = NotificationMessage::Post(post_entry.clone())
        .to_data_frame()
        .unwrap();
    let dismiss_frame = NotificationMessage::Dismiss {
        origin: remote_origin,
        notification_id: "notif:once".to_string(),
        sequence: 2,
    }
    .to_data_frame()
    .unwrap();

    engine.handle_incoming_frame(&post_frame).await.unwrap();
    events.try_recv().unwrap(); // Received event

    engine.handle_incoming_frame(&dismiss_frame).await.unwrap();
    events.try_recv().unwrap(); // Dismissed event

    // Re-post of identical content after dismiss: guard cleared hash, so it
    // will be attempted — but the dismissed state should suppress it.
    engine.handle_incoming_frame(&post_frame).await.unwrap();
    // No new Received event expected (dismissed state blocks re-post of same ID).
    assert!(
        events.try_recv().is_err(),
        "dismissed notification must not be re-delivered with same content"
    );
}

// ─── Engine: clear all ───────────────────────────────────────────────────────

#[tokio::test]
async fn test_engine_clear_all_emits_event() {
    let engine = make_engine(0x15);
    let mut events = engine.subscribe();

    let remote_origin = NodeId::from_bytes([0x25; 32]);
    let msg = NotificationMessage::ClearAll {
        origin: remote_origin,
        sequence: 1,
    };
    engine
        .handle_incoming_frame(&msg.to_data_frame().unwrap())
        .await
        .unwrap();

    let event = events.try_recv().unwrap();
    assert!(matches!(event, NotificationSyncEvent::ClearedAll { .. }));
}

// ─── Policy filtering ────────────────────────────────────────────────────────

#[tokio::test]
async fn test_engine_policy_blocks_2fa_notification() {
    let engine = make_engine(0x16);
    let mut events = engine.subscribe();

    let remote_origin = NodeId::from_bytes([0x26; 32]);
    let entry = bridge_notifications::NotificationEntry::new(
        remote_origin,
        1,
        "otp:1",
        "com.bank",
        "MyBank",
        "Verification Code: 492817",
        "Use this code within 60 seconds",
        NotificationUrgency::High,
    );
    let frame = NotificationMessage::Post(entry).to_data_frame().unwrap();
    engine.handle_incoming_frame(&frame).await.unwrap();

    let event = events.try_recv().unwrap();
    assert!(
        matches!(event, NotificationSyncEvent::PolicyBlocked { .. }),
        "2FA notification must be blocked by policy, got: {event:?}"
    );
}

// ─── Outbound builders ───────────────────────────────────────────────────────

#[tokio::test]
async fn test_engine_build_post_returns_message() {
    let engine = make_engine(0x17);
    let result = engine
        .build_post(
            "notif:local",
            "com.myapp",
            "MyApp",
            "You have a new message",
            "Click to view",
            NotificationUrgency::Normal,
        )
        .await
        .unwrap();

    assert!(result.is_some(), "should produce a Post message");
    if let Some(NotificationMessage::Post(entry)) = result {
        assert_eq!(entry.app_id, "com.myapp");
        assert_eq!(entry.sequence, 1);
    } else {
        panic!("expected Post variant");
    }
}

#[tokio::test]
async fn test_engine_build_post_sensitive_returns_none() {
    let engine = make_engine(0x18);
    let result = engine
        .build_post(
            "otp:out",
            "com.authenticator",
            "Authenticator",
            "Verification Code: 123456",
            "30s remaining",
            NotificationUrgency::High,
        )
        .await
        .unwrap();

    assert!(
        result.is_none(),
        "sensitive outbound notification must be suppressed"
    );
}

#[tokio::test]
async fn test_engine_build_dismiss_and_action() {
    let engine = make_engine(0x19);

    let dismiss_msg = engine.build_dismiss("notif:abc").await;
    assert!(matches!(dismiss_msg, NotificationMessage::Dismiss { .. }));

    let action_msg = engine
        .build_action(
            "notif:abc",
            NotificationAction::Reply {
                text: "Sure, on my way!".to_string(),
            },
        )
        .await;
    assert!(matches!(
        action_msg,
        NotificationMessage::ActionInvoked { .. }
    ));
}

// ─── Size limit enforcement ───────────────────────────────────────────────────

#[test]
fn test_size_limit_truncates_title() {
    let origin = NodeId::from_bytes([0x01; 32]);
    let long_title = "A".repeat(512);
    let mut entry = bridge_notifications::NotificationEntry::new(
        origin,
        1,
        "size:1",
        "com.verbose",
        "VerboseApp",
        long_title, // 512 bytes, limit is 256
        "Body text",
        NotificationUrgency::Normal,
    );
    entry.enforce_size_limits();
    assert!(
        entry.title.len() <= 256,
        "title must be truncated to 256 bytes"
    );
}

// ─── Active notification snapshot ────────────────────────────────────────────

#[tokio::test]
async fn test_engine_active_notification_snapshot() {
    let engine = make_engine(0x1a);
    let remote_origin = NodeId::from_bytes([0x2a; 32]);

    let entry = bridge_notifications::NotificationEntry::new(
        remote_origin,
        1,
        "snap:1",
        "com.gmail",
        "Gmail",
        "New email",
        "Hello!",
        NotificationUrgency::Normal,
    );
    let frame = NotificationMessage::Post(entry).to_data_frame().unwrap();
    engine.handle_incoming_frame(&frame).await.unwrap();

    let active = engine.active_notifications(&remote_origin).await;
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].notification_id, "snap:1");
}
