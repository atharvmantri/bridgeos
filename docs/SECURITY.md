# BridgeOS: Security Architecture & Threat Model

BridgeOS coordinates sensitive cross-device operations including file transfers, clipboard contents, device inputs, and notifications. Security is an architectural prerequisite, not an afterthought.

---

## 1. Threat Model

### Adversary Capabilities
- **Adversary on same LAN/Wi-Fi:** Can sniff unencrypted traffic, inject rogue packets, spoof mDNS announcements, and attempt MITM attacks.
- **Untrusted Relay:** If relaying through a remote server, the relay operator must never have access to plaintext traffic or session keys (zero-knowledge forwarding).
- **Rogue Peer:** An unpaired device that attempts to send commands, flood requests, or trigger state changes without authorization.

### Security Invariants
1. **Zero-Trust Network:** The local network is treated as hostile. No communication occurs without mutual cryptographic authentication.
2. **Confidentiality & Integrity:** All application frames are authenticated and encrypted in-transit.
3. **Replay Protection:** Handshakes use fresh ephemeral nonces generated via cryptographic RNG (`OsRng`).
4. **Denial of Service Resilience:** Strict maximum frame size limits (16 MB default) are enforced prior to allocation to prevent buffer exhaustion.

---

## 2. Cryptographic Primitives

BridgeOS uses standard, audited, modern cryptographic primitives:

| Component | Primitive | Library / Crate |
| :--- | :--- | :--- |
| **Node Identity** | Ed25519 (Edwards-curve Digital Signature) | `ed25519-dalek` |
| **Key Derivation** | HKDF-SHA256 | `ring` / `sha2` |
| **Transport Encryption** | TLS 1.3 / QUIC (ChaCha20-Poly1305, AES-256-GCM) | `rustls` / `quinn` |
| **File Integrity** | Blake3 cryptographic tree hash | `blake3` |
| **Entropy Source** | OS CSPRNG (`CryptGenRandom` / `getrandom`) | `rand::rngs::OsRng` |

*Under no circumstances may custom cryptographic algorithms or hand-rolled ciphers be introduced into this codebase.*

---

## 3. Trust Boundaries & Pairing

1. **Unpaired State:** Discovered peers can only negotiate protocol versions and initiate the pairing ceremony. All functional subsystems (clipboard, file transfer, input control) are inaccessible.
2. **Pairing Ceremony:** Mutual out-of-band verification via Short Authentication String (SAS) numeric PIN or QR code scanning. Once confirmed by the user on both devices, public keys are pinned in local storage.
3. **Paired State:** Devices verify mutual Ed25519 signatures during every connection handshake before granting access to authorized subsystems.

---

## 4. Secure Coding Rules for AI Agents

1. `#![forbid(unsafe_code)]` must be applied to all pure-Rust crates.
2. Never log sensitive payloads (clipboard contents, private keys, authentication tokens).
3. Enforce bounded allocations when deserializing user-controlled input.
4. Always handle failure branches safely—never default to "allow" on errors.
