pub mod memory;
#[cfg(windows)]
pub mod windows;

pub use memory::MemoryClipboardBackend;
#[cfg(windows)]
pub use windows::WindowsClipboardBackend;

use crate::error::ClipboardError;
use crate::types::ClipboardContent;
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
