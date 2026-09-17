# BridgeOS

An open-source, self-hostable cross-device continuity platform built in safe, idiomatic Rust.

BridgeOS is designed to seamlessly interconnect desktop and mobile devices over local networks with zero third-party cloud dependencies, peer-to-peer authenticated transport, and high-performance streaming.

> [!WARNING]
> **Pre-Release / Active Foundation Development (v0.1.0-alpha)**
> BridgeOS is currently under active engineering development. Foundational protocols, authenticated transport, LAN discovery, and chunked streaming file transfer engines are fully implemented and verified with automated test suites. Desktop graphical interfaces, mobile client UI, and clipboard listeners are in active development. **Not yet recommended for production use.**

---

## Current Implemented Capabilities

Every capability listed below is fully implemented, strictly linted, and verified via automated workspace unit and integration tests:

- **Cryptographic Node Identity (`bridge-identity`):**
  - Cryptographically secure Ed25519 keypair generation using OS CSPRNG.
  - Public key fingerprinting and deterministic 32-byte `NodeId` derivation.
  - Mutual nonce challenge signing and cryptographic signature verification.

- **Bounded Wire Protocol & Framing (`bridge-protocol`):**
  - Protocol framing prefixed with 4-byte magic header `BRG1` (`0x42 0x52 0x47 0x31`) for immediate rejection of spurious network traffic.
  - Bounded 16 MB maximum frame length to prevent Out-Of-Memory (OOM) denial-of-service attacks.
  - Versioned binary envelope serialization via Postcard.
  - Three-way handshake payloads (`ClientHello`, `ServerHello`, `AuthResponse`, `AuthResult`) with capability bitflag intersection negotiation (`Capabilities`).

- **Async Framed Transport (`bridge-transport`):**
  - Asynchronous length-delimited `FramedStream<T>` wrapper over any `tokio::io::AsyncRead + AsyncWrite` stream (TCP, Unix domain sockets, in-memory duplex).
  - Clean TCP EOF termination and connection error propagation.
  - Multiplexed data channel encapsulation (`CHANNEL_CLIPBOARD`, `CHANNEL_FILE_TRANSFER`, `CHANNEL_NOTIFICATIONS`, `CHANNEL_REMOTE_INPUT`).

- **Multi-Channel LAN Peer Discovery (`bridge-discovery`):**
  - Zero-configuration DNS-SD / mDNS service advertising and browsing under `_bridgeos._tcp.local.` using `mdns-sd`.
  - DNS-SD TXT record encoding and parsing for pre-connection metadata (`node_id`, `name`, `type`, `caps`, `ver`).
  - UDP broadcast beacon fallback (`BRGB` magic prefix, port 42424) with bounded 1400-byte datagrams for subnets blocking multicast DNS.
  - Immediate `Query` probe elicitation for sub-second discovery upon entering a network, periodic heartbeats, and explicit `Goodbye` announcements upon shutdown.
  - Thread-safe `PeerDirectory` with background TTL heartbeat expiration pruning, self-discovery filtering, and multi-homed address deduplication.
  - `UnifiedDiscovery` coordinating concurrent mDNS and UDP beacon fallback over a shared peer directory.

- **Resumable Chunked File Transfer Engine (`bridge-transfer`):**
  - Fixed 64KB chunk streaming (`FileSender`) with arbitrary chunk offset seeking across files and memory buffers.
  - Receiver pipeline (`FileReceiver`) supporting temporary `.part` staging files, memory buffers, and automatic `.part` file truncation to the last contiguous verified chunk boundary upon resuming interrupted transfers.
  - Per-chunk Blake3 checksum validation and whole-file Blake3 root hash integrity verification.
  - Wire protocol message envelopes (`TransferMessage::{Offer, Accept, Data, Ack, Finished, Complete, Cancel}`) mapped onto protocol data channels.

---

## Security Architecture

1. **Zero-Trust LAN Boundary:**
   Physical presence on the same Wi-Fi or local subnet **never implies trust**. Discovery beacons (both mDNS and UDP) are treated strictly as unauthenticated transport hints.
2. **Mutual Cryptographic Authentication:**
   No application data or file payload is exchanged until both peers successfully verify Ed25519 digital signatures over mutual cryptographic nonces during the connection handshake.
3. **Bounded Allocations:**
   Strict buffer limits are enforced prior to allocation: maximum 1400 bytes for discovery datagrams, 64 bytes for device names, and 16 MB for stream frames.
4. **Audited Cryptographic Primitives:**
   BridgeOS never rolls custom ciphers. All identity and hashing mechanisms utilize established, widely audited crates: `ed25519-dalek`, `blake3`, `sha2`, and `rand`.

For the complete threat model and security specifications, see [SECURITY.md](docs/SECURITY.md).

---

## Workspace Layout

```
bridgeos/
├── AGENTS.md               # Universal multi-agent engineering operating manual
├── Cargo.toml              # Root Cargo workspace manifest
├── crates/
│   ├── bridge-core/        # Shared types: NodeId, DeviceType, ProtocolVersion, Capabilities
│   ├── bridge-protocol/    # Wire frames, magic headers, codecs, handshake messages
│   ├── bridge-identity/    # Ed25519 key generation, signing, challenge verification
│   ├── bridge-discovery/   # mDNS daemon, UDP broadcast fallback, unified peer directory
│   ├── bridge-transport/   # Async length-delimited framed stream (Tokio)
│   └── bridge-transfer/    # 64KB chunked resumable file streaming engine (Blake3)
├── apps/
│   ├── desktop/            # Tauri desktop shell with React & TypeScript (in development)
│   └── android/            # Android Kotlin client (in development)
├── services/
│   └── relay/              # Zero-knowledge end-to-end encrypted relay daemon (planned)
├── tools/                  # Developer CLI tools and test harnesses (in development)
├── tests/
│   └── integration/        # Multi-node integration test harness
└── docs/                   # Engineering ledgers and architectural documentation
    ├── ARCHITECTURE.md     # Subsystem decomposition and concurrency model
    ├── PROTOCOL.md         # Wire framing format and handshake specification
    ├── ROADMAP.md          # Multi-milestone progress and planning
    ├── TASKS.md            # Actionable engineering backlog
    ├── STATE.md            # Compact current-state ledger
    ├── DECISIONS.md        # Architecture Decision Records (ADRs)
    ├── SECURITY.md         # Threat model and trust boundaries
    ├── TESTING.md          # Test strategy and execution commands
    └── AI_USAGE.md         # Transparent record of AI-assisted engineering
```

---

## Getting Started

### Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) (stable 1.83+ recommended, edition 2021)
- Cargo (included with Rust)
- Operating Systems: Windows 10/11, Linux, macOS

### Building

Build the entire workspace in release or debug mode:

```bash
cargo build --workspace
```

### Running Tests

Run the complete test suite across all workspace crates and integration harnesses:

```bash
# Run all unit and multi-node integration tests
cargo test --workspace

# Run strict clippy linter across all targets
cargo clippy --workspace --all-targets -- -D warnings

# Verify formatting
cargo fmt --check
```

---

## Project Status & Roadmap

| Milestone | Subsystem | Status | Description |
|---|---|---|---|
| **M0** | Core & Handshake | **COMPLETED** | Workspace layout, framing codec, Ed25519 identity, authenticated handshake |
| **M1** | Discovery & Directory | **COMPLETED** | mDNS zero-config, UDP broadcast beacon fallback, dynamic peer directory |
| **M2** | Transfer Engine | **COMPLETED** | 64KB chunk streaming, Blake3 hash verification, interrupted transfer resumption |
| **M2** | Pairing & Trust Store | *In Progress* | Short Authentication String (SAS) numeric PIN verification, SQLite device store |
| **M3** | Clipboard Synchronization | *Planned* | Real-time bi-directional clipboard sync with privacy filtering |
| **M4** | Windows Desktop App | *Planned* | Tauri desktop shell, system tray daemon, Windows Explorer integration |
| **M5** | Android Client | *Planned* | Android Jetpack Compose continuity client, background discovery service |
| **M6** | Encrypted Relay Daemon | *Planned* | Zero-knowledge end-to-end encrypted relay for NAT traversal |

For detailed milestone breakdown, see [ROADMAP.md](docs/ROADMAP.md) and [TASKS.md](docs/TASKS.md).

---

## AI-Assisted Engineering Disclosure

In accordance with open engineering transparency, development on BridgeOS utilizes AI coding assistants operating under the strict engineering constraints specified in [AGENTS.md](AGENTS.md).

All architectural decisions, protocol revisions, and implementations are peer-verified through deterministic automated test suites (`cargo test`) and strict compiler checks (`-D warnings`). A transparent ledger of all significant AI contributions is maintained in [docs/AI_USAGE.md](docs/AI_USAGE.md).

---

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.
