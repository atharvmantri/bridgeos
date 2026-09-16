# BridgeOS: Engineering Tasks Backlog

This document is the actionable task tracker. Every piece of non-trivial engineering work must have an ID, defined acceptance criteria, and a concrete verification command.

---

## Active & Milestone 0 Tasks

### BRG-CORE-001: Workspace & Core Types Scaffolding
- **Status:** DONE
- **Subsystem:** core
- **Goal:** Establish Rust Cargo workspace with strict lints, shared error types, `NodeId`, `DeviceId`, `DeviceType`, `ProtocolVersion`, and `Capabilities` bitflags.
- **Acceptance Criteria:**
  - Workspace compiles with `#![forbid(unsafe_code)]` on all core crates.
  - `NodeId` can be derived from public keys and formatted as hex/base58.
  - `Capabilities` bitflags support serialization/deserialization.
  - Unit tests verify serialization and flag operations.
- **Relevant Files:** `Cargo.toml`, `crates/bridge-core/src/lib.rs`, `crates/bridge-core/src/types.rs`, `crates/bridge-core/src/error.rs`
- **Dependencies:** None
- **Verification:** `cargo test -p bridge-core`

---

### BRG-IDN-001: Ed25519 Cryptographic Node Identity
- **Status:** DONE
- **Subsystem:** identity
- **Goal:** Provide generation, signing, and verification of device identity keys using audited primitives.
- **Acceptance Criteria:**
  - `IdentityKey` generates cryptographically secure Ed25519 keypairs.
  - Signing data produces standard 64-byte Ed25519 signatures.
  - Verification validates signatures against public keys and rejects tampering.
  - Deterministic `NodeId` derivation from `PublicKey`.
- **Relevant Files:** `crates/bridge-identity/src/lib.rs`, `crates/bridge-identity/src/keys.rs`
- **Dependencies:** BRG-CORE-001
- **Verification:** `cargo test -p bridge-identity`

---

### BRG-PROTO-001: Wire Frame Delimiting & Envelopes
- **Status:** DONE
- **Subsystem:** protocol
- **Goal:** Implement the "BRG1" magic-prefixed length-delimited framing and serialization envelope.
- **Acceptance Criteria:**
  - Frame length is bounded (max 16MB) to prevent OOM DOS attacks.
  - Magic header `BRG1` is strictly enforced; invalid magic immediately errors.
  - Roundtrip serialization and deserialization tests pass for all core frame types.
- **Relevant Files:** `crates/bridge-protocol/src/lib.rs`, `crates/bridge-protocol/src/frame.rs`, `crates/bridge-protocol/src/codec.rs`
- **Dependencies:** BRG-CORE-001
- **Verification:** `cargo test -p bridge-protocol`

---

### BRG-PROTO-002: Capability Negotiation & Handshake Payloads
- **Status:** DONE
- **Subsystem:** protocol
- **Goal:** Define structured handshake messages (`ClientHello`, `ServerHello`, `AuthChallenge`, `AuthResponse`, `AuthResult`) with capability intersection logic.
- **Acceptance Criteria:**
  - `ClientHello` and `ServerHello` encode compatible protocol versions.
  - Incompatible major versions result in a clean version mismatch rejection.
  - Capability intersection correctly identifies mutually supported features.
- **Relevant Files:** `crates/bridge-protocol/src/handshake.rs`
- **Dependencies:** BRG-PROTO-001
- **Verification:** `cargo test -p bridge-protocol`

---

### BRG-TRANS-001: Async Framed Transport Stream
- **Status:** DONE
- **Subsystem:** transport
- **Goal:** Provide an async framed stream abstraction wrapping `tokio::io::AsyncRead + AsyncWrite`.
- **Acceptance Criteria:**
  - `FramedStream<T>` can send and receive protocol frames asynchronously over any Tokio duplex/stream.
  - Handles partial reads and writes gracefully without data corruption.
  - Unit tests verify in-memory bidirectional streaming via `tokio::io::duplex`.
- **Relevant Files:** `crates/bridge-transport/src/lib.rs`, `crates/bridge-transport/src/framed.rs`
- **Dependencies:** BRG-PROTO-001
- **Verification:** `cargo test -p bridge-transport`

---

### BRG-INT-001: Milestone 0 Local Process Interconnect
- **Status:** DONE
- **Subsystem:** integration
- **Goal:** Two local Tokio tasks connect over TCP / in-memory duplex, perform mutual handshake, verify identity, and exchange an authenticated message.
- **Acceptance Criteria:**
  - Client initiates connection to server.
  - Server and client exchange version and capabilities handshake.
  - Client proves identity key ownership via signed nonce challenge.
  - Ping/Pong keepalive and sample application data frame are successfully exchanged.
  - Graceful disconnect terminates connection cleanly.
- **Relevant Files:** `tests/integration/tests/milestone0_handshake.rs`
- **Dependencies:** BRG-CORE-001, BRG-IDN-001, BRG-PROTO-001, BRG-PROTO-002, BRG-TRANS-001
- **Verification:** `cargo test --test milestone0_handshake`

---

## Future Backlog Tasks

### BRG-DISC-001: mDNS Discovery Daemon
- **Status:** DONE
- **Subsystem:** discovery
- **Goal:** Broadcast and discover BridgeOS peers on the local LAN using mDNS / DNS-SD.
- **Acceptance Criteria:** Discovers peers within 2 seconds of network entry; handles peer departure on heartbeat expiry.
- **Relevant Files:** `crates/bridge-discovery/src/lib.rs`, `crates/bridge-discovery/src/peer.rs`, `crates/bridge-discovery/src/service.rs`, `crates/bridge-discovery/tests/discovery_test.rs`, `tests/integration/tests/milestone1_discovery.rs`
- **Dependencies:** BRG-CORE-001
- **Verification:** `cargo test -p bridge-discovery`

### BRG-XFER-001: Resumable File Streaming Engine
- **Status:** TODO
- **Subsystem:** transfer
- **Goal:** Stream files in 64KB chunks with Blake3 per-chunk and whole-file hashes.
- **Acceptance Criteria:** Resumes interrupted transfers from last verified chunk offset.
- **Verification:** `cargo test -p bridge-transfer`
