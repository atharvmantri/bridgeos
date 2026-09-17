//! Peer session orchestration, cryptographic handshake, trust verification,
//! out-of-band SAS pairing, and application channel dispatch for BridgeOS.

#![forbid(unsafe_code)]

pub mod error;
pub mod pairing_flow;
pub mod session;
pub mod state;

pub use error::SessionError;
pub use pairing_flow::{AutoConfirm, InteractiveCliConfirm, PairingConfirmation, PairingMessage};
pub use session::ActiveSession;
pub use state::SessionState;
