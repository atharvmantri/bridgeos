# BridgeOS: Architecture Decision Records (ADR)

This document records significant architectural and engineering decisions. Settled decisions must be adhered to unless explicitly superseded by a new ADR with clear justification.

---

## ADR-0001: Rust Monorepo Cargo Workspace
- **Date:** 2026-09-16
- **Status:** Accepted
- **Decision:** Use a single Cargo workspace monorepo containing modular crates (`bridge-core`, `bridge-protocol`, `bridge-identity`, `bridge-discovery`, `bridge-transport`, `bridge-transfer`, `apps/`, `services/`).
- **Reasoning:** Ensures atomic versioning, shared type definitions without publishing crates to crates.io during pre-1.0 development, unified CI/CD linting/testing, and zero dependency drift between client, daemon, and relay.
- **Alternatives Considered:** Multi-repo setup (rejected due to excessive coordination friction across repos during rapid early-stage iteration).
- **Consequences:** Workspace-level dependency versions must be maintained in root `Cargo.toml`.

---

## ADR-0002: Bounded Length-Delimited Framing with Magic Prefix
- **Date:** 2026-09-16
- **Status:** Accepted
- **Decision:** All stream-based connections use a 4-byte magic header (`BRG1`) followed by a 4-byte big-endian unsigned integer length prefix, capped at a maximum of 16 MB.
- **Reasoning:** Immediate rejection of spurious or malicious network noise; bounded frame lengths prevent Denial of Service via unbounded memory allocation (OOM).
- **Alternatives Considered:** Raw JSON lines / newline delimiting (rejected due to binary transfer inefficiency and escaping overhead); gRPC/HTTP2 (rejected due to unnecessary overhead for custom P2P streaming).
- **Consequences:** Transports must implement a framing codec (e.g. via `tokio_util::codec::Framed` or custom `AsyncRead`/`AsyncWrite` helper).

---

## ADR-0003: Cryptographic Primitives: Ed25519 Identity
- **Date:** 2026-09-16
- **Status:** Accepted
- **Decision:** Standard Ed25519 keypairs (`ed25519-dalek`) for node identity, challenge-response authentication, and device certificates.
- **Reasoning:** High speed, compact 32-byte public keys and 64-byte signatures, resistance to side-channel attacks, and widely audited implementation.
- **Alternatives Considered:** RSA (rejected: too large, slow, legacy); secp256k1 (rejected: tailored for blockchains, not general peer identity).
- **Consequences:** All nodes require Ed25519 keypair generation and storage.

---

## ADR-0004: Async Runtime: Tokio
- **Date:** 2026-09-16
- **Status:** Accepted
- **Decision:** Tokio is the mandatory async runtime for all Rust components.
- **Reasoning:** Tokio is the industry standard in the Rust ecosystem with first-class support from `quinn` (QUIC), `tokio-rustls`, `tokio-util`, and cross-platform networking primitives.
- **Alternatives Considered:** `async-std` (declining ecosystem momentum); `smol` (smaller ecosystem for QUIC/TLS).
- **Consequences:** All async I/O must remain compatible with Tokio's reactor.

---

## ADR-0005: Canonical Multi-Agent Repository OS
- **Date:** 2026-09-16
- **Status:** Accepted
- **Decision:** The repository functions as persistent memory. Universal entry point is `AGENTS.md`, complemented by `docs/STATE.md`, `docs/TASKS.md`, and targeted specs. Agent-specific files (`CLAUDE.md`, `GEMINI.md`) act only as thin pointers.
- **Reasoning:** Allows different coding agents to resume work cold without losing context or requiring massive chat transcripts. Avoids context drift across duplicated files.
- **Alternatives Considered:** Storing state only in chat instructions (leads to immediate context loss when starting new sessions); duplicating entire manual in each agent file (causes synchronization drift).
- **Consequences:** Agents must update `docs/STATE.md` and `docs/TASKS.md` when closing work.

---

## ADR-0006: Dedicated Byte Visitor for 64-Byte Ed25519 Signatures
- **Date:** 2026-09-16
- **Status:** Accepted
- **Decision:** Implement explicit byte and sequence visitor serde helper (`signature_serde`) for 64-byte Ed25519 signature arrays in wire handshake frames.
- **Reasoning:** Rust standard library and `serde` natively derive `Serialize`/`Deserialize` for array lengths up to 32 only. A custom visitor ensures zero-allocation byte slicing during Postcard deserialization and prevents type bloat.
- **Alternatives Considered:** External crate `serde_big_array` (adds extra dependency); `Vec<u8>` (allocates heap memory on every handshake packet).
- **Consequences:** Signatures remain stack-allocated fixed-size 64-byte arrays with compact binary encoding.

---

## ADR-0007: DNS-SD Service Discovery via `mdns-sd` with Heartbeat TTL Pruning
- **Date:** 2026-09-17
- **Status:** Accepted
- **Decision:** Use `mdns-sd` for zero-configuration LAN peer discovery under service type `_bridgeos._tcp.local.`, publishing device identity and capability bitflags via DNS-SD TXT records (`node_id`, `name`, `type`, `caps`, `ver`). Track peers locally in a thread-safe `PeerDirectory` with background TTL pruning and lifecycle event broadcast.
- **Reasoning:** Standardized DNS-SD enables cross-platform interoperability (Windows, macOS, Linux, Android) without requiring elevated raw socket privileges. TXT records provide pre-connection capability negotiation and node filtering before socket connection. Background TTL pruning reliably detects stale or dropped peers even on silent connection termination.
- **Alternatives Considered:** Raw UDP multicast beaconing (less standard across OS networks and often blocked or throttled by enterprise routers); central rendezvous server (violates zero-trust LAN offline continuity goal).
- **Consequences:** Network interfaces must allow multicast DNS traffic (UDP port 5353). A UDP broadcast beacon fallback will be implemented for restricted environments where mDNS is disabled.

---

## ADR-0008: Chunked Resumable File Streaming Engine with Blake3 Validation
- **Date:** 2026-09-17
- **Status:** Accepted
- **Decision:** Stream files in fixed 64KB chunks (`CHUNK_SIZE`) with per-chunk Blake3 checksums and a whole-file Blake3 root hash. Destination files are written to `.part` files during transit, truncated to the highest contiguous verified 64KB chunk boundary on resumption, and atomically finalized upon full file hash verification.
- **Reasoning:** 64KB chunks fit well within TCP/QUIC frames without fragmenting buffers or causing memory exhaustion. Per-chunk Blake3 hashes allow instant rejection of corrupted chunks without wasting bandwidth on the remainder of the file. Automatic truncation of `.part` files to the verified chunk boundary cleanly handles unexpected disconnections mid-chunk without corrupting state.
- **Alternatives Considered:** Streaming raw file streams without chunk checksums (rejected: requires whole-file retransfer if corruption occurs); SHA-256 chunking (rejected: Blake3 provides significantly higher throughput on both modern desktop CPUs and mobile devices).
- **Consequences:** File manifests must calculate whole-file Blake3 hashes before transfer; receivers track chunk sets and write verified chunks into temporary `.part` files.

---

## ADR-0009: UDP Broadcast Beacon Fallback & Unified Multi-Channel Discovery
- **Date:** 2026-09-17
- **Status:** Accepted
- **Decision:** Implement a bounded UDP broadcast beacon fallback mechanism using magic header `BRGB`, version 1 byte, and Postcard-serialized `BeaconMessage` payloads (`Announcement`, `Goodbye`, `Query`). Provide a `UnifiedDiscovery` abstraction that manages concurrent mDNS and UDP discovery channels over a shared, thread-safe `PeerDirectory`.
- **Reasoning:** In certain LANs, enterprise environments, or restrictive Wi-Fi access points, multicast DNS (UDP port 5353) is filtered or disabled. UDP broadcast (default port 42424) operates reliably on local subnets without multicast routing dependencies. Bounding datagrams to 1400 bytes prevents IP fragmentation. The immediate `Query` probe eliminates latency on network entry, while `Goodbye` avoids lingering stale entries.
- **Alternatives Considered:** UDP broadcast-only (rejected: mDNS is the cross-platform zero-config standard for macOS/iOS and zero-permission LAN environments); raw ICMP ping scans (rejected: unprivileged sockets cannot issue raw ICMP on Windows and Android).
- **Consequences:** Nodes can now discover each other across both mDNS-enabled and mDNS-restricted subnets; `PeerDirectory` merges and deduplicates multi-homed addresses discovered via different channels.

---

## ADR-0010: Developer Multi-Node CLI Harness Architecture (`tools/bridge-cli`)
- **Date:** 2026-09-17
- **Status:** Accepted
- **Decision:** Provide an interactive, developer-focused multi-node CLI harness (`tools/bridge-cli`) exposing commands `node`, `discover`, `ping`, `send-file`, and `identity`. Structure the crate with both library (`bridge_cli`) and binary (`bridge-cli`) targets to allow direct integration testing and embedding.
- **Reasoning:** Developing a cross-device P2P platform requires real, multi-terminal manual testing workflows before writing desktop UI or mobile applications. Building a developer harness validates that core crates (`bridge-core`, `bridge-identity`, `bridge-protocol`, `bridge-discovery`, `bridge-transport`, `bridge-transfer`) compose cleanly into an end-to-end operational node without code duplication.
- **Alternatives Considered:** Writing standalone throwaway scripts or manual test programs outside the workspace (rejected: causes code duplication and bit-rot as protocols evolve); relying purely on automated mock tests (rejected: fails to reveal real-world OS socket and terminal UX issues).
- **Consequences:** Developers can test full two-node continuity workflows (peer discovery, mutual Ed25519 authentication, capability exchange, ping latency, and chunked Blake3 file transfer) on localhost or across LAN devices directly from command-line terminals.

---

## ADR-0011: Out-of-Band SAS Pairing Ceremony & SQLite Persistent Trust Store
- **Date:** 2026-09-17
- **Status:** Accepted
- **Decision:** Enforce an explicit out-of-band Short Authentication String (SAS) 6-digit numeric PIN verification ceremony before any peer is marked trusted. Persist trusted peers and their identity keys in an embedded SQLite database (`TrustStore`) with WAL mode and schema migrations. Enforce strict public key consistency checks (`TrustStore::is_trusted`) that immediately reject any node attempting to present a previously trusted `NodeId` with a different public key. Persist local private identity keys exclusively through `IdentityStorage` with restricted file permissions (0600 on Unix) outside the SQLite trust database.
- **Reasoning:** In a zero-trust LAN model, network presence and mDNS/UDP discovery do NOT confer trust. Anyone can spin up a node with an arbitrary display name or claim a NodeId. Generating a symmetric Blake3-derived 6-digit SAS code with mutual cryptographic signature confirmation prevents man-in-the-middle (MITM) attacks during initial discovery. Storing public keys in an embedded SQLite database provides durable persistence across restarts, transaction safety, and quick queries without managing raw flat files. Storing local private keys separately ensures database corruption or accidental sharing of the trust database never exposes local private key material.
- **Consequences:** All discovered peers remain in `Untrusted` or `PendingPairing` state until human confirmation. Discovered peers presenting swapped keys are rejected with `IdentityError::KeyMismatch`. Local nodes require writable directory access for `trust.db` and private key files.

---

## ADR-0012: Cross-Device Clipboard Synchronization Architecture & Echo Suppression
- **Date:** 2026-09-17
- **Status:** Accepted
- **Decision:** Implement bidirectional clipboard synchronization via `bridge-clipboard` over protocol multiplex channel 1 (`DataFrame::CHANNEL_CLIPBOARD`). Use deterministic Blake3 hashing for all content representations (text, HTML, images) and enforce strict O(1) loopback echo suppression (`EchoGuard`) using bounded ring buffers and origin sequence monotonicity. Provide pluggable clipboard provider abstractions (`ClipboardBackend`) with an in-memory implementation for testing and headless CLI operation.
- **Reasoning:** Cross-device clipboard synchronization is prone to recursive ping-pong echo storms: when device B applies a clipboard update received from device A, device B's OS clipboard listener fires, which without suppression would broadcast the same text back to device A indefinitely. By hashing all outgoing and incoming content into a bounded `EchoGuard` ring buffer and tracking origin sequence numbers, local monitor triggers for recently received or transmitted hashes are immediately dropped before hitting the network. Enforcing size budgets (2MB text, 10MB images) and sensitive content filtering prevents clipboard memory exhaustion and accidental password/secret leakage.
- **Alternatives Considered:** Polling OS clipboard with timestamps (rejected: clock skew across devices makes timestamp ordering unreliable and causes missed copies); raw string comparisons (rejected: O(N) memory and compute cost for large images/rich text compared to fixed 32-byte Blake3 hashes).
---

## ADR-0013: Live Session Trust Gating, SAS Pairing Protocol & Reusable Session Architecture (`bridge-session`)
- **Date:** 2026-09-17
- **Status:** Accepted
- **Decision:** Introduce a dedicated, reusable `bridge-session` crate to orchestrate connection lifecycles across all BridgeOS frontends (CLI, Tauri desktop, Android). The session transitions through explicit states: `Handshaking` $\to$ `AuthenticatedUntrusted` $\to$ `Pairing` $\to$ `Trusted` (or `Terminated`). Upon connection, mutual Ed25519 authentication verifies the peer's public key; `TrustStore` is immediately consulted. If a known NodeId presents a mismatched key, the session immediately aborts (`SessionError::KeyMismatch`). If the peer is untrusted, all application channels (channel 1 clipboard, channel 2 file transfer) are hard-blocked at the framing layer. Pairing messages are exchanged over dedicated channel 5 (`CHANNEL_PAIRING`), deriving an identical 6-digit numeric SAS code via Blake3 HKDF. Only after both users confirm the SAS code is the peer persisted to `trust.db` and promoted to `Trusted`.
- **Reasoning:** Moving session orchestration out of `tools/bridge-cli` into `bridge-session` ensures that future graphical clients (Tauri desktop and Android Jetpack) share identical zero-trust security guarantees, handshake logic, and pairing state machines. Zero trust on LAN demands that discovery metadata (mDNS/UDP) is NEVER accepted as proof of identity. Hard-gating data channels at the framed layer guarantees unauthenticated or untrusted peers can never inject clipboard contents or stream files.
- **Alternatives Considered:** Embedding pairing and trust logic directly into `bridge-cli` (rejected: would force code duplication in desktop and mobile apps); allowing untrusted peers to send clipboard data under confirmation prompts (rejected: vulnerable to denial of service, prompt fatigue, and unverified data injection).
- **Consequences:** All BridgeOS nodes enforce strict zero-trust gating on live TCP/QUIC streams; un-paired peers must complete SAS verification before data channels activate; CLI and graphical frontends share the identical `PairingConfirmation` abstraction.

---

## ADR-0014: Native Windows Clipboard Integration via Win32 Format Listeners (`bridge-clipboard`)
- **Date:** 2026-09-17
- **Status:** Accepted
- **Decision:** Implement a real, event-driven Windows clipboard backend (`WindowsClipboardBackend`) for `bridge-clipboard` using direct Win32 API bindings via `windows-sys` (`0.59`) with minimal subsystem features (`Win32_Foundation`, `Win32_Graphics_Gdi`, `Win32_System_DataExchange`, `Win32_System_Memory`, `Win32_UI_WindowsAndMessaging`). The backend spawns a dedicated message-pump thread owning an invisible message-only window (`HWND_MESSAGE`) registered with `AddClipboardFormatListener`. The backend reads and writes Unicode UTF-16 text (`CF_UNICODETEXT`) using `GlobalAlloc`/`GlobalLock`/`GlobalUnlock`/`SetClipboardData`, handles OS-level clipboard access contention with exponential backoff retries, and coordinates in-process access serialization with `Arc<Mutex<()>>`. A `--memory-clipboard` flag is provided in `bridge-cli node` for headless environments, containerized testing, or user-selected memory isolation.
- **Reasoning:** Cross-device clipboard continuity cannot rely on polling: polling generates excessive CPU wakeups, introduces latency between copy and paste, misses brief intermediate updates, and causes avoidable contention with other Windows applications. Win32 `AddClipboardFormatListener` dispatches `WM_CLIPBOARDUPDATE` messages instantaneously when any application modifies clipboard contents. Using standard `windows-sys` maintains minimal binary size without bulky GUI frameworks. Contention handling is essential on Windows because other applications (browsers, office suites, password managers) frequently hold the clipboard open briefly during copy operations; backoff retries ensure BridgeOS does not crash or lose clipboard events under normal desktop workload contention.
- **Alternatives Considered:** Polling `GetClipboardSequenceNumber` on a sleep loop (rejected: high CPU overhead, jitter, and missed copies); external third-party clipboard crates like `arboard` or `copypasta` (rejected: they lack asynchronous event-driven listeners and would force inefficient polling loops); pretending `MemoryClipboardBackend` is sufficient for OS clipboard sync (rejected: violates zero-placeholder rule; BridgeOS requires authentic native continuity).
- **Consequences:** Windows nodes achieve instant, zero-polling, bidirectional text clipboard synchronization with connected trusted peers. Non-Windows platforms cleanly fall back to `MemoryClipboardBackend` until native Linux (`wl-clipboard`/`x11`) and Android backends are added. Unsafe code is strictly limited to isolated Win32 FFI blocks with thorough safety documentation and lifetime bounds.


