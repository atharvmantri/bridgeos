use crate::error::NotificationError;
use bridge_core::NodeId;
use bridge_protocol::DataFrame;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

// ─── Urgency ────────────────────────────────────────────────────────────────

/// Importance level mirroring Android notification channels / Linux urgency hints.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, Default,
)]
pub enum NotificationUrgency {
    /// Low-priority informational notification (e.g. background sync complete).
    Low,
    /// Default urgency for normal activity (e.g. new message, missed call).
    #[default]
    Normal,
    /// High-urgency alerts demanding immediate attention (e.g. incoming call).
    High,
    /// Critical / urgent alerts that bypass do-not-disturb settings.
    Critical,
}

// ─── Actions ────────────────────────────────────────────────────────────────

/// An action that the receiving device can invoke on a mirrored notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotificationAction {
    /// Dismiss the notification on both devices.
    Dismiss,
    /// Open the associated application.
    Open,
    /// Send a canned or typed reply (e.g. reply to a message notification).
    Reply { text: String },
    /// Invoke a named custom action defined by the app (e.g. "Mark as read").
    Custom { label: String, key: String },
}

// ─── State ──────────────────────────────────────────────────────────────────

/// Lifecycle state of a notification in the mirror ring buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum NotificationState {
    /// Active and visible on originating device.
    #[default]
    Active,
    /// Dismissed by user action (on either device).
    Dismissed,
    /// Timed out / expired according to originating app.
    Expired,
}

// ─── Entry ──────────────────────────────────────────────────────────────────

/// A single notification entry to mirror across devices.
///
/// Designed to be compatible with Android `Notification` and Windows toast
/// notification metadata, with fields reduced to the portable minimum.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationEntry {
    /// Originating node (the device that generated the notification).
    pub origin: NodeId,
    /// Monotonically increasing per-origin sequence for ordering and dedup.
    pub sequence: u64,
    /// UNIX epoch timestamp in milliseconds when the notification was posted.
    pub posted_at_ms: u64,
    /// Stable unique identifier for the notification within the originating app.
    /// Follows Android's `(package_name, notification_id)` convention as a string.
    pub notification_id: String,
    /// Application package or bundle identifier (e.g. `com.whatsapp`).
    pub app_id: String,
    /// Human-readable application name (e.g. "WhatsApp").
    pub app_name: String,
    /// Short notification title (truncated to 256 bytes on wire).
    pub title: String,
    /// Full notification body text (truncated to 4096 bytes on wire).
    pub text: String,
    /// Optional sub-text / summary (e.g. conversation count in group message).
    pub sub_text: Option<String>,
    /// Urgency / priority of this notification.
    pub urgency: NotificationUrgency,
    /// Current lifecycle state.
    pub state: NotificationState,
    /// Blake3 hash of the icon image, if the icon is transmitted separately.
    pub icon_hash: Option<[u8; 32]>,
    /// Raw icon image bytes (PNG preferred; bounded to 64 KB).
    pub icon_data: Option<Vec<u8>>,
    /// Whether the originating app marked this as an ongoing / persistent notification.
    pub is_ongoing: bool,
}

impl NotificationEntry {
    /// Create a new active notification entry with current timestamp.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        origin: NodeId,
        sequence: u64,
        notification_id: impl Into<String>,
        app_id: impl Into<String>,
        app_name: impl Into<String>,
        title: impl Into<String>,
        text: impl Into<String>,
        urgency: NotificationUrgency,
    ) -> Self {
        let posted_at_ms = u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
        )
        .unwrap_or(u64::MAX);

        Self {
            origin,
            sequence,
            posted_at_ms,
            notification_id: notification_id.into(),
            app_id: app_id.into(),
            app_name: app_name.into(),
            title: title.into(),
            text: text.into(),
            sub_text: None,
            urgency,
            state: NotificationState::Active,
            icon_hash: None,
            icon_data: None,
            is_ongoing: false,
        }
    }

    /// Compute Blake3 hash of the canonical notification payload (for dedup).
    pub fn compute_hash(&self) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(self.origin.0.as_ref());
        hasher.update(self.notification_id.as_bytes());
        hasher.update(self.app_id.as_bytes());
        hasher.update(self.title.as_bytes());
        hasher.update(self.text.as_bytes());
        *hasher.finalize().as_bytes()
    }

    /// Apply truncation limits so wire payloads remain bounded.
    ///
    /// - Title: max 256 bytes (UTF-8 truncated at char boundary)
    /// - Text:  max 4096 bytes
    /// - Icon:  max 65536 bytes (64 KB)
    pub fn enforce_size_limits(&mut self) {
        truncate_str_bytes(&mut self.title, 256);
        truncate_str_bytes(&mut self.text, 4096);
        if let Some(icon) = &mut self.icon_data {
            if icon.len() > 65536 {
                icon.truncate(65536);
                // Recompute hash after truncation – icon may be corrupt but we
                // still emit so the remote can show the app name at minimum.
                let mut h = blake3::Hasher::new();
                h.update(icon);
                self.icon_hash = Some(*h.finalize().as_bytes());
            }
        }
    }
}

/// Truncate a UTF-8 string to at most `max_bytes`, preserving char boundaries.
fn truncate_str_bytes(s: &mut String, max_bytes: usize) {
    if s.len() <= max_bytes {
        return;
    }
    // Walk back from the limit to find a valid char boundary.
    let mut boundary = max_bytes;
    while !s.is_char_boundary(boundary) {
        boundary -= 1;
    }
    s.truncate(boundary);
}

// ─── Wire Messages ──────────────────────────────────────────────────────────

/// Wire envelope exchanged over `DataFrame::CHANNEL_NOTIFICATIONS` (channel 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NotificationMessage {
    /// A new notification posted (or updated) on the originating device.
    Post(NotificationEntry),
    /// The originating device dismissed the notification.
    Dismiss {
        origin: NodeId,
        notification_id: String,
        sequence: u64,
    },
    /// Remote device executed an action on a mirrored notification.
    ActionInvoked {
        origin: NodeId,
        notification_id: String,
        sequence: u64,
        action: NotificationAction,
    },
    /// Bulk dismiss of all active notifications (e.g. "Clear all").
    ClearAll { origin: NodeId, sequence: u64 },
}

impl NotificationMessage {
    /// Serialize message into a `DataFrame` on `CHANNEL_NOTIFICATIONS`.
    pub fn to_data_frame(&self) -> Result<DataFrame, NotificationError> {
        let payload = postcard::to_allocvec(self)
            .map_err(|e| NotificationError::Serialization(e.to_string()))?;
        Ok(DataFrame::new(DataFrame::CHANNEL_NOTIFICATIONS, payload))
    }

    /// Deserialize a `NotificationMessage` from a `DataFrame`.
    pub fn from_data_frame(frame: &DataFrame) -> Result<Self, NotificationError> {
        if frame.channel != DataFrame::CHANNEL_NOTIFICATIONS {
            return Err(NotificationError::ChannelMismatch {
                expected: DataFrame::CHANNEL_NOTIFICATIONS,
                actual: frame.channel,
            });
        }
        postcard::from_bytes(&frame.payload)
            .map_err(|e| NotificationError::Deserialization(e.to_string()))
    }

    /// Return the `origin` `NodeId` of whichever variant this is.
    pub fn origin(&self) -> &NodeId {
        match self {
            Self::Post(e) => &e.origin,
            Self::Dismiss { origin, .. }
            | Self::ActionInvoked { origin, .. }
            | Self::ClearAll { origin, .. } => origin,
        }
    }
}
