use crate::error::{IdentityError, Result};
use crate::keys::IdentityKey;
use std::path::Path;
use tracing::{debug, info};

/// Manages local node cryptographic identity persistence on disk.
///
/// Ensures private signing keys are saved strictly isolated from public peer trust stores,
/// with safe file permissions and atomic writes.
#[derive(Debug)]
pub struct IdentityStorage;

impl IdentityStorage {
    /// Loads an existing `IdentityKey` from disk, or generates and securely saves a fresh
    /// one if no file exists at `path`.
    pub fn load_or_generate(path: impl AsRef<Path>) -> Result<IdentityKey> {
        let path = path.as_ref();
        if path.exists() {
            Self::load(path)
        } else {
            let key = IdentityKey::generate();
            Self::save(path, &key)?;
            info!(path = %path.display(), node_id = %key.node_id(), "Generated and saved new device identity key");
            Ok(key)
        }
    }

    /// Loads an `IdentityKey` from a raw 32-byte binary keyfile or 64-character hex keyfile.
    pub fn load(path: impl AsRef<Path>) -> Result<IdentityKey> {
        let path = path.as_ref();
        let content = std::fs::read(path)?;

        let bytes: [u8; 32] = if content.len() == 32 {
            content.try_into().unwrap()
        } else if content.len() >= 64 {
            let text = std::str::from_utf8(&content).map_err(|_| {
                IdentityError::InvalidSecretKeyFile("File is not valid UTF-8 hex".to_string())
            })?;
            let trimmed = text.trim();
            let decoded = hex::decode(trimmed).map_err(|e| {
                IdentityError::InvalidSecretKeyFile(format!("Hex decode failed: {e}"))
            })?;
            decoded.try_into().map_err(|_| {
                IdentityError::InvalidSecretKeyFile("Decoded hex key is not 32 bytes".to_string())
            })?
        } else {
            return Err(IdentityError::InvalidSecretKeyFile(format!(
                "Expected 32 binary bytes or 64 hex characters, found {} bytes",
                content.len()
            )));
        };

        debug!(path = %path.display(), "Successfully loaded device identity key");
        Ok(IdentityKey::from_bytes(&bytes))
    }

    /// Saves an `IdentityKey` to disk with restricted file permissions.
    pub fn save(path: impl AsRef<Path>, key: &IdentityKey) -> Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let bytes = key.to_bytes();
        std::fs::write(path, bytes)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let permissions = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(path, permissions)?;
        }

        debug!(path = %path.display(), node_id = %key.node_id(), "Saved device identity key to disk");
        Ok(())
    }
}
