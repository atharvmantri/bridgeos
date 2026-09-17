use anyhow::{bail, Context, Result};
use bridge_core::{Capabilities, DeviceType, NodeId, ProtocolVersion};
use bridge_identity::{IdentityKey, IdentityStorage, TrustStore};
use bridge_protocol::{
    AuthResponse, AuthResult, ClientHello, ControlFrame, DisconnectReason, Frame, HandshakeFrame,
    ServerHello,
};
use bridge_session::{ActiveSession, InteractiveCliConfirm};
use bridge_transfer::{create_manifest_from_file, FileSender, TransferMessage};
use bridge_transport::FramedStream;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tokio::net::TcpStream;
use tracing::{debug, info};

/// Establishes a TCP connection to a remote peer, performs mutual Ed25519 authentication,
/// and returns the authenticated framed stream along with the negotiated `ServerHello`.
pub async fn connect_and_handshake(
    peer_addr: SocketAddr,
    client_key: &IdentityKey,
    device_name: &str,
) -> Result<(FramedStream<TcpStream>, ServerHello)> {
    info!(target = %peer_addr, "Connecting to BridgeOS peer...");
    let socket = TcpStream::connect(peer_addr)
        .await
        .with_context(|| format!("Failed to connect to peer at {peer_addr}"))?;
    let mut framed = FramedStream::new(socket);

    // 1. Send ClientHello
    let client_nonce = rand::random::<[u8; 32]>();
    let client_hello = ClientHello {
        version: ProtocolVersion::CURRENT,
        node_id: client_key.node_id(),
        device_name: device_name.to_string(),
        client_nonce,
        capabilities: Capabilities::all(),
    };
    framed
        .send_frame(&Frame::Handshake(HandshakeFrame::ClientHello(client_hello)))
        .await
        .context("Failed to send ClientHello")?;

    // 2. Await ServerHello
    let frame = framed
        .recv_frame()
        .await
        .context("Failed to receive ServerHello frame")?
        .context("Connection closed unexpectedly while awaiting ServerHello")?;

    let server_hello = match frame {
        Frame::Handshake(HandshakeFrame::ServerHello(sh)) => sh,
        other => bail!("Expected ServerHello from peer, received: {other:?}"),
    };

    debug!(
        peer_name = %server_hello.device_name,
        peer_node = %server_hello.node_id,
        "Received ServerHello"
    );

    // 3. Sign mutual handshake challenge and send AuthResponse
    let signature = client_key.sign_handshake_challenge(&client_nonce, &server_hello.server_nonce);
    framed
        .send_frame(&Frame::Handshake(HandshakeFrame::AuthResponse(
            AuthResponse { signature },
        )))
        .await
        .context("Failed to send AuthResponse")?;

    // 4. Await AuthResult
    let auth_frame = framed
        .recv_frame()
        .await
        .context("Failed to receive AuthResult")?
        .context("Connection closed unexpectedly while awaiting AuthResult")?;

    match auth_frame {
        Frame::Handshake(HandshakeFrame::AuthResult(AuthResult { success: true, .. })) => {
            info!(
                peer_node = %server_hello.node_id,
                peer_name = %server_hello.device_name,
                "Mutual Ed25519 authentication successful"
            );
        }
        Frame::Handshake(HandshakeFrame::AuthResult(AuthResult {
            success: false,
            reason,
        })) => {
            bail!(
                "Peer rejected authentication: {}",
                reason.unwrap_or_else(|| "unspecified error".to_string())
            );
        }
        other => bail!("Expected AuthResult frame, received: {other:?}"),
    }

    Ok((framed, server_hello))
}

/// Executes a ping sequence to measure round-trip latency to an authenticated peer.
#[allow(clippy::cast_precision_loss, clippy::cast_lossless)]
pub async fn run_ping(peer_addr: SocketAddr, count: usize, device_name: &str) -> Result<()> {
    let client_key = IdentityKey::generate();
    let (mut framed, server_hello) =
        connect_and_handshake(peer_addr, &client_key, device_name).await?;

    println!(
        "\n--- Pinging peer '{}' ({}) at {peer_addr} ---",
        server_hello.device_name, server_hello.node_id
    );

    let mut successful = 0;
    let mut total_duration = std::time::Duration::ZERO;

    for seq in 1..=count {
        let nonce = rand::random::<u64>();
        let start = Instant::now();

        framed
            .send_frame(&Frame::Control(ControlFrame::Ping { nonce }))
            .await
            .context("Failed to send Ping frame")?;

        let reply = framed
            .recv_frame()
            .await
            .context("Failed to receive Pong frame")?
            .context("Peer closed connection while awaiting Pong")?;

        let rtt = start.elapsed();

        match reply {
            Frame::Control(ControlFrame::Pong { nonce: reply_nonce }) => {
                if reply_nonce == nonce {
                    println!(
                        "seq={seq}: Pong from {peer_addr} - rtt={:.2}ms",
                        rtt.as_secs_f64() * 1000.0
                    );
                    successful += 1;
                    total_duration += rtt;
                } else {
                    println!("seq={seq}: Corrupted Pong nonce received");
                }
            }
            other => {
                println!("seq={seq}: Unexpected frame received: {other:?}");
            }
        }

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }

    // Graceful disconnect
    let _ = framed
        .send_frame(&Frame::Control(ControlFrame::Disconnect {
            reason: DisconnectReason::Graceful,
        }))
        .await;

    println!("\n--- Ping statistics for {peer_addr} ---");
    println!(
        "{count} packets transmitted, {successful} received, {:.1}% packet loss",
        (1.0 - (successful as f64 / count as f64)) * 100.0
    );
    if successful > 0 {
        println!(
            "Average RTT: {:.2}ms",
            (total_duration.as_secs_f64() * 1000.0) / (successful as f64)
        );
    }

    Ok(())
}

/// Streams an on-disk file to a remote peer using the chunked resumable `bridge-transfer` engine.
#[allow(clippy::cast_precision_loss, clippy::cast_lossless)]
pub async fn run_send_file(
    peer_addr: SocketAddr,
    file_path: &Path,
    device_name: &str,
    client_key: Option<&IdentityKey>,
) -> Result<()> {
    if !file_path.exists() {
        bail!("File not found: {}", file_path.display());
    }

    println!(
        "Calculating file manifest and Blake3 checksums for {}...",
        file_path.display()
    );
    let manifest = create_manifest_from_file(file_path)
        .await
        .with_context(|| format!("Failed to create file manifest for {}", file_path.display()))?;

    let file_size = manifest.total_size;
    let total_chunks = manifest.total_chunks;
    println!(
        "File: '{}' ({file_size} bytes, {total_chunks} chunks, Blake3: {})",
        manifest.filename,
        hex::encode(manifest.blake3_root_hash)
    );

    let generated_key;
    let effective_key = if let Some(k) = client_key {
        k
    } else {
        generated_key = IdentityKey::generate();
        &generated_key
    };
    let (mut framed, server_hello) =
        connect_and_handshake(peer_addr, effective_key, device_name).await?;

    println!(
        "Connected to peer '{}' ({}). Offering file transfer...",
        server_hello.device_name, server_hello.node_id
    );

    // 1. Send Offer
    let offer = TransferMessage::Offer(manifest.clone());
    framed
        .send_frame(&Frame::Data(offer.to_data_frame()?))
        .await
        .context("Failed to send TransferMessage::Offer")?;

    // 2. Await Accept
    let accept_frame = framed
        .recv_frame()
        .await
        .context("Failed to receive Accept frame")?
        .context("Peer closed connection while awaiting Accept")?;

    let (file_id, start_chunk) = match accept_frame {
        Frame::Data(df) => match TransferMessage::from_data_frame(&df)? {
            TransferMessage::Accept {
                file_id,
                start_chunk,
            } => (file_id, start_chunk),
            TransferMessage::Cancel { reason, .. } => {
                bail!("Peer rejected file transfer: {reason}");
            }
            other => bail!("Expected Accept frame from peer, received: {other:?}"),
        },
        other => bail!("Expected Data frame, received: {other:?}"),
    };

    if file_id != manifest.file_id {
        bail!(
            "File ID mismatch in Accept message: expected {}, got {}",
            manifest.file_id,
            file_id
        );
    }

    if start_chunk > 0 {
        println!("Resuming transfer from chunk {start_chunk}/{total_chunks}...");
    } else {
        println!("Starting file transfer from chunk 0/{total_chunks}...");
    }

    let mut sender = FileSender::from_file(file_path, manifest.clone(), start_chunk)
        .await
        .context("Failed to initialize FileSender")?;

    let start_time = Instant::now();
    let mut transferred_bytes = 0u64;

    while let Some(chunk) = sender
        .next_chunk()
        .await
        .context("Failed to read next file chunk")?
    {
        let chunk_idx = chunk.chunk_index;
        let chunk_len = chunk.data.len() as u64;

        // Send Chunk
        let data_msg = TransferMessage::Data(chunk);
        framed
            .send_frame(&Frame::Data(data_msg.to_data_frame()?))
            .await
            .context("Failed to send file chunk")?;

        // Await Ack
        let ack_frame = framed
            .recv_frame()
            .await
            .context("Failed to receive Chunk Ack")?
            .context("Peer disconnected mid-transfer")?;

        match ack_frame {
            Frame::Data(df) => match TransferMessage::from_data_frame(&df)? {
                TransferMessage::Ack {
                    file_id: ack_fid,
                    chunk_index: ack_idx,
                } => {
                    if ack_fid != manifest.file_id || ack_idx != chunk_idx {
                        bail!("Ack mismatch: expected chunk {chunk_idx}, got {ack_idx}");
                    }
                    transferred_bytes += chunk_len;
                    let progress = (chunk_idx + 1) as f64 / total_chunks as f64 * 100.0;
                    print!(
                        "\rStreaming chunks: [{}/{}] {:.1}% ({transferred_bytes}/{file_size} bytes)",
                        chunk_idx + 1,
                        total_chunks,
                        progress
                    );
                    let _ = std::io::Write::flush(&mut std::io::stdout());
                }
                TransferMessage::Cancel { reason, .. } => {
                    bail!("Transfer cancelled by peer: {reason}");
                }
                other => bail!("Expected Ack frame, got: {other:?}"),
            },
            other => bail!("Expected Data frame, got: {other:?}"),
        }
    }

    println!();
    // 3. Send Finished signal
    let finished = TransferMessage::Finished {
        file_id: manifest.file_id.clone(),
        blake3_root_hash: manifest.blake3_root_hash,
    };
    framed
        .send_frame(&Frame::Data(finished.to_data_frame()?))
        .await
        .context("Failed to send Finished signal")?;

    // 4. Await Complete
    let complete_frame = framed
        .recv_frame()
        .await
        .context("Failed to receive Complete frame")?
        .context("Peer closed connection while awaiting Complete confirmation")?;

    match complete_frame {
        Frame::Data(df) => match TransferMessage::from_data_frame(&df)? {
            TransferMessage::Complete { file_id } => {
                if file_id != manifest.file_id {
                    bail!(
                        "Complete file ID mismatch: expected {}, got {file_id}",
                        manifest.file_id
                    );
                }
            }
            TransferMessage::Cancel { reason, .. } => {
                bail!("Peer rejected file hash verification: {reason}");
            }
            other => bail!("Expected Complete confirmation, got: {other:?}"),
        },
        other => bail!("Expected Data frame, got: {other:?}"),
    }

    let elapsed = start_time.elapsed();
    let throughput_mb =
        (transferred_bytes as f64 / (1024.0 * 1024.0)) / elapsed.as_secs_f64().max(0.001);

    println!(
        "Transfer complete! Sent {transferred_bytes} bytes in {:.2}s ({throughput_mb:.2} MB/s). Verified by peer Blake3 hash.",
        elapsed.as_secs_f64()
    );

    // Clean disconnect
    let _ = framed
        .send_frame(&Frame::Control(ControlFrame::Disconnect {
            reason: DisconnectReason::Graceful,
        }))
        .await;

    Ok(())
}

/// Executes an explicit SAS pairing ceremony with a remote peer node.
pub async fn run_pair(peer_addr: SocketAddr, device_name: &str, data_dir: &Path) -> Result<()> {
    tokio::fs::create_dir_all(data_dir).await?;
    let key_path = data_dir.join("identity").join("secret.key");
    let key = Arc::new(IdentityStorage::load_or_generate(&key_path)?);
    let trust_store = Arc::new(TrustStore::open(data_dir.join("trust.db"))?);

    println!("Connecting to {peer_addr} to initiate pairing ceremony...");
    let socket = TcpStream::connect(peer_addr)
        .await
        .with_context(|| format!("Failed to connect to {peer_addr}"))?;

    let mut session = ActiveSession::client_handshake(
        socket,
        key,
        device_name.to_string(),
        DeviceType::Windows,
        trust_store,
        None,
        None,
    )
    .await
    .context("Cryptographic authentication handshake failed")?;

    println!(
        "Authenticated with '{}' ({}). Negotiating SAS verification PIN...",
        session.remote_name, session.remote_node_id
    );

    let sas = session
        .execute_pairing(true, &InteractiveCliConfirm)
        .await
        .context("Pairing ceremony failed or was rejected")?;

    println!("\n========================================================");
    println!("  PAIRING SUCCESSFUL!");
    println!("--------------------------------------------------------");
    println!("  Device Name:       {}", session.remote_name);
    println!("  Node ID:           {}", session.remote_node_id);
    println!(
        "  Verified SAS PIN:  \x1b[1;32m{}\x1b[0m",
        sas.formatted_pin
    );
    println!(
        "  Trust Record:      {}",
        data_dir.join("trust.db").display()
    );
    println!("========================================================\n");

    Ok(())
}

/// Lists all trusted peers recorded in the local trust store.
pub fn run_trust_list(data_dir: &Path) -> Result<()> {
    let trust_path = data_dir.join("trust.db");
    if !trust_path.exists() {
        println!("No trust database found at {}", trust_path.display());
        return Ok(());
    }

    let trust_store = TrustStore::open(&trust_path)?;
    let peers = trust_store.list_peers()?;

    if peers.is_empty() {
        println!("No trusted peers recorded in {}", trust_path.display());
    } else {
        println!(
            "\nTrusted Devices ({}) in {}:",
            peers.len(),
            trust_path.display()
        );
        println!("{:-<75}", "");
        for (i, p) in peers.iter().enumerate() {
            println!(
                "[{}] '{}' ({:?}) - Trust State: {:?}",
                i + 1,
                p.device_name,
                p.device_type,
                p.trust_state
            );
            println!("    Node ID:      {}", p.node_id);
            println!("    Public Key:   {}", hex::encode(p.public_key.to_bytes()));
            println!("    First Paired: unix:{}", p.first_paired_at);
            println!("    Last Seen:    unix:{}", p.last_seen_at);
        }
        println!("{:-<75}\n", "");
    }

    Ok(())
}

/// Revokes trust for a specific peer NodeId in the local trust store.
pub fn run_trust_revoke(data_dir: &Path, node_id_hex: &str) -> Result<()> {
    let trust_path = data_dir.join("trust.db");
    let trust_store = TrustStore::open(&trust_path)?;
    let node_id = NodeId::from_hex(node_id_hex)
        .with_context(|| format!("Invalid hexadecimal NodeId '{node_id_hex}'"))?;

    trust_store.revoke_peer(&node_id)?;
    println!("Successfully revoked trust for peer {node_id}");
    Ok(())
}

/// Discovers peers on LAN and displays their trust authorization status.
pub async fn run_peers(duration_secs: u64, broadcast_port: u16, data_dir: &Path) -> Result<()> {
    use bridge_discovery::{
        DiscoveryConfig, UdpConfig, UnifiedDiscovery, UnifiedDiscoveryConfig, UnifiedDiscoveryMode,
    };

    let trust_path = data_dir.join("trust.db");
    let trust_store = if trust_path.exists() {
        Some(TrustStore::open(&trust_path)?)
    } else {
        None
    };

    println!("Scanning local LAN for BridgeOS peers for {duration_secs} seconds...",);

    let udp_cfg = UdpConfig {
        broadcast_port,
        bind_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), broadcast_port),
        broadcast_targets: vec![SocketAddr::new(
            IpAddr::V4(Ipv4Addr::BROADCAST),
            broadcast_port,
        )],
        ..Default::default()
    };

    let unified_cfg = UnifiedDiscoveryConfig {
        mode: UnifiedDiscoveryMode::Both,
        mdns: DiscoveryConfig::default(),
        udp: udp_cfg,
    };

    let mut discovery = UnifiedDiscovery::new(unified_cfg)?;
    discovery.start_discovery()?;

    tokio::time::sleep(std::time::Duration::from_secs(duration_secs)).await;

    let peers = discovery.directory().list();
    if peers.is_empty() {
        println!("No BridgeOS peers discovered on the local network.");
    } else {
        println!("\nFound {} BridgeOS peer(s):", peers.len());
        for (i, peer) in peers.iter().enumerate() {
            let trust_status = if let Some(ref ts) = trust_store {
                match ts.get_peer(&peer.node_id) {
                    Ok(Some(p)) => format!("[{:?}]", p.trust_state),
                    Ok(None) => "[UNTRUSTED / UNPAIRED]".to_string(),
                    Err(_) => "[UNKNOWN]".to_string(),
                }
            } else {
                "[UNTRUSTED / NO TRUST DB]".to_string()
            };

            println!(
                "\n[{}] Name:         {}  \x1b[1;33m{}\x1b[0m",
                i + 1,
                peer.device_name,
                trust_status
            );
            println!("    Node ID:      {}", peer.node_id);
            println!("    Type:         {:?}", peer.device_type);
            println!("    Addresses:    {:?}", peer.addresses);
            println!("    Capabilities: {:?}", peer.capabilities);
        }
        println!();
    }

    discovery.shutdown().await?;
    Ok(())
}
