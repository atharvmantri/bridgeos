use bridge_core::BridgeError;
use thiserror::Error;

/// Specific errors occurring during file transfer operations.
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum TransferError {
    #[error("Chunk {index} Blake3 hash mismatch: expected {expected}, calculated {actual}")]
    ChunkHashMismatch {
        index: u32,
        expected: String,
        actual: String,
    },

    #[error("Whole-file Blake3 root hash mismatch: expected {expected}, calculated {actual}")]
    RootHashMismatch { expected: String, actual: String },

    #[error("Chunk index {index} out of bounds (total chunks: {total_chunks})")]
    ChunkOutOfBounds { index: u32, total_chunks: u32 },

    #[error("Chunk {index} size {actual} bytes is invalid (expected {expected} bytes)")]
    InvalidChunkSize {
        index: u32,
        expected: usize,
        actual: usize,
    },

    #[error("Requested resume chunk {requested_chunk} exceeds total chunks {total_chunks}")]
    InvalidResumeOffset {
        requested_chunk: u32,
        total_chunks: u32,
    },

    #[error("I/O error during transfer: {0}")]
    Io(String),

    #[error("Transfer protocol error: {0}")]
    Protocol(String),

    #[error("Transfer cancelled: {0}")]
    Cancelled(String),

    #[error("Underlying bridge error: {0}")]
    Bridge(#[from] BridgeError),
}

impl From<std::io::Error> for TransferError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, TransferError>;
