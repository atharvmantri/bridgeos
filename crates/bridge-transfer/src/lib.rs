//! Resumable chunked file streaming and hash verification for BridgeOS.

#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};

/// Standard file streaming chunk size: 64 kilobytes.
pub const CHUNK_SIZE: usize = 64 * 1024;

/// Metadata manifest for a file transfer session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileManifest {
    pub file_id: String,
    pub filename: String,
    pub total_size: u64,
    pub total_chunks: u32,
    pub blake3_root_hash: [u8; 32],
}

impl FileManifest {
    pub fn compute_hash(data: &[u8]) -> [u8; 32] {
        *blake3::hash(data).as_bytes()
    }
}
