//! Cross-device notification mirroring protocol and engine for BridgeOS.
//!
//! Provides notification envelopes, privacy filtering, deduplication, action
//! synchronisation, and integration with [`bridge_protocol::DataFrame`] over
//! `CHANNEL_NOTIFICATIONS` (channel 3).

#![forbid(unsafe_code)]
#![allow(
    clippy::missing_panics_doc,
    clippy::struct_excessive_bools,
    clippy::must_use_candidate
)]

pub mod engine;
pub mod error;
pub mod guard;
pub mod policy;
pub mod types;

pub use engine::{NotificationSyncEngine, NotificationSyncEvent};
pub use error::NotificationError;
pub use guard::NotificationGuard;
pub use policy::NotificationPolicy;
pub use types::{
    NotificationAction, NotificationEntry, NotificationMessage, NotificationState,
    NotificationUrgency,
};
