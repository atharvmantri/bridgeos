use crate::error::Result;
use crate::message::{FileManifest, CHUNK_SIZE};
use std::path::Path;
use tokio::fs::File;
use tokio::io::AsyncReadExt;

/// Creates a `FileManifest` from an in-memory byte slice.
pub fn create_manifest_from_bytes(
    file_id: impl Into<String>,
    filename: impl Into<String>,
    data: &[u8],
) -> FileManifest {
    let root_hash = FileManifest::compute_hash(data);
    FileManifest::new(file_id, filename, data.len() as u64, root_hash)
}

/// Asynchronously generates a `FileManifest` for a local filesystem file.
///
/// Streams through the file in 64KB blocks to compute the Blake3 whole-file hash
/// with constant memory usage.
pub async fn create_manifest_from_file(path: impl AsRef<Path>) -> Result<FileManifest> {
    let path = path.as_ref();
    let mut file = File::open(path).await?;
    let metadata = file.metadata().await?;
    let total_size = metadata.len();

    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown_file")
        .to_string();

    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0u8; CHUNK_SIZE];

    loop {
        let bytes_read = file.read(&mut buffer).await?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    let root_hash = *hasher.finalize().as_bytes();
    let file_id = hex::encode(&root_hash[0..16]);

    Ok(FileManifest::new(file_id, filename, total_size, root_hash))
}
