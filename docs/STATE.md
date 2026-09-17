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
- **Developer Multi-Node CLI Harness (`tools/bridge-cli`)**:
  - Interactive terminal executable `bridge-cli` with subcommands: `node`, `discover`, `ping`, `send-file`, `identity`.
  - Enables launching and manually testing multiple BridgeOS nodes across terminals on localhost or local LAN.
  - Full end-to-end integration of `bridge-core`, `bridge-identity`, `bridge-protocol`, `bridge-discovery`, `bridge-transport`, and `bridge-transfer`.
- **Integration Test Suite (`tests/integration` & `tools/bridge-cli/tests`):**
  - Milestone 0: Local in-memory and TCP socket mutual Ed25519 authentication, capability negotiation, and framed data transfer.
  - Milestone 1: Multi-node live mDNS discovery AND UDP broadcast beacon discovery, dynamic socket resolution, automated TCP connection establishment, and authenticated session negotiation.
  - Milestone 2: Multi-node TCP file streaming, whole-file root hash validation, and interrupted session resumption from last verified chunk offset.
  - CLI Harness: Command-line parsing, node lifecycle, authenticated ping/pong roundtrips, multi-chunk file transfer, and cryptographic signature rejection.
- **Continuous Integration (`.github/workflows/ci.yml`):** Ubuntu and Windows matrix testing with strict clippy and formatting checks.

### What Is Partially Implemented?
- None in current milestones (M0, M1, and M2 transfer core & CLI harness are fully implemented and verified).

### What Is Broken?
- Nothing. All 32 unit and integration tests pass with zero warnings under `cargo test` and `cargo clippy --all-targets -- -D warnings`.

### What Was Most Recently Completed?
- **Multi-Node Developer CLI Harness (`BRG-CLI-001`)**:
  - Implemented `tools/bridge-cli` with `node` daemon, `discover` scanner, `ping` probe, `send-file` streamer, and `identity` inspector.
  - Added dual binary (`bridge-cli`) and library (`bridge_cli`) targets for CLI execution and automated integration testing.
  - Implemented mutual Ed25519 authentication challenge verification on incoming and outgoing sessions.
  - Implemented whole-file Blake3 chunk streaming and `.part` file reassembly with throughput and progress reporting.
  - Built comprehensive unit & integration tests covering arguments, ping latency roundtrips, file transfer integrity, and imposter signature rejection.
  - Documented ADR-0010 in `docs/DECISIONS.md`.

### Major Known Issues
- None.

### What Should The Next Agent Do?
1. Implement Milestone 2 pairing ceremony & cryptographic trust store (`BRG-PAIR-001`: `bridge-identity` SQLite persistence and SAS numeric PIN / QR verification).
2. Begin Milestone 3 (Clipboard synchronization engine: `bridge-clipboard`).
3. Implement Standalone Relay daemon service (`services/relay`).
