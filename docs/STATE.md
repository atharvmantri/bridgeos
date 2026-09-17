# BridgeOS: Current State Ledger

*Last updated: 2026-09-17 (Milestone 1 BRG-DISC-002 Completed & Verified)*

---

### What Currently Works?
- **Universal Multi-Agent OS:** `AGENTS.md` universal entry point, thin agent pointers (`CLAUDE.md`, `GEMINI.md`, `.cursorrules`), full docs suite (`docs/`).
- **Workspace & Core Types (`bridge-core`):** `NodeId`, `ProtocolVersion`, `Capabilities` bitmask, `DeviceType`, `BridgeError`, `FromStr` & hex conversions.
- **Identity & Cryptography (`bridge-identity`):** Ed25519 keypair generation, public key verification, mutual challenge signing and verification over OS CSPRNG.
- **Wire Protocol & Codec (`bridge-protocol`):** `BRG1` magic header, bounded 16MB length prefix framing, postcard serialization envelopes, `ClientHello`/`ServerHello`/`AuthResponse`/`AuthResult` handshake payloads.
- **Async Transport (`bridge-transport`):** Length-delimited `FramedStream<T>` over Tokio async read/write with clean EOF handling.
- **LAN Discovery & Peer Directory (`bridge-discovery`):**
  - Zero-config mDNS / DNS-SD advertising (`_bridgeos._tcp.local.`), peer browsing, TXT record metadata parsing (`node_id`, `name`, `type`, `caps`, `ver`).
  - UDP broadcast beaconing (`BRGB` magic prefix, versioned Postcard envelopes) with immediate `Query` probe elicitation, periodic heartbeat beaconing, and explicit `Goodbye` departure.
  - Multi-channel `UnifiedDiscovery` coordinating concurrent mDNS and UDP beacon fallback over a shared `PeerDirectory` with address deduplication and TTL expiration pruning.
- **Resumable Chunked Streaming Transfer (`bridge-transfer`):** 64KB chunk streaming (`FileSender`), disk/memory chunk ingestion & verification (`FileReceiver`), per-chunk Blake3 checksums, whole-file root hash validation, automatic `.part` file truncation to verified chunk boundary upon resumption, and protocol wire framing (`TransferMessage` on `CHANNEL_FILE_TRANSFER`).
- **Integration Test Suite (`tests/integration`):**
  - Milestone 0: Local in-memory and TCP socket mutual Ed25519 authentication, capability negotiation, and framed data transfer.
  - Milestone 1: Multi-node live mDNS discovery AND UDP broadcast beacon discovery, dynamic socket resolution, automated TCP connection establishment, and authenticated session negotiation.
  - Milestone 2: Multi-node TCP file streaming, whole-file root hash validation, and interrupted session resumption from last verified chunk offset.
- **Continuous Integration (`.github/workflows/ci.yml`):** Ubuntu and Windows matrix testing with strict clippy and formatting checks.

### What Is Partially Implemented?
- None in current milestones (M0, M1, and M2 transfer core are fully implemented and verified).

### What Is Broken?
- Nothing. All 30 unit and integration tests pass with zero warnings under `cargo test` and `cargo clippy --all-targets -- -D warnings`.

### What Was Most Recently Completed?
- **Milestone 1 UDP Broadcast Beacon Fallback & Unified Discovery (`BRG-DISC-002`)**:
  - Implemented bounded UDP beacon envelopes (`BRGB`, version 1, max 1400 bytes) with `Announcement`, `Goodbye`, and `Query` probes (`beacon.rs`).
  - Implemented `UdpDiscovery` daemon with `SO_REUSEADDR` broadcast socket binding, immediate query probes, periodic heartbeats, explicit goodbye broadcasts, self-discovery protection, and TTL expiry pruning (`udp.rs`).
  - Enhanced `PeerDirectory` to merge and deduplicate multi-homed addresses across discovery mechanisms.
  - Implemented `UnifiedDiscovery` coordinating mDNS and UDP fallback with shared directory and event bus (`unified.rs`).
  - Added unit tests in `beacon.rs`, `udp.rs`, and `unified.rs`.
  - Added integration tests for live UDP discovery, query probe immediate response, goodbye departure, and TTL expiry in `crates/bridge-discovery/tests/udp_discovery_test.rs`.
  - Added end-to-end UDP beacon discovery and TCP authenticated session test in `tests/integration/tests/milestone1_discovery.rs`.
  - Documented ADR-0009 in `docs/DECISIONS.md`.

### Major Known Issues
- None.

### What Should The Next Agent Do?
1. Build CLI developer harness in `tools/` for launching and interacting with BridgeOS peer nodes from the terminal.
2. Implement Milestone 2 pairing ceremony & cryptographic trust store (`bridge-identity` on-disk persistence and SAS numeric PIN / QR verification).
3. Begin Milestone 3 (Clipboard synchronization engine) or Relay daemon service.
