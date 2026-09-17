# BridgeOS: Current State Ledger

*Last updated: 2026-09-17 (Milestone 2 BRG-XFER-001 Completed & Verified)*

---

### What Currently Works?
- **Universal Multi-Agent OS:** `AGENTS.md` universal entry point, thin agent pointers (`CLAUDE.md`, `GEMINI.md`, `.cursorrules`), full docs suite (`docs/`).
- **Workspace & Core Types (`bridge-core`):** `NodeId`, `ProtocolVersion`, `Capabilities` bitmask, `DeviceType`, `BridgeError`, `FromStr` & hex conversions.
- **Identity & Cryptography (`bridge-identity`):** Ed25519 keypair generation, public key verification, mutual challenge signing and verification over OS CSPRNG.
- **Wire Protocol & Codec (`bridge-protocol`):** `BRG1` magic header, bounded 16MB length prefix framing, postcard serialization envelopes, `ClientHello`/`ServerHello`/`AuthResponse`/`AuthResult` handshake payloads.
- **Async Transport (`bridge-transport`):** Length-delimited `FramedStream<T>` over Tokio async read/write with clean EOF handling.
- **LAN Discovery & Peer Directory (`bridge-discovery`):** Zero-config mDNS / DNS-SD advertising (`_bridgeos._tcp.local.`), peer browsing, TXT record metadata parsing (`node_id`, `name`, `type`, `caps`, `ver`), thread-safe `PeerDirectory` with heartbeat freshness tracking, explicit departure handling (`ServiceRemoved`), and TTL expiration pruning.
- **Resumable Chunked Streaming Transfer (`bridge-transfer`):** 64KB chunk streaming (`FileSender`), disk/memory chunk ingestion & verification (`FileReceiver`), per-chunk Blake3 checksums, whole-file root hash validation, automatic `.part` file truncation to verified chunk boundary upon resumption, and protocol wire framing (`TransferMessage` on `CHANNEL_FILE_TRANSFER`).
- **Integration Test Suite (`tests/integration`):**
  - Milestone 0: Local in-memory and TCP socket mutual Ed25519 authentication, capability negotiation, and framed data transfer.
  - Milestone 1: Multi-node live mDNS discovery, dynamic socket resolution, automated TCP connection establishment, and authenticated session negotiation.
  - Milestone 2: Multi-node TCP file streaming, whole-file root hash validation, and interrupted session resumption from last verified chunk offset.
- **Continuous Integration (`.github/workflows/ci.yml`):** Ubuntu and Windows matrix testing with strict clippy and formatting checks.

### What Is Partially Implemented?
- `bridge-discovery`: UDP broadcast beacon fallback for networks blocking mDNS multicast.

### What Is Broken?
- Nothing. All 21 unit and integration tests pass with zero warnings under `cargo test` and `cargo clippy --all-targets -- -D warnings`.

### What Was Most Recently Completed?
- **Milestone 2 Resumable File Streaming (`BRG-XFER-001`)**:
  - Implemented `FileSender` supporting filesystem files and memory buffers with arbitrary chunk seeking.
  - Implemented `FileReceiver` supporting destination directories with `.part` staging and memory buffers.
  - Implemented automatic truncation of incomplete trailing writes to last contiguous 64KB verified boundary.
  - Implemented Blake3 per-chunk and whole-file root hash calculation and verification.
  - Added wire protocol envelopes `TransferMessage::{Offer, Accept, Data, Ack, Finished, Complete, Cancel}` with `DataFrame` encapsulation.
  - Added 7 unit tests in `crates/bridge-transfer/tests/transfer_test.rs`.
  - Added multi-node TCP streaming and network drop resumption integration tests in `tests/integration/tests/milestone2_transfer.rs`.
  - Documented ADR-0008 in `docs/DECISIONS.md`.

### Major Known Issues
- None.

### What Should The Next Agent Do?
1. Implement UDP broadcast beacon fallback in `bridge-discovery` for mDNS-restricted subnets.
2. Build CLI developer harness in `tools/` for launching and interacting with BridgeOS peer nodes from the terminal.
3. Begin Milestone 3 (Clipboard sync engine) or Relay daemon service.
