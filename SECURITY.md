# Security Policy

FerrisPass handles secrets people consider sensitive enough to never lose and never expose — credentials, TOTP seeds, OAuth refresh tokens. We take vulnerability reports seriously and respond fast.

## Reporting a vulnerability

**Please don't open a public issue for security problems.** A public issue makes the bug exploitable for the days/weeks before a patch ships.

Instead, use **GitHub's private vulnerability reporting**:

1. Go to https://github.com/elias-tilegant/ferrispass/security/advisories/new
2. Describe the issue, ideally with a minimal reproduction
3. We acknowledge within 72 hours, target a fix within 14 days for high-severity issues

If GitHub is unavailable to you, email a brief description to the address listed on the maintainer's GitHub profile and we'll move the conversation to a private channel.

## What counts as a vulnerability

In scope:

- **Vault decryption / re-encryption flaws** — incorrect KDBX 4 parsing, weak key derivation parameters, plaintext leakage on disk
- **Credential exfiltration** — any path where a malicious .kdbx, malicious favicon URL, or malicious SharePoint response could read out secrets the running process holds
- **OAuth token handling** — refresh tokens leaking outside the macOS Keychain, access tokens persisting beyond their TTL
- **Update mechanism** — bypassing the minisign signature check, downgrade attacks, replay attacks against the manifest endpoint
- **Build supply chain** — compromise paths through our pinned dependencies (`keepass-rs` fork in particular) or the GitHub Actions release pipeline
- **macOS Keychain access** — incorrect ACLs that allow other apps to read FerrisPass-owned items

Out of scope (will be acknowledged but won't be patched as security issues):

- Risks from a compromised host OS — we trust macOS to be honest about which app is asking for Keychain items
- Hardware key-loggers, screen recorders, evil-maid attacks on a laptop the attacker has physical access to
- Memory-dump attacks on a running unlocked vault — the master password is unavoidably in process memory while you're using the app
- Brute-force against weak master passwords — this is a user-side issue, not a FerrisPass bug
- Denial of service against the update endpoint (GitHub's problem)

## Supported versions

We patch the **latest minor release line only**. As of 2026-05, that's `0.2.x`. Older versions don't receive security updates — auto-update is on by default specifically so users land on the patched version within ~24 hours of release.

## Cryptographic summary

For transparency about the trust assumptions:

| Surface | Algorithm | Key location |
|---|---|---|
| Vault file encryption | AES-256-CBC + HMAC-SHA-256 (KDBX 4 standard) | derived from master password via Argon2id |
| Master-password KDF | Argon2id | parameters from the .kdbx header (default ≥64 MiB / 2 iterations / 8 lanes) |
| OAuth refresh tokens | none — opaque strings stored as-is | macOS Keychain, service `ferrispass-sync` |
| Update bundle signing | minisign Ed25519 | public key embedded in binary at compile time, private key under maintainer custody |
| Update bundle delivery | TLS via `reqwest` (rustls) | system root CAs |
| Binary signing | Apple Developer ID + notarization | Apple PKI |

The dual signing (Apple Developer ID *and* minisign) is intentional: each layer protects against a different compromise. An attacker would need to steal both Apple's signing infrastructure AND our minisign private key to push a malicious update that the running app accepts.

## Master-password handling — what FerrisPass does and doesn't do

| | |
|---|---|
| Password held in memory while vault is unlocked | yes — required to re-encrypt on save |
| Password persisted to disk | **never** |
| Password sent over the network | **never** — the cloud provider only sees ciphertext |
| Password stored in Keychain | **no** — we don't auto-fill it; you re-type to unlock |
| Auto-lock timeout | configurable in Settings; default 4 minutes idle |
| Clipboard auto-clear after copy | configurable; default 10 seconds |
