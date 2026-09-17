use bridge_clipboard::{ClipboardContent, ClipboardEntry, ClipboardMessage, ClipboardMetadata};
use bridge_core::{Capabilities, DeviceType, NodeId, ProtocolVersion};
use bridge_discovery::{decode_beacon, encode_beacon, BeaconMessage};
use bridge_identity::{PairingConfirm, PairingRequest, PairingResponse, SasVerification};
use bridge_protocol::{
    encode_frame, AuthResponse, AuthResult, ClientHello, ControlFrame, DisconnectReason, Frame,
    HandshakeFrame, ServerHello,
};
use bridge_session::PairingMessage;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

fn fixtures_dir() -> std::path::PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    Path::new(&manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests")
        .join("interop")
        .join("fixtures")
}

#[test]
fn test_generate_and_verify_golden_fixtures() {
    let out_dir = fixtures_dir();
    fs::create_dir_all(&out_dir).expect("Failed to create fixtures directory");

    // 1. Framing Header Vector (BRG1 + 4-byte BE length)
    let sample_payload = b"TESTPAYLOAD";
    let len_u32 = u32::try_from(sample_payload.len()).unwrap();
    let mut frame_header = Vec::new();
    frame_header.extend_from_slice(b"BRG1");
    frame_header.extend_from_slice(&len_u32.to_be_bytes());
    frame_header.extend_from_slice(sample_payload);
    fs::write(out_dir.join("frame_header.bin"), &frame_header).unwrap();

    // 2. NodeId Derivation Vector
    let pubkey_bytes = [
        0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e,
        0x1f, 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2a, 0x2b, 0x2c, 0x2d,
        0x2e, 0x2f,
    ];
    let mut hasher = Sha256::new();
    hasher.update(pubkey_bytes);
    let expected_node_id = hasher.finalize();
    let node_id = NodeId::from_bytes(expected_node_id.into());
    let node_id_json = serde_json::json!({
        "public_key_hex": hex::encode(pubkey_bytes),
        "expected_node_id_hex": hex::encode(expected_node_id),
    });
    fs::write(
        out_dir.join("node_id_derivation.json"),
        serde_json::to_string_pretty(&node_id_json).unwrap(),
    )
    .unwrap();

    // 3. ClientHello Frame
    let client_hello = ClientHello {
        version: ProtocolVersion::new(1, 0),
        node_id,
        device_name: "Pixel-7-Android".to_string(),
        client_nonce: [0xaa; 32],
        capabilities: Capabilities::all(),
    };
    let client_hello_frame = Frame::Handshake(HandshakeFrame::ClientHello(client_hello));
    let client_hello_bytes = encode_frame(&client_hello_frame).unwrap();
    fs::write(
        out_dir.join("handshake_client_hello.bin"),
        &client_hello_bytes,
    )
    .unwrap();

    // 4. ServerHello Frame
    let server_hello = ServerHello {
        agreed_version: ProtocolVersion::new(1, 0),
        node_id: NodeId::from_bytes([0x55; 32]),
        device_name: "Desktop-Win11".to_string(),
        server_nonce: [0xbb; 32],
        negotiated_capabilities: Capabilities::all(),
    };
    let server_hello_frame = Frame::Handshake(HandshakeFrame::ServerHello(server_hello));
    let server_hello_bytes = encode_frame(&server_hello_frame).unwrap();
    fs::write(
        out_dir.join("handshake_server_hello.bin"),
        &server_hello_bytes,
    )
    .unwrap();

    // 5. AuthResponse Frame
    let auth_response = AuthResponse {
        signature: [0xcc; 64],
    };
    let auth_response_frame = Frame::Handshake(HandshakeFrame::AuthResponse(auth_response));
    let auth_response_bytes = encode_frame(&auth_response_frame).unwrap();
    fs::write(
        out_dir.join("handshake_auth_response.bin"),
        &auth_response_bytes,
    )
    .unwrap();

    // 6. AuthResult Frame
    let auth_result = AuthResult::ok();
    let auth_result_frame = Frame::Handshake(HandshakeFrame::AuthResult(auth_result));
    let auth_result_bytes = encode_frame(&auth_result_frame).unwrap();
    fs::write(
        out_dir.join("handshake_auth_result.bin"),
        &auth_result_bytes,
    )
    .unwrap();

    // 7. Control Ping & Pong Frames
    let ping_frame = Frame::Control(ControlFrame::Ping {
        nonce: 1_234_567_890,
    });
    let ping_bytes = encode_frame(&ping_frame).unwrap();
    fs::write(out_dir.join("control_ping.bin"), &ping_bytes).unwrap();

    let pong_frame = Frame::Control(ControlFrame::Pong {
        nonce: 1_234_567_890,
    });
    let pong_bytes = encode_frame(&pong_frame).unwrap();
    fs::write(out_dir.join("control_pong.bin"), &pong_bytes).unwrap();

    let disconnect_frame = Frame::Control(ControlFrame::Disconnect {
        reason: DisconnectReason::Graceful,
    });
    let disconnect_bytes = encode_frame(&disconnect_frame).unwrap();
    fs::write(out_dir.join("control_disconnect.bin"), &disconnect_bytes).unwrap();

    // 8. SAS Verification Derivation Vector
    let node_a = NodeId::from_bytes([0x11; 32]);
    let node_b = NodeId::from_bytes([0x22; 32]);
    let nonce_a = [0x33; 32];
    let nonce_b = [0x44; 32];
    let sas = SasVerification::derive(&node_a, &node_b, &nonce_a, &nonce_b);
    let sas_json = serde_json::json!({
        "node_a_hex": hex::encode(node_a.0),
        "node_b_hex": hex::encode(node_b.0),
        "nonce_a_hex": hex::encode(nonce_a),
        "nonce_b_hex": hex::encode(nonce_b),
        "expected_numeric_pin": sas.numeric_pin,
        "expected_formatted_pin": sas.formatted_pin,
        "expected_hex_fingerprint": sas.hex_fingerprint,
    });
    fs::write(
        out_dir.join("sas_derivation.json"),
        serde_json::to_string_pretty(&sas_json).unwrap(),
    )
    .unwrap();

    // 9. Pairing Messages (Postcard encoded inside DataFrame CHANNEL_PAIRING)
    let pairing_req = PairingMessage::Request(PairingRequest {
        node_id,
        public_key: pubkey_bytes,
        device_name: "Pixel-7-Android".to_string(),
        device_type: DeviceType::Android,
        nonce: [0x44; 32],
        timestamp: 1_700_000_000,
    });
    let req_df = pairing_req.to_data_frame().unwrap();
    let req_frame = Frame::Data(req_df);
    let req_bytes = encode_frame(&req_frame).unwrap();
    fs::write(out_dir.join("pairing_request_frame.bin"), &req_bytes).unwrap();

    let pairing_resp = PairingMessage::Response(PairingResponse {
        node_id: NodeId::from_bytes([0x55; 32]),
        public_key: [0x66; 32],
        device_name: "Desktop-Win11".to_string(),
        device_type: DeviceType::Windows,
        nonce: [0x77; 32],
        timestamp: 1_700_000_001,
    });
    let resp_df = pairing_resp.to_data_frame().unwrap();
    let resp_frame = Frame::Data(resp_df);
    let resp_bytes = encode_frame(&resp_frame).unwrap();
    fs::write(out_dir.join("pairing_response_frame.bin"), &resp_bytes).unwrap();

    let pairing_confirm = PairingMessage::Confirm(PairingConfirm {
        node_id,
        confirmed: true,
        signature: vec![0x88; 64],
    });
    let conf_df = pairing_confirm.to_data_frame().unwrap();
    let conf_frame = Frame::Data(conf_df);
    let conf_bytes = encode_frame(&conf_frame).unwrap();
    fs::write(out_dir.join("pairing_confirm_frame.bin"), &conf_bytes).unwrap();

    // 10. Clipboard Synchronization Message Vector
    let clip_text = "Hello from BridgeOS cross-platform clipboard!";
    let clip_content = ClipboardContent::Text(clip_text.to_string());
    let clip_entry = ClipboardEntry {
        metadata: ClipboardMetadata {
            origin: node_id,
            sequence: 1,
            timestamp_ms: 1_700_000_000_000,
            content_hash: clip_content.compute_hash(),
            format: clip_content.format(),
            byte_size: clip_content.byte_len(),
            is_sensitive: false,
        },
        content: clip_content,
    };
    let clip_msg = ClipboardMessage::Sync(clip_entry);
    let clip_df = clip_msg.to_data_frame().unwrap();
    let clip_frame = Frame::Data(clip_df);
    let clip_bytes = encode_frame(&clip_frame).unwrap();
    fs::write(out_dir.join("clipboard_sync_frame.bin"), &clip_bytes).unwrap();

    // 11. UDP Broadcast Beacon Vector
    let beacon_msg = BeaconMessage::Announcement {
        version: ProtocolVersion::new(1, 0),
        node_id,
        device_name: "Pixel-7-Android".to_string(),
        device_type: DeviceType::Android,
        port: 45100,
        capabilities: Capabilities::all(),
        seq: 1,
    };
    let beacon_bytes = encode_beacon(&beacon_msg).unwrap();
    fs::write(out_dir.join("udp_beacon_announcement.bin"), &beacon_bytes).unwrap();

    // Self-verification: Assert all written frames decode back to exact structures in Rust
    let decoded_client_hello = bridge_protocol::decode_frame(&client_hello_bytes).unwrap();
    assert_eq!(client_hello_frame, decoded_client_hello);

    let decoded_server_hello = bridge_protocol::decode_frame(&server_hello_bytes).unwrap();
    assert_eq!(server_hello_frame, decoded_server_hello);

    let decoded_auth_resp = bridge_protocol::decode_frame(&auth_response_bytes).unwrap();
    assert_eq!(auth_response_frame, decoded_auth_resp);

    let decoded_auth_res = bridge_protocol::decode_frame(&auth_result_bytes).unwrap();
    assert_eq!(auth_result_frame, decoded_auth_res);

    let decoded_ping = bridge_protocol::decode_frame(&ping_bytes).unwrap();
    assert_eq!(ping_frame, decoded_ping);

    let decoded_clip = bridge_protocol::decode_frame(&clip_bytes).unwrap();
    assert_eq!(clip_frame, decoded_clip);

    let decoded_beacon = decode_beacon(&beacon_bytes).unwrap();
    assert_eq!(beacon_msg, decoded_beacon);
}
