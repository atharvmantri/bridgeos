use thiserror::Error;

/// Errors produced by the notification mirroring subsystem.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum NotificationError {
    /// Postcard serialization of a [`NotificationMessage`] failed.
    #[error("notification serialization error: {0}")]
    Serialization(String),

    /// Postcard deserialization of a wire frame failed.
    #[error("notification deserialization error: {0}")]
    Deserialization(String),

    /// Incoming `DataFrame` arrived on an unexpected channel.
    #[error("channel mismatch: expected {expected}, got {actual}")]
    ChannelMismatch { expected: u16, actual: u16 },

    /// Notification payload exceeds the maximum permitted size.
    #[error("notification payload exceeds maximum size limit ({0} bytes)")]
    PayloadTooLarge(usize),

    /// Action was invoked on a notification that is no longer active.
    #[error("action invoked on unknown or expired notification: {0}")]
    NotificationNotFound(String),

    /// An operation was attempted on a channel that is not open.
    #[error("notification channel is closed")]
    ChannelClosed,
}
