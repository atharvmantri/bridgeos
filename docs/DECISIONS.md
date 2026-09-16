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
