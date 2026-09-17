use crate::error::{IdentityError, Result};
use crate::keys::PublicKey;
use bridge_core::{DeviceType, NodeId};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use tracing::{debug, info, warn};

/// Cryptographic trust and authorization state for a paired peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrustState {
    /// Peer is fully paired, authorized, and trusted.
    Trusted,
    /// Pairing is in progress or awaiting mutual SAS confirmation.
    Pending,
    /// Trust has been revoked; peer is blocked from establishing sessions.
    Revoked,
}

impl TrustState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Trusted => "trusted",
            Self::Pending => "pending",
            Self::Revoked => "revoked",
        }
    }
}

impl FromStr for TrustState {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(match s.trim().to_ascii_lowercase().as_str() {
            "trusted" => Self::Trusted,
            "revoked" => Self::Revoked,
            _ => Self::Pending,
        })
    }
}

impl fmt::Display for TrustState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A persistent record representing an authenticated remote peer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustedPeer {
    pub node_id: NodeId,
    pub public_key: PublicKey,
    pub device_name: String,
    pub device_type: DeviceType,
    pub first_paired_at: u64,
    pub last_seen_at: u64,
    pub trust_state: TrustState,
}

/// SQLite-backed persistent trust store for authorized BridgeOS peers.
///
/// Ensures zero-trust security: peers discovered via LAN are untrusted until
/// an explicit out-of-band pairing ceremony has occurred. Enforces key consistency
/// to immediately detect and block public key substitution or spoofing attempts.
#[derive(Debug, Clone)]
pub struct TrustStore {
    conn: Arc<Mutex<Connection>>,
}

impl TrustStore {
    /// Opens or creates an SQLite trust store at the specified filesystem path.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(path)?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.init_schema()?;
        info!(path = %path.display(), "Initialized SQLite peer trust store");
        Ok(store)
    }

    /// Creates an in-memory SQLite trust store for fast, ephemeral testing.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.init_schema()?;
        debug!("Initialized in-memory SQLite peer trust store");
        Ok(store)
    }

    /// Initializes tables and indices with migration support.
    fn init_schema(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        conn.execute_batch(
            r"
            PRAGMA journal_mode = WAL;
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS schema_version (
                version INTEGER PRIMARY KEY
            );

            CREATE TABLE IF NOT EXISTS trusted_peers (
                node_id TEXT PRIMARY KEY NOT NULL,
                public_key TEXT NOT NULL,
                device_name TEXT NOT NULL,
                device_type TEXT NOT NULL,
                first_paired_at INTEGER NOT NULL,
                last_seen_at INTEGER NOT NULL,
                trust_state TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_trusted_peers_state ON trusted_peers(trust_state);
            ",
        )?;

        // Ensure schema version row
        let mut stmt = conn.prepare("SELECT COUNT(*) FROM schema_version")?;
        let count: i64 = stmt.query_row([], |r| r.get(0))?;
        if count == 0 {
            conn.execute("INSERT INTO schema_version (version) VALUES (1)", [])?;
        }

        Ok(())
    }

    /// Saves or updates a peer record in the trust database.
    pub fn save_peer(&self, peer: &TrustedPeer) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        conn.execute(
            r"
            INSERT INTO trusted_peers (
                node_id, public_key, device_name, device_type, first_paired_at, last_seen_at, trust_state
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(node_id) DO UPDATE SET
                public_key = excluded.public_key,
                device_name = excluded.device_name,
                device_type = excluded.device_type,
                last_seen_at = excluded.last_seen_at,
                trust_state = excluded.trust_state
            ",
            params![
                peer.node_id.to_hex(),
                hex::encode(peer.public_key.to_bytes()),
                peer.device_name,
                peer.device_type.to_string(),
                peer.first_paired_at as i64,
                peer.last_seen_at as i64,
                peer.trust_state.as_str(),
            ],
        )?;

        debug!(node_id = %peer.node_id, state = %peer.trust_state, "Saved peer in trust store");
        Ok(())
    }

    /// Retrieves a peer record by its canonical `NodeId`.
    pub fn get_peer(&self, node_id: &NodeId) -> Result<Option<TrustedPeer>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            r"
            SELECT node_id, public_key, device_name, device_type, first_paired_at, last_seen_at, trust_state
            FROM trusted_peers WHERE node_id = ?1
            ",
        )?;

        let mut rows = stmt.query(params![node_id.to_hex()])?;
        if let Some(row) = rows.next()? {
            Ok(Some(Self::row_to_peer(row)?))
        } else {
            Ok(None)
        }
    }

    /// Lists all peer records regardless of trust status.
    pub fn list_peers(&self) -> Result<Vec<TrustedPeer>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            r"
            SELECT node_id, public_key, device_name, device_type, first_paired_at, last_seen_at, trust_state
            FROM trusted_peers ORDER BY last_seen_at DESC
            ",
        )?;

        let peer_iter = stmt.query_map([], Self::row_to_peer)?;
        let mut peers = Vec::new();
        for peer_res in peer_iter {
            peers.push(peer_res?);
        }
        Ok(peers)
    }

    /// Lists only peers whose state is explicitly `TrustState::Trusted`.
    pub fn list_trusted_peers(&self) -> Result<Vec<TrustedPeer>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            r"
            SELECT node_id, public_key, device_name, device_type, first_paired_at, last_seen_at, trust_state
            FROM trusted_peers WHERE trust_state = 'trusted' ORDER BY last_seen_at DESC
            ",
        )?;

        let peer_iter = stmt.query_map([], Self::row_to_peer)?;
        let mut peers = Vec::new();
        for peer_res in peer_iter {
            peers.push(peer_res?);
        }
        Ok(peers)
    }

    /// Verifies whether a peer is trusted, strictly validating that the presented public key
    /// matches the cryptographically paired public key in the database.
    ///
    /// # Security Guarantees:
    /// - Returns `Ok(true)` if peer is found, state is `Trusted`, and public key matches.
    /// - Returns `Ok(false)` if peer is unknown or still in `Pending` state.
    /// - Returns `Err(IdentityError::KeyMismatch)` if the node ID matches but the presented
    ///   public key does not match the stored public key (spoofing/impersonation attempt).
    /// - Returns `Err(IdentityError::PeerRevoked)` if the peer was previously revoked.
    pub fn is_trusted(&self, node_id: &NodeId, presented_key: &PublicKey) -> Result<bool> {
        let peer_opt = self.get_peer(node_id)?;
        let Some(peer) = peer_opt else {
            return Ok(false);
        };

        // 1. Enforce public key consistency
        if peer.public_key != *presented_key {
            warn!(
                node_id = %node_id,
                stored = %hex::encode(peer.public_key.to_bytes()),
                presented = %hex::encode(presented_key.to_bytes()),
                "CRITICAL SECURITY: Public key mismatch / impersonation attempt detected!"
            );
            return Err(IdentityError::KeyMismatch {
                node_id: *node_id,
                expected: hex::encode(peer.public_key.to_bytes()),
                presented: hex::encode(presented_key.to_bytes()),
            });
        }

        // 2. Check revocation status
        match peer.trust_state {
            TrustState::Revoked => {
                warn!(node_id = %node_id, "Connection attempt by revoked peer rejected");
                Err(IdentityError::PeerRevoked(*node_id))
            }
            TrustState::Trusted => Ok(true),
            TrustState::Pending => Ok(false),
        }
    }

    /// Updates a peer's `last_seen_at` timestamp.
    pub fn update_last_seen(&self, node_id: &NodeId, timestamp: u64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE trusted_peers SET last_seen_at = ?1 WHERE node_id = ?2",
            params![timestamp as i64, node_id.to_hex()],
        )?;
        Ok(())
    }

    /// Updates a peer's authorization trust state.
    pub fn set_trust_state(&self, node_id: &NodeId, state: TrustState) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE trusted_peers SET trust_state = ?1 WHERE node_id = ?2",
            params![state.as_str(), node_id.to_hex()],
        )?;
        info!(node_id = %node_id, new_state = %state, "Updated peer trust state");
        Ok(())
    }

    /// Revokes an existing peer, preventing all future sessions.
    pub fn revoke_peer(&self, node_id: &NodeId) -> Result<()> {
        self.set_trust_state(node_id, TrustState::Revoked)
    }

    /// Completely removes a peer from the trust store.
    pub fn delete_peer(&self, node_id: &NodeId) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let affected = conn.execute(
            "DELETE FROM trusted_peers WHERE node_id = ?1",
            params![node_id.to_hex()],
        )?;
        Ok(affected > 0)
    }

    fn row_to_peer(row: &rusqlite::Row<'_>) -> rusqlite::Result<TrustedPeer> {
        let node_id_hex: String = row.get(0)?;
        let pubkey_hex: String = row.get(1)?;
        let device_name: String = row.get(2)?;
        let device_type_str: String = row.get(3)?;
        let first_paired_at: i64 = row.get(4)?;
        let last_seen_at: i64 = row.get(5)?;
        let trust_state_str: String = row.get(6)?;

        let node_id = NodeId::from_str(&node_id_hex).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })?;

        let pubkey_bytes = hex::decode(&pubkey_hex).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e))
        })?;
        let pubkey_arr: [u8; 32] = pubkey_bytes.try_into().map_err(|_| {
            rusqlite::Error::FromSqlConversionFailure(
                1,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Invalid pubkey length",
                )),
            )
        })?;
        let public_key = PublicKey::from_bytes(&pubkey_arr).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, Box::new(e))
        })?;

        let device_type = DeviceType::from_str(&device_type_str).unwrap_or(DeviceType::Unknown);
        let trust_state = TrustState::from_str(&trust_state_str).unwrap_or(TrustState::Pending);

        Ok(TrustedPeer {
            node_id,
            public_key,
            device_name,
            device_type,
            first_paired_at: first_paired_at.max(0) as u64,
            last_seen_at: last_seen_at.max(0) as u64,
            trust_state,
        })
    }
}
