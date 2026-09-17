use crate::error::ClipboardError;
use crate::types::ClipboardContent;
use std::sync::{Arc, RwLock};
use tokio::sync::broadcast;

/// Abstract interface for platform clipboard integrations.
pub trait ClipboardBackend: Send + Sync {
    /// Read the current content of the clipboard.
    fn get_content(&self) -> Result<Option<ClipboardContent>, ClipboardError>;

    /// Set the content of the local system clipboard.
    fn set_content(&self, content: ClipboardContent) -> Result<(), ClipboardError>;

    /// Clear the content of the local system clipboard.
    fn clear(&self) -> Result<(), ClipboardError>;

    /// Subscribe to notifications when local clipboard changes occur.
    fn subscribe(&self) -> broadcast::Receiver<ClipboardContent>;
}

/// Fully-featured in-memory clipboard backend for testing, CLI headless mode,
/// and environments without display server access.
#[derive(Debug, Clone)]
pub struct MemoryClipboardBackend {
    content: Arc<RwLock<Option<ClipboardContent>>>,
    notifier: broadcast::Sender<ClipboardContent>,
}

impl MemoryClipboardBackend {
    pub const DEFAULT_CHANNEL_CAPACITY: usize = 64;

    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(Self::DEFAULT_CHANNEL_CAPACITY);
        Self {
            content: Arc::new(RwLock::new(None)),
            notifier: tx,
        }
    }

    pub fn with_initial_content(content: ClipboardContent) -> Self {
        let backend = Self::new();
        if let Ok(mut guard) = backend.content.write() {
            *guard = Some(content);
        }
        backend
    }
}

impl Default for MemoryClipboardBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl ClipboardBackend for MemoryClipboardBackend {
    fn get_content(&self) -> Result<Option<ClipboardContent>, ClipboardError> {
        let guard = self
            .content
            .read()
            .map_err(|e| ClipboardError::BackendError(e.to_string()))?;
        Ok(guard.clone())
    }

    fn set_content(&self, content: ClipboardContent) -> Result<(), ClipboardError> {
        {
            let mut guard = self
                .content
                .write()
                .map_err(|e| ClipboardError::BackendError(e.to_string()))?;
            *guard = Some(content.clone());
        }

        // Notify local subscribers (ignore error if no receivers are currently active)
        let _ = self.notifier.send(content);
        Ok(())
    }

    fn clear(&self) -> Result<(), ClipboardError> {
        let mut guard = self
            .content
            .write()
            .map_err(|e| ClipboardError::BackendError(e.to_string()))?;
        *guard = None;
        Ok(())
    }

    fn subscribe(&self) -> broadcast::Receiver<ClipboardContent> {
        self.notifier.subscribe()
    }
}
