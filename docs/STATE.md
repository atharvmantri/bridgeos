# BridgeOS: Current State Ledger

*Last updated: 2026-09-17 (Milestone 1 BRG-DISC-001 Completed & Verified)*

---

### What Currently Works?
- **Universal Multi-Agent OS:** `AGENTS.md` universal entry point, thin agent pointers (`CLAUDE.md`, `GEMINI.md`, `.cursorrules`), full docs suite (`docs/`).
- **Workspace & Core Types (`bridge-core`):** `NodeId`, `ProtocolVersion`, `Capabilities` bitmask, `DeviceType`, `BridgeError`, `FromStr` & hex conversions.
- **Identity & Cryptography (`bridge-identity`):** Ed25519 keypair generation, public key verification, mutual challenge signing and verification over OS CSPRNG.
- **Wire Protocol & Codec (`bridge-protocol`):** `BRG1` magic header, bounded 16MB length prefix framing, postcard serialization envelopes, `ClientHello`/`ServerHello`/`AuthResponse`/`AuthResult` handshake payloads.
- **Async Transport (`bridge-transport`):** Length-delimited `FramedStream<T>` over Tokio async read/write with clean EOF handling.
- **LAN Discovery & Peer Directory (`bridge-discovery`):** Zero-config mDNS / DNS-SD advertising (`_bridgeos._tcp.local.`), peer browsing, TXT record metadata parsing (`node_id`, `name`, `type`, `caps`, `ver`), thread-safe `PeerDirectory` with heartbeat freshness tracking, explicit departure handling (`ServiceRemoved`), and TTL expiration pruning.
- **Integration Test Suite (`tests/integration`):**
  - Milestone 0: Local in-memory and TCP socket mutual Ed25519 authentication, capability negotiation, and framed data transfer.
  - Milestone 1: Multi-node live mDNS discovery, dynamic socket resolution, automated TCP connection establishment, and authenticated session negotiation.
- **Continuous Integration (`.github/workflows/ci.yml`):** Ubuntu and Windows matrix testing with strict clippy and formatting checks.

### What Is Partially Implemented?
- `bridge-discovery`: UDP broadcast beacon fallback for networks blocking mDNS multicast.
- `bridge-transfer`: Session manifest and Blake3 hash validation model. Chunk streaming engine pending Milestone 4.

### What Is Broken?
- Nothing. All 19 unit and integration tests pass with zero warnings under `cargo test` and `cargo clippy --all-targets -- -D warnings`.

### What Was Most Recently Completed?
- **Milestone 1 LAN Discovery (`BRG-DISC-001`)**:
  - Implemented `MdnsDiscovery` daemon using `mdns-sd`.
  - Implemented `PeerDirectory` with thread-safe `RwLock<HashMap<NodeId, DiscoveredPeer>>` and configurable TTL freshness pruning.
  - Implemented bidirectional TXT record formatting and parsing.
  - Added unit tests for peer directory CRUD, expiry pruning, and TXT serialization.
  - Added integration tests for live mDNS discovery, peer departure, and full discovery-to-handshake lifecycle over localhost TCP.

### Major Known Issues
- None.

### What Should The Next Agent Do?
1. Begin **Milestone 2 or BRG-XFER-001**: Implement chunked streaming file transfer engine in `crates/bridge-transfer`.
2. Implement UDP broadcast beacon fallback in `bridge-discovery` for mDNS-restricted subnets.
3. Build CLI harness in `tools/` for launching peer nodes from terminal.
