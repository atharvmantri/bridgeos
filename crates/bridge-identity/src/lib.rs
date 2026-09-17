//! Cryptographic identity, key generation, persistent trust storage, and explicit pairing for BridgeOS.

#![forbid(unsafe_code)]
#![allow(
    clippy::missing_panics_doc,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss
)]

pub mod error;
pub mod keys;
pub mod pairing;
pub mod storage;
pub mod trust;

pub use error::{IdentityError, Result};
pub use keys::{IdentityKey, PublicKey};
pub use pairing::{
    PairingConfirm, PairingRequest, PairingResponse, PairingSession, SasVerification,
};
pub use storage::IdentityStorage;
pub use trust::{TrustState, TrustStore, TrustedPeer};
