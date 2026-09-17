use crate::error::{Result, TransferError};
use crate::message::{FileChunk, FileManifest, CHUNK_SIZE};
use std::io::{Cursor, SeekFrom};
use std::path::Path;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

enum SenderSource {
    File(File),
    Memory(Cursor<Vec<u8>>),
}

/// Streams file data in discrete 64KB verified chunks.
pub struct FileSender {
    manifest: FileManifest,
    source: SenderSource,
    current_chunk: u32,
}

impl std::fmt::Debug for FileSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileSender")
            .field("manifest", &self.manifest)
            .field("current_chunk", &self.current_chunk)
            .finish_non_exhaustive()
    }
}

impl FileSender {
    /// Opens a filesystem file for chunked transmission, seeking to `start_chunk`.
    pub async fn from_file(
        path: impl AsRef<Path>,
        manifest: FileManifest,
        start_chunk: u32,
    ) -> Result<Self> {
        let mut file = File::open(path.as_ref()).await?;
        Self::validate_and_seek_file(&mut file, &manifest, start_chunk).await?;

        Ok(Self {
            manifest,
            source: SenderSource::File(file),
            current_chunk: start_chunk,
        })
    }

    /// Creates a chunk sender from an in-memory byte buffer.
    pub fn from_bytes(data: Vec<u8>, manifest: FileManifest, start_chunk: u32) -> Result<Self> {
        if start_chunk > manifest.total_chunks {
            return Err(TransferError::InvalidResumeOffset {
                requested_chunk: start_chunk,
                total_chunks: manifest.total_chunks,
            });
        }

        let mut cursor = Cursor::new(data);
        let offset = u64::from(start_chunk) * (CHUNK_SIZE as u64);
        cursor.set_position(offset);

        Ok(Self {
            manifest,
            source: SenderSource::Memory(cursor),
            current_chunk: start_chunk,
        })
    }

    async fn validate_and_seek_file(
        file: &mut File,
        manifest: &FileManifest,
        start_chunk: u32,
    ) -> Result<()> {
        if start_chunk > manifest.total_chunks {
            return Err(TransferError::InvalidResumeOffset {
                requested_chunk: start_chunk,
                total_chunks: manifest.total_chunks,
            });
        }

        let offset = u64::from(start_chunk) * (CHUNK_SIZE as u64);
        file.seek(SeekFrom::Start(offset)).await?;
        Ok(())
    }

    /// Seeks the sender to a specific chunk index for transfer resumption.
    pub async fn seek_to_chunk(&mut self, chunk_index: u32) -> Result<()> {
        if chunk_index > self.manifest.total_chunks {
            return Err(TransferError::InvalidResumeOffset {
                requested_chunk: chunk_index,
                total_chunks: self.manifest.total_chunks,
            });
        }

        let offset = u64::from(chunk_index) * (CHUNK_SIZE as u64);
        match &mut self.source {
            SenderSource::File(file) => {
                file.seek(SeekFrom::Start(offset)).await?;
            }
            SenderSource::Memory(cursor) => {
                cursor.set_position(offset);
            }
        }
        self.current_chunk = chunk_index;
        Ok(())
    }

    /// Reads and returns the next 64KB chunk.
    ///
    /// Returns `Ok(None)` once all chunks up to `manifest.total_chunks` have been read.
    pub async fn next_chunk(&mut self) -> Result<Option<FileChunk>> {
        if self.current_chunk >= self.manifest.total_chunks {
            return Ok(None);
        }

        let chunk_index = self.current_chunk;
        let expected_size = self.manifest.expected_chunk_size(chunk_index)?;
        let offset = u64::from(chunk_index) * (CHUNK_SIZE as u64);

        let mut buf = vec![0u8; expected_size];
        match &mut self.source {
            SenderSource::File(file) => {
                file.read_exact(&mut buf).await?;
            }
            SenderSource::Memory(cursor) => {
                cursor.read_exact(&mut buf).await?;
            }
        }

        let chunk = FileChunk::new(&self.manifest.file_id, chunk_index, offset, buf);
        self.current_chunk += 1;
        Ok(Some(chunk))
    }

    /// Returns a reference to the transfer's manifest.
    pub fn manifest(&self) -> &FileManifest {
        &self.manifest
    }

    /// Returns the current chunk index to be dispatched.
    pub fn current_chunk(&self) -> u32 {
        self.current_chunk
    }

    /// Returns true if all chunks have been dispatched.
    pub fn is_finished(&self) -> bool {
        self.current_chunk >= self.manifest.total_chunks
    }
}
