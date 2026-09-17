use crate::error::{Result, TransferError};
use crate::message::{FileChunk, FileManifest, CHUNK_SIZE};
use std::collections::BTreeSet;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

enum DestinationTarget {
    File {
        final_path: PathBuf,
        part_path: PathBuf,
        file: File,
    },
    Memory {
        buffer: Vec<u8>,
    },
}

/// Receives, verifies, and reassembles file chunks.
pub struct FileReceiver {
    manifest: FileManifest,
    target: DestinationTarget,
    received_chunks: BTreeSet<u32>,
}

impl std::fmt::Debug for FileReceiver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileReceiver")
            .field("manifest", &self.manifest)
            .field("received_count", &self.received_chunks.len())
            .finish_non_exhaustive()
    }
}

impl FileReceiver {
    /// Creates a receiver that writes to a destination directory on disk.
    ///
    /// If an existing `.part` file is found, calculates the last valid contiguous 64KB
    /// chunk boundary and truncates any incomplete trailing writes, enabling resumption.
    pub async fn to_dir(dest_dir: impl AsRef<Path>, manifest: FileManifest) -> Result<Self> {
        let dest_dir = dest_dir.as_ref();
        tokio::fs::create_dir_all(dest_dir).await?;

        let final_path = dest_dir.join(&manifest.filename);
        let part_path = dest_dir.join(format!("{}.part", manifest.filename));

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&part_path)
            .await?;

        let metadata = file.metadata().await?;
        let existing_len = metadata.len();

        let mut received_chunks = BTreeSet::new();
        if existing_len > 0 && manifest.total_chunks > 0 {
            #[allow(clippy::cast_possible_truncation)]
            let full_chunks = (existing_len / (CHUNK_SIZE as u64)) as u32;
            let valid_chunks = full_chunks.min(manifest.total_chunks);

            // Truncate to verified chunk boundary
            let truncated_len = u64::from(valid_chunks) * (CHUNK_SIZE as u64);
            file.set_len(truncated_len).await?;

            for idx in 0..valid_chunks {
                received_chunks.insert(idx);
            }
        }

        Ok(Self {
            manifest,
            target: DestinationTarget::File {
                final_path,
                part_path,
                file,
            },
            received_chunks,
        })
    }

    /// Creates an in-memory receiver for fast testing and small payloads.
    pub fn to_memory(manifest: FileManifest) -> Self {
        #[allow(clippy::cast_possible_truncation)]
        let buffer = vec![0u8; manifest.total_size as usize];
        Self {
            manifest,
            target: DestinationTarget::Memory { buffer },
            received_chunks: BTreeSet::new(),
        }
    }

    /// Returns the manifest associated with this transfer.
    pub fn manifest(&self) -> &FileManifest {
        &self.manifest
    }

    /// Returns the next contiguous unreceived chunk index for transfer resumption.
    pub fn next_expected_chunk(&self) -> u32 {
        let mut expected = 0u32;
        while self.received_chunks.contains(&expected) {
            expected += 1;
        }
        expected.min(self.manifest.total_chunks)
    }

    /// Returns true if all chunks have been received and verified.
    pub fn is_complete(&self) -> bool {
        if self.manifest.total_chunks == 0 {
            return true;
        }
        #[allow(clippy::cast_possible_truncation)]
        let received_count = self.received_chunks.len() as u32;
        received_count == self.manifest.total_chunks
    }

    /// Verifies and writes an incoming file chunk.
    pub async fn receive_chunk(&mut self, chunk: &FileChunk) -> Result<()> {
        if chunk.file_id != self.manifest.file_id {
            return Err(TransferError::Protocol(format!(
                "Mismatched file_id: expected {}, received {}",
                self.manifest.file_id, chunk.file_id
            )));
        }

        if chunk.chunk_index >= self.manifest.total_chunks {
            return Err(TransferError::ChunkOutOfBounds {
                index: chunk.chunk_index,
                total_chunks: self.manifest.total_chunks,
            });
        }

        // 1. Verify per-chunk Blake3 hash
        if !chunk.verify_hash() {
            let actual = hex::encode(blake3::hash(&chunk.data).as_bytes());
            let expected = hex::encode(chunk.blake3_hash);
            return Err(TransferError::ChunkHashMismatch {
                index: chunk.chunk_index,
                expected,
                actual,
            });
        }

        // 2. Verify expected chunk length
        let expected_size = self.manifest.expected_chunk_size(chunk.chunk_index)?;
        if chunk.data.len() != expected_size {
            return Err(TransferError::InvalidChunkSize {
                index: chunk.chunk_index,
                expected: expected_size,
                actual: chunk.data.len(),
            });
        }

        // 3. Verify chunk offset
        let expected_offset = u64::from(chunk.chunk_index) * (CHUNK_SIZE as u64);
        if chunk.offset != expected_offset {
            return Err(TransferError::Protocol(format!(
                "Chunk {} has invalid offset {} (expected {expected_offset})",
                chunk.chunk_index, chunk.offset
            )));
        }

        // 4. Write data to destination
        match &mut self.target {
            DestinationTarget::File { file, .. } => {
                file.seek(SeekFrom::Start(chunk.offset)).await?;
                file.write_all(&chunk.data).await?;
                file.flush().await?;
            }
            DestinationTarget::Memory { buffer } => {
                #[allow(clippy::cast_possible_truncation)]
                let start = chunk.offset as usize;
                let end = start + chunk.data.len();
                buffer[start..end].copy_from_slice(&chunk.data);
            }
        }

        self.received_chunks.insert(chunk.chunk_index);
        Ok(())
    }

    /// Validates whole-file integrity and finalizes the file.
    ///
    /// For disk targets, verifies the entire file against `blake3_root_hash`
    /// and renames `.part` to the final destination path.
    pub async fn finalize_file(&mut self) -> Result<PathBuf> {
        if !self.is_complete() {
            return Err(TransferError::Protocol(format!(
                "Cannot finalize: received {} of {} chunks",
                self.received_chunks.len(),
                self.manifest.total_chunks
            )));
        }

        match &mut self.target {
            DestinationTarget::File {
                final_path,
                part_path,
                file,
            } => {
                file.flush().await?;
                file.seek(SeekFrom::Start(0)).await?;

                // Compute whole-file Blake3 root hash
                let mut hasher = blake3::Hasher::new();
                let mut buffer = vec![0u8; CHUNK_SIZE];
                loop {
                    let bytes_read = file.read(&mut buffer).await?;
                    if bytes_read == 0 {
                        break;
                    }
                    hasher.update(&buffer[..bytes_read]);
                }

                let computed_root = *hasher.finalize().as_bytes();
                if computed_root != self.manifest.blake3_root_hash {
                    return Err(TransferError::RootHashMismatch {
                        expected: hex::encode(self.manifest.blake3_root_hash),
                        actual: hex::encode(computed_root),
                    });
                }

                // File verified, rename .part -> final
                tokio::fs::rename(&part_path, &final_path).await?;
                Ok(final_path.clone())
            }
            DestinationTarget::Memory { .. } => Err(TransferError::Protocol(
                "Cannot call finalize_file on memory receiver; use finalize_memory instead"
                    .to_string(),
            )),
        }
    }

    /// Validates whole-file integrity and returns the assembled byte buffer for memory targets.
    pub fn finalize_memory(self) -> Result<Vec<u8>> {
        if !self.is_complete() {
            return Err(TransferError::Protocol(format!(
                "Cannot finalize: received {} of {} chunks",
                self.received_chunks.len(),
                self.manifest.total_chunks
            )));
        }

        match self.target {
            DestinationTarget::Memory { buffer } => {
                let computed_root = *blake3::hash(&buffer).as_bytes();
                if computed_root != self.manifest.blake3_root_hash {
                    return Err(TransferError::RootHashMismatch {
                        expected: hex::encode(self.manifest.blake3_root_hash),
                        actual: hex::encode(computed_root),
                    });
                }
                Ok(buffer)
            }
            DestinationTarget::File { .. } => Err(TransferError::Protocol(
                "Cannot call finalize_memory on file receiver; use finalize_file instead"
                    .to_string(),
            )),
        }
    }
}
