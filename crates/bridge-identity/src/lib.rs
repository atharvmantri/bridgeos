//! Cryptographic identity, key generation, and peer verification for BridgeOS.

#![forbid(unsafe_code)]

pub mod keys;

pub use keys::{IdentityKey, PublicKey};
