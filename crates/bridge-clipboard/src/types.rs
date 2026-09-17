use crate::error::ClipboardError;
use bridge_core::NodeId;
use bridge_protocol::DataFrame;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Supported image compression / container formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Bmp,
    Webp,
}

impl ImageFormat {
    pub fn mime_type(&self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Bmp => "image/bmp",
            Self::Webp => "image/webp",
        }
    }
}

/// Discriminator for clipboard payload formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ClipboardFormat {
    Text,
    Html,
    Rtf,
    Image(ImageFormat),
}

/// High-level clipboard payload data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipboardContent {
    /// Plain UTF-8 text.
    Text(String),
    /// Rich text HTML with optional fallback plain text.
    Html { html: String, plain: Option<String> },
    /// Compressed or raw image data with dimensions.
    Image {
        format: ImageFormat,
        data: Vec<u8>,
        width: u32,
        height: u32,
    },
}

impl ClipboardContent {
    /// Return the format descriptor for this content.
    pub fn format(&self) -> ClipboardFormat {
        match self {
            Self::Text(_) => ClipboardFormat::Text,
            Self::Html { .. } => ClipboardFormat::Html,
            Self::Image { format, .. } => ClipboardFormat::Image(*format),
        }
    }

    /// Calculate the memory/byte size of the content payload.
    pub fn byte_len(&self) -> usize {
        match self {
            Self::Text(s) => s.len(),
            Self::Html { html, plain } => {
                html.len() + plain.as_ref().map_or(0, std::string::String::len)
            }
            Self::Image { data, .. } => data.len(),
        }
    }

    /// Determine if the payload contains zero bytes.
    pub fn is_empty(&self) -> bool {
        self.byte_len() == 0
    }

    /// Compute deterministic Blake3 hash of the payload.
    pub fn compute_hash(&self) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        match self {
            Self::Text(s) => {
                hasher.update(b"TXT:");
                hasher.update(s.as_bytes());
            }
            Self::Html { html, plain } => {
                hasher.update(b"HTM:");
                hasher.update(html.as_bytes());
                if let Some(p) = plain {
                    hasher.update(b"|PLN:");
                    hasher.update(p.as_bytes());
                }
            }
            Self::Image {
                format,
                data,
                width,
                height,
            } => {
                hasher.update(b"IMG:");
                hasher.update(&[*format as u8]);
                hasher.update(&width.to_le_bytes());
                hasher.update(&height.to_le_bytes());
                hasher.update(data);
            }
        }
        *hasher.finalize().as_bytes()
    }
}

/// Metadata describing a clipboard synchronization item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardMetadata {
    /// Identity of the originating node.
    pub origin: NodeId,
    /// Monotonically increasing sequence number from the origin node.
    pub sequence: u64,
    /// Timestamp in milliseconds since UNIX epoch.
    pub timestamp_ms: u64,
    /// Blake3 checksum of the clipboard content.
    pub content_hash: [u8; 32],
    /// Type of clipboard data.
    pub format: ClipboardFormat,
    /// Payload size in bytes.
    pub byte_size: usize,
    /// Whether content was flagged as sensitive (e.g. password manager).
    pub is_sensitive: bool,
}

/// Full clipboard synchronization entry containing both metadata and content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardEntry {
    pub metadata: ClipboardMetadata,
    pub content: ClipboardContent,
}

impl ClipboardEntry {
    /// Construct a new clipboard entry with automatic metadata calculation.
    pub fn new(
        origin: NodeId,
        sequence: u64,
        content: ClipboardContent,
        is_sensitive: bool,
    ) -> Self {
        let content_hash = content.compute_hash();
        let format = content.format();
        let byte_size = content.byte_len();
        let timestamp_ms = u64::try_from(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
        )
        .unwrap_or(u64::MAX);

        Self {
            metadata: ClipboardMetadata {
                origin,
                sequence,
                timestamp_ms,
                content_hash,
                format,
                byte_size,
                is_sensitive,
            },
            content,
        }
    }

    /// Verify that the content matches its advertised hash and size.
    pub fn validate_integrity(&self) -> Result<(), ClipboardError> {
        let actual_hash = self.content.compute_hash();
        if actual_hash != self.metadata.content_hash {
            return Err(ClipboardError::HashMismatch {
                expected: hex::encode(self.metadata.content_hash),
                actual: hex::encode(actual_hash),
            });
        }

        let actual_size = self.content.byte_len();
        if actual_size != self.metadata.byte_size {
            return Err(ClipboardError::HashMismatch {
                expected: format!("size:{}", self.metadata.byte_size),
                actual: format!("size:{actual_size}"),
            });
        }

        Ok(())
    }
}

/// Wire envelope exchanged over DataFrame channel 1 (`CHANNEL_CLIPBOARD`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClipboardMessage {
    /// New clipboard item to sync.
    Sync(ClipboardEntry),
    /// Explicit instruction to clear clipboard.
    Clear {
        origin: NodeId,
        sequence: u64,
        timestamp_ms: u64,
    },
    /// Acknowledgment of receipt.
    Ack {
        origin: NodeId,
        sequence: u64,
        content_hash: [u8; 32],
    },
}

impl ClipboardMessage {
    /// Serialize message into a `DataFrame` on `DataFrame::CHANNEL_CLIPBOARD`.
    pub fn to_data_frame(&self) -> Result<DataFrame, ClipboardError> {
        let payload = postcard::to_allocvec(self)
            .map_err(|e| ClipboardError::Serialization(e.to_string()))?;
        Ok(DataFrame::new(DataFrame::CHANNEL_CLIPBOARD, payload))
    }

    /// Deserialize a `ClipboardMessage` from a `DataFrame`.
    pub fn from_data_frame(frame: &DataFrame) -> Result<Self, ClipboardError> {
        if frame.channel != DataFrame::CHANNEL_CLIPBOARD {
            return Err(ClipboardError::ChannelMismatch {
                expected: DataFrame::CHANNEL_CLIPBOARD,
                actual: frame.channel,
            });
        }

        postcard::from_bytes(&frame.payload)
            .map_err(|e| ClipboardError::Deserialization(e.to_string()))
    }
}
