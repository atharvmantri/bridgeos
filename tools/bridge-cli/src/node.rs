use anyhow::{bail, Context, Result};
use bridge_clipboard::{
    ClipboardPolicy, ClipboardSyncEngine, ClipboardSyncEvent, MemoryClipboardBackend,
};
use bridge_core::{Capabilities, DeviceType, NodeId};
use bridge_discovery::{
    DiscoveryConfig, DiscoveryEvent, UdpConfig, UnifiedDiscovery, UnifiedDiscoveryConfig,
    UnifiedDiscoveryMode,
};
use bridge_identity::{IdentityStorage, TrustStore};
use bridge_protocol::{ControlFrame, DataFrame, Frame};
use bridge_session::{ActiveSession, InteractiveCliConfirm};
use bridge_transfer::{FileReceiver, TransferMessage};
use bridge_transport::FramedStream;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, error, info, warn};

/// Configuration parameters for running an active BridgeOS node.
#[derive(Debug, Clone)]
pub struct NodeConfig {
    pub name: String,
    pub port: u16,
    pub device_type: DeviceType,
    pub receive_dir: PathBuf,
    pub data_dir: PathBuf,
    pub broadcast_port: u16,
    pub enable_mdns: bool,
    pub enable_udp: bool,
    pub memory_clipboard: bool,
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

    tokio::fs::create_dir_all(&config.data_dir)
        .await
        .with_context(|| {
            format!(
                "Failed to create data directory: {}",
                config.data_dir.display()
            )
        })?;

    // 1. Persistent Node Identity & Trust Store
    let key_path = config.data_dir.join("identity").join("secret.key");
    let identity = Arc::new(
        IdentityStorage::load_or_generate(&key_path)
            .context("Failed to initialize node identity")?,
    );
    let node_id = identity.node_id();

    let trust_store = Arc::new(
        TrustStore::open(config.data_dir.join("trust.db"))
            .context("Failed to open persistent trust store")?,
    );

    // 2. Clipboard Synchronization Engine
    let (clip_backend, backend_name): (Arc<dyn bridge_clipboard::ClipboardBackend>, &'static str) =
        if config.memory_clipboard {
            (Arc::new(MemoryClipboardBackend::new()), "In-Memory")
        } else {
            #[cfg(windows)]
            {
                match bridge_clipboard::WindowsClipboardBackend::new() {
                    Ok(b) => {
                        info!("Using native Windows event-driven clipboard backend");
                        (Arc::new(b), "Native OS (Windows Win32 Event-Driven)")
                    }
                    Err(e) => {
                        warn!(
                            "Failed to initialize native Windows clipboard ({e}); falling back to in-memory clipboard"
                        );
                        (
                            Arc::new(MemoryClipboardBackend::new()),
                            "In-Memory (Windows Init Fallback)",
                        )
                    }
                }
            }
            #[cfg(not(windows))]
            {
                (
                    Arc::new(MemoryClipboardBackend::new()),
                    "In-Memory (Non-Windows Platform Fallback)",
                )
            }
        };

    let clip_engine = Arc::new(ClipboardSyncEngine::new(
        node_id,
        clip_backend.clone(),
        ClipboardPolicy::default(),
    ));
    let _clip_monitor = clip_engine.start_monitor();

    // Registry of active trusted peer channels for outbound clipboard broadcasting
    let active_sessions = Arc::new(Mutex::new(HashMap::<NodeId, mpsc::Sender<DataFrame>>::new()));

    // Spawn outbound clipboard broadcaster
    let active_sessions_clip = active_sessions.clone();
    let mut clip_event_rx = clip_engine.subscribe();
    let _clip_broadcast_task = tokio::spawn(async move {
        while let Ok(event) = clip_event_rx.recv().await {
            if let ClipboardSyncEvent::OutgoingBroadcastReady { frame, .. } = event {
                let sessions = active_sessions_clip.lock().await;
                for (peer_id, tx) in sessions.iter() {
                    debug!(%peer_id, "Broadcasting clipboard update to active trusted peer");
                    let _ = tx.send(frame.clone()).await;
                }
            }
        }
    });

    // 3. Establish TCP Listener
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
    println!("  Clipboard:     {backend_name}");
    println!("  Data Dir:      {}", config.data_dir.display());
    println!("  Receive Dir:   {}", config.receive_dir.display());
    println!("  mDNS Enabled:  {}", config.enable_mdns);
    println!(
        "  UDP Broadcast: {} (Port {})",
        config.enable_udp, config.broadcast_port
    );
    println!("============================================================");
    println!("Press Ctrl+C to shut down gracefully.\n");

    // 4. Configure Unified Discovery
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

    // 5. Spawn Discovery Event Logger
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

    // 6. Accept Connections Loop
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
                        let trust_store_clone = trust_store.clone();
                        let clip_engine_clone = clip_engine.clone();
                        let active_sessions_clone = active_sessions.clone();

                        tokio::spawn(async move {
                            if let Err(e) = handle_connection(
                                socket,
                                remote_addr,
                                identity_clone,
                                receive_dir_clone,
                                server_name_clone,
                                trust_store_clone,
                                clip_engine_clone,
                                active_sessions_clone,
                            ).await {
                                warn!(remote = %remote_addr, error = %e, "Connection closed with error");
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

/// Handles an incoming authenticated peer connection using `ActiveSession`.
#[allow(clippy::too_many_arguments, clippy::implicit_hasher)]
pub async fn handle_connection(
    socket: TcpStream,
    remote_addr: SocketAddr,
    identity: Arc<bridge_identity::IdentityKey>,
    receive_dir: Arc<PathBuf>,
    server_name: Arc<String>,
    trust_store: Arc<TrustStore>,
    clip_engine: Arc<ClipboardSyncEngine>,
    active_sessions: Arc<Mutex<HashMap<NodeId, mpsc::Sender<DataFrame>>>>,
) -> Result<()> {
    let mut session = ActiveSession::server_handshake(
        socket,
        identity,
        server_name.to_string(),
        DeviceType::Windows,
        trust_store,
        Some(clip_engine.clone()),
        Some((*receive_dir).clone()),
    )
    .await
    .context("Handshake failed")?;

    let (outbound_tx, mut outbound_rx) = mpsc::channel::<DataFrame>(32);

    if session.state.is_trusted() {
        println!(
            "\n[Session] Connected with \x1b[1;32mTRUSTED\x1b[0m peer '{}' ({}) from {}",
            session.remote_name, session.remote_node_id, remote_addr
        );
        active_sessions
            .lock()
            .await
            .insert(session.remote_node_id, outbound_tx.clone());
    } else {
        println!(
            "\n[Session] Connected with \x1b[1;33mUNTRUSTED\x1b[0m peer '{}' ({}) from {}. Application channels blocked pending pairing.",
            session.remote_name, session.remote_node_id, remote_addr
        );
    }

    loop {
        tokio::select! {
            frame_res = session.framed.recv_frame() => {
                let frame_opt = frame_res.context("Failed to read frame")?;
                let Some(frame) = frame_opt else {
                    println!("[Session] Peer '{}' disconnected (EOF)", session.remote_name);
                    break;
                };

                match frame {
                    Frame::Control(ControlFrame::Ping { nonce }) => {
                        debug!(nonce, "Received Ping from peer");
                        session.framed.send_frame(&Frame::Control(ControlFrame::Pong { nonce })).await?;
                    }
                    Frame::Control(ControlFrame::Pong { .. }) => {}
                    Frame::Control(ControlFrame::Disconnect { reason }) => {
                        println!("[Session] Peer '{}' disconnected gracefully: {reason:?}", session.remote_name);
                        break;
                    }
                    Frame::Data(df) => {
                        match df.channel {
                            DataFrame::CHANNEL_PAIRING => {
                                println!(
                                    "\n[Pairing] Incoming pairing ceremony request from '{}' ({})",
                                    session.remote_name, session.remote_node_id
                                );
                                match session.execute_pairing(false, &InteractiveCliConfirm).await {
                                    Ok(sas) => {
                                        println!(
                                            "\n[Pairing] SUCCESS! Device '{}' is now trusted (PIN: {}).\n",
                                            session.remote_name, sas.formatted_pin
                                        );
                                        active_sessions
                                            .lock()
                                            .await
                                            .insert(session.remote_node_id, outbound_tx.clone());
                                    }
                                    Err(e) => {
                                        warn!("Pairing ceremony failed or was rejected: {e}");
                                    }
                                }
                            }
                            DataFrame::CHANNEL_CLIPBOARD => {
                                if !session.state.is_trusted() {
                                    warn!(
                                        peer = %session.remote_name,
                                        "BLOCKED: Rejected clipboard update from UNTRUSTED peer"
                                    );
                                } else if let Err(e) = clip_engine.handle_incoming_frame(&df) {
                                    warn!(error = %e, "Failed to apply incoming clipboard frame");
                                }
                            }
                            DataFrame::CHANNEL_FILE_TRANSFER => {
                                if session.state.is_trusted() {
                                    handle_file_transfer(&df.payload, &mut session.framed, &receive_dir, &session.remote_name).await?;
                                } else {
                                    warn!(
                                        peer = %session.remote_name,
                                        "BLOCKED: Rejected file transfer from UNTRUSTED peer"
                                    );
                                }
                            }
                            other => {
                                debug!(channel = other, "Received data on unhandled channel");
                            }
                        }
                    }
                    Frame::Handshake(h) => {
                        warn!(handshake = ?h, "Unexpected post-handshake message received");
                    }
                }
            }

            Some(outbound_df) = outbound_rx.recv() => {
                if session.state.is_trusted() {
                    let _ = session.framed.send_frame(&Frame::Data(outbound_df)).await;
                }
            }
        }
    }

    active_sessions.lock().await.remove(&session.remote_node_id);
    Ok(())
}

/// Handles incoming file transfer messages and coordinates chunk streaming.
async fn handle_file_transfer<T: AsyncRead + AsyncWrite + Unpin>(
    payload: &[u8],
    framed: &mut FramedStream<T>,
    receive_dir: &std::path::Path,
    client_name: &str,
) -> Result<()> {
    let msg = TransferMessage::from_bytes(payload)
        .context("Failed to deserialize TransferMessage payload")?;

    match msg {
        TransferMessage::Offer(manifest) => {
            let file_name = manifest.filename.clone();
            let file_size = manifest.total_size;
            let total_chunks = manifest.total_chunks;
            let file_id = manifest.file_id.clone();

            println!(
                "\n[Transfer] Incoming file offer from '{client_name}': '{file_name}' ({file_size} bytes, {total_chunks} chunks)"
            );

            let mut receiver = FileReceiver::to_dir(receive_dir, manifest.clone())
                .await
                .context("Failed to initialize file receiver")?;
            let start_chunk = receiver.next_expected_chunk();
            let accept = TransferMessage::Accept {
                file_id: file_id.clone(),
                start_chunk,
            };
            framed
                .send_frame(&Frame::Data(accept.to_data_frame()?))
                .await
                .context("Failed to send Accept frame")?;

            info!("Sent Accept frame to peer");

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
        _ => {
            debug!("Processed intermediate file transfer message");
        }
    }

    Ok(())
}
