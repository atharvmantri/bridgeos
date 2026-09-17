# BridgeOS: Current State Ledger

*Last updated: 2026-09-17 (Milestone 1 BRG-DISC-002 Completed & Verified)*

---

### What Currently Works?
- **Universal Multi-Agent OS:** `AGENTS.md` universal entry point, thin agent pointers (`CLAUDE.md`, `GEMINI.md`, `.cursorrules`), full docs suite (`docs/`).
- **Workspace & Core Types (`bridge-core`):** `NodeId`, `ProtocolVersion`, `Capabilities` bitmask, `DeviceType`, `BridgeError`, `FromStr` & hex conversions.
- **Identity & Cryptographic Trust (`bridge-identity`):**
  - Ed25519 keypair generation, public key verification, mutual challenge signing and verification over OS CSPRNG.
  - SQLite-backed persistent `TrustStore` for authorized peers with WAL mode, schema versioning, and transactional updates.
  - Strict zero-trust LAN enforcement: unknown peers are untrusted until an out-of-band pairing ceremony is completed.
  - Cryptographic impersonation detection: instantly detects and rejects attempts to spoof a known `NodeId` using a different public key.
  - Explicit pairing ceremony state machine (`PairingSession`) with symmetric 6-digit SAS (Short Authentication String) derivation and commitment signatures.
  - Secure isolated disk storage (`IdentityStorage`) for private device keys with restricted file permissions (`0o600` on Unix).
  - Full revocation lifecycle preventing revoked devices from establishing sessions.
- **Wire Protocol & Codec (`bridge-protocol`):** `BRG1` magic header, bounded 16MB length prefix framing, postcard serialization envelopes, `ClientHello`/`ServerHello`/`AuthResponse`/`AuthResult` handshake payloads.
- **Async Transport (`bridge-transport`):** Length-delimited `FramedStream<T>` over Tokio async read/write with clean EOF handling.
- **LAN Discovery & Peer Directory (`bridge-discovery`):**
  - Zero-config mDNS / DNS-SD advertising (`_bridgeos._tcp.local.`), peer browsing, TXT record metadata parsing (`node_id`, `name`, `type`, `caps`, `ver`).
  - UDP broadcast beaconing (`BRGB` magic prefix, versioned Postcard envelopes) with immediate `Query` probe elicitation, periodic heartbeat beaconing, and explicit `Goodbye` departure.
  - Multi-channel `UnifiedDiscovery` coordinating concurrent mDNS and UDP beacon fallback over a shared `PeerDirectory` with address deduplication and TTL expiration pruning.
- **Resumable Chunked Streaming Transfer (`bridge-transfer`):** 64KB chunk streaming (`FileSender`), disk/memory chunk ingestion & verification (`FileReceiver`), per-chunk Blake3 checksums, whole-file root hash validation, automatic `.part` file truncation to verified chunk boundary upon resumption, and protocol wire framing (`TransferMessage` on `CHANNEL_FILE_TRANSFER`).
- **Cross-Device Clipboard Synchronization (`bridge-clipboard`):**
  - Bidirectional text, rich text HTML, and image synchronization (`ClipboardContent`, `ClipboardEntry`).
  - Strict O(1) loopback echo suppression (`EchoGuard`) preventing ping-pong oscillations when local clipboard updates are applied.
  - Monotonic sequence tracking per origin node preventing out-of-order or duplicate message injection.
  - Bounded size budgeting (2MB text, 10MB images) and sensitive password/credential filtering (`ClipboardPolicy`).
  - Pluggable platform clipboard backend abstraction (`ClipboardBackend`) with thread-safe `MemoryClipboardBackend`.
  - Wire protocol integration over `DataFrame` channel 1 (`DataFrame::CHANNEL_CLIPBOARD`).
- **Developer Multi-Node CLI Harness (`tools/bridge-cli`)**:
  - Interactive terminal executable `bridge-cli` with subcommands: `node`, `discover`, `ping`, `send-file`, `identity`.
  - Enables launching and manually testing multiple BridgeOS nodes across terminals on localhost or local LAN.
  - Full end-to-end integration of `bridge-core`, `bridge-identity`, `bridge-protocol`, `bridge-discovery`, `bridge-transport`, and `bridge-transfer`.
- **Integration Test Suite (`tests/integration`, `tools/bridge-cli/tests`, `crates/bridge-identity/tests`, `crates/bridge-clipboard/tests`):**
  - Milestone 0: Local in-memory and TCP socket mutual Ed25519 authentication, capability negotiation, and framed data transfer.
  - Milestone 1: Multi-node live mDNS discovery AND UDP broadcast beacon discovery, dynamic socket resolution, automated TCP connection establishment, and authenticated session negotiation.
  - Milestone 2: Multi-node TCP file streaming, whole-file root hash validation, and interrupted session resumption from last verified chunk offset.
  - Milestone 2 Trust: In-memory and SQLite disk persistence, public key consistency enforcement, impersonation attack rejection, peer revocation, and symmetric SAS pairing finalization.
  - Milestone 3 Clipboard: Payload hashing, wire encoding/decoding, bounded ring-buffer deduplication, loopback echo suppression, sequence tracking, policy enforcement, and image synchronization.
  - CLI Harness: Command-line parsing, node lifecycle, authenticated ping/pong roundtrips, multi-chunk file transfer, and cryptographic signature rejection.
- **Continuous Integration (`.github/workflows/ci.yml`):** Ubuntu and Windows matrix testing with strict clippy and formatting checks.

### What Is Partially Implemented?
- None in current milestones (M0, M1, M2 transfer & pairing, and M3 clipboard sync core are fully implemented and verified).

### What Is Broken?
- Nothing. All 46 unit and integration tests pass with zero warnings under `cargo test` and `cargo clippy --all-targets -- -D warnings`.

### What Was Most Recently Completed?
- **Cross-Device Clipboard Synchronization Engine (`BRG-CLIP-001`)**:
  - Built `crates/bridge-clipboard` supporting text, HTML, and compressed image payloads with deterministic Blake3 content hashing.
  - Implemented `EchoGuard` bounded ring-buffer deduplicator providing O(1) loopback suppression and monotonic sequence validation.
  - Implemented `ClipboardPolicy` enforcing size limits, format restrictions, and sensitive credential protection.
  - Implemented `ClipboardBackend` trait and `MemoryClipboardBackend` for testing and headless execution.
  - Implemented `ClipboardSyncEngine` coordinating backend monitoring, wire packaging on `CHANNEL_CLIPBOARD`, and incoming frame verification.
  - Added full test suite in `crates/bridge-clipboard/tests/clipboard_test.rs`.
  - Documented ADR-0012 in `docs/DECISIONS.md`.

### Major Known Issues
- None.

### What Should The Next Agent Do?
1. Integrate `bridge-clipboard` and `TrustStore` into `bridge-cli` node session handling for live terminal-to-terminal clipboard sync.
2. Implement cross-device Notification Mirroring protocol and engine (`bridge-notifications`).
3. Implement Standalone Relay daemon service (`services/relay`).
