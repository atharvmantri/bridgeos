# BridgeOS: Current State Ledger

*Last updated: 2026-09-17 (BRG-NOTIF-001 Completed & Verified)*

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
- **Cross-Device Notification Mirroring (`bridge-notifications`):**
  - Portable `NotificationEntry` envelope supporting app ID, title, text, urgency (`Low`/`Normal`/`High`/`Critical`), state (`Active`/`Dismissed`/`Expired`), icon hash/data, is_ongoing flag, and size enforcement.
  - Four wire message types: `Post`, `Dismiss`, `ActionInvoked`, `ClearAll` — Postcard-encoded over `DataFrame::CHANNEL_NOTIFICATIONS` (channel 3).
  - Action synchronization: `Dismiss`, `Open`, `Reply { text }`, and `Custom { label, key }` actions.
  - `NotificationGuard`: O(1) dedup via Blake3 hash ring buffer (256 cap) with LRU eviction, dismiss state tracking, and `ClearAll` bulk dismiss.
  - `NotificationPolicy`: privacy filter blocking 2FA/OTP codes, banking apps, password manager apps, with configurable urgency threshold gating and ongoing notification suppression. Default/strict/permissive profiles.
  - `NotificationSyncEngine`: async inbound frame handler with policy + dedup pipeline, outbound message builders, broadcast event channel (`NotificationSyncEvent`), and active notification snapshot per origin.
- **Developer Multi-Node CLI Harness (`tools/bridge-cli`)**:
  - Interactive terminal executable `bridge-cli` with subcommands: `node`, `discover`, `peers`, `pair`, `trust`, `ping`, `send-file`, `identity`.
  - Enables launching and manually testing multiple BridgeOS nodes across terminals on localhost or local LAN.
  - Default native OS clipboard integration on Windows with `--memory-clipboard` flag available for isolated/testing runs.
  - Full end-to-end integration of `bridge-core`, `bridge-identity`, `bridge-protocol`, `bridge-discovery`, `bridge-transport`, `bridge-transfer`, `bridge-clipboard`, and `bridge-session`.
- **Protocol Interoperability Fixtures (`tests/interop/fixtures/`):**
  - 15 canonical golden binary/JSON vectors covering: BRG1 framing, NodeId derivation, ClientHello/ServerHello/AuthResponse/AuthResult handshake frames, Ping/Pong/Disconnect control frames, SAS derivation, Pairing Request/Response/Confirm frames, Clipboard Sync frame, and UDP beacon announcement.
  - Self-verifying Rust test (`tests/integration/tests/interop_fixtures.rs`) generates and validates all vectors on every `cargo test` run.
- **Integration Test Suite (`tests/integration`, `tools/bridge-cli/tests`, `crates/bridge-identity/tests`, `crates/bridge-clipboard/tests`, `crates/bridge-session/tests`, `crates/bridge-notifications/tests`):**
  - Milestone 0: Local in-memory and TCP socket mutual Ed25519 authentication, capability negotiation, and framed data transfer.
  - Milestone 1: Multi-node live mDNS discovery AND UDP broadcast beacon discovery, dynamic socket resolution, automated TCP connection establishment, and authenticated session negotiation.
  - Milestone 2: Multi-node TCP file streaming, whole-file root hash validation, and interrupted session resumption from last verified chunk offset.
  - Milestone 2 Trust: In-memory and SQLite disk persistence, public key consistency enforcement, impersonation attack rejection, peer revocation, and symmetric SAS pairing finalization.
  - Milestone 3 Clipboard: Payload hashing, wire encoding/decoding, bounded ring-buffer deduplication, loopback echo suppression, sequence tracking, policy enforcement, image synchronization, and native Win32 clipboard read/write/event notification tests.
  - Milestone 3 Notifications: Wire roundtrip (all message types), dedup same/updated content, dismiss lifecycle, clear-all, 2FA policy blocking, outbound builders, size limit enforcement, active snapshot query.
  - CLI & Session Harness: Command-line parsing, node lifecycle, authenticated ping/pong roundtrips, multi-chunk file transfer, cryptographic signature rejection, SAS pairing ceremonies, and application channel gating.
- **Continuous Integration (`.github/workflows/ci.yml`):** Ubuntu and Windows matrix testing with strict clippy and formatting checks.

### What Is Partially Implemented?
- **Android Client (`apps/android/`):** Root Gradle project scaffold only (`settings.gradle.kts`, `build.gradle.kts`). No Kotlin source modules yet.

### What Is Broken?
- `backend::windows::tests::test_windows_clipboard_set_get_unicode` is flaky when run with multi-threaded test harness (`GetLastError=1418` clipboard contention). Use `cargo test -p bridge-clipboard -- --test-threads=1` for reliable Windows clipboard test runs.

### What Was Most Recently Completed?
- **Cross-Device Notification Mirroring Engine (`BRG-NOTIF-001`)**:
  - New `crates/bridge-notifications` crate registered in workspace.
  - `NotificationEntry` portable envelope with size enforcement (`title` 256B, `text` 4096B, icon 64KB).
  - `NotificationUrgency` / `NotificationState` / `NotificationAction` typed enums.
  - `NotificationMessage` wire enum serialized with Postcard over `CHANNEL_NOTIFICATIONS`.
  - `NotificationGuard` bounded LRU ring-buffer dedup (cap 256) with dismiss and clear-all tracking.
  - `NotificationPolicy` privacy filter with 2FA/banking/password-manager/medical keyword and app-ID blocklists.
  - `NotificationSyncEngine` async inbound/outbound orchestrator with tokio broadcast event channel.
  - 28 tests (11 unit + 17 integration) — all passing, clippy clean, `cargo fmt` clean.
  - Committed golden interop protocol fixtures (15 binary/JSON vectors) in `tests/interop/fixtures/`.
  - Bootstrapped Android project Gradle scaffold.

### Major Known Issues
- None.

### What Should The Next Agent Do?
1. Connect `bridge-notifications` into `bridge-session` / `bridge-cli` (analogous to BRG-CLIP-002 clipboard integration).
2. Implement Windows native notification toast emitter backend using Windows Runtime (WinRT) toast APIs or `win32-toast`.
3. Continue Android client implementation: implement `core/` Kotlin module with BRG1 framing, NodeId derivation, and ClientHello/ServerHello handshake against golden fixtures.
4. Implement Standalone Relay daemon service (`services/relay`).
