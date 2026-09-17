use crate::error::ClipboardError;
use crate::types::{ClipboardContent, ClipboardFormat};

/// Configuration and security policy governing clipboard synchronization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardPolicy {
    /// Maximum bytes permitted for text or rich-text payloads (default 2 MB).
    pub max_text_bytes: usize,
    /// Maximum bytes permitted for image payloads (default 10 MB).
    pub max_image_bytes: usize,
    /// Whether image clipboard synchronization is permitted.
    pub allow_images: bool,
    /// Whether HTML rich-text clipboard synchronization is permitted.
    pub allow_html: bool,
    /// Whether to drop content marked sensitive (passwords, tokens).
    pub filter_sensitive: bool,
    /// Whether clipboard synchronization is active.
    pub enabled: bool,
}

impl Default for ClipboardPolicy {
    fn default() -> Self {
        Self {
            max_text_bytes: 2 * 1024 * 1024,   // 2 MB
            max_image_bytes: 10 * 1024 * 1024, // 10 MB
            allow_images: true,
            allow_html: true,
            filter_sensitive: true,
            enabled: true,
        }
    }
}

impl ClipboardPolicy {
    /// Validate content against configured security and size policies.
    pub fn validate(
        &self,
        content: &ClipboardContent,
        is_sensitive: bool,
    ) -> Result<(), ClipboardError> {
        if !self.enabled {
            return Err(ClipboardError::BackendError(
                "Clipboard synchronization is disabled".into(),
            ));
        }

        if is_sensitive && self.filter_sensitive {
            return Err(ClipboardError::SensitiveContentBlocked);
        }

        match content {
            ClipboardContent::Text(text) => {
                if text.len() > self.max_text_bytes {
                    return Err(ClipboardError::OversizedPayload {
                        size: text.len(),
                        max: self.max_text_bytes,
                    });
                }
            }
            ClipboardContent::Html { html, plain } => {
                if !self.allow_html {
                    return Err(ClipboardError::UnsupportedFormat(
                        "HTML format disabled by policy".into(),
                    ));
                }
                let total = html.len() + plain.as_ref().map_or(0, std::string::String::len);
                if total > self.max_text_bytes {
                    return Err(ClipboardError::OversizedPayload {
                        size: total,
                        max: self.max_text_bytes,
                    });
                }
            }
            ClipboardContent::Image { data, format, .. } => {
                if !self.allow_images {
                    return Err(ClipboardError::UnsupportedFormat(format!(
                        "Image format ({format:?}) disabled by policy"
                    )));
                }
                if data.len() > self.max_image_bytes {
                    return Err(ClipboardError::OversizedPayload {
                        size: data.len(),
                        max: self.max_image_bytes,
                    });
                }
            }
        }

        Ok(())
    }

    /// Check if a format is permitted by current settings.
    pub fn is_format_allowed(&self, format: ClipboardFormat) -> bool {
        match format {
            ClipboardFormat::Text | ClipboardFormat::Rtf => true,
            ClipboardFormat::Html => self.allow_html,
            ClipboardFormat::Image(_) => self.allow_images,
        }
    }
}
