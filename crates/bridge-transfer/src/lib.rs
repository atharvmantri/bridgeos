//! Resumable chunked file streaming and hash verification for BridgeOS.

#![forbid(unsafe_code)]

pub mod error;
pub mod manifest;
pub mod message;
pub mod receiver;
pub mod sender;

pub use error::{Result, TransferError};
pub use manifest::{create_manifest_from_bytes, create_manifest_from_file};
pub use message::{FileChunk, FileManifest, TransferMessage, CHUNK_SIZE};
pub use receiver::FileReceiver;
pub use sender::FileSender;
