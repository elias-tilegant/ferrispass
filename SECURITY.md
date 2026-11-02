# Security Policy

FerrisPass handles secrets people consider sensitive enough to never lose and never expose - credentials, TOTP seeds, OAuth refresh tokens. We take vulnerability reports seriously and respond fast.

## Reporting a vulnerability

**Please don't open a public issue for security problems.** A public issue makes the bug exploitable for the days/weeks before a patch ships.

Instead, use **GitHub's private vulnerability reporting**:

1. Go to https://github.com/elias-tilegant/ferrispass/security/advisories/new
2. Describe the issue, ideally with a minimal reproduction
3. We acknowledge within 72 hours, target a fix within 14 days for high-severity issues

If GitHub is unavailable to you, email a brief description to the address listed on the maintainer's GitHub profile and we'll move the conversation to a private channel.

## What counts as a vulnerability

In scope:

- **Vault decryption / re-encryption flaws** - incorrect KDBX 4 parsing, weak key derivation parameters, plaintext leakage on disk
- **Credential exfiltration** - any path where a malicious .kdbx, malicious favicon URL, or malicious SharePoint response could read out secrets the running process holds
- **OAuth token handling** - refresh tokens leaking outside the macOS Keychain, access tokens persisting beyond their TTL
- **Update mechanism** - bypassing the minisign signature check, downgrade attacks, replay attacks against the manifest endpoint
- **Build supply chain** - compromise paths through our pinned dependencies (`keepass-rs` fork in particular) or the GitHub Actions release pipeline
- **macOS Keychain access** - incorrect ACLs that allow other apps to read FerrisPass-owned items
- **Clipboard** - any path on which FerrisPass copies a secret for you without the concealed, transient and current-host markers, or leaves its own copy behind after the clear timer, a lock, quit or launch. Which copies count, and when FerrisPass looks at launch, is in the security contract below

Out of scope (will be acknowledged but won't be patched as security issues):

- Risks from a compromised host OS - we trust macOS to be honest about which app is asking for Keychain items
- Hardware key-loggers, screen recorders, evil-maid attacks on a laptop the attacker has physical access to
- Memory-dump attacks on a running unlocked vault - the decrypted entries and the composite key are unavoidably in process memory while you're using the app. The master password itself is not: it is reduced to its SHA-256 at unlock, which is the only form KDBX uses. That hash still opens this vault, so the gain is narrow and worth stating plainly: what a dump does not yield is the password itself, which is the thing people reuse elsewhere. The exception is the login keychain when you enable Touch ID
- Brute-force against weak master passwords - this is a user-side issue, not a FerrisPass bug
- Denial of service against the update endpoint (GitHub's problem)

## Security contract

What FerrisPass promises from the release in the second column on. A build that breaks one of these has a vulnerability in the sense above.

| Rule | Since |
|---|---|
| Every secret FerrisPass copies for you goes through one clipboard path: concealed, transient, this Mac only, cleared after the timer, at lock, at quit and at launch; only unchanged clipboard contents are ever cleared, and the launch check reads types, never contents. The launch check only looks when "Paste from Other Apps" is set to Always Allow for FerrisPass; with any other setting it leaves the clipboard alone, so macOS never asks | the redesign release |

In the clipboard rule, a secret FerrisPass copies for you is a password, a username, a URL, a TOTP code or a custom field that you copy with a button, a shortcut or a menu item. Text you select yourself in one of the entry editor's unprotected fields (title, username, URL, notes, custom fields not marked protected) and copy with Cmd+C, Cmd+X or the context menu is an ordinary copy, as it always was: it does not go through this path, carries none of the markers and is not cleared. The password, the TOTP secret and custom fields marked protected can't be copied out of the editor by hand at all.

## Supported versions

We patch the **latest minor release line only**. As of 2026-09, that's `0.9.x`. Older versions don't receive security updates. Auto-update is on by default and checks once per launch, so how quickly a user lands on a patched version depends on how often they start the app.

## Cryptographic summary

For transparency about the trust assumptions:

| Surface | Algorithm | Key location |
|---|---|---|
| Vault file encryption | whatever the .kdbx header specifies; a vault FerrisPass writes keeps the cipher the file already had. New KeePassXC and KeePass2 files are AES-256 | derived from the master password by the KDF below |
| Master-password KDF | whatever the .kdbx header specifies: Argon2d, Argon2id, or the older AES-KDF that KDBX 3 files carry | parameters come from the file, so they are as strong as the client that wrote it. `keepass::limits` refuses a header asking for more memory than we will allocate, but nothing raises a weak one |
| OAuth refresh tokens | none - opaque strings stored as-is | macOS Keychain, service `ferrispass-sync` |
| Update bundle signing | minisign Ed25519 | public key embedded in binary at compile time, private key under maintainer custody |
| Update bundle delivery | TLS via `reqwest` (rustls) | system root CAs |
| Binary signing | Apple Developer ID + notarization | Apple PKI |

The two signatures cover different things and different routes. A DMG downloaded from the website carries the Apple signature and notarization, which is what Gatekeeper checks on first launch; it carries no minisign signature.

An update the running app installs is the other way round: the archive's minisign signature is checked before anything is unpacked, and the staged bundle is then checked for shape, not for an Apple signature. Gatekeeper checks the replacement on the next launch, so a bundle that is not properly signed fails then rather than at install time. Verifying it at install time is in the backlog.

So an attacker needs the minisign key to push an update the app installs, and Apple's signing infrastructure to get past Gatekeeper afterwards. Neither key alone gets a malicious build both installed and running.

## Master-password handling - what FerrisPass does and doesn't do

| | |
|---|---|
| Password held in memory while vault is unlocked | its SHA-256, which is what KDBX uses; the password itself is dropped at unlock |
| Password persisted to disk | **never** |
| Password sent over the network | **never** - the cloud provider only sees ciphertext |
| Password stored in Keychain | **only if you enable Touch ID for that vault** - see below |
| Auto-lock timeout | configurable in Settings; default 4 minutes idle |
| Clipboard auto-clear after copy | for what FerrisPass copies for you: configurable, default 10 seconds; also at quit and at launch (at launch only with "Paste from Other Apps" set to Always Allow). Text you copy by hand out of the entry editor is an ordinary copy and is not cleared |

### Touch ID and the login keychain

Enabling Touch ID for a vault writes that vault's master password into your
**login keychain**, under the service `ferrispass-biometric`. This is what
makes an unlock without typing possible at all.

What that means, precisely:

- The Touch ID prompt runs inside FerrisPass. It gates *our* read of the item,
  not the item itself. macOS guards the item with the login keychain's own
  access control.
- Another program running as you can therefore ask macOS for it. On a standard
  setup macOS shows an "allow" dialog for a program that is not FerrisPass, and
  you can deny it; it is not a silent read for an arbitrary process, but it is
  also not gated on your fingerprint.
- We would rather bind the item to FerrisPass's code signature with a
  `SecAccess` list. The `security-framework` crate exposes no safe way to do
  that for a generic password item, and doing it by hand means Core Foundation
  FFI around the most sensitive secret in the app. That is a change worth
  making on its own, with its own review, and it has not been made yet.
- Turning the feature off, or choosing "Forget Touch ID" on the unlock screen,
  removes the item. A vault without Touch ID enabled never has its password in
  the keychain.

The enrolment checkbox says this in one line where you decide.
