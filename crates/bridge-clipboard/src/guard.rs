use bridge_core::NodeId;
use std::collections::{HashMap, HashSet, VecDeque};

/// Thread-safe loopback echo suppression and sequence deduplicator.
///
/// Prevents ping-pong oscillations when a locally applied remote clipboard
/// entry triggers the local OS clipboard monitor.
#[derive(Debug, Clone)]
pub struct EchoGuard {
    capacity: usize,
    history: VecDeque<[u8; 32]>,
    seen_hashes: HashSet<[u8; 32]>,
    sequences: HashMap<NodeId, u64>,
}

impl EchoGuard {
    pub const DEFAULT_CAPACITY: usize = 256;

    pub fn new(capacity: usize) -> Self {
        let cap = if capacity == 0 {
            Self::DEFAULT_CAPACITY
        } else {
            capacity
        };
        Self {
            capacity: cap,
            history: VecDeque::with_capacity(cap),
            seen_hashes: HashSet::with_capacity(cap),
            sequences: HashMap::new(),
        }
    }

    /// Check whether a hash has already been seen or originated locally recently.
    pub fn should_suppress(&self, hash: &[u8; 32]) -> bool {
        self.seen_hashes.contains(hash)
    }

    /// Record a locally copied hash that will be transmitted to peers.
    pub fn record_sent(&mut self, hash: [u8; 32]) {
        self.push_hash(hash);
    }

    /// Record an incoming remote clipboard entry.
    ///
    /// Returns `true` if the entry is fresh and should be applied to the local clipboard.
    /// Returns `false` if the entry is an echo loopback or has a stale sequence number.
    pub fn record_received(&mut self, origin: NodeId, sequence: u64, hash: [u8; 32]) -> bool {
        // 1. Check if we've already seen or sent this exact content hash
        if self.should_suppress(&hash) {
            return false;
        }

        // 2. Check if sequence number is strictly increasing for this origin
        if let Some(&last_seq) = self.sequences.get(&origin) {
            if sequence <= last_seq {
                return false;
            }
        }

        // 3. Update sequence and push hash
        self.sequences.insert(origin, sequence);
        self.push_hash(hash);
        true
    }

    /// Get the highest recorded sequence number for a peer node.
    pub fn last_sequence(&self, origin: &NodeId) -> Option<u64> {
        self.sequences.get(origin).copied()
    }

    /// Clear all history.
    pub fn clear(&mut self) {
        self.history.clear();
        self.seen_hashes.clear();
        self.sequences.clear();
    }

    fn push_hash(&mut self, hash: [u8; 32]) {
        if self.seen_hashes.contains(&hash) {
            return;
        }

        if self.history.len() >= self.capacity {
            if let Some(oldest) = self.history.pop_front() {
                self.seen_hashes.remove(&oldest);
            }
        }

        self.history.push_back(hash);
        self.seen_hashes.insert(hash);
    }
}

impl Default for EchoGuard {
    fn default() -> Self {
        Self::new(Self::DEFAULT_CAPACITY)
    }
}
