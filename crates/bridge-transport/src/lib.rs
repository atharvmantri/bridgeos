//! Asynchronous framing and transport primitives for BridgeOS.

#![forbid(unsafe_code)]

pub mod framed;

pub use framed::FramedStream;
