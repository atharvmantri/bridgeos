use anyhow::{bail, Context, Result};
use bridge_core::{Capabilities, DeviceType, ProtocolVersion};
use bridge_discovery::{
    DiscoveryConfig, DiscoveryEvent, UdpConfig, UnifiedDiscovery, UnifiedDiscoveryConfig,
    UnifiedDiscoveryMode,
};
use bridge_identity::{IdentityKey, PublicKey};
use bridge_protocol::{AuthResult, ControlFrame, DataFrame, Frame, HandshakeFrame, ServerHello};
use bridge_transfer::{FileReceiver, TransferMessage};
use bridge_transport::FramedStream;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, error, warn};

/// Configuration parameters for running an active BridgeOS node.
#[derive(Debug, Clone)]
pub struct NodeConfig {
    pub name: String,
    pub port: u16,
    pub device_type: DeviceType,
    pub receive_dir: PathBuf,
    pub broadcast_port: u16,
    pub enable_mdns: bool,
    pub enable_udp: bool,
}

/// Runs a persistent, interactive BridgeOS peer node.
pub async fn run_node(config: NodeConfig) -> Result<()> {
    tokio::fs::create_dir_all(&config.receive_dir)
        .await
        .with_context(|| {
            format!(
                "Failed to create receive directory: {}",
                config.receive_dir.display()
            )
        })?;

    let identity = Arc::new(IdentityKey::generate());
    let node_id = identity.node_id();

    // 1. Establish TCP Listener
    let bind_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), config.port);
    let listener = TcpListener::bind(bind_addr)
        .await
        .with_context(|| format!("Failed to bind TCP listener on {bind_addr}"))?;
    let actual_port = listener.local_addr()?.port();

    println!("============================================================");
    println!("  BridgeOS Peer Node Running");
    println!("------------------------------------------------------------");
    println!("  Device Name:   {}", config.name);
    println!("  Node ID:       {node_id}");
    println!("  Device Type:   {:?}", config.device_type);
    println!("  TCP Listener:  0.0.0.0:{actual_port}");
    println!("  Receive Dir:   {}", config.receive_dir.display());
    println!("  mDNS Enabled:  {}", config.enable_mdns);
    println!(
        "  UDP Broadcast: {} (Port {})",
        config.enable_udp, config.broadcast_port
    );
    println!("============================================================");
    println!("Press Ctrl+C to shut down gracefully.\n");

    // 2. Configure Unified Discovery
    let mode = match (config.enable_mdns, config.enable_udp) {
        (true, true) => UnifiedDiscoveryMode::Both,
        (true, false) => UnifiedDiscoveryMode::MdnsOnly,
        (false, true) => UnifiedDiscoveryMode::UdpOnly,
        (false, false) => bail!("At least one discovery mechanism (mDNS or UDP) must be enabled"),
    };

    let udp_cfg = UdpConfig {
        broadcast_port: config.broadcast_port,
        bind_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), config.broadcast_port),
        broadcast_targets: vec![SocketAddr::new(
            IpAddr::V4(Ipv4Addr::BROADCAST),
            config.broadcast_port,
        )],
        ..Default::default()
    };

    let unified_cfg = UnifiedDiscoveryConfig {
        mode,
        mdns: DiscoveryConfig::default(),
        udp: udp_cfg,
    };

    let mut discovery =
        UnifiedDiscovery::new(unified_cfg).context("Failed to initialize UnifiedDiscovery")?;

    discovery
        .advertise(
            node_id,
            &config.name,
            config.device_type,
            actual_port,
            Capabilities::all(),
            None,
        )
        .await
        .context("Failed to advertise node service")?;

    discovery
        .start_discovery()
        .context("Failed to start LAN peer discovery")?;

    let mut event_rx = discovery.subscribe();

    // 3. Spawn Discovery Event Logger
    let disc_task = tokio::spawn(async move {
        while let Ok(event) = event_rx.recv().await {
            match event {
                DiscoveryEvent::PeerDiscovered(peer) => {
                    println!(
                        "\n[LAN Discovery] Discovered Peer: '{}' ({})",
                        peer.device_name, peer.node_id
                    );
                    println!("  Type: {:?}", peer.device_type);
                    println!("  Addresses: {:?}", peer.addresses);
                    println!("  Capabilities: {:?}", peer.capabilities);
                }
                DiscoveryEvent::PeerUpdated(peer) => {
                    debug!(node_id = %peer.node_id, "Peer updated metadata/addresses");
                }
                DiscoveryEvent::PeerLost(id) => {
                    println!("\n[LAN Discovery] Peer Departed (Goodbye): {id}");
                }
                DiscoveryEvent::PeerExpired(id) => {
                    println!("\n[LAN Discovery] Peer Expired (Heartbeat TTL): {id}");
                }
            }
        }
    });

    // 4. Accept Connections Loop
    let receive_dir = Arc::new(config.receive_dir.clone());
    let server_name = Arc::new(config.name.clone());

    loop {
        tokio::select! {
            accept_res = listener.accept() => {
                match accept_res {
                    Ok((socket, remote_addr)) => {
                        let identity_clone = identity.clone();
                        let receive_dir_clone = receive_dir.clone();
                        let server_name_clone = server_name.clone();

                        tokio::spawn(async move {
                            if let Err(e) = handle_connection(
                                socket,
                                remote_addr,
                                identity_clone,
                                receive_dir_clone,
                                server_name_clone,
                            ).await {
                                warn!(remote = %remote_addr, error = %e, "Connection ended with error");
                            }
                        });
                    }
                    Err(e) => {
                        error!(error = %e, "TCP accept failed");
                    }
                }
            }

            _ = tokio::signal::ctrl_c() => {
                println!("\nReceived shutdown signal. Stopping BridgeOS node...");
                break;
            }
        }
    }

    disc_task.abort();
    discovery
        .shutdown()
        .await
        .context("Failed to shut down discovery")?;
    println!("Node shut down successfully.");
    Ok(())
}

/// Handles an incoming authenticated peer connection.
pub async fn handle_connection(
    socket: TcpStream,
    remote_addr: SocketAddr,
    identity: Arc<IdentityKey>,
    receive_dir: Arc<PathBuf>,
    server_name: Arc<String>,
) -> Result<()> {
    let mut framed = FramedStream::new(socket);

    // 1. Await ClientHello
    let hello_frame = framed
        .recv_frame()
        .await
        .context("Failed to read ClientHello")?
        .context("Client disconnected before sending ClientHello")?;

    let client_hello = match hello_frame {
        Frame::Handshake(HandshakeFrame::ClientHello(h)) => h,
        other => bail!("Expected ClientHello from {remote_addr}, got {other:?}"),
    };

    let client_node_id = client_hello.node_id;
    let client_name = client_hello.device_name.clone();

    // 2. Reconstruct Client's PublicKey from NodeId
    let client_pub = PublicKey::from_bytes(&client_node_id.0)
        .context("Invalid Ed25519 public key in ClientHello NodeId")?;

    // 3. Send ServerHello
    let server_nonce = rand::random::<[u8; 32]>();
    let negotiated_caps = client_hello.capabilities.intersect(&Capabilities::all());
    let server_hello = ServerHello {
        agreed_version: ProtocolVersion::CURRENT,
        node_id: identity.node_id(),
        device_name: (*server_name).clone(),
        server_nonce,
        negotiated_capabilities: negotiated_caps,
    };

    framed
        .send_frame(&Frame::Handshake(HandshakeFrame::ServerHello(server_hello)))
        .await
        .context("Failed to send ServerHello")?;

    // 4. Await AuthResponse
    let auth_frame = framed
        .recv_frame()
        .await
        .context("Failed to read AuthResponse")?
        .context("Client disconnected before sending AuthResponse")?;

    let auth_resp = match auth_frame {
        Frame::Handshake(HandshakeFrame::AuthResponse(r)) => r,
        other => bail!("Expected AuthResponse, got {other:?}"),
    };

    // 5. Verify Challenge Signature
    if let Err(e) = client_pub.verify_handshake_challenge(
        &client_hello.client_nonce,
        &server_nonce,
        &auth_resp.signature,
    ) {
        let _ = framed
            .send_frame(&Frame::Handshake(HandshakeFrame::AuthResult(
                AuthResult::failed(format!(
                    "Cryptographic challenge signature verification failed: {e}"
                )),
            )))
            .await;
        bail!("Client {client_name} ({client_node_id}) signature verification failed: {e}");
    }

    // 6. Send AuthResult::ok()
    framed
        .send_frame(&Frame::Handshake(HandshakeFrame::AuthResult(
            AuthResult::ok(),
        )))
        .await
        .context("Failed to send AuthResult")?;

    println!(
        "[Session] Authenticated connection established with '{client_name}' ({client_node_id}) from {remote_addr}"
    );

    // 7. Process session frames
    loop {
        let frame_opt = framed.recv_frame().await.context("Failed to read frame")?;
        let Some(frame) = frame_opt else {
            println!("[Session] Peer '{client_name}' disconnected (EOF)");
            break;
        };

        match frame {
            Frame::Control(ControlFrame::Ping { nonce }) => {
                debug!(nonce, "Received Ping from client");
                framed
                    .send_frame(&Frame::Control(ControlFrame::Pong { nonce }))
                    .await
                    .context("Failed to send Pong")?;
            }
            Frame::Control(ControlFrame::Pong { .. }) => {}
            Frame::Control(ControlFrame::Disconnect { reason }) => {
                println!("[Session] Peer '{client_name}' disconnected gracefully: {reason:?}");
                break;
            }
            Frame::Data(DataFrame { channel, payload }) => {
                if channel == DataFrame::CHANNEL_FILE_TRANSFER {
                    handle_file_transfer(&payload, &mut framed, &receive_dir, &client_name).await?;
                } else {
                    debug!(channel, "Received data on non-file channel");
                }
            }
            Frame::Handshake(h) => {
                warn!(handshake = ?h, "Unexpected post-handshake message received");
            }
        }
    }

    Ok(())
}

/// Manages incoming file transfer reception using the `bridge-transfer` engine.
async fn handle_file_transfer(
    payload: &[u8],
    framed: &mut FramedStream<TcpStream>,
    receive_dir: &PathBuf,
    client_name: &str,
) -> Result<()> {
    let msg: TransferMessage =
        TransferMessage::from_bytes(payload).context("Failed to deserialize TransferMessage")?;

    match msg {
        TransferMessage::Offer(manifest) => {
            let file_name = manifest.filename.clone();
            let file_size = manifest.total_size;
            let total_chunks = manifest.total_chunks;
            let file_id = manifest.file_id.clone();

            println!(
                "\n[File Transfer] Incoming offer from '{client_name}': '{file_name}' ({file_size} bytes, {total_chunks} chunks)"
            );

            let mut receiver = FileReceiver::to_dir(receive_dir, manifest.clone())
                .await
                .context("Failed to initialize FileReceiver")?;

            let start_chunk = receiver.next_expected_chunk();
            if start_chunk > 0 {
                println!("[File Transfer] Resuming from chunk {start_chunk}/{total_chunks}");
            }

            // Send Accept
            let accept = TransferMessage::Accept {
                file_id: file_id.clone(),
                start_chunk,
            };
            framed
                .send_frame(&Frame::Data(accept.to_data_frame()?))
                .await
                .context("Failed to send TransferMessage::Accept")?;

            // Receive file chunks
            loop {
                let frame = framed
                    .recv_frame()
                    .await
                    .context("Failed to receive chunk frame")?
                    .context("Sender disconnected during file transfer")?;

                let chunk_msg = match frame {
                    Frame::Data(df) => TransferMessage::from_data_frame(&df)?,
                    Frame::Control(ControlFrame::Disconnect { .. }) => {
                        bail!("Sender aborted transfer with Disconnect signal");
                    }
                    other => bail!("Expected Data frame for file transfer, received {other:?}"),
                };

                match chunk_msg {
                    TransferMessage::Data(chunk) => {
                        let idx = chunk.chunk_index;
                        receiver
                            .receive_chunk(&chunk)
                            .await
                            .context("Failed to write received file chunk")?;

                        let ack = TransferMessage::Ack {
                            file_id: file_id.clone(),
                            chunk_index: idx,
                        };
                        framed
                            .send_frame(&Frame::Data(ack.to_data_frame()?))
                            .await
                            .context("Failed to send chunk Ack")?;
                    }
                    TransferMessage::Finished {
                        file_id: fin_id,
                        blake3_root_hash,
                    } => {
                        if fin_id != file_id {
                            bail!("Finished file ID mismatch");
                        }
                        if blake3_root_hash != manifest.blake3_root_hash {
                            bail!(
                                "Finished message root hash mismatch with initial offer manifest"
                            );
                        }

                        let final_path = receiver
                            .finalize_file()
                            .await
                            .context("Blake3 root hash verification failed for received file")?;

                        let complete = TransferMessage::Complete {
                            file_id: file_id.clone(),
                        };
                        framed
                            .send_frame(&Frame::Data(complete.to_data_frame()?))
                            .await
                            .context("Failed to send TransferMessage::Complete")?;

                        println!(
                            "[File Transfer] SUCCESS: '{}' received and verified -> {}",
                            file_name,
                            final_path.display()
                        );
                        break;
                    }
                    TransferMessage::Cancel { reason, .. } => {
                        bail!("Transfer cancelled by sender: {reason}");
                    }
                    other => bail!("Unexpected message during chunk streaming: {other:?}"),
                }
            }
        }
        other => {
            debug!("Ignoring non-offer transfer message: {other:?}");
        }
    }

    Ok(())
}
