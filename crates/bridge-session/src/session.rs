use crate::error::SessionError;
use crate::pairing_flow::{PairingConfirmation, PairingMessage};
use crate::state::SessionState;
use bridge_clipboard::ClipboardSyncEngine;
use bridge_core::{Capabilities, DeviceType, NodeId, ProtocolVersion};
use bridge_identity::{
    IdentityError, IdentityKey, PairingSession, PublicKey, SasVerification, TrustStore,
};
use bridge_protocol::{
    AuthResponse, AuthResult, ClientHello, ControlFrame, DataFrame, DisconnectReason, Frame,
    HandshakeFrame, ServerHello,
};
use bridge_transfer::{FileReceiver, TransferMessage};
use bridge_transport::FramedStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncWrite};
use tracing::{debug, error, info, warn};

fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Orchestrates an authenticated, trust-gated peer session.
#[derive(Debug)]
pub struct ActiveSession<T> {
    pub local_identity: Arc<IdentityKey>,
    pub local_name: String,
    pub local_device_type: DeviceType,
    pub remote_node_id: NodeId,
    pub remote_pubkey: PublicKey,
    pub remote_name: String,
    pub remote_device_type: DeviceType,
    pub state: SessionState,
    pub trust_store: Arc<TrustStore>,
    pub framed: FramedStream<T>,
    pub clipboard_engine: Option<Arc<ClipboardSyncEngine>>,
    pub receive_dir: Option<PathBuf>,
}

impl<T: AsyncRead + AsyncWrite + Unpin + Send> ActiveSession<T> {
    /// Perform the server-side handshake on an incoming connection and consult `TrustStore`.
    pub async fn server_handshake(
        stream: T,
        local_identity: Arc<IdentityKey>,
        local_name: String,
        local_device_type: DeviceType,
        trust_store: Arc<TrustStore>,
        clipboard_engine: Option<Arc<ClipboardSyncEngine>>,
        receive_dir: Option<PathBuf>,
    ) -> Result<Self, SessionError> {
        let mut framed = FramedStream::new(stream);

        // 1. Await ClientHello
        let hello_frame = framed
            .recv_frame()
            .await
            .map_err(|e| SessionError::Transport(e.to_string()))?
            .ok_or_else(|| SessionError::Closed("Peer disconnected before ClientHello".into()))?;

        let client_hello = match hello_frame {
            Frame::Handshake(HandshakeFrame::ClientHello(h)) => h,
            other => {
                return Err(SessionError::Protocol(format!(
                    "Expected ClientHello, got {other:?}"
                )))
            }
        };

        let remote_node_id = client_hello.node_id;
        let remote_name = client_hello.device_name;
        let remote_device_type = DeviceType::Unknown;

        // 2. Reconstruct Client's PublicKey from NodeId
        let remote_pubkey = PublicKey::from_bytes(&remote_node_id.0).map_err(|e| {
            SessionError::AuthenticationFailed(format!("Invalid Ed25519 public key: {e}"))
        })?;

        // 3. Send ServerHello
        let server_nonce = rand::random::<[u8; 32]>();
        let negotiated_caps = client_hello.capabilities.intersect(&Capabilities::all());
        let server_hello = ServerHello {
            agreed_version: ProtocolVersion::CURRENT,
            node_id: local_identity.node_id(),
            device_name: local_name.clone(),
            server_nonce,
            negotiated_capabilities: negotiated_caps,
        };

        framed
            .send_frame(&Frame::Handshake(HandshakeFrame::ServerHello(server_hello)))
            .await
            .map_err(|e| SessionError::Transport(e.to_string()))?;

        // 4. Await AuthResponse
        let auth_frame = framed
            .recv_frame()
            .await
            .map_err(|e| SessionError::Transport(e.to_string()))?
            .ok_or_else(|| SessionError::Closed("Peer disconnected before AuthResponse".into()))?;

        let auth_resp = match auth_frame {
            Frame::Handshake(HandshakeFrame::AuthResponse(r)) => r,
            other => {
                return Err(SessionError::Protocol(format!(
                    "Expected AuthResponse, got {other:?}"
                )))
            }
        };

        // 5. Verify Challenge Signature
        if let Err(e) = remote_pubkey.verify_handshake_challenge(
            &client_hello.client_nonce,
            &server_nonce,
            &auth_resp.signature,
        ) {
            let _ = framed
                .send_frame(&Frame::Handshake(HandshakeFrame::AuthResult(
                    AuthResult::failed(format!("Cryptographic challenge failed: {e}")),
                )))
                .await;
            return Err(SessionError::AuthenticationFailed(format!(
                "Signature verification failed for peer {remote_name} ({remote_node_id}): {e}"
            )));
        }

        // 6. Send AuthResult::ok()
        framed
            .send_frame(&Frame::Handshake(HandshakeFrame::AuthResult(
                AuthResult::ok(),
            )))
            .await
            .map_err(|e| SessionError::Transport(e.to_string()))?;

        // 7. Consult TrustStore for security boundary
        let initial_state = match trust_store.is_trusted(&remote_node_id, &remote_pubkey) {
            Ok(true) => {
                let _ = trust_store.update_last_seen(&remote_node_id, current_timestamp());
                info!(
                    peer = %remote_name,
                    node_id = %remote_node_id,
                    "Peer is fully trusted in TrustStore"
                );
                SessionState::Trusted
            }
            Ok(false) => {
                info!(
                    peer = %remote_name,
                    node_id = %remote_node_id,
                    "Peer identity authenticated but UNTRUSTED (pairing required)"
                );
                SessionState::AuthenticatedUntrusted
            }
            Err(IdentityError::KeyMismatch {
                node_id,
                expected,
                presented,
            }) => {
                error!(
                    node_id = %node_id,
                    expected = %expected,
                    presented = %presented,
                    "CRITICAL SECURITY ALERT: Rejecting connection - key mismatch / impersonation attempt!"
                );

                let _ = framed
                    .send_frame(&Frame::Control(ControlFrame::Disconnect {
                        reason: DisconnectReason::AuthenticationFailed,
                    }))
                    .await;

                return Err(SessionError::KeyMismatch {
                    node_id: node_id.to_string(),
                    expected,
                    actual: presented,
                });
            }
            Err(IdentityError::PeerRevoked(id)) => {
                warn!(
                    node_id = %id,
                    "Rejecting connection from revoked peer"
                );
                let _ = framed
                    .send_frame(&Frame::Control(ControlFrame::Disconnect {
                        reason: DisconnectReason::AuthenticationFailed,
                    }))
                    .await;
                return Err(SessionError::PeerRevoked(id.to_string()));
            }
            Err(e) => return Err(SessionError::Identity(e)),
        };

        Ok(Self {
            local_identity,
            local_name,
            local_device_type,
            remote_node_id,
            remote_pubkey,
            remote_name,
            remote_device_type,
            state: initial_state,
            trust_store,
            framed,
            clipboard_engine,
            receive_dir,
        })
    }

    /// Perform the client-side handshake to an outgoing connection and consult `TrustStore`.
    pub async fn client_handshake(
        stream: T,
        local_identity: Arc<IdentityKey>,
        local_name: String,
        _local_device_type: DeviceType,
        trust_store: Arc<TrustStore>,
        clipboard_engine: Option<Arc<ClipboardSyncEngine>>,
        receive_dir: Option<PathBuf>,
    ) -> Result<Self, SessionError> {
        let mut framed = FramedStream::new(stream);

        // 1. Send ClientHello
        let client_nonce = rand::random::<[u8; 32]>();
        let client_hello = ClientHello {
            version: ProtocolVersion::CURRENT,
            node_id: local_identity.node_id(),
            device_name: local_name.clone(),
            client_nonce,
            capabilities: Capabilities::all(),
        };

        framed
            .send_frame(&Frame::Handshake(HandshakeFrame::ClientHello(client_hello)))
            .await
            .map_err(|e| SessionError::Transport(e.to_string()))?;

        // 2. Await ServerHello
        let hello_frame = framed
            .recv_frame()
            .await
            .map_err(|e| SessionError::Transport(e.to_string()))?
            .ok_or_else(|| SessionError::Closed("Server disconnected before ServerHello".into()))?;

        let server_hello = match hello_frame {
            Frame::Handshake(HandshakeFrame::ServerHello(h)) => h,
            other => {
                return Err(SessionError::Protocol(format!(
                    "Expected ServerHello, got {other:?}"
                )))
            }
        };

        let remote_node_id = server_hello.node_id;
        let remote_name = server_hello.device_name;
        let remote_device_type = DeviceType::Unknown;

        // 3. Reconstruct Server's PublicKey from NodeId
        let remote_pubkey = PublicKey::from_bytes(&remote_node_id.0).map_err(|e| {
            SessionError::AuthenticationFailed(format!("Invalid Ed25519 public key: {e}"))
        })?;

        // 4. Sign Challenge and send AuthResponse
        let signature =
            local_identity.sign_handshake_challenge(&client_nonce, &server_hello.server_nonce);
        framed
            .send_frame(&Frame::Handshake(HandshakeFrame::AuthResponse(
                AuthResponse { signature },
            )))
            .await
            .map_err(|e| SessionError::Transport(e.to_string()))?;

        // 5. Await AuthResult
        let result_frame = framed
            .recv_frame()
            .await
            .map_err(|e| SessionError::Transport(e.to_string()))?
            .ok_or_else(|| SessionError::Closed("Server disconnected before AuthResult".into()))?;

        let auth_result = match result_frame {
            Frame::Handshake(HandshakeFrame::AuthResult(r)) => r,
            other => {
                return Err(SessionError::Protocol(format!(
                    "Expected AuthResult, got {other:?}"
                )))
            }
        };

        if !auth_result.success {
            return Err(SessionError::AuthenticationFailed(
                auth_result.reason.unwrap_or_else(|| "Unknown error".into()),
            ));
        }

        // 6. Consult TrustStore
        let initial_state = match trust_store.is_trusted(&remote_node_id, &remote_pubkey) {
            Ok(true) => {
                let _ = trust_store.update_last_seen(&remote_node_id, current_timestamp());
                info!(
                    peer = %remote_name,
                    node_id = %remote_node_id,
                    "Remote server is fully trusted in TrustStore"
                );
                SessionState::Trusted
            }
            Ok(false) => {
                info!(
                    peer = %remote_name,
                    node_id = %remote_node_id,
                    "Remote server authenticated but UNTRUSTED (pairing required)"
                );
                SessionState::AuthenticatedUntrusted
            }
            Err(IdentityError::KeyMismatch {
                node_id,
                expected,
                presented,
            }) => {
                error!(
                    node_id = %node_id,
                    expected = %expected,
                    presented = %presented,
                    "CRITICAL SECURITY ALERT: Server public key mismatch / impersonation attempt!"
                );

                let _ = framed
                    .send_frame(&Frame::Control(ControlFrame::Disconnect {
                        reason: DisconnectReason::AuthenticationFailed,
                    }))
                    .await;

                return Err(SessionError::KeyMismatch {
                    node_id: node_id.to_string(),
                    expected,
                    actual: presented,
                });
            }
            Err(IdentityError::PeerRevoked(id)) => {
                warn!(node_id = %id, "Server peer is revoked");
                let _ = framed
                    .send_frame(&Frame::Control(ControlFrame::Disconnect {
                        reason: DisconnectReason::AuthenticationFailed,
                    }))
                    .await;
                return Err(SessionError::PeerRevoked(id.to_string()));
            }
            Err(e) => return Err(SessionError::Identity(e)),
        };

        Ok(Self {
            local_identity,
            local_name,
            local_device_type: DeviceType::Unknown,
            remote_node_id,
            remote_pubkey,
            remote_name,
            remote_device_type,
            state: initial_state,
            trust_store,
            framed,
            clipboard_engine,
            receive_dir,
        })
    }

    /// Execute the explicit out-of-band SAS pairing ceremony across the network stream.
    pub async fn execute_pairing(
        &mut self,
        is_initiator: bool,
        confirmer: &dyn PairingConfirmation,
    ) -> Result<SasVerification, SessionError> {
        self.state = SessionState::Pairing;

        if is_initiator {
            // 1. Create initiator pairing session
            let (mut session, req) = PairingSession::new_initiator(
                (*self.local_identity).clone(),
                &self.local_name,
                self.local_device_type,
            );

            // 2. Send PairingMessage::Request
            let msg = PairingMessage::Request(req);
            self.framed
                .send_frame(&Frame::Data(msg.to_data_frame()?))
                .await
                .map_err(|e| SessionError::Transport(e.to_string()))?;

            // 3. Await PairingMessage::Response
            let frame = self.recv_data_frame(DataFrame::CHANNEL_PAIRING).await?;
            let resp_msg = PairingMessage::from_data_frame(&frame)?;
            let resp = match resp_msg {
                PairingMessage::Response(r) => r,
                PairingMessage::Failed { reason } => {
                    return Err(SessionError::Closed(format!(
                        "Pairing rejected by peer: {reason}"
                    )))
                }
                other => {
                    return Err(SessionError::Protocol(format!(
                        "Expected PairingResponse, got {other:?}"
                    )))
                }
            };

            // 4. Derive SAS PIN
            let sas = session.initiator_receive_response(&resp)?;

            // 5. Ask user confirmation
            let confirmed = confirmer.confirm(&sas, &self.remote_name).await;
            if !confirmed {
                let fail_msg = PairingMessage::Failed {
                    reason: "Local user rejected pairing".into(),
                };
                let _ = self
                    .framed
                    .send_frame(&Frame::Data(fail_msg.to_data_frame()?))
                    .await;
                self.state = SessionState::AuthenticatedUntrusted;
                return Err(SessionError::PairingRejected);
            }

            // 6. Send PairingConfirm
            let my_confirm = session.confirm()?;
            let confirm_msg = PairingMessage::Confirm(my_confirm);
            self.framed
                .send_frame(&Frame::Data(confirm_msg.to_data_frame()?))
                .await
                .map_err(|e| SessionError::Transport(e.to_string()))?;

            // 7. Await remote PairingConfirm
            let frame = self.recv_data_frame(DataFrame::CHANNEL_PAIRING).await?;
            let peer_confirm_msg = PairingMessage::from_data_frame(&frame)?;
            let peer_confirm = match peer_confirm_msg {
                PairingMessage::Confirm(c) => c,
                PairingMessage::Failed { reason } => {
                    return Err(SessionError::Closed(format!(
                        "Pairing rejected by peer: {reason}"
                    )))
                }
                other => {
                    return Err(SessionError::Protocol(format!(
                        "Expected PairingConfirm, got {other:?}"
                    )))
                }
            };

            // 8. Verify remote confirmation and finalize
            let trusted_peer = session.finalize(&peer_confirm)?;

            // 9. Persist to TrustStore
            self.trust_store.save_peer(&trusted_peer)?;
            self.state = SessionState::Trusted;
            info!(
                peer = %self.remote_name,
                node_id = %self.remote_node_id,
                pin = %sas.formatted_pin,
                "Pairing succeeded and trusted peer saved to TrustStore"
            );

            Ok(sas)
        } else {
            // Responder Flow:
            // 1. Await PairingMessage::Request
            let frame = self.recv_data_frame(DataFrame::CHANNEL_PAIRING).await?;
            let req_msg = PairingMessage::from_data_frame(&frame)?;
            let req = match req_msg {
                PairingMessage::Request(r) => r,
                other => {
                    return Err(SessionError::Protocol(format!(
                        "Expected PairingRequest, got {other:?}"
                    )))
                }
            };

            // 2. Create responder pairing session
            let (mut session, resp, sas) = PairingSession::new_responder(
                (*self.local_identity).clone(),
                &self.local_name,
                self.local_device_type,
                &req,
            )?;

            // 3. Send PairingMessage::Response
            let resp_msg = PairingMessage::Response(resp);
            self.framed
                .send_frame(&Frame::Data(resp_msg.to_data_frame()?))
                .await
                .map_err(|e| SessionError::Transport(e.to_string()))?;

            // 4. Ask user confirmation
            let confirmed = confirmer.confirm(&sas, &self.remote_name).await;
            if !confirmed {
                let fail_msg = PairingMessage::Failed {
                    reason: "Local user rejected pairing".into(),
                };
                let _ = self
                    .framed
                    .send_frame(&Frame::Data(fail_msg.to_data_frame()?))
                    .await;
                self.state = SessionState::AuthenticatedUntrusted;
                return Err(SessionError::PairingRejected);
            }

            // 5. Send PairingConfirm
            let my_confirm = session.confirm()?;
            let confirm_msg = PairingMessage::Confirm(my_confirm);
            self.framed
                .send_frame(&Frame::Data(confirm_msg.to_data_frame()?))
                .await
                .map_err(|e| SessionError::Transport(e.to_string()))?;

            // 6. Await remote PairingConfirm
            let frame = self.recv_data_frame(DataFrame::CHANNEL_PAIRING).await?;
            let peer_confirm_msg = PairingMessage::from_data_frame(&frame)?;
            let peer_confirm = match peer_confirm_msg {
                PairingMessage::Confirm(c) => c,
                PairingMessage::Failed { reason } => {
                    return Err(SessionError::Closed(format!(
                        "Pairing rejected by peer: {reason}"
                    )))
                }
                other => {
                    return Err(SessionError::Protocol(format!(
                        "Expected PairingConfirm, got {other:?}"
                    )))
                }
            };

            // 7. Verify remote confirmation and finalize
            let trusted_peer = session.finalize(&peer_confirm)?;

            // 8. Persist to TrustStore
            self.trust_store.save_peer(&trusted_peer)?;
            self.state = SessionState::Trusted;
            info!(
                peer = %self.remote_name,
                node_id = %self.remote_node_id,
                pin = %sas.formatted_pin,
                "Pairing succeeded and trusted peer saved to TrustStore"
            );

            Ok(sas)
        }
    }

    /// Process a single incoming frame, enforcing the trust boundary on application channels.
    pub async fn process_next_frame(&mut self) -> Result<bool, SessionError> {
        let frame_opt = self
            .framed
            .recv_frame()
            .await
            .map_err(|e| SessionError::Transport(e.to_string()))?;

        let Some(frame) = frame_opt else {
            return Ok(false); // Clean EOF
        };

        match frame {
            Frame::Control(ControlFrame::Ping { nonce }) => {
                debug!(nonce, "Received Ping, replying with Pong");
                self.framed
                    .send_frame(&Frame::Control(ControlFrame::Pong { nonce }))
                    .await
                    .map_err(|e| SessionError::Transport(e.to_string()))?;
            }
            Frame::Control(ControlFrame::Pong { .. }) => {}
            Frame::Control(ControlFrame::Disconnect { reason }) => {
                info!(peer = %self.remote_name, ?reason, "Peer disconnected gracefully");
                self.state = SessionState::Terminated(format!("Disconnected: {reason:?}"));
                return Ok(false);
            }
            Frame::Data(data_frame) => match data_frame.channel {
                DataFrame::CHANNEL_PAIRING => {
                    debug!("Received unhandled pairing frame outside pairing flow");
                }
                DataFrame::CHANNEL_CLIPBOARD => {
                    if !self.state.is_trusted() {
                        warn!(
                            peer = %self.remote_name,
                            node_id = %self.remote_node_id,
                            "BLOCKED: Dropping incoming clipboard update from UNTRUSTED peer"
                        );
                        return Ok(true);
                    }

                    if let Some(ref engine) = self.clipboard_engine {
                        if let Err(e) = engine.handle_incoming_frame(&data_frame) {
                            warn!(error = %e, "Failed to apply clipboard frame");
                        }
                    }
                }
                DataFrame::CHANNEL_FILE_TRANSFER => {
                    if !self.state.is_trusted() {
                        warn!(
                            peer = %self.remote_name,
                            node_id = %self.remote_node_id,
                            "BLOCKED: Dropping incoming file transfer from UNTRUSTED peer"
                        );
                        return Ok(true);
                    }

                    if let Some(ref rdir) = self.receive_dir {
                        handle_file_transfer_frame(
                            &data_frame.payload,
                            &mut self.framed,
                            rdir,
                            &self.remote_name,
                        )
                        .await?;
                    }
                }
                other => {
                    debug!(channel = other, "Received data on unhandled channel");
                }
            },
            Frame::Handshake(h) => {
                warn!(?h, "Unexpected handshake frame in active session");
            }
        }

        Ok(true)
    }

    /// Helper to read specifically the next `DataFrame` on a specific channel.
    async fn recv_data_frame(&mut self, expected_channel: u16) -> Result<DataFrame, SessionError> {
        loop {
            let frame_opt = self
                .framed
                .recv_frame()
                .await
                .map_err(|e| SessionError::Transport(e.to_string()))?;

            let Some(frame) = frame_opt else {
                return Err(SessionError::Closed(
                    "Peer disconnected while awaiting data frame".into(),
                ));
            };

            match frame {
                Frame::Data(df) if df.channel == expected_channel => return Ok(df),
                Frame::Control(ControlFrame::Ping { nonce }) => {
                    let _ = self
                        .framed
                        .send_frame(&Frame::Control(ControlFrame::Pong { nonce }))
                        .await;
                }
                Frame::Control(ControlFrame::Disconnect { reason }) => {
                    return Err(SessionError::Closed(format!(
                        "Peer disconnected: {reason:?}"
                    )));
                }
                other => {
                    debug!(
                        ?other,
                        expected_channel, "Skipping frame while awaiting expected channel"
                    );
                }
            }
        }
    }
}

/// Helper function to handle chunked file transfer messages.
async fn handle_file_transfer_frame<T: AsyncRead + AsyncWrite + Unpin>(
    payload: &[u8],
    framed: &mut FramedStream<T>,
    receive_dir: &std::path::Path,
    client_name: &str,
) -> Result<(), SessionError> {
    let msg = TransferMessage::from_bytes(payload)
        .map_err(|e| SessionError::Protocol(format!("Transfer payload error: {e}")))?;

    match msg {
        TransferMessage::Offer(manifest) => {
            println!(
                "\n[Transfer] Incoming file offer from '{client_name}': '{}' ({} bytes, {} chunks)",
                manifest.filename, manifest.total_size, manifest.total_chunks
            );

            let receiver = FileReceiver::to_dir(receive_dir, manifest)
                .await
                .map_err(|e| SessionError::Protocol(e.to_string()))?;
            let accept = TransferMessage::Accept {
                file_id: receiver.manifest().file_id.clone(),
                start_chunk: receiver.next_expected_chunk(),
            };
            let accept_bytes = accept
                .to_bytes()
                .map_err(|e| SessionError::Protocol(e.to_string()))?;
            framed
                .send_frame(&Frame::Data(DataFrame::new(
                    DataFrame::CHANNEL_FILE_TRANSFER,
                    accept_bytes,
                )))
                .await
                .map_err(|e| SessionError::Transport(e.to_string()))?;
        }
        _ => {
            debug!("Handled intermediate file transfer message");
        }
    }

    Ok(())
}
