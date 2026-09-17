use crate::error::SessionError;
use bridge_identity::{PairingConfirm, PairingRequest, PairingResponse, SasVerification};
use bridge_protocol::DataFrame;
use serde::{Deserialize, Serialize};

/// Wire messages exchanged over protocol multiplex channel `DataFrame::CHANNEL_PAIRING`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PairingMessage {
    /// Step 1: Initiator sends pairing request with public identity and nonce.
    Request(PairingRequest),
    /// Step 2: Responder sends pairing response with public identity and nonce.
    Response(PairingResponse),
    /// Step 3: Human-verified confirmation containing cryptographic commitment signature.
    Confirm(PairingConfirm),
    /// Pairing aborted or rejected.
    Failed { reason: String },
}

impl PairingMessage {
    /// Package the pairing message into a `DataFrame` on `DataFrame::CHANNEL_PAIRING`.
    pub fn to_data_frame(&self) -> Result<DataFrame, SessionError> {
        let payload = postcard::to_allocvec(self)
            .map_err(|e| SessionError::Protocol(format!("Failed to encode PairingMessage: {e}")))?;
        Ok(DataFrame::new(DataFrame::CHANNEL_PAIRING, payload))
    }

    /// Decode a `PairingMessage` from a `DataFrame`.
    pub fn from_data_frame(frame: &DataFrame) -> Result<Self, SessionError> {
        if frame.channel != DataFrame::CHANNEL_PAIRING {
            return Err(SessionError::Protocol(format!(
                "Expected channel {}, got {}",
                DataFrame::CHANNEL_PAIRING,
                frame.channel
            )));
        }

        postcard::from_bytes(&frame.payload)
            .map_err(|e| SessionError::Protocol(format!("Failed to decode PairingMessage: {e}")))
    }
}

/// Abstract interface to ask human/user for out-of-band SAS PIN confirmation.
pub trait PairingConfirmation: Send + Sync {
    fn confirm(
        &self,
        sas: &SasVerification,
        peer_name: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send>>;
}

/// Automatic confirmation callback for unit testing and automated integration tests.
#[derive(Debug, Clone, Copy)]
pub struct AutoConfirm(pub bool);

impl PairingConfirmation for AutoConfirm {
    fn confirm(
        &self,
        _sas: &SasVerification,
        _peer_name: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send>> {
        let val = self.0;
        Box::pin(async move { val })
    }
}

/// Interactive console stdin confirmation callback for the CLI.
#[derive(Debug, Clone, Default)]
pub struct InteractiveCliConfirm;

impl PairingConfirmation for InteractiveCliConfirm {
    fn confirm(
        &self,
        sas: &SasVerification,
        peer_name: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send>> {
        let pin = sas.formatted_pin.clone();
        let name = peer_name.to_string();

        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                use std::io::{stdin, stdout, Write};
                println!("\n========================================================");
                println!("           EXPLICIT PAIRING REQUEST");
                println!("--------------------------------------------------------");
                println!("  Device:            {name}");
                println!("  Verification PIN:  \x1b[1;32m{pin}\x1b[0m");
                println!("========================================================");
                print!("Does this 6-digit PIN match the other screen? (y/N): ");
                let _ = stdout().flush();

                let mut input = String::new();
                if stdin().read_line(&mut input).is_ok() {
                    let trimmed = input.trim().to_lowercase();
                    trimmed == "y" || trimmed == "yes"
                } else {
                    false
                }
            })
            .await
            .unwrap_or(false)
        })
    }
}
