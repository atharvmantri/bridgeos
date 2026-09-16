use bridge_core::{Capabilities, NodeId, ProtocolVersion};
use serde::{Deserialize, Serialize};

/// Handshake message types exchanged during initial peer session establishment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HandshakeFrame {
    /// Initiator initiates session with version and capabilities.
    ClientHello(ClientHello),

    /// Responder acknowledges with mutually agreed version and capability intersection.
    ServerHello(ServerHello),

    /// Initiator responds with Ed25519 signature over nonces.
    AuthResponse(AuthResponse),

    /// Responder issues final session authorization status.
    AuthResult(AuthResult),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientHello {
    pub version: ProtocolVersion,
    pub node_id: NodeId,
    pub device_name: String,
    pub client_nonce: [u8; 32],
    pub capabilities: Capabilities,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerHello {
    pub agreed_version: ProtocolVersion,
    pub node_id: NodeId,
    pub device_name: String,
    pub server_nonce: [u8; 32],
    pub negotiated_capabilities: Capabilities,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthResponse {
    #[serde(with = "signature_serde")]
    pub signature: [u8; 64],
}

mod signature_serde {
    use serde::{de::Error, Deserializer, Serializer};

    pub fn serialize<S>(sig: &[u8; 64], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_bytes(sig)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 64], D::Error>
    where
        D: Deserializer<'de>,
    {
        struct SigVisitor;

        impl<'de> serde::de::Visitor<'de> for SigVisitor {
            type Value = [u8; 64];

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a 64-byte signature")
            }

            fn visit_bytes<E>(self, v: &[u8]) -> Result<Self::Value, E>
            where
                E: Error,
            {
                if v.len() == 64 {
                    let mut arr = [0u8; 64];
                    arr.copy_from_slice(v);
                    Ok(arr)
                } else {
                    Err(E::custom(format!("expected 64 bytes, got {}", v.len())))
                }
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::SeqAccess<'de>,
            {
                let mut arr = [0u8; 64];
                for (i, slot) in arr.iter_mut().enumerate() {
                    *slot = seq
                        .next_element()?
                        .ok_or_else(|| Error::invalid_length(i, &"64 bytes"))?;
                }
                Ok(arr)
            }
        }

        deserializer.deserialize_bytes(SigVisitor)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthResult {
    pub success: bool,
    pub reason: Option<String>,
}

impl AuthResult {
    pub fn ok() -> Self {
        Self {
            success: true,
            reason: None,
        }
    }

    pub fn failed(reason: impl Into<String>) -> Self {
        Self {
            success: false,
            reason: Some(reason.into()),
        }
    }
}
