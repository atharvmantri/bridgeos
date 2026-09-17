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
- **Cross-Device Clipboard Synchronization & Windows Integration (`bridge-clipboard`):**
  - Bidirectional text, rich text HTML, and image synchronization (`ClipboardContent`, `ClipboardEntry`).
  - Strict O(1) loopback echo suppression (`EchoGuard`) preventing ping-pong oscillations when local clipboard updates are applied.
  - Monotonic sequence tracking per origin node preventing out-of-order or duplicate message injection.
  - Bounded size budgeting (2MB text, 10MB images) and sensitive password/credential filtering (`ClipboardPolicy`).
  - Pluggable platform clipboard backend abstraction (`ClipboardBackend`) with thread-safe `MemoryClipboardBackend`.
  - Native Windows backend (`WindowsClipboardBackend`) utilizing Win32 message-only window (`HWND_MESSAGE`), `AddClipboardFormatListener`, `WM_CLIPBOARDUPDATE` event dispatch, Unicode UTF-16 (`CF_UNICODETEXT`) reading/writing, exponential backoff contention retry, and clean thread teardown.
  - Wire protocol integration over `DataFrame` channel 1 (`DataFrame::CHANNEL_CLIPBOARD`).
- **Developer Multi-Node CLI Harness (`tools/bridge-cli`)**:
  - Interactive terminal executable `bridge-cli` with subcommands: `node`, `discover`, `peers`, `pair`, `trust`, `ping`, `send-file`, `identity`.
  - Enables launching and manually testing multiple BridgeOS nodes across terminals on localhost or local LAN.
  - Default native OS clipboard integration on Windows with `--memory-clipboard` flag available for isolated/testing runs.
  - Full end-to-end integration of `bridge-core`, `bridge-identity`, `bridge-protocol`, `bridge-discovery`, `bridge-transport`, `bridge-transfer`, `bridge-clipboard`, and `bridge-session`.
- **Integration Test Suite (`tests/integration`, `tools/bridge-cli/tests`, `crates/bridge-identity/tests`, `crates/bridge-clipboard/tests`, `crates/bridge-session/tests`):**
  - Milestone 0: Local in-memory and TCP socket mutual Ed25519 authentication, capability negotiation, and framed data transfer.
  - Milestone 1: Multi-node live mDNS discovery AND UDP broadcast beacon discovery, dynamic socket resolution, automated TCP connection establishment, and authenticated session negotiation.
  - Milestone 2: Multi-node TCP file streaming, whole-file root hash validation, and interrupted session resumption from last verified chunk offset.
  - Milestone 2 Trust: In-memory and SQLite disk persistence, public key consistency enforcement, impersonation attack rejection, peer revocation, and symmetric SAS pairing finalization.
  - Milestone 3 Clipboard: Payload hashing, wire encoding/decoding, bounded ring-buffer deduplication, loopback echo suppression, sequence tracking, policy enforcement, image synchronization, and native Win32 clipboard read/write/event notification tests.
  - CLI & Session Harness: Command-line parsing, node lifecycle, authenticated ping/pong roundtrips, multi-chunk file transfer, cryptographic signature rejection, SAS pairing ceremonies, and application channel gating.
- **Continuous Integration (`.github/workflows/ci.yml`):** Ubuntu and Windows matrix testing with strict clippy and formatting checks.

### What Is Partially Implemented?
- None in current milestones (M0, M1, M2 transfer & pairing, and M3 clipboard sync core with Windows integration are fully implemented and verified).

### What Is Broken?
- Nothing. All 47 unit and integration tests pass with zero warnings under `cargo test` and `cargo clippy --all-targets -- -D warnings`.

### What Was Most Recently Completed?
- **Native Windows Clipboard Backend (`BRG-WINCLIP-001`)**:
  - Modularized `crates/bridge-clipboard/src/backend/` into `mod.rs`, `memory.rs`, and `windows.rs`.
  - Implemented `WindowsClipboardBackend` with Win32 message-only window (`HWND_MESSAGE`) and `AddClipboardFormatListener` for zero-polling, event-driven clipboard monitoring.
  - Implemented safe Unicode UTF-16 (`CF_UNICODETEXT`) reading and writing via `GlobalAlloc`, `GlobalLock`, `GlobalUnlock`, and `SetClipboardData`.
  - Added exponential backoff retry for OS clipboard contention (e.g., when another application holds the clipboard open).
  - Coordinated thread synchronization with `sync_lock: Arc<Mutex<()>>` preventing race conditions between the listener message pump and direct read/write calls.
  - Connected `WindowsClipboardBackend` into `bridge-cli node` by default on Windows, with `--memory-clipboard` flag for headless/testing runs.
  - Documented ADR-0014 in `docs/DECISIONS.md`.

### Major Known Issues
- None.

### What Should The Next Agent Do?
1. Implement cross-device Notification Mirroring protocol and engine (`BRG-NOTIF-001` in `crates/bridge-notifications`).
2. Implement Standalone Relay daemon service (`services/relay`).
3. Connect native Windows notification listener / toast emitter.
