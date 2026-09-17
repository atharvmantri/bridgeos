use crate::types::{NotificationEntry, NotificationUrgency};

/// Regex-free keyword categories used for privacy filtering.
const SENSITIVE_TITLE_KEYWORDS: &[&str] = &[
    // 2FA / OTP codes
    "verification code",
    "one-time password",
    "otp:",
    "2fa",
    "two-factor",
    "authentication code",
    "login code",
    "security code",
    // Banking / financial
    "transaction alert",
    "payment received",
    "payment sent",
    "low balance",
    "account balance",
    "bank alert",
    "credit card",
    "debit card",
    "wire transfer",
    // Password managers
    "1password",
    "lastpass",
    "bitwarden",
    "dashlane",
    "keepass",
    "keychain",
    // Medical / health
    "medical record",
    "prescription",
    "lab result",
    "health record",
];

const SENSITIVE_APP_IDS: &[&str] = &[
    // Banking
    "com.chase.sig.android",
    "com.bankofamerica.mobile",
    "com.wellsfargo.mobile",
    "com.barclays.android",
    "uk.co.hsbc",
    "com.ing.mobile",
    // Authenticators
    "com.google.android.apps.authenticator2",
    "com.authy.authy",
    "com.microsoft.authenticator",
    "com.lastpass.lastpassauthenticator",
    // Password managers
    "com.agilebits.onepassword",
    "com.lastpass.lpandroid",
    "com.bitwarden.mobile",
    "com.dashlane",
    "org.keepass2android",
    // Health / medical
    "com.apple.health",
    "com.google.android.apps.fitness",
];

/// Policy gate for notification privacy and size enforcement.
#[derive(Debug, Clone)]
pub struct NotificationPolicy {
    /// If `true`, notifications flagged as sensitive are dropped entirely.
    pub block_sensitive: bool,
    /// Minimum urgency level to forward (notifications below this are silently dropped).
    pub min_urgency: NotificationUrgency,
    /// Maximum byte size of the full postcard-encoded wire frame (0 = unlimited).
    pub max_payload_bytes: usize,
    /// Block ongoing (persistent) notifications such as media player controls.
    pub block_ongoing: bool,
}

impl Default for NotificationPolicy {
    fn default() -> Self {
        Self {
            block_sensitive: true,
            min_urgency: NotificationUrgency::Low,
            max_payload_bytes: 128 * 1024, // 128 KB hard ceiling
            block_ongoing: false,
        }
    }
}

impl NotificationPolicy {
    /// Create a strict policy: block sensitive, block ongoing, min urgency = Normal.
    pub fn strict() -> Self {
        Self {
            block_sensitive: true,
            min_urgency: NotificationUrgency::Normal,
            max_payload_bytes: 64 * 1024,
            block_ongoing: true,
        }
    }

    /// Create a permissive policy (pass everything except obviously sensitive apps).
    pub fn permissive() -> Self {
        Self {
            block_sensitive: true,
            min_urgency: NotificationUrgency::Low,
            max_payload_bytes: 256 * 1024,
            block_ongoing: false,
        }
    }

    /// Returns `true` if this notification should be suppressed and NOT forwarded.
    pub fn should_block(&self, entry: &NotificationEntry) -> bool {
        // 1. Urgency gate
        if entry.urgency < self.min_urgency {
            tracing::debug!(
                app_id = %entry.app_id,
                urgency = ?entry.urgency,
                "notification blocked: urgency below threshold"
            );
            return true;
        }

        // 2. Ongoing / persistent gate
        if self.block_ongoing && entry.is_ongoing {
            tracing::debug!(
                app_id = %entry.app_id,
                "notification blocked: ongoing notification suppressed"
            );
            return true;
        }

        // 3. Sensitive app ID gate
        if self.block_sensitive && Self::is_sensitive_app(&entry.app_id) {
            tracing::debug!(
                app_id = %entry.app_id,
                "notification blocked: sensitive app ID"
            );
            return true;
        }

        // 4. Sensitive keyword gate on title/text
        if self.block_sensitive && Self::has_sensitive_keywords(entry) {
            tracing::debug!(
                app_id = %entry.app_id,
                title = %entry.title,
                "notification blocked: sensitive keyword detected"
            );
            return true;
        }

        false
    }

    fn is_sensitive_app(app_id: &str) -> bool {
        let lower = app_id.to_lowercase();
        SENSITIVE_APP_IDS.iter().any(|s| lower.contains(s))
    }

    fn has_sensitive_keywords(entry: &NotificationEntry) -> bool {
        let title_lower = entry.title.to_lowercase();
        let text_lower = entry.text.to_lowercase();
        SENSITIVE_TITLE_KEYWORDS
            .iter()
            .any(|kw| title_lower.contains(kw) || text_lower.contains(kw))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bridge_core::NodeId;

    fn make_entry(app_id: &str, title: &str, text: &str) -> NotificationEntry {
        NotificationEntry::new(
            NodeId::from_bytes([0x01; 32]),
            1,
            "notif:1",
            app_id,
            "TestApp",
            title,
            text,
            NotificationUrgency::Normal,
        )
    }

    #[test]
    fn test_policy_blocks_sensitive_app() {
        let policy = NotificationPolicy::default();
        let mut entry = make_entry("com.google.android.apps.authenticator2", "Test", "Body");
        entry.app_id = "com.google.android.apps.authenticator2".to_string();
        assert!(policy.should_block(&entry));
    }

    #[test]
    fn test_policy_blocks_otp_keyword() {
        let policy = NotificationPolicy::default();
        let entry = make_entry("com.myapp", "Verification Code: 123456", "Use within 30s.");
        assert!(policy.should_block(&entry));
    }

    #[test]
    fn test_policy_passes_normal_notification() {
        let policy = NotificationPolicy::default();
        let entry = make_entry("com.whatsapp", "Alice: Hey!", "How are you?");
        assert!(!policy.should_block(&entry));
    }

    #[test]
    fn test_policy_blocks_low_urgency_in_strict_mode() {
        let policy = NotificationPolicy::strict();
        let mut entry = make_entry("com.myapp", "Background sync", "Synced 5 files.");
        entry.urgency = NotificationUrgency::Low;
        assert!(policy.should_block(&entry));
    }

    #[test]
    fn test_policy_blocks_ongoing_in_strict_mode() {
        let policy = NotificationPolicy::strict();
        let mut entry = make_entry("com.spotify", "Now Playing", "Track name here");
        entry.urgency = NotificationUrgency::Normal;
        entry.is_ongoing = true;
        assert!(policy.should_block(&entry));
    }

    #[test]
    fn test_policy_passes_ongoing_in_default_mode() {
        let policy = NotificationPolicy::default();
        let mut entry = make_entry("com.spotify", "Now Playing", "Track name here");
        entry.urgency = NotificationUrgency::Normal;
        entry.is_ongoing = true;
        assert!(!policy.should_block(&entry));
    }
}
