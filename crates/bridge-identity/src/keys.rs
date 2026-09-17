use bridge_core::{BridgeError, NodeId, Result};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};

/// Cryptographic device identity keypair backed by Ed25519.
#[derive(Debug, Clone)]
pub struct IdentityKey {
    signing_key: SigningKey,
}

impl IdentityKey {
    /// Generates a new cryptographically secure random Ed25519 identity key using OS CSPRNG.
    pub fn generate() -> Self {
        let mut rng = OsRng;
        let signing_key = SigningKey::generate(&mut rng);
        Self { signing_key }
    }

    /// Reconstitutes an identity key from raw 32-byte secret key material.
    pub fn from_bytes(bytes: &[u8; 32]) -> Self {
        let signing_key = SigningKey::from_bytes(bytes);
        Self { signing_key }
    }

    /// Exposes the 32-byte secret key material.
    pub fn to_bytes(&self) -> [u8; 32] {
        self.signing_key.to_bytes()
    }

    /// Returns the public verifying key.
    pub fn public_key(&self) -> PublicKey {
        PublicKey(self.signing_key.verifying_key())
    }

    /// Derives the canonical 32-byte `NodeId` from this key's public key.
    pub fn node_id(&self) -> NodeId {
        self.public_key().node_id()
    }

    /// Signs a message using Ed25519.
    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        let signature: Signature = self.signing_key.sign(message);
        signature.to_bytes()
    }

    /// Signs a mutual authentication challenge composed of client and server nonces.
    pub fn sign_handshake_challenge(
        &self,
        client_nonce: &[u8; 32],
        server_nonce: &[u8; 32],
    ) -> [u8; 64] {
        let mut hasher = Sha256::new();
        hasher.update(b"BridgeOS-Handshake-v1");
        hasher.update(client_nonce);
        hasher.update(server_nonce);
        let digest = hasher.finalize();
        self.sign(&digest)
    }
}

/// Public verifying key of a peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublicKey(pub VerifyingKey);

impl PublicKey {
    pub fn from_bytes(bytes: &[u8; 32]) -> Result<Self> {
        let verifying_key = VerifyingKey::from_bytes(bytes)
            .map_err(|e| BridgeError::Identity(format!("Invalid Ed25519 public key bytes: {e}")))?;
        Ok(Self(verifying_key))
    }

    pub fn to_bytes(&self) -> [u8; 32] {
        self.0.to_bytes()
    }

    pub fn node_id(&self) -> NodeId {
        NodeId::from_bytes(self.to_bytes())
    }

    pub fn verify(&self, message: &[u8], signature_bytes: &[u8; 64]) -> Result<()> {
        let signature = Signature::from_bytes(signature_bytes);
        self.0
            .verify_strict(message, &signature)
            .map_err(|e| BridgeError::Identity(format!("Signature verification failed: {e}")))?;
        Ok(())
    }

    pub fn verify_handshake_challenge(
        &self,
        client_nonce: &[u8; 32],
        server_nonce: &[u8; 32],
        signature_bytes: &[u8; 64],
    ) -> Result<()> {
        let mut hasher = Sha256::new();
        hasher.update(b"BridgeOS-Handshake-v1");
        hasher.update(client_nonce);
        hasher.update(server_nonce);
        let digest = hasher.finalize();
        self.verify(&digest, signature_bytes)
    }
}

impl serde::Serialize for PublicKey {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.to_bytes().serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for PublicKey {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let bytes = <[u8; 32]>::deserialize(deserializer)?;
        PublicKey::from_bytes(&bytes).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identity_key_generation_and_node_id() {
        let key1 = IdentityKey::generate();
        let key2 = IdentityKey::generate();

        assert_ne!(key1.node_id(), key2.node_id());
        assert_eq!(key1.node_id().as_bytes(), &key1.public_key().to_bytes());
    }

    #[test]
    fn test_signing_and_verification() {
        let key = IdentityKey::generate();
        let message = b"Hello, BridgeOS zero-trust network!";
        let sig = key.sign(message);

        let pubkey = key.public_key();
        assert!(pubkey.verify(message, &sig).is_ok());

        // Tampered message should fail verification
        let tampered = b"Tampered message";
        assert!(pubkey.verify(tampered, &sig).is_err());
    }

    #[test]
    fn test_handshake_challenge_verification() {
        let client_key = IdentityKey::generate();
        let client_nonce = [1u8; 32];
        let server_nonce = [2u8; 32];

        let signature = client_key.sign_handshake_challenge(&client_nonce, &server_nonce);

        // Verification with matching nonces succeeds
        assert!(client_key
            .public_key()
            .verify_handshake_challenge(&client_nonce, &server_nonce, &signature)
            .is_ok());

        // Verification with corrupted nonce fails
        let wrong_nonce = [3u8; 32];
        assert!(client_key
            .public_key()
            .verify_handshake_challenge(&wrong_nonce, &server_nonce, &signature)
            .is_err());
    }
}
