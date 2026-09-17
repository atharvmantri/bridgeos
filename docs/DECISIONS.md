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


