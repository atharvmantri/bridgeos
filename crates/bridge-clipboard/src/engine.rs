use crate::backend::ClipboardBackend;
use crate::error::ClipboardError;
use crate::guard::EchoGuard;
use crate::policy::ClipboardPolicy;
use crate::types::{ClipboardContent, ClipboardEntry, ClipboardFormat, ClipboardMessage};
use bridge_core::NodeId;
use bridge_protocol::DataFrame;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

/// Events emitted during clipboard synchronization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardSyncEvent {
    /// Local clipboard was updated and a new sync message was packaged.
    LocalCopied {
        hash: [u8; 32],
        format: ClipboardFormat,
        size: usize,
    },
    /// A new outbound network frame is ready to be sent to connected peers.
    OutgoingBroadcastReady {
        message: ClipboardMessage,
        frame: DataFrame,
    },
    /// A remote peer's clipboard content was successfully validated and applied locally.
    RemoteApplied {
        origin: NodeId,
        sequence: u64,
        hash: [u8; 32],
        format: ClipboardFormat,
    },
    /// An echo loopback or duplicate hash was detected and suppressed.
    EchoSuppressed {
        hash: [u8; 32],
        reason: &'static str,
    },
    /// A clipboard update was rejected due to size or privacy policy.
    Rejected { reason: String },
}

/// Central cross-device clipboard synchronization engine.
pub struct ClipboardSyncEngine {
    local_node_id: NodeId,
    sequence: AtomicU64,
    backend: Arc<dyn ClipboardBackend>,
    guard: Arc<Mutex<EchoGuard>>,
    policy: ClipboardPolicy,
    event_tx: broadcast::Sender<ClipboardSyncEvent>,
}

impl std::fmt::Debug for ClipboardSyncEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClipboardSyncEngine")
            .field("local_node_id", &self.local_node_id)
            .field("sequence", &self.sequence.load(Ordering::Relaxed))
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

impl ClipboardSyncEngine {
    pub const DEFAULT_EVENT_CAPACITY: usize = 128;

    pub fn new(
        local_node_id: NodeId,
        backend: Arc<dyn ClipboardBackend>,
        policy: ClipboardPolicy,
    ) -> Self {
        let (tx, _) = broadcast::channel(Self::DEFAULT_EVENT_CAPACITY);
        Self {
            local_node_id,
            sequence: AtomicU64::new(1),
            backend,
            guard: Arc::new(Mutex::new(EchoGuard::default())),
            policy,
            event_tx: tx,
        }
    }

    /// Access the current local node ID.
    pub fn local_node_id(&self) -> &NodeId {
        &self.local_node_id
    }

    /// Access the underlying backend.
    pub fn backend(&self) -> &Arc<dyn ClipboardBackend> {
        &self.backend
    }

    /// Access the configured policy.
    pub fn policy(&self) -> &ClipboardPolicy {
        &self.policy
    }

    /// Subscribe to engine synchronization events.
    pub fn subscribe(&self) -> broadcast::Receiver<ClipboardSyncEvent> {
        self.event_tx.subscribe()
    }

    /// Process a new local clipboard copy event.
    ///
    /// Validates policy, checks echo suppression, increments the local sequence counter,
    /// packages the payload into a `DataFrame`, and records the hash in the echo guard.
    pub fn handle_local_change(
        &self,
        content: ClipboardContent,
        is_sensitive: bool,
    ) -> Result<Option<DataFrame>, ClipboardError> {
        if let Err(e) = self.policy.validate(&content, is_sensitive) {
            let reason = e.to_string();
            warn!(reason = %reason, "Local clipboard change rejected by policy");
            let _ = self.event_tx.send(ClipboardSyncEvent::Rejected { reason });
            return Err(e);
        }

        let hash = content.compute_hash();

        // Check if this content is already recorded (e.g. was just received from remote)
        {
            let guard = self.guard.lock().unwrap();
            if guard.should_suppress(&hash) {
                debug!(hash = %hex::encode(hash), "Local clipboard change suppressed (known echo)");
                let _ = self.event_tx.send(ClipboardSyncEvent::EchoSuppressed {
                    hash,
                    reason: "Local change matches recent known hash",
                });
                return Ok(None);
            }
        }

        let seq = self.sequence.fetch_add(1, Ordering::SeqCst);
        let format = content.format();
        let size = content.byte_len();

        // Record into echo guard before broadcasting
        {
            let mut guard = self.guard.lock().unwrap();
            guard.record_sent(hash);
        }

        let entry = ClipboardEntry::new(self.local_node_id, seq, content, is_sensitive);
        let message = ClipboardMessage::Sync(entry);
        let frame = message.to_data_frame()?;

        let _ = self
            .event_tx
            .send(ClipboardSyncEvent::LocalCopied { hash, format, size });
        let _ = self
            .event_tx
            .send(ClipboardSyncEvent::OutgoingBroadcastReady {
                message,
                frame: frame.clone(),
            });

        info!(
            seq = seq,
            format = ?format,
            size = size,
            hash = %hex::encode(hash),
            "Packaged local clipboard update for broadcast"
        );

        Ok(Some(frame))
    }

    /// Process an incoming network `DataFrame` from a connected peer.
    ///
    /// Validates origin, integrity, policy, and sequence freshness, prevents
    /// echo loops, and applies the content to the local clipboard backend.
    pub fn handle_incoming_frame(
        &self,
        frame: &DataFrame,
    ) -> Result<Option<ClipboardSyncEvent>, ClipboardError> {
        let message = ClipboardMessage::from_data_frame(frame)?;

        match message {
            ClipboardMessage::Sync(entry) => {
                // 1. Ignore self-echoes if we receive our own node ID
                if entry.metadata.origin == self.local_node_id {
                    debug!("Ignoring incoming clipboard sync from self");
                    return Ok(None);
                }

                // 2. Validate cryptographic hash and advertised size
                if let Err(e) = entry.validate_integrity() {
                    let reason = e.to_string();
                    error!(error = %reason, "Received corrupted clipboard payload");
                    let _ = self.event_tx.send(ClipboardSyncEvent::Rejected { reason });
                    return Err(e);
                }

                // 3. Validate against local policy (size limits, disallowed formats)
                if let Err(e) = self
                    .policy
                    .validate(&entry.content, entry.metadata.is_sensitive)
                {
                    let reason = e.to_string();
                    warn!(error = %reason, "Received clipboard payload blocked by local policy");
                    let _ = self.event_tx.send(ClipboardSyncEvent::Rejected { reason });
                    return Err(e);
                }

                // 4. Check echo guard and sequence tracking
                {
                    let mut guard = self.guard.lock().unwrap();
                    if !guard.record_received(
                        entry.metadata.origin,
                        entry.metadata.sequence,
                        entry.metadata.content_hash,
                    ) {
                        debug!(
                            origin = %entry.metadata.origin,
                            seq = entry.metadata.sequence,
                            hash = %hex::encode(entry.metadata.content_hash),
                            "Incoming clipboard entry suppressed as duplicate or stale"
                        );
                        let evt = ClipboardSyncEvent::EchoSuppressed {
                            hash: entry.metadata.content_hash,
                            reason: "Incoming entry already seen or has stale sequence",
                        };
                        let _ = self.event_tx.send(evt.clone());
                        return Ok(Some(evt));
                    }
                }

                // 5. Apply to local backend
                let format = entry.metadata.format;
                let origin = entry.metadata.origin;
                let seq = entry.metadata.sequence;
                let hash = entry.metadata.content_hash;

                self.backend.set_content(entry.content)?;

                let event = ClipboardSyncEvent::RemoteApplied {
                    origin,
                    sequence: seq,
                    hash,
                    format,
                };
                let _ = self.event_tx.send(event.clone());

                info!(
                    origin = %origin,
                    seq = seq,
                    format = ?format,
                    hash = %hex::encode(hash),
                    "Applied remote clipboard update to local backend"
                );

                Ok(Some(event))
            }
            ClipboardMessage::Clear {
                origin, sequence, ..
            } => {
                if origin == self.local_node_id {
                    return Ok(None);
                }

                {
                    let mut guard = self.guard.lock().unwrap();
                    if !guard.record_received(origin, sequence, [0u8; 32]) {
                        return Ok(None);
                    }
                }

                self.backend.clear()?;
                info!(origin = %origin, "Cleared local clipboard per remote instruction");
                Ok(None)
            }
            ClipboardMessage::Ack { .. } => {
                // Reserved for selective delivery receipts
                Ok(None)
            }
        }
    }

    /// Spawn a background task that continuously monitors the backend for local changes
    /// and invokes `handle_local_change`.
    pub fn start_monitor(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let engine = Arc::clone(self);
        let mut rx = engine.backend.subscribe();

        tokio::spawn(async move {
            debug!("Clipboard backend monitor loop started");
            while let Ok(content) = rx.recv().await {
                if let Err(e) = engine.handle_local_change(content, false) {
                    debug!(error = %e, "Local clipboard monitor skipped content");
                }
            }
            debug!("Clipboard backend monitor loop terminated");
        })
    }
}
