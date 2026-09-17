use std::collections::{HashMap, VecDeque};

use crate::types::{NotificationEntry, NotificationState};

/// Capacity of the ring buffer: maximum number of active notification IDs
/// tracked for deduplication per remote origin.
const RING_CAPACITY: usize = 256;

/// Tracks seen notification hashes and active notification states per origin,
/// providing O(1) duplicate detection and bounded memory use.
///
/// The guard maintains two data structures per (`origin`, `notification_id`) pair:
/// - A **hash ring** to detect duplicate `Post` messages with identical content.
/// - A **state map** to track whether a notification is `Active` / `Dismissed` so
///   the engine can suppress re-posting an already-dismissed notification ID.
#[derive(Debug)]
pub struct NotificationGuard {
    /// Per-notification-id content hash cache for dedup.
    /// Key: `(origin_hex, notification_id)`, Value: last content hash.
    seen_hashes: HashMap<(String, String), [u8; 32]>,
    /// LRU eviction ring to bound memory: stores `(origin_hex, notification_id)`
    /// in insertion order so we can evict the oldest when capacity is exceeded.
    eviction_ring: VecDeque<(String, String)>,
    /// Lifecycle state per `(origin_hex, notification_id)`.
    states: HashMap<(String, String), NotificationState>,
}

impl Default for NotificationGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl NotificationGuard {
    pub fn new() -> Self {
        Self {
            seen_hashes: HashMap::new(),
            eviction_ring: VecDeque::with_capacity(RING_CAPACITY),
            states: HashMap::new(),
        }
    }

    /// Returns `true` if this notification entry is a duplicate of one already
    /// delivered (same `notification_id` and same content hash).
    pub fn is_duplicate(&self, entry: &NotificationEntry) -> bool {
        let key = Self::key(entry);
        self.seen_hashes
            .get(&key)
            .is_some_and(|h| h == &entry.compute_hash())
    }

    /// Returns `true` if the notification with this ID has already been dismissed.
    pub fn is_dismissed(&self, origin_hex: &str, notification_id: &str) -> bool {
        let key = (origin_hex.to_string(), notification_id.to_string());
        self.states
            .get(&key)
            .is_some_and(|s| *s == NotificationState::Dismissed)
    }

    /// Record that we have delivered this notification (updates hash + state).
    /// Evicts the oldest entry if the ring is at capacity.
    pub fn record_posted(&mut self, entry: &NotificationEntry) {
        let key = Self::key(entry);
        let hash = entry.compute_hash();

        if !self.seen_hashes.contains_key(&key) {
            // New entry – check eviction.
            if self.eviction_ring.len() >= RING_CAPACITY {
                if let Some(oldest) = self.eviction_ring.pop_front() {
                    self.seen_hashes.remove(&oldest);
                    self.states.remove(&oldest);
                }
            }
            self.eviction_ring.push_back(key.clone());
        }

        self.seen_hashes.insert(key.clone(), hash);
        self.states.insert(key, NotificationState::Active);
    }

    /// Record that a notification with this ID has been dismissed.
    pub fn record_dismissed(&mut self, origin_hex: &str, notification_id: &str) {
        let key = (origin_hex.to_string(), notification_id.to_string());
        if let Some(state) = self.states.get_mut(&key) {
            *state = NotificationState::Dismissed;
        }
        // Also remove from hash cache so a re-post of the same content after
        // dismiss will be accepted (re-posted as a new notification).
        self.seen_hashes.remove(&key);
    }

    /// Record a ClearAll event: mark all tracked notifications from `origin` as
    /// dismissed.
    pub fn record_clear_all(&mut self, origin_hex: &str) {
        for ((o, _), state) in &mut self.states {
            if o == origin_hex {
                *state = NotificationState::Dismissed;
            }
        }
        // Remove all hash records for this origin.
        self.seen_hashes.retain(|(o, _), _| o != origin_hex);
    }

    /// Number of tracked notification entries (active + dismissed in cache).
    pub fn len(&self) -> usize {
        self.states.len()
    }

    /// True when the guard has not yet seen any notifications.
    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }

    fn key(entry: &NotificationEntry) -> (String, String) {
        let origin_hex = hex::encode(entry.origin.0);
        (origin_hex, entry.notification_id.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bridge_core::NodeId;

    fn make_entry(origin: NodeId, id: &str, text: &str) -> NotificationEntry {
        NotificationEntry::new(
            origin,
            1,
            id,
            "com.test",
            "TestApp",
            "Title",
            text,
            crate::types::NotificationUrgency::Normal,
        )
    }

    #[test]
    fn test_dedup_same_content() {
        let mut guard = NotificationGuard::new();
        let origin = NodeId::from_bytes([0x01; 32]);
        let entry = make_entry(origin, "notif:1", "Hello");

        assert!(!guard.is_duplicate(&entry));
        guard.record_posted(&entry);
        assert!(guard.is_duplicate(&entry));
    }

    #[test]
    fn test_dedup_updated_content() {
        let mut guard = NotificationGuard::new();
        let origin = NodeId::from_bytes([0x01; 32]);
        let entry_v1 = make_entry(origin, "notif:1", "Hello");
        let entry_v2 = make_entry(origin, "notif:1", "Updated text");

        guard.record_posted(&entry_v1);
        assert!(guard.is_duplicate(&entry_v1));
        // Same ID but different content → not a duplicate → should be delivered.
        assert!(!guard.is_duplicate(&entry_v2));
    }

    #[test]
    fn test_dismiss_suppresses_redelivery() {
        let mut guard = NotificationGuard::new();
        let origin = NodeId::from_bytes([0x02; 32]);
        let origin_hex = hex::encode(origin.0);
        let entry = make_entry(origin, "notif:2", "Dismissed notification");

        guard.record_posted(&entry);
        assert!(!guard.is_dismissed(&origin_hex, "notif:2"));

        guard.record_dismissed(&origin_hex, "notif:2");
        assert!(guard.is_dismissed(&origin_hex, "notif:2"));
        // After dismiss, hash cache cleared → same content is no longer a dup.
        assert!(!guard.is_duplicate(&entry));
    }

    #[test]
    fn test_clear_all_marks_all_dismissed() {
        let mut guard = NotificationGuard::new();
        let origin = NodeId::from_bytes([0x03; 32]);
        let origin_hex = hex::encode(origin.0);

        let e1 = make_entry(origin, "notif:a", "Notification A");
        let e2 = make_entry(origin, "notif:b", "Notification B");
        guard.record_posted(&e1);
        guard.record_posted(&e2);

        guard.record_clear_all(&origin_hex);
        assert!(guard.is_dismissed(&origin_hex, "notif:a"));
        assert!(guard.is_dismissed(&origin_hex, "notif:b"));
    }

    #[test]
    fn test_ring_eviction_at_capacity() {
        let mut guard = NotificationGuard::new();
        let origin = NodeId::from_bytes([0x04; 32]);

        // Fill ring to capacity + 1 to trigger eviction.
        for i in 0..=(RING_CAPACITY) {
            let entry = make_entry(origin, &format!("notif:{i}"), &format!("Text {i}"));
            guard.record_posted(&entry);
        }
        // Guard should not exceed capacity.
        assert!(guard.len() <= RING_CAPACITY);
    }
}
