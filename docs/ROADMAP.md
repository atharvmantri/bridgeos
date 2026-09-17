# BridgeOS: Engineering Roadmap

This document outlines the phased development roadmap for BridgeOS from foundational protocol verification to multi-platform release.

---

## Milestone 0: Core Architecture & Authenticated Handshake (COMPLETED)
**Objective:** Prove core architecture, wire format, capability negotiation, and authenticated peer framing between two local processes.
- [x] Monorepo workspace setup with modular crates.
- [x] Universal multi-agent context system (`AGENTS.md`, `docs/`).
- [x] Core types, error enums, and node identities (`bridge-core`, `bridge-identity`).
- [x] Wire framing codec, message types, and serialization (`bridge-protocol`).
- [x] Async transport layer and length-delimited framed stream (`bridge-transport`).
- [x] Integration test harness verifying two local nodes handshaking and exchanging messages.

## Milestone 1: LAN Discovery & Peer Directory (COMPLETED)
**Objective:** Devices discover each other on the local network without manual IP entry.
- [x] mDNS / DNS-SD service broadcasting and discovery (`bridge-discovery`).
- [x] UDP beacon fallback for restrictive subnets (`bridge-discovery`).
- [x] Dynamic peer directory tracking discovered nodes, addresses, and signal freshness.
- [x] Unified multi-channel discovery coordinating mDNS and UDP beacon fallback over shared directory.

## Milestone 2: Pairing & Cryptographic Trust Store
**Objective:** Cryptographic mutual authentication and pairing ceremony.
- [x] Ed25519 keypair generation and secure on-disk identity persistence.
- [x] Short Authentication String (SAS) numeric PIN / QR verification ceremony.
- [x] SQLite-backed trusted device whitelist and permissions store.

## Milestone 3: Bi-directional Clipboard Synchronization
**Objective:** Real-time synchronization of text and images across paired devices.
- [ ] Windows clipboard listener and injector.
- [ ] Privacy filtering (e.g. ignoring password manager copies).
- [ ] Delta compression and image clipboard streaming.

## Milestone 4: Resumable Streaming File Transfer Engine
**Objective:** High-performance, fault-tolerant file streaming.
- [x] Chunked file streaming with Blake3 hash trees (`bridge-transfer`).
- [x] Interrupted transfer resumption and progress reporting.
- [ ] Transfer approval prompts and rate limiting.

## Milestone 5: Windows Desktop Application
**Objective:** Polished native desktop experience for Windows 10/11.
- [ ] Tauri v2 desktop shell with React & TypeScript.
- [ ] System tray daemon with quick action notifications.
- [ ] Windows Explorer context menu integration ("Send via BridgeOS").

## Milestone 6: Android Client
**Objective:** Native mobile continuity client.
- [ ] Android Jetpack Compose app wrapping the Rust core library.
- [ ] Background service for persistent discovery and connection maintenance.
- [ ] Android System Share Sheet integration and Notification Mirroring.

## Milestone 7: Encrypted Relay Daemon
**Objective:** Continuity when devices are on separate networks (NAT traversal).
- [ ] Zero-knowledge relay server in Rust.
- [ ] End-to-end encrypted packet forwarding without relay visibility.
