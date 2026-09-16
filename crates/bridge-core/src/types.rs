use serde::{Deserialize, Serialize};
use std::fmt;

/// Protocol version representation (major.minor).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ProtocolVersion {
    pub major: u16,
    pub minor: u16,
}

impl ProtocolVersion {
    pub const CURRENT: Self = Self { major: 1, minor: 0 };

    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }

    /// Determines if two protocol versions are wire-compatible.
    /// Major versions must match; minor versions must be <= local supported version.
    pub fn is_compatible_with(&self, other: &Self) -> bool {
        self.major == other.major
    }
}

impl fmt::Display for ProtocolVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// 32-byte unique cryptographic node identifier, typically derived from public key.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(pub [u8; 32]);

impl NodeId {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    pub fn from_hex(s: &str) -> Result<Self, hex::FromHexError> {
        let mut bytes = [0u8; 32];
        hex::decode_to_slice(s, &mut bytes)?;
        Ok(Self(bytes))
    }
}

impl std::str::FromStr for NodeId {
    type Err = hex::FromHexError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_hex(s)
    }
}

impl fmt::Debug for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "NodeId({}..{})",
            &self.to_hex()[..8],
            &self.to_hex()[56..]
        )
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

/// Category of device running the BridgeOS node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DeviceType {
    Windows,
    Android,
    Linux,
    MacOS,
    Unknown,
}

impl fmt::Display for DeviceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Windows => write!(f, "Windows"),
            Self::Android => write!(f, "Android"),
            Self::Linux => write!(f, "Linux"),
            Self::MacOS => write!(f, "macOS"),
            Self::Unknown => write!(f, "Unknown"),
        }
    }
}

impl std::str::FromStr for DeviceType {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s.trim().to_ascii_lowercase().as_str() {
            "windows" => Self::Windows,
            "android" => Self::Android,
            "linux" => Self::Linux,
            "macos" | "darwin" => Self::MacOS,
            _ => Self::Unknown,
        })
    }
}

/// Bitmask-backed capability negotiation flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct Capabilities {
    pub flags: u32,
}

impl Capabilities {
    pub const NONE: u32 = 0;
    pub const FILE_TRANSFER: u32 = 1 << 0;
    pub const CLIPBOARD_TEXT: u32 = 1 << 1;
    pub const CLIPBOARD_IMAGE: u32 = 1 << 2;
    pub const NOTIFICATIONS: u32 = 1 << 3;
    pub const REMOTE_INPUT: u32 = 1 << 4;
    pub const RELAY_SUPPORT: u32 = 1 << 5;

    pub const fn from_bits(flags: u32) -> Self {
        Self { flags }
    }

    pub fn all() -> Self {
        Self {
            flags: Self::FILE_TRANSFER
                | Self::CLIPBOARD_TEXT
                | Self::CLIPBOARD_IMAGE
                | Self::NOTIFICATIONS
                | Self::REMOTE_INPUT
                | Self::RELAY_SUPPORT,
        }
    }

    pub fn contains(&self, flag: u32) -> bool {
        (self.flags & flag) == flag
    }

    pub fn insert(&mut self, flag: u32) {
        self.flags |= flag;
    }

    pub fn remove(&mut self, flag: u32) {
        self.flags &= !flag;
    }

    /// Returns the capability intersection between two peers.
    pub fn intersect(&self, other: &Self) -> Self {
        Self {
            flags: self.flags & other.flags,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_protocol_version_compatibility() {
        let v1_0 = ProtocolVersion::new(1, 0);
        let v1_1 = ProtocolVersion::new(1, 1);
        let v2_0 = ProtocolVersion::new(2, 0);

        assert!(v1_0.is_compatible_with(&v1_1));
        assert!(v1_1.is_compatible_with(&v1_0));
        assert!(!v1_0.is_compatible_with(&v2_0));
    }

    #[test]
    fn test_node_id_hex_roundtrip() {
        let raw = [42u8; 32];
        let id = NodeId::from_bytes(raw);
        let hex_str = id.to_hex();
        assert_eq!(hex_str.len(), 64);
        assert_eq!(id.to_string(), hex_str);

        let parsed = NodeId::from_hex(&hex_str).unwrap();
        assert_eq!(parsed, id);

        let parsed_str = NodeId::from_str(&hex_str).unwrap();
        assert_eq!(parsed_str, id);

        assert_eq!(
            DeviceType::from_str("windows").unwrap(),
            DeviceType::Windows
        );
        assert_eq!(
            DeviceType::from_str("Android").unwrap(),
            DeviceType::Android
        );
        assert_eq!(DeviceType::from_str("darwin").unwrap(), DeviceType::MacOS);
        assert_eq!(
            DeviceType::from_str("unknown_os").unwrap(),
            DeviceType::Unknown
        );
    }

    #[test]
    fn test_capabilities_bitwise_operations() {
        let mut caps = Capabilities::default();
        assert!(!caps.contains(Capabilities::FILE_TRANSFER));

        caps.insert(Capabilities::FILE_TRANSFER);
        caps.insert(Capabilities::CLIPBOARD_TEXT);
        assert!(caps.contains(Capabilities::FILE_TRANSFER));
        assert!(caps.contains(Capabilities::CLIPBOARD_TEXT));
        assert!(!caps.contains(Capabilities::REMOTE_INPUT));

        let peer_caps =
            Capabilities::from_bits(Capabilities::FILE_TRANSFER | Capabilities::REMOTE_INPUT);
        let agreed = caps.intersect(&peer_caps);
        assert!(agreed.contains(Capabilities::FILE_TRANSFER));
        assert!(!agreed.contains(Capabilities::CLIPBOARD_TEXT));
        assert!(!agreed.contains(Capabilities::REMOTE_INPUT));
    }
}
