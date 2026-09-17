use std::collections::HashMap;

use tokio::sync::{broadcast, Mutex};
use tracing::{debug, warn};

use crate::{
    error::NotificationError,
    guard::NotificationGuard,
    policy::NotificationPolicy,
    types::{NotificationAction, NotificationEntry, NotificationMessage, NotificationState},
};
use bridge_core::NodeId;
use bridge_protocol::DataFrame;

/// Events emitted by [`NotificationSyncEngine`] to the local application layer.
#[derive(Debug, Clone)]
pub enum NotificationSyncEvent {
    /// A new notification arrived from a remote peer (passed policy + dedup).
    Received(NotificationEntry),
    /// A remote peer dismissed a notification we are mirroring.
    Dismissed {
        origin_hex: String,
        notification_id: String,
    },
    /// A remote peer cleared all its notifications.
    ClearedAll { origin_hex: String },
    /// A remote peer invoked an action on a mirrored notification.
    ActionInvoked {
        origin_hex: String,
        notification_id: String,
        action: NotificationAction,
    },
    /// Notification was blocked by policy.
    PolicyBlocked {
        app_id: String,
        notification_id: String,
    },
}

/// Shared mutable core of the engine, protected by a single mutex.
#[derive(Debug)]
struct EngineInner {
    /// Local origin node identity.
    local_node_id: NodeId,
    /// Deduplication and state tracking.
    guard: NotificationGuard,
    /// Privacy and urgency policy.
    policy: NotificationPolicy,
    /// Active notification ring per origin (max 256 per origin peer).
    active: HashMap<String, Vec<NotificationEntry>>,
    /// Monotonic sequence counter for locally-originated messages.
    local_sequence: u64,
}

impl EngineInner {
    fn new(local_node_id: NodeId, policy: NotificationPolicy) -> Self {
        Self {
            local_node_id,
            guard: NotificationGuard::new(),
            policy,
            active: HashMap::new(),
            local_sequence: 0,
        }
    }

    fn next_sequence(&mut self) -> u64 {
        self.local_sequence += 1;
        self.local_sequence
    }
}

/// Orchestrates inbound and outbound notification mirroring.
///
/// # Usage
/// ```ignore
/// let engine = NotificationSyncEngine::new(local_node_id, NotificationPolicy::default());
/// let mut events = engine.subscribe();
///
/// // On incoming DataFrame from network:
/// engine.handle_incoming_frame(&frame).await?;
///
/// // On local OS notification:
/// let msg = engine.build_post("com.whatsapp", "WhatsApp", "Alice: Hi!", ...).await?;
/// // Serialize msg.to_data_frame() and send over the wire.
/// ```
#[derive(Debug)]
pub struct NotificationSyncEngine {
    inner: Mutex<EngineInner>,
    tx: broadcast::Sender<NotificationSyncEvent>,
}

impl NotificationSyncEngine {
    /// Construct a new engine with the given local identity and policy.
    pub fn new(local_node_id: NodeId, policy: NotificationPolicy) -> Self {
        let (tx, _) = broadcast::channel(256);
        Self {
            inner: Mutex::new(EngineInner::new(local_node_id, policy)),
            tx,
        }
    }

    /// Subscribe to notification events for the local application layer.
    pub fn subscribe(&self) -> broadcast::Receiver<NotificationSyncEvent> {
        self.tx.subscribe()
    }

    // ── Inbound ─────────────────────────────────────────────────────────────

    /// Process an incoming `DataFrame` from a remote trusted peer.
    ///
    /// Returns `Ok(())` on success. Policy-blocked or duplicate messages are
    /// silently discarded (an event is still emitted for `PolicyBlocked`).
    pub async fn handle_incoming_frame(&self, frame: &DataFrame) -> Result<(), NotificationError> {
        let msg = NotificationMessage::from_data_frame(frame)?;
        self.handle_message(msg).await
    }

    /// Process a decoded [`NotificationMessage`].
    pub async fn handle_message(&self, msg: NotificationMessage) -> Result<(), NotificationError> {
        let mut inner = self.inner.lock().await;

        match msg {
            NotificationMessage::Post(mut entry) => {
                // Enforce wire size limits.
                entry.enforce_size_limits();

                // Policy gate.
                if inner.policy.should_block(&entry) {
                    debug!(
                        app_id = %entry.app_id,
                        notification_id = %entry.notification_id,
                        "notification blocked by policy"
                    );
                    let _ = self.tx.send(NotificationSyncEvent::PolicyBlocked {
                        app_id: entry.app_id.clone(),
                        notification_id: entry.notification_id.clone(),
                    });
                    return Ok(());
                }

                // Deduplication.
                if inner.guard.is_duplicate(&entry) {
                    debug!(
                        notification_id = %entry.notification_id,
                        "duplicate notification suppressed"
                    );
                    return Ok(());
                }

                // Check if we have previously dismissed this ID.
                let origin_hex = hex::encode(entry.origin.0);
                if inner
                    .guard
                    .is_dismissed(&origin_hex, &entry.notification_id)
                {
                    debug!(
                        notification_id = %entry.notification_id,
                        "previously-dismissed notification suppressed"
                    );
                    return Ok(());
                }

                inner.guard.record_posted(&entry);

                // Store in active ring (max 256 per origin).
                let bucket = inner.active.entry(origin_hex).or_default();
                // Remove any existing entry with same notification_id (update scenario).
                bucket.retain(|e| e.notification_id != entry.notification_id);
                bucket.push(entry.clone());
                if bucket.len() > 256 {
                    bucket.remove(0);
                }

                let _ = self.tx.send(NotificationSyncEvent::Received(entry));
            }

            NotificationMessage::Dismiss {
                origin,
                notification_id,
                ..
            } => {
                let origin_hex = hex::encode(origin.0);
                inner.guard.record_dismissed(&origin_hex, &notification_id);

                // Update state in active ring.
                if let Some(bucket) = inner.active.get_mut(&origin_hex) {
                    for entry in bucket.iter_mut() {
                        if entry.notification_id == notification_id {
                            entry.state = NotificationState::Dismissed;
                        }
                    }
                }

                let _ = self.tx.send(NotificationSyncEvent::Dismissed {
                    origin_hex,
                    notification_id,
                });
            }

            NotificationMessage::ClearAll { origin, .. } => {
                let origin_hex = hex::encode(origin.0);
                inner.guard.record_clear_all(&origin_hex);

                if let Some(bucket) = inner.active.get_mut(&origin_hex) {
                    for entry in bucket.iter_mut() {
                        entry.state = NotificationState::Dismissed;
                    }
                }

                let _ = self
                    .tx
                    .send(NotificationSyncEvent::ClearedAll { origin_hex });
            }

            NotificationMessage::ActionInvoked {
                origin,
                notification_id,
                action,
                ..
            } => {
                let origin_hex = hex::encode(origin.0);
                debug!(
                    ?action,
                    notification_id = %notification_id,
                    "remote action invoked on mirrored notification"
                );

                // Dismiss on remote-dismiss action.
                if matches!(action, NotificationAction::Dismiss) {
                    inner.guard.record_dismissed(&origin_hex, &notification_id);
                    if let Some(bucket) = inner.active.get_mut(&origin_hex) {
                        for entry in bucket.iter_mut() {
                            if entry.notification_id == notification_id {
                                entry.state = NotificationState::Dismissed;
                            }
                        }
                    }
                }

                let _ = self.tx.send(NotificationSyncEvent::ActionInvoked {
                    origin_hex,
                    notification_id,
                    action,
                });
            }
        }

        Ok(())
    }

    // ── Outbound ─────────────────────────────────────────────────────────────

    /// Build a `Post` message for a locally-originated notification.
    ///
    /// Runs the same policy check so we don't forward local sensitive content.
    /// Returns `None` if the policy suppresses the notification.
    #[allow(clippy::too_many_arguments)]
    pub async fn build_post(
        &self,
        notification_id: impl Into<String>,
        app_id: impl Into<String>,
        app_name: impl Into<String>,
        title: impl Into<String>,
        text: impl Into<String>,
        urgency: crate::types::NotificationUrgency,
    ) -> Result<Option<NotificationMessage>, NotificationError> {
        let mut inner = self.inner.lock().await;
        let seq = inner.next_sequence();
        let local_id = inner.local_node_id;

        let mut entry = NotificationEntry::new(
            local_id,
            seq,
            notification_id,
            app_id,
            app_name,
            title,
            text,
            urgency,
        );
        entry.enforce_size_limits();

        if inner.policy.should_block(&entry) {
            warn!(
                app_id = %entry.app_id,
                "outbound notification suppressed by policy"
            );
            return Ok(None);
        }

        Ok(Some(NotificationMessage::Post(entry)))
    }

    /// Build a `Dismiss` message for a locally-dismissed notification.
    pub async fn build_dismiss(&self, notification_id: impl Into<String>) -> NotificationMessage {
        let mut inner = self.inner.lock().await;
        let seq = inner.next_sequence();
        let local_id = inner.local_node_id;
        NotificationMessage::Dismiss {
            origin: local_id,
            notification_id: notification_id.into(),
            sequence: seq,
        }
    }

    /// Build a `ClearAll` message signalling the local device cleared all notifications.
    pub async fn build_clear_all(&self) -> NotificationMessage {
        let mut inner = self.inner.lock().await;
        let seq = inner.next_sequence();
        let local_id = inner.local_node_id;
        NotificationMessage::ClearAll {
            origin: local_id,
            sequence: seq,
        }
    }

    /// Build an `ActionInvoked` message for a user action on a mirrored notification.
    pub async fn build_action(
        &self,
        notification_id: impl Into<String>,
        action: NotificationAction,
    ) -> NotificationMessage {
        let mut inner = self.inner.lock().await;
        let seq = inner.next_sequence();
        let local_id = inner.local_node_id;
        NotificationMessage::ActionInvoked {
            origin: local_id,
            notification_id: notification_id.into(),
            sequence: seq,
            action,
        }
    }

    /// Snapshot of active (non-dismissed) notifications from a given origin.
    pub async fn active_notifications(&self, origin: &NodeId) -> Vec<NotificationEntry> {
        let inner = self.inner.lock().await;
        let origin_hex = hex::encode(origin.0);
        inner
            .active
            .get(&origin_hex)
            .map(|bucket| {
                bucket
                    .iter()
                    .filter(|e| e.state == NotificationState::Active)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }
}
