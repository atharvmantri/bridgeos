//! Wire protocol definitions, frame encodings, handshakes, and codecs for BridgeOS.

#![forbid(unsafe_code)]

pub mod codec;
pub mod frame;
pub mod handshake;

pub use codec::{decode_frame, decode_payload, encode_frame};
pub use frame::{ControlFrame, DataFrame, DisconnectReason, Frame, MAGIC, MAX_FRAME_LENGTH};
pub use handshake::{AuthResponse, AuthResult, ClientHello, HandshakeFrame, ServerHello};
