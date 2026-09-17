use crate::error::{IdentityError, Result};
use crate::keys::{IdentityKey, PublicKey};
use crate::trust::{TrustState, TrustedPeer};
use bridge_core::{DeviceType, NodeId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::SystemTime;

/// Verification value displayed to humans on both devices for out-of-band confirmation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SasVerification {
    /// 6-digit numeric PIN (000000 - 999999).
    pub numeric_pin: u32,
    /// Formatted hyphenated PIN display (e.g. "482-910").
    pub formatted_pin: String,
    /// Short hex fingerprint (first 8 characters of derivation hash).
    pub hex_fingerprint: String,
}

impl SasVerification {
    /// Derives a deterministic, symmetric Short Authentication String (SAS) from mutual peer
    /// identities and nonces.
    ///
    /// The derivation sorts node IDs canonically to ensure both peers compute the EXACT same
    /// verification string regardless of which peer initiated the pairing handshake.
    pub fn derive(
        node_a: &NodeId,
        node_b: &NodeId,
        nonce_a: &[u8; 32],
        nonce_b: &[u8; 32],
    ) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"BridgeOS-Pairing-SAS-v1");

        // Canonically sort identities for symmetry
        if node_a.0 <= node_b.0 {
            hasher.update(node_a.0);
            hasher.update(node_b.0);
            hasher.update(nonce_a);
            hasher.update(nonce_b);
        } else {
            hasher.update(node_b.0);
            hasher.update(node_a.0);
            hasher.update(nonce_b);
            hasher.update(nonce_a);
        }

        let digest = hasher.finalize();
        let hex_fingerprint = hex::encode(&digest[0..4]);

        // Take first 8 bytes as big-endian u64 and modulo 1,000,000 for 6-digit PIN
        let mut num_bytes = [0u8; 8];
        num_bytes.copy_from_slice(&digest[0..8]);
        let val = u64::from_be_bytes(num_bytes);
        #[allow(clippy::cast_possible_truncation)]
        let numeric_pin = (val % 1_000_000) as u32;

        let formatted_pin = format!("{:03}-{:03}", numeric_pin / 1000, numeric_pin % 1000);

        Self {
            numeric_pin,
            formatted_pin,
            hex_fingerprint,
        }
    }
}

/// Out-of-band wire message sent to request pairing with a discovered peer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingRequest {
    pub node_id: NodeId,
    pub public_key: [u8; 32],
    pub device_name: String,
    pub device_type: DeviceType,
    pub nonce: [u8; 32],
    pub timestamp: u64,
}

/// Wire message sent by the responder agreeing to initiate pairing ceremony.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingResponse {
    pub node_id: NodeId,
    pub public_key: [u8; 32],
    pub device_name: String,
    pub device_type: DeviceType,
    pub nonce: [u8; 32],
    pub timestamp: u64,
}

/// Final human-confirmed commitment signature sealing the pairing ceremony.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingConfirm {
    pub node_id: NodeId,
    pub confirmed: bool,
    pub signature: Vec<u8>,
}

/// Active ephemeral state machine orchestrating explicit peer pairing.
#[derive(Debug)]
pub struct PairingSession {
    local_key: IdentityKey,
    local_name: String,
    local_type: DeviceType,
    local_nonce: [u8; 32],
    remote_peer: Option<TrustedPeer>,
    remote_nonce: Option<[u8; 32]>,
    sas: Option<SasVerification>,
}

impl PairingSession {
    /// Initializes a pairing session from the perspective of the initiator.
    pub fn new_initiator(
        local_key: IdentityKey,
        local_name: impl Into<String>,
        local_type: DeviceType,
    ) -> (Self, PairingRequest) {
        let local_nonce = rand::random::<[u8; 32]>();
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let req = PairingRequest {
            node_id: local_key.node_id(),
            public_key: local_key.public_key().to_bytes(),
            device_name: local_name.into(),
            device_type: local_type,
            nonce: local_nonce,
            timestamp: now,
        };

        let session = Self {
            local_name: req.device_name.clone(),
            local_key,
            local_type,
            local_nonce,
            remote_peer: None,
            remote_nonce: None,
            sas: None,
        };

        (session, req)
    }

    /// Initializes a pairing session from the perspective of the responder upon receiving a `PairingRequest`.
    pub fn new_responder(
        local_key: IdentityKey,
        local_name: impl Into<String>,
        local_type: DeviceType,
        request: &PairingRequest,
    ) -> Result<(Self, PairingResponse, SasVerification)> {
        let remote_pubkey = PublicKey::from_bytes(&request.public_key)?;
        if remote_pubkey.node_id() != request.node_id {
            return Err(IdentityError::KeyMismatch {
                node_id: request.node_id,
                expected: request.node_id.to_hex(),
                presented: remote_pubkey.node_id().to_hex(),
            });
        }

        let local_nonce = rand::random::<[u8; 32]>();
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let sas = SasVerification::derive(
            &local_key.node_id(),
            &request.node_id,
            &local_nonce,
            &request.nonce,
        );

        let resp = PairingResponse {
            node_id: local_key.node_id(),
            public_key: local_key.public_key().to_bytes(),
            device_name: local_name.into(),
            device_type: local_type,
            nonce: local_nonce,
            timestamp: now,
        };

        let remote_peer = TrustedPeer {
            node_id: request.node_id,
            public_key: remote_pubkey,
            device_name: request.device_name.clone(),
            device_type: request.device_type,
            first_paired_at: now,
            last_seen_at: now,
            trust_state: TrustState::Pending,
        };

        let session = Self {
            local_name: resp.device_name.clone(),
            local_key,
            local_type,
            local_nonce,
            remote_peer: Some(remote_peer),
            remote_nonce: Some(request.nonce),
            sas: Some(sas.clone()),
        };

        Ok((session, resp, sas))
    }

    /// Initiator handles incoming `PairingResponse`, validating identity consistency and deriving SAS.
    pub fn initiator_receive_response(
        &mut self,
        response: &PairingResponse,
    ) -> Result<SasVerification> {
        let remote_pubkey = PublicKey::from_bytes(&response.public_key)?;
        if remote_pubkey.node_id() != response.node_id {
            return Err(IdentityError::KeyMismatch {
                node_id: response.node_id,
                expected: response.node_id.to_hex(),
                presented: remote_pubkey.node_id().to_hex(),
            });
        }

        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let sas = SasVerification::derive(
            &self.local_key.node_id(),
            &response.node_id,
            &self.local_nonce,
            &response.nonce,
        );

        let remote_peer = TrustedPeer {
            node_id: response.node_id,
            public_key: remote_pubkey,
            device_name: response.device_name.clone(),
            device_type: response.device_type,
            first_paired_at: now,
            last_seen_at: now,
            trust_state: TrustState::Pending,
        };

        self.remote_peer = Some(remote_peer);
        self.remote_nonce = Some(response.nonce);
        self.sas = Some(sas.clone());

        Ok(sas)
    }

    /// Returns the local human-readable device name.
    pub fn local_name(&self) -> &str {
        &self.local_name
    }

    /// Returns the local device type.
    pub fn local_type(&self) -> DeviceType {
        self.local_type
    }

    /// Returns the derived SAS verification value, if available.
    pub fn sas(&self) -> Option<&SasVerification> {
        self.sas.as_ref()
    }

    /// Returns the active remote peer record, if negotiated.
    pub fn remote_peer(&self) -> Option<&TrustedPeer> {
        self.remote_peer.as_ref()
    }

    /// Generates a human-approved `PairingConfirm` signature over the SAS digest.
    pub fn confirm(&self) -> Result<PairingConfirm> {
        let sas = self.sas.as_ref().ok_or_else(|| {
            IdentityError::PairingFailed("Cannot confirm pairing before SAS derivation".to_string())
        })?;

        let commitment = self.compute_confirmation_hash(sas);
        let signature = self.local_key.sign(&commitment);

        Ok(PairingConfirm {
            node_id: self.local_key.node_id(),
            confirmed: true,
            signature: signature.to_vec(),
        })
    }

    /// Verifies the remote peer's `PairingConfirm` message.
    /// If valid and human approved, transitions remote peer to `TrustState::Trusted` and returns it.
    pub fn finalize(&mut self, confirmation: &PairingConfirm) -> Result<TrustedPeer> {
        if !confirmation.confirmed {
            return Err(IdentityError::PairingFailed(
                "Remote peer rejected SAS confirmation".to_string(),
            ));
        }

        let sas = self.sas.as_ref().ok_or_else(|| {
            IdentityError::PairingFailed(
                "Cannot finalize pairing before SAS derivation".to_string(),
            )
        })?;
        let commitment = self.compute_confirmation_hash(sas);

        let remote = self.remote_peer.as_mut().ok_or_else(|| {
            IdentityError::PairingFailed("No active remote peer in pairing session".to_string())
        })?;

        if confirmation.node_id != remote.node_id {
            return Err(IdentityError::KeyMismatch {
                node_id: confirmation.node_id,
                expected: remote.node_id.to_hex(),
                presented: confirmation.node_id.to_hex(),
            });
        }

        let sig_bytes: [u8; 64] = confirmation.signature.as_slice().try_into().map_err(|_| {
            IdentityError::Crypto(
                "Invalid confirmation signature length: expected 64 bytes".to_string(),
            )
        })?;

        // Verify remote confirmation signature over commitment hash
        remote.public_key.verify(&commitment, &sig_bytes)?;

        // Transition to fully trusted
        remote.trust_state = TrustState::Trusted;
        Ok(remote.clone())
    }

    fn compute_confirmation_hash(&self, sas: &SasVerification) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(b"BridgeOS-Pairing-Confirm-v1");
        hasher.update(sas.formatted_pin.as_bytes());

        if let (Some(remote), Some(rn)) = (self.remote_peer.as_ref(), self.remote_nonce) {
            if self.local_key.node_id().0 <= remote.node_id.0 {
                hasher.update(self.local_nonce);
                hasher.update(rn);
            } else {
                hasher.update(rn);
                hasher.update(self.local_nonce);
            }
        } else {
            hasher.update(self.local_nonce);
        }

        let digest = hasher.finalize();
        digest.into()
    }
}
