//! Test harness and helpers for BridgeOS integration tests.

#![forbid(unsafe_code)]

use bridge_identity::IdentityKey;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Initializes structured logging for integration test runs if not already initialized.
pub fn init_test_logging() {
    let _ = tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer())
        .try_init();
}

/// Generates a test node identity with deterministic metadata.
pub fn create_test_node(name: &str) -> (IdentityKey, String) {
    (IdentityKey::generate(), name.to_string())
}
