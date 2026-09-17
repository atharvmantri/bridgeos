use anyhow::{bail, Context, Result};
use bridge_core::{Capabilities, ProtocolVersion};
use bridge_identity::IdentityKey;
use bridge_protocol::{
    AuthResponse, AuthResult, ClientHello, ControlFrame, DisconnectReason, Frame, HandshakeFrame,
    ServerHello,
};
use bridge_transfer::{create_manifest_from_file, FileSender, TransferMessage};
use bridge_transport::FramedStream;
use std::net::SocketAddr;
use std::path::Path;
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

    let client_key = IdentityKey::generate();
    let (mut framed, server_hello) =
        connect_and_handshake(peer_addr, &client_key, device_name).await?;

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
