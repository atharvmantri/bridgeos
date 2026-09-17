//! Real-time cross-device clipboard synchronization engine for BridgeOS.
//!
//! Provides loopback suppression (`EchoGuard`), bounded size enforcement,
//! sensitivity filtering, and pluggable platform adapters (`ClipboardBackend`).

#![cfg_attr(not(windows), forbid(unsafe_code))]
#![allow(clippy::missing_panics_doc, clippy::struct_excessive_bools)]

pub mod backend;
pub mod engine;
pub mod error;
pub mod guard;
pub mod policy;
pub mod types;

#[cfg(windows)]
pub use backend::WindowsClipboardBackend;
pub use backend::{ClipboardBackend, MemoryClipboardBackend};
pub use engine::{ClipboardSyncEngine, ClipboardSyncEvent};
pub use error::ClipboardError;
pub use guard::EchoGuard;
pub use policy::ClipboardPolicy;
pub use types::{
    ClipboardContent, ClipboardEntry, ClipboardFormat, ClipboardMessage, ClipboardMetadata,
    ImageFormat,
};
