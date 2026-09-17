use crate::error::{Result, TransferError};
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
    /// Computes the Blake3 root hash of the entire data buffer.
    pub fn compute_hash(data: &[u8]) -> [u8; 32] {
        *blake3::hash(data).as_bytes()
    }

    /// Computes total 64KB chunks required for a given file size.
    #[allow(clippy::cast_possible_truncation)]
    pub fn calculate_total_chunks(total_size: u64) -> u32 {
        if total_size == 0 {
            0
        } else {
            let chunk_size = CHUNK_SIZE as u64;
            total_size.div_ceil(chunk_size) as u32
        }
    }

    /// Helper to construct a new manifest.
    pub fn new(
        file_id: impl Into<String>,
        filename: impl Into<String>,
        total_size: u64,
        blake3_root_hash: [u8; 32],
    ) -> Self {
        let total_chunks = Self::calculate_total_chunks(total_size);
        Self {
            file_id: file_id.into(),
            filename: filename.into(),
            total_size,
            total_chunks,
            blake3_root_hash,
        }
    }

    /// Calculates expected byte length of a specific chunk.
    pub fn expected_chunk_size(&self, chunk_index: u32) -> Result<usize> {
        if chunk_index >= self.total_chunks {
            return Err(TransferError::ChunkOutOfBounds {
                index: chunk_index,
                total_chunks: self.total_chunks,
            });
        }

        let is_last = chunk_index == self.total_chunks - 1;
        if is_last {
            #[allow(clippy::cast_possible_truncation)]
            let remainder = (self.total_size % (CHUNK_SIZE as u64)) as usize;
            if remainder == 0 && self.total_size > 0 {
                Ok(CHUNK_SIZE)
            } else {
                Ok(remainder)
            }
        } else {
            Ok(CHUNK_SIZE)
        }
    }
}

/// A discrete 64KB chunk of file data with per-chunk Blake3 validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChunk {
    pub file_id: String,
    pub chunk_index: u32,
    pub offset: u64,
    pub data: Vec<u8>,
    pub blake3_hash: [u8; 32],
}

impl FileChunk {
    /// Creates a new `FileChunk` and automatically calculates its Blake3 checksum.
    pub fn new(
        file_id: impl Into<String>,
        chunk_index: u32,
        offset: u64,
        data: impl Into<Vec<u8>>,
    ) -> Self {
        let data = data.into();
        let blake3_hash = *blake3::hash(&data).as_bytes();
        Self {
            file_id: file_id.into(),
            chunk_index,
            offset,
            data,
            blake3_hash,
        }
    }

    /// Verifies that the payload's computed Blake3 hash matches `blake3_hash`.
    pub fn verify_hash(&self) -> bool {
        let calculated = blake3::hash(&self.data);
        calculated.as_bytes() == &self.blake3_hash
    }
}

/// File transfer wire protocol message envelopes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferMessage {
    /// Sender offers a file to the remote peer.
    Offer(FileManifest),

    /// Receiver accepts the offer and indicates starting chunk for (resumable) transfer.
    Accept { file_id: String, start_chunk: u32 },

    /// Transmitted file chunk payload.
    Data(FileChunk),

    /// Receiver acknowledges receipt and disk write of a chunk.
    Ack { file_id: String, chunk_index: u32 },

    /// Sender indicates that all chunks have been dispatched.
    Finished {
        file_id: String,
        blake3_root_hash: [u8; 32],
    },

    /// Receiver confirms that the full file has been assembled and verified.
    Complete { file_id: String },

    /// Either peer cancels or aborts the transfer.
    Cancel { file_id: String, reason: String },
}

impl TransferMessage {
    /// Serializes this transfer message into a Postcard binary buffer.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        postcard::to_allocvec(self)
            .map_err(|e| TransferError::Protocol(format!("Serialization: {e}")))
    }

    /// Deserializes a Postcard binary buffer into a `TransferMessage`.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        postcard::from_bytes(bytes)
            .map_err(|e| TransferError::Protocol(format!("Deserialization: {e}")))
    }

    /// Encapsulates this message in a protocol `DataFrame` on channel `CHANNEL_FILE_TRANSFER`.
    pub fn to_data_frame(&self) -> Result<bridge_protocol::DataFrame> {
        let payload = self.to_bytes()?;
        Ok(bridge_protocol::DataFrame::new(
            bridge_protocol::DataFrame::CHANNEL_FILE_TRANSFER,
            payload,
        ))
    }

    /// Extracts a `TransferMessage` from a `bridge_protocol::DataFrame`.
    pub fn from_data_frame(frame: &bridge_protocol::DataFrame) -> Result<Self> {
        if frame.channel != bridge_protocol::DataFrame::CHANNEL_FILE_TRANSFER {
            return Err(TransferError::Protocol(format!(
                "Invalid channel: expected {}, received {}",
                bridge_protocol::DataFrame::CHANNEL_FILE_TRANSFER,
                frame.channel
            )));
        }
        Self::from_bytes(&frame.payload)
    }
}
