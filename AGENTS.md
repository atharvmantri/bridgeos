# BridgeOS Agent Operating Manual

Welcome to BridgeOS. This repository is built to be developed iteratively and interchangeably by multiple AI coding agents (Gemini, Claude, Codex, Kiro, Cursor, etc.) across extended periods.

Follow this manual strictly. The repository itself is persistent memory.

---

## 1. What BridgeOS Is

BridgeOS is an open-source, self-hostable cross-device continuity platform.
- **Initial Target:** Windows PC $\leftrightarrow$ Android phone.
- **Long-term Target:** Seamless Windows, Android, Linux, and macOS interoperability.
- **Key Capabilities:** Peer-to-peer authenticated connections, LAN discovery, encrypted relay fallback, clipboard synchronization, fast & resumable file transfer, notification mirroring, remote input control, and native OS shell integration.

---

## 2. Technology Stack

- **Core & Protocols:** Rust (latest stable edition 2021/2024), Tokio async runtime.
- **Networking:** QUIC (`quinn`) / TLS 1.3 over TCP (`tokio-rustls`) + fallback channels.
- **Discovery:** mDNS / DNS-SD (`mdns-sd`) & authenticated local broadcast.
- **Identity & Security:** Ed25519 for node identity & signing, X25519 + ChaCha20-Poly1305 / AES-256-GCM for transport encryption. Established crates only (`ed25519-dalek`, `ring`, `rustls`). Never roll custom crypto.
- **Serialization:** Versioned binary envelopes (`postcard` / `bincode`) and canonical JSON where human-readable debugging is required.
- **State & Storage:** SQLite (`rusqlite` / `sqlx`).
- **Windows Desktop UI:** Tauri + React + TypeScript.
- **Android App:** Kotlin + Android Jetpack.
- **Relay Server:** Standalone Rust daemon.

---

## 3. Repository Map

```
bridgeos/
├── AGENTS.md               # [Universal Entry Point] You are here
├── CLAUDE.md               # Pointer to AGENTS.md for Claude Code
├── GEMINI.md               # Pointer to AGENTS.md for Gemini / Antigravity
├── Cargo.toml              # Root workspace manifest
├── docs/                   # Canonical context and engineering ledger
│   ├── PROJECT.md          # Vision, scope, personas, capability matrix
│   ├── ARCHITECTURE.md     # Subsystem layout, data flow, thread model
│   ├── PROTOCOL.md         # Wire protocol, framing, handshakes, versioning
│   ├── ROADMAP.md          # Multi-milestone horizon (M0 to M5)
│   ├── TASKS.md            # Actionable engineering backlog (ID, status, spec)
│   ├── STATE.md            # Compact current-state ledger (Read every session)
│   ├── DECISIONS.md        # Architecture Decision Records (ADRs)
│   ├── TESTING.md          # Test strategy, fixtures, harness, commands
│   ├── SECURITY.md         # Threat model, crypto primitives, trust boundaries
│   └── AI_USAGE.md         # Transparent record of significant agent contributions
├── crates/
│   ├── bridge-core/        # Shared domain types, error handling, configuration
│   ├── bridge-protocol/    # Frame encodings, wire formats, capabilities, codecs
│   ├── bridge-identity/    # Key generation, device IDs, pairing certificates
│   ├── bridge-discovery/   # Local LAN beaconing, mDNS, peer announcement
│   ├── bridge-transport/   # Async connection abstractions, framing, Tokio QUIC/TCP
│   └── bridge-transfer/    # Chunked, resumable streaming file engine
├── apps/
│   ├── desktop/            # Tauri desktop frontend + native Windows integration
│   └── android/            # Android Kotlin client (Jetpack, NDK bridges)
├── services/
│   └── relay/              # Zero-knowledge end-to-end encrypted relay daemon
├── tools/                  # Developer CLI tools, test runners, protocol fuzzers
└── tests/
    └── integration/        # Multi-node integration test harness
```

---

## 4. Canonical Context Files

Never invent new locations for canonical data.
1. `docs/STATE.md`: **Ground truth of current progress.** What works, what is broken, what to do next. Keep it compact.
2. `docs/TASKS.md`: **Backlog.** Every piece of work must have an ID and clear acceptance criteria.
3. `docs/DECISIONS.md`: **ADRs.** Settled decisions stay settled unless formally superseded.
4. `docs/PROTOCOL.md`: **Wire contract.** Do not alter message payloads without updating this.

---

## 5. Session Startup Procedure (Cold Start)

When you enter this repository cold:
1. **Read `AGENTS.md`** (this file) to ground yourself in rules.
2. **Read `docs/STATE.md`** to know the current operational reality.
3. **Read `docs/TASKS.md`** to locate the highest-priority unblocked task.
4. **Read only relevant docs** (e.g. `docs/PROTOCOL.md` if working on framing; `docs/SECURITY.md` if touching keys). Do not read all docs.
5. **Inspect the relevant source files** under `crates/`.
6. **Execute, test, and verify** the task.

---

## 6. How Agents Choose and Update Tasks

- Check `docs/TASKS.md` for tasks marked `TODO` or `IN PROGRESS`.
- Claim the task by updating its status to `IN PROGRESS` if not already set.
- Respect task dependencies.
- Once completed and verified via its test command:
  1. Set status to `DONE`.
  2. Update `docs/STATE.md` with what was completed and what works now.
  3. If architectural choices were made, add an entry to `docs/DECISIONS.md`.
  4. Record the contribution type in `docs/AI_USAGE.md`.
  5. Select the next unblocked task and keep working.

---

## 7. Coding & Engineering Standards

- **Rust:** Idiomatic, safe Rust. `#![forbid(unsafe_code)]` in all crates unless strictly required for OS FFI (and then isolated with clear safety comments).
- **Error Handling:** Use `thiserror` for library crate error enums; use `anyhow` or concrete types in binaries/tests. No silent `.unwrap()` or `.expect()` in production paths.
- **Async:** Use Tokio primitives. Avoid blocking operations inside async tasks (`tokio::task::spawn_blocking` when needed).
- **Logging:** Use `tracing` crate (`tracing::info!`, `debug!`, `error!`, etc.). Never use raw `println!` in library code.

---

## 8. Testing & Validation Requirements

No code is complete without validation.
Before closing any task:
1. `cargo fmt --check` must pass.
2. `cargo clippy --workspace --all-targets -- -D warnings` must pass.
3. `cargo test --workspace` must pass.
4. Targeted unit or integration tests verifying the specific acceptance criteria must be present.

---

## 9. Security Expectations

- **Zero-Trust LAN:** Being on the same Wi-Fi does NOT mean a peer is trusted. All sessions must be authenticated via cryptographic key pairs.
- **No Hand-Rolled Crypto:** Only use standard, audited crates (`ed25519-dalek`, `ring`, `rustls`, `chacha20poly1305`).
- **No Secrets in Repo:** Never commit private keys, certificates, or tokens. Dev harnesses must generate ephemeral keys on the fly.
- **Input Validation:** All deserialized network packets must have maximum size limits enforced before allocation.

---

## 10. Commit Expectations

- Commit after each cohesive, verified milestone or task.
- Clear commit messages using Conventional Commits format:
  `feat(transport): add frame length delimiting decoder`
  `test(protocol): add handshake capability negotiation roundtrip`
  `docs(state): update completed Milestone 0 tasks`
- Never squash unrelated changes into huge blind commits.

---

## 11. Context & Token Efficiency Rules

Tokens are financially and cognitively scarce:
1. **Never do full repository scans** unless initializing or refactoring global traits.
2. **Read selectively**: Read line ranges or target files instead of entire trees.
3. **Avoid narration**: Use tools directly; do not produce pages of conversational chatter before calling a tool.
4. **Do not dump code in chat**: The user and other agents inspect files directly in the filesystem.
5. **Run targeted tests first**: e.g., `cargo test -p bridge-protocol` before the full workspace suite.

---

## 12. What Must NEVER Be Done

- ❌ **NEVER leave knowledge solely in conversation chat.** If an architectural constraint, bug, or decision is found, write it to `docs/`.
- ❌ **NEVER write mock or placeholder code that pretends to work.** Implement real, testable vertical slices.
- ❌ **NEVER design custom cryptographic handshakes or ciphers.**
- ❌ **NEVER disable security/verification checks without an explicit ADR in `docs/DECISIONS.md`.**
- ❌ **NEVER stop autonomously when the next task is clear, unblocked, and ready.** Keep building.
