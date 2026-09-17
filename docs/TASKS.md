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
- **Status:** DONE
- **Subsystem:** transfer
- **Goal:** Stream files in 64KB chunks with Blake3 per-chunk and whole-file hashes.
- **Acceptance Criteria:** Resumes interrupted transfers from last verified chunk offset; automatically validates per-chunk and root Blake3 checksums; truncates trailing invalid writes on resumption; supports memory and disk targets.
- **Relevant Files:** `crates/bridge-transfer/src/lib.rs`, `crates/bridge-transfer/src/error.rs`, `crates/bridge-transfer/src/manifest.rs`, `crates/bridge-transfer/src/message.rs`, `crates/bridge-transfer/src/sender.rs`, `crates/bridge-transfer/src/receiver.rs`, `crates/bridge-transfer/tests/transfer_test.rs`, `tests/integration/tests/milestone2_transfer.rs`
- **Dependencies:** BRG-CORE-001, BRG-PROTO-001, BRG-TRANS-001
- **Verification:** `cargo test -p bridge-transfer && cargo test --test milestone2_transfer`

---

### BRG-DISC-002: UDP Broadcast Beacon Fallback & Unified LAN Discovery
- **Status:** DONE
- **Subsystem:** discovery
- **Goal:** Implement UDP broadcast beacon fallback for restricted LANs blocking mDNS multicast, and provide unified discovery coordinating mDNS and UDP beaconing over a shared peer directory.
- **Acceptance Criteria:**
  - Bounded beacon packet encoding/decoding with magic header `BRGB`, protocol version, and Postcard envelope.
  - Broadcast beacon sender and listener with configurable broadcast port, targets, and beacon interval.
  - Immediate `Query` probe on startup to rapidly discover peers without waiting for next beacon interval.
  - Explicit `Goodbye` announcement on shutdown to notify peers of departure immediately.
  - Protects against self-discovery and safely discards malformed/oversized packets.
  - Address list deduplication and merge in `PeerDirectory` with TTL heartbeat pruning.
  - `UnifiedDiscovery` coordinating both mDNS and UDP fallback with shared directory and event bus.
  - Comprehensive unit and integration tests verifying discovery, query response, goodbye departure, and TTL expiry.
- **Relevant Files:** `Cargo.toml`, `crates/bridge-discovery/Cargo.toml`, `crates/bridge-discovery/src/beacon.rs`, `crates/bridge-discovery/src/udp.rs`, `crates/bridge-discovery/src/unified.rs`, `crates/bridge-discovery/src/peer.rs`, `crates/bridge-discovery/src/error.rs`, `crates/bridge-discovery/src/lib.rs`, `crates/bridge-discovery/tests/udp_discovery_test.rs`
- **Dependencies:** BRG-CORE-001, BRG-DISC-001
- **Verification:** `cargo test -p bridge-discovery`

---

### BRG-CLI-001: Multi-Node Developer CLI Harness (`tools/bridge-cli`)
- **Status:** DONE
- **Subsystem:** tools
- **Goal:** Build an interactive developer CLI (`bridge-cli`) for launching and testing multiple BridgeOS peer nodes from the terminal, with discovery, authenticated sessions, keepalives, and file streaming.
- **Acceptance Criteria:**
  - `bridge-cli node --name <NAME> [--port <PORT>] [--receive-dir <DIR>]` runs an active peer node with live discovery, TCP listener, incoming connection handling, and chunked file reception.
  - `bridge-cli discover [--duration <SECS>]` scans the LAN using `UnifiedDiscovery` and lists active peers with address, type, and capabilities.
  - `bridge-cli ping --peer <ADDR>` establishes an authenticated TCP session, verifies Ed25519 identity, performs ping/pong, and reports roundtrip latency.
  - `bridge-cli send-file --peer <ADDR> --file <PATH>` streams an on-disk file to a remote node using `bridge-transfer` (`FileSender`), verifying Blake3 chunk and root hashes.
  - `bridge-cli identity [--generate]` displays or generates Ed25519 identity keys and public `NodeId`.
  - Comprehensive `--help` documentation explaining two-node execution in separate terminal windows.
  - Integration and unit tests validating CLI commands, ping/pong roundtrips, file streaming, and cryptographic signature rejection.
- **Relevant Files:** `Cargo.toml`, `tools/bridge-cli/Cargo.toml`, `tools/bridge-cli/src/lib.rs`, `tools/bridge-cli/src/main.rs`, `tools/bridge-cli/src/node.rs`, `tools/bridge-cli/src/client.rs`, `tools/bridge-cli/tests/cli_tests.rs`
- **Dependencies:** BRG-CORE-001, BRG-IDN-001, BRG-PROTO-001, BRG-TRANS-001, BRG-DISC-001, BRG-DISC-002, BRG-XFER-001
- **Verification:** `cargo test -p bridge-cli && cargo run -p bridge-cli -- --help`

---

### BRG-PAIR-001: Explicit Secure Pairing & Persistent Trust Store
- **Status:** TODO
- **Subsystem:** identity
- **Goal:** Implement explicit out-of-band verified peer pairing (SAS verification / QR codes) and persistent SQLite-backed cryptographic trust storage to establish trust boundaries beyond zero-trust LAN discovery.
- **Acceptance Criteria:**
  - `TrustStore` abstraction backed by SQLite database with schema versioning and transactional updates.
  - Stores trusted peer records (`node_id`, `public_key`, `device_name`, `device_type`, `first_paired_at`, `last_seen_at`, `revocation_state`).
  - Strict separation of private device key (file system storage with restricted permissions) and trusted peer directory (SQLite).
  - Explicit pairing ceremony state machine: initiation, mutual key exchange, SAS (Short Authentication String) derivation, human confirmation, persistence.
  - Rejection of unknown, unverified, or revoked peers during session establishment.
  - Rejection of spoofed NodeIds presenting a different public key than recorded in the trust store.
  - Comprehensive unit and integration tests covering successful pairing, user rejection, SAS mismatch, reconnection after restart, and trust revocation.
- **Relevant Files:** `crates/bridge-identity/Cargo.toml`, `crates/bridge-identity/src/trust.rs`, `crates/bridge-identity/src/pairing.rs`, `crates/bridge-identity/src/lib.rs`
- **Dependencies:** BRG-CORE-001, BRG-IDN-001, BRG-PROTO-001
- **Verification:** `cargo test -p bridge-identity`


