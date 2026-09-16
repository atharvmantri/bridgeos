# BridgeOS: Architecture & Subsystems

This document describes the high-level architecture, module decomposition, and data flow of BridgeOS.

---

## 1. System Decomposition

BridgeOS is organized as a Rust workspace with distinct, decoupled crates:

```
                  ┌──────────────────────────────┐
                  │    UI Layer (Tauri / JNI)    │
                  └──────────────┬───────────────┘
                                 │
     ┌───────────────────────────┴───────────────────────────┐
     │                                                       │
┌────▼─────────────┐   ┌──────────────────┐   ┌──────────────▼─────┐
│  bridge-transfer │   │ bridge-discovery │   │   bridge-identity  │
│ (Streaming/Sync) │   │ (mDNS / Beacons) │   │  (Ed25519 Keys)    │
└────┬─────────────┘   └─────────┬────────┘   └──────────────┬─────┘
     │                           │                           │
     └───────────────────────────┼───────────────────────────┘
                                 │
                    ┌────────────▼───────────┐
                    │    bridge-transport    │
                    │ (QUIC / TLS-over-TCP)  │
                    └────────────┬───────────┘
                                 │
                    ┌────────────▼───────────┐
                    │    bridge-protocol     │
                    │   (Framing / Codecs)   │
                    └────────────┬───────────┘
                                 │
                    ┌────────────▼───────────┐
                    │      bridge-core       │
                    │(Shared Types & Errors) │
                    └────────────────────────┘
```

### Subsystems

1. **`bridge-core`**:
   - Fundamental types: `NodeId`, `DeviceId`, `DeviceType`, `ProtocolVersion`, `Capabilities`.
   - Core error handling traits and workspace error types.
   - Configuration models and shared utilities.

2. **`bridge-protocol`**:
   - Canonical wire frames, message serialization/deserialization.
   - Handshake frame definitions, capability negotiation payloads.
   - Control signals (Ping, Pong, Disconnect, AuthChallenge, AuthResponse).

3. **`bridge-identity`**:
   - Ed25519 cryptographic identity generation and storage.
   - Public key fingerprinting and deterministic `NodeId` derivation.
   - Mutual pairing challenges, PIN-based SAS (Short Authentication Strings), and cryptographic signatures.

4. **`bridge-discovery`**:
   - Local network discovery via mDNS / DNS-SD (`_bridgeos._tcp.local`).
   - UDP broadcast beaconing for restricted LAN environments.
   - Peer table management and freshness tracking with heartbeat timeouts.

5. **`bridge-transport`**:
   - Transport abstraction trait: `AsyncRead` + `AsyncWrite` framing over streams.
   - Length-delimited framing codec with strict frame size bounds.
   - Multiplexed channel management (control channel, data streaming channels).
   - QUIC (`quinn`) and TCP with TLS fallback.

6. **`bridge-transfer`**:
   - Chunked file transfer protocol with Blake3 checksum validation.
   - Resumable transfer state machines and rate limiting.

---

## 2. Concurrency & Async Model

- **Runtime:** Tokio multithreaded runtime.
- **Message Passing:** Asynchronous actors communicating via `tokio::sync::mpsc` channels and event distribution via `tokio::sync::broadcast`.
- **Zero Blocking in Async:** All disk I/O for file transfers uses `tokio::fs` or `tokio::task::spawn_blocking` to prevent starving the async reactor.

---

## 3. Boundary Contracts

- **Desktop UI (Tauri):** Rust backend exposes Tauri commands that return serializable JSON/Protobuf models and emit events into the frontend webview.
- **Android Integration:** Kotlin service wraps the Rust core via JNI (or UniFFI) exporting clean lifecycle methods (`start_service`, `connect_peer`, `send_file`).
