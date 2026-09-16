# BridgeOS: Testing Strategy & Harness

This document defines testing expectations, execution procedures, and harness patterns for BridgeOS.

---

## 1. Test Pyramid

1. **Unit Tests (Crate-level):**
   - Located in `src/` files or adjacent `tests/` directories within each crate.
   - Tests pure domain logic: framing, serialization, capability math, state transitions.
   - Must run in milliseconds with zero external network or filesystem side-effects.

2. **Integration Tests (`tests/integration`):**
   - End-to-end handshake, session establishment, and data transfer between multiple nodes.
   - Uses `tokio::io::duplex` for fast, deterministic in-memory transports that run reliably in any CI environment without needing socket permissions or firewall bypass.
   - Real socket tests (TCP / QUIC) on localhost for transport validation.

3. **Fuzzing & Security Tests:**
   - Protocol codecs and frame decoders must be fuzz-tested against malformed frames, oversized length prefixes, and truncated packets to prevent panics or excessive allocation.

---

## 2. Standard Test & Validation Commands

All agents must run and verify these checks before completing any task:

```bash
# 1. Format check
cargo fmt --all -- --check

# 2. Strict linter check
cargo clippy --workspace --all-targets -- -D warnings

# 3. Unit and integration tests
cargo test --workspace

# 4. Targeted crate test (faster for iterative development)
cargo test -p bridge-protocol
cargo test -p bridge-transport
cargo test -p bridge-core
```

---

## 3. In-Memory Transport Harness Pattern

When writing tests that involve two peers interacting over an async stream, avoid binding real TCP ports where possible. Prefer `tokio::io::duplex`:

```rust
use tokio::io::duplex;
use bridge_transport::FramedStream;

#[tokio::test]
async fn test_in_memory_peer_exchange() {
    let (client_io, server_io) = duplex(64 * 1024);
    let mut client = FramedStream::new(client_io);
    let mut server = FramedStream::new(server_io);

    tokio::spawn(async move {
        // Server actor loop
    });

    // Client actor execution
}
```
This guarantees:
- Complete isolation from external network state.
- Instant execution speed.
- Zero port collision issues when running tests in parallel (`cargo test -- --test-threads=...`).
