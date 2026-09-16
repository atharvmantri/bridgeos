//! Core shared types, error definitions, and fundamental abstractions for BridgeOS.

#![forbid(unsafe_code)]

pub mod error;
pub mod types;

pub use error::{BridgeError, Result};
pub use types::{Capabilities, DeviceType, NodeId, ProtocolVersion};
