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
- **Status:** DONE
- **Subsystem:** identity
- **Goal:** Implement explicit out-of-band verified peer pairing (SAS verification / QR codes) and persistent SQLite-backed cryptographic trust storage to establish trust boundaries beyond zero-trust LAN discovery.
- **Acceptance Criteria:**
  - `TrustStore` abstraction backed by SQLite database with schema versioning and transactional updates.
  - Stores trusted peer records (`node_id`, `public_key`, `device_name`, `device_type`, `first_paired_at`, `last_seen_at`, `trust_state`).
  - Strict separation of private device key (isolated file system storage with restricted permissions) and trusted peer directory (SQLite).
  - Explicit pairing ceremony state machine: initiation, mutual key exchange, symmetric SAS (Short Authentication String) derivation, human confirmation, persistence.
  - Rejection of unknown, unverified, or revoked peers during session establishment.
  - Rejection of spoofed NodeIds presenting a different public key than recorded in the trust store.
  - Comprehensive unit and integration tests covering successful pairing, user rejection, SAS symmetry, reconnection after restart, and trust revocation.
- **Relevant Files:** `crates/bridge-identity/Cargo.toml`, `crates/bridge-identity/src/trust.rs`, `crates/bridge-identity/src/pairing.rs`, `crates/bridge-identity/src/storage.rs`, `crates/bridge-identity/src/error.rs`, `crates/bridge-identity/src/lib.rs`, `crates/bridge-identity/tests/pairing_and_trust_test.rs`
- **Dependencies:** BRG-CORE-001, BRG-IDN-001, BRG-PROTO-001
- **Verification:** `cargo test -p bridge-identity`

---

### BRG-CLIP-001: Cross-Device Clipboard Synchronization Engine
- **Status:** DONE
- **Subsystem:** continuity
- **Goal:** Build the bidirectional clipboard synchronization engine (`bridge-clipboard`) for text and image payloads, with encryption, size budgeting, loopback echo suppression, and deduplication.
- **Acceptance Criteria:**
  - Clipboard payload representation supporting plain text, rich text, and bounded images (PNG/JPEG).
  - Loopback suppression using blake3 hash cache to prevent echo oscillations between connected peers.
  - Bounded clipboard size limits (e.g. max 10MB text/images) with graceful truncation or rejection.
  - OS clipboard integration abstraction / traits for platform adapters (Windows Win32, Android Jetpack).
  - Integration with `bridge_protocol::DataFrame` over `CHANNEL_CLIPBOARD`.
  - Comprehensive unit and integration tests verifying deduplication, payload framing, and echo avoidance.
- **Relevant Files:** `Cargo.toml`, `crates/bridge-clipboard/Cargo.toml`, `crates/bridge-clipboard/src/lib.rs`, `crates/bridge-clipboard/src/types.rs`, `crates/bridge-clipboard/src/guard.rs`, `crates/bridge-clipboard/src/policy.rs`, `crates/bridge-clipboard/src/backend.rs`, `crates/bridge-clipboard/src/engine.rs`, `crates/bridge-clipboard/tests/clipboard_test.rs`
- **Dependencies:** BRG-CORE-001, BRG-IDN-001, BRG-PAIR-001, BRG-PROTO-001, BRG-TRANS-001
- **Verification:** `cargo test -p bridge-clipboard`

---

### BRG-SESSION-001: Trusted Live Peer Session Orchestration & Pairing Integration
- **Status:** DONE
- **Subsystem:** session
- **Goal:** Build a unified session orchestration layer (`bridge-session`) integrating mutual Ed25519 authentication, SQLite `TrustStore` validation, and interactive/programmatic SAS pairing into live peer connections.
- **Acceptance Criteria:**
  - `ActiveSession` state machine: `Handshaking` -> `AuthenticatedUntrusted` -> `Pairing` -> `Trusted` -> `Closed`.
  - Rejection of key-substitution attacks (hard fail if a known `NodeId` presents a different public key than recorded in `TrustStore`).
  - Gated application channels: untrusted peers cannot send/receive clipboard or file transfer data.
  - Interactive/programmatic SAS pairing protocol over `CHANNEL_PAIRING` with `PairingConfirmation` callback.
  - CLI commands in `bridge-cli`: `node --data-dir`, `pair --peer`, `trust list`, `trust revoke`, and `peers` with trust indicators.
  - Comprehensive tests verifying handshake trust gating, key spoofing rejection, successful pairing transition, and revoked peer blocking.
- **Relevant Files:** `Cargo.toml`, `crates/bridge-session/`, `tools/bridge-cli/`
- **Dependencies:** BRG-CORE-001, BRG-IDN-001, BRG-PROTO-001, BRG-PAIR-001, BRG-TRANS-001
- **Verification:** `cargo test -p bridge-session && cargo test -p bridge-cli`

---

### BRG-CLIP-002: Live Authenticated & Trusted Clipboard Session Integration
- **Status:** DONE
- **Subsystem:** continuity
- **Goal:** Connect `ClipboardSyncEngine` into active trusted sessions with automatic broadcast to connected peers and incoming frame validation.
- **Acceptance Criteria:**
  - Outgoing local clipboard changes are automatically forwarded to all connected trusted peer sessions.
  - Incoming clipboard frames on `CHANNEL_CLIPBOARD` are validated and applied to the local backend only if the peer is `Trusted`.
  - Untrusted, revoked, or key-mismatched peers cannot inject clipboard content.
  - Loopback suppression and sequence deduplication work across real network sockets.
- **Relevant Files:** `crates/bridge-session/`, `tools/bridge-cli/src/node.rs`, `tools/bridge-cli/tests/cli_tests.rs`
- **Dependencies:** BRG-SESSION-001, BRG-CLIP-001
- **Verification:** `cargo test --workspace`

---

### BRG-WINCLIP-001: Native Windows Clipboard Backend
- **Status:** DONE
- **Subsystem:** continuity
- **Goal:** Implement a real native Windows clipboard backend in `bridge-clipboard` (`WindowsClipboardBackend`) implementing `ClipboardBackend` using safe Win32 API interactions.
- **Acceptance Criteria:**
  - Reads and writes Unicode UTF-16 text to/from Windows clipboard (`CF_UNICODETEXT`).
  - Gracefully handles clipboard locking/contention with exponential backoff retry.
  - Event-driven clipboard change monitoring using Win32 clipboard listener hooks (`AddClipboardFormatListener`).
  - Clean shutdown and background worker thread lifecycle.
  - Connects to `bridge-cli node` by default when running on Windows, with `--memory-clipboard` flag available.
  - Comprehensive unit and integration tests verifying text read/write, event notifications, and clean teardown.
- **Relevant Files:** `crates/bridge-clipboard/Cargo.toml`, `crates/bridge-clipboard/src/backend/mod.rs`, `crates/bridge-clipboard/src/backend/windows.rs`, `crates/bridge-clipboard/src/backend/memory.rs`, `tools/bridge-cli/src/node.rs`, `tools/bridge-cli/src/lib.rs`, `tools/bridge-cli/tests/cli_tests.rs`
- **Dependencies:** BRG-CLIP-001, BRG-SESSION-001, BRG-CLIP-002
- **Verification:** `cargo test -p bridge-clipboard -- --test-threads=1`

---

### BRG-NOTIF-001: Cross-Device Notification Mirroring Protocol & Engine
- **Status:** DONE
- **Subsystem:** continuity
- **Goal:** Design and implement cross-device notification synchronization (`bridge-notifications`) over protocol channel 3 (`CHANNEL_NOTIFICATIONS`) with notification state, actions (dismiss, reply), deduplication, and privacy filtering.
- **Acceptance Criteria:**
  - Standardized notification envelope (`NotificationEntry`) supporting app ID, title, text, timestamp, urgency, icon hash, and dismiss/action flags. ✅
  - Bounded ring buffer for active notifications and dismiss synchronization. ✅
  - Action synchronization allowing remote dismissal and canned text reply transmission. ✅
  - Privacy policy filtering sensitive notification categories (banking, 2FA codes, password managers). ✅
  - Integration with `bridge-session` application channels. *(follow-on: BRG-NOTIF-002)*
- **Relevant Files:** `crates/bridge-notifications/`
- **Dependencies:** BRG-CORE-001, BRG-PROTO-001, BRG-SESSION-001
- **Verification:** `cargo test -p bridge-notifications`

---

### BRG-NOTIF-002: Notification Session Integration & Windows Toast Emitter
- **Status:** TODO
- **Subsystem:** continuity
- **Goal:** Connect `NotificationSyncEngine` into `bridge-session` trusted session channels (analogous to BRG-CLIP-002), and implement a native Windows WinRT toast emitter backend.
- **Acceptance Criteria:**
  - Outgoing local OS notifications forwarded to all connected trusted peer sessions via `CHANNEL_NOTIFICATIONS`.
  - Incoming notification frames validated and applied to local OS notification center only if peer is `Trusted`.
  - Native Windows `WindowsNotificationBackend` using WinRT toast APIs or `windows-notifications` crate.
  - CLI integration in `bridge-cli node`.
  - Integration tests verifying trust gating on notification channel.
- **Relevant Files:** `crates/bridge-notifications/`, `crates/bridge-session/`, `tools/bridge-cli/`
- **Dependencies:** BRG-NOTIF-001, BRG-SESSION-001
- **Verification:** `cargo test --workspace`

---

### BRG-ANDROID-001: Android Core Protocol Module (Kotlin)
- **Status:** TODO
- **Subsystem:** android
- **Goal:** Implement the `core/` Kotlin module in `apps/android/` — BRG1 framing, NodeId derivation, ClientHello/ServerHello handshake — verified against golden interop fixtures in `tests/interop/fixtures/`.
- **Acceptance Criteria:**
  - Kotlin `BridgeFrame` encoder/decoder matching BRG1 magic + 4-byte BE length framing exactly.
  - `NodeId` derivation (SHA-256 of Ed25519 public key bytes) matching `node_id_derivation.json` golden vector.
  - `ClientHello` / `ServerHello` Postcard deserialization matching `handshake_client_hello.bin` / `handshake_server_hello.bin`.
  - Interop test reading golden fixtures and asserting byte-exact decode.
- **Relevant Files:** `apps/android/core/`, `tests/interop/fixtures/`
- **Dependencies:** BRG-PROTO-001 (interop fixtures must already exist)
- **Verification:** `./gradlew :core:test` in `apps/android/`

---

### BRG-RELAY-001: Standalone Encrypted Relay Daemon
- **Status:** TODO
- **Subsystem:** relay
- **Goal:** Implement a standalone Rust relay daemon (`services/relay`) providing zero-knowledge end-to-end encrypted relay for peers that cannot connect directly (NAT traversal fallback).
- **Acceptance Criteria:**
  - Relay daemon accepts authenticated peer connections and routes opaque encrypted frames between paired peers.
  - Zero-knowledge: relay cannot read payload content.
  - Rate limiting and session timeout enforcement.
  - Relay address configurable in `bridge-cli`.
- **Relevant Files:** `services/relay/`
- **Dependencies:** BRG-CORE-001, BRG-PROTO-001, BRG-IDN-001
- **Verification:** `cargo test -p bridge-relay`
