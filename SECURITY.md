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
- **Clipboard** - a secret that one of FerrisPass's own copy actions puts on the clipboard without the concealed, transient and current-host markers, or that stays behind after the clear timer, a lock, quit or launch. Which copy actions count, and when FerrisPass looks at launch, is in the security contract below

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
| Every secret you copy with one of FerrisPass's own copy actions goes through one clipboard path: concealed, transient, this Mac only, cleared after the timer, at lock, at quit and at launch; only unchanged clipboard contents are ever cleared, and the launch check reads types, never contents. The launch check only looks when "Paste from Other Apps" is set to Always Allow for FerrisPass; with any other setting it leaves the clipboard alone, so macOS never asks | the redesign release |
| While the master password field has focus, secure event input is on, so other programs do not see the keystrokes; it is released on blur, window deactivation, lock, quit and panic | the redesign release |
| New `unsafe` code lives only in audited modules under `src/platform/`, listed under [Unsafe code](#unsafe-code) | the redesign release |
| Activity for the auto-lock is input in a FerrisPass window, in the quick access panel, a global shortcut or a click on the menu bar item, and nothing that happens in other apps | the redesign release |
| If you choose to lock after the Mac has been idle instead, FerrisPass locks when the login session has seen no input for that long, counting input from any program, and falls back to its own idle clock with the same threshold when macOS cannot report it; sleep, the lock screen and the screen saver lock at once as before | the redesign release |

In the clipboard rule, FerrisPass's own copy actions are the ones that copy a value out of an entry for you: the copy buttons at the bottom of an entry's details and a click on its username, password, TOTP code or a custom field there; Cmd+Shift+U, Cmd+Shift+L, and Cmd+C while no text field has focus; and Copy Username, Copy Password and Copy URL in the Edit menu. The standard editing commands on text you select in a field are not among them. Text you select in one of the entry editor's unprotected fields (title, username, URL, notes, custom field names and the values of custom fields not marked protected) and copy or cut with Cmd+C, Cmd+X, Edit > Copy, Edit > Cut or the field's context menu is an ordinary copy, as it always was: it does not go through this path, carries none of the markers and is not cleared. The password, the TOTP secret and the values of custom fields marked protected can't be copied or cut out of the editor at all.

The Mac idle rule changes what holds off the auto-lock. With it chosen, any input macOS sees in your login session counts, in any app, and so does input that a program posts as if it were typed or moved: FerrisPass's own Auto-Type, a presentation remote, remote-control software, or a tool that nudges the mouse to keep the Mac awake. While one of those runs, the Mac never looks idle and the vault stays open until you lock it, the Mac sleeps or the screen locks. If that is not what you want, keep the default, which counts only input you give FerrisPass. Going back to 0.9.2 drops the Mac idle setting, because 0.9.2 rewrites `settings.json` with only the fields it knows, so with the FerrisPass auto-lock at Never there is then no idle lock until you choose the Mac idle time again; sleep and the lock screen still lock the vault at once.

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

## Unsafe code

The crate denies `unsafe_code`. The places below opt in, each for a system API that our dependencies offer no safe way to call. `tests/architecture.rs` keeps the same list and fails the build when an opt-in appears or disappears without it. New `unsafe` code goes only into its own module under `src/platform/`, with the reason in the module header.

| Where | Why |
|---|---|
| `src/platform/secure_input.rs` | Carbon secure event input around the master password field; a guard that releases on blur, window deactivation, lock, quit and panic |
| `src/biometric/macos.rs` | the LocalAuthentication bindings mark ordinary calls `unsafe`: creating the context, `evaluatePolicy` for the Touch ID prompt, and reading the error in its reply |
| `src/session_lock.rs` | an Objective-C observer for the screen lock, sleep and user switch notifications, so the vault locks when the Mac does |
| `src/update/macos_installer.rs` | `renamex_np` with `RENAME_SWAP`, which swaps the old and the new app bundle in one step, so an interrupted update never leaves the user with neither |
| `src/launch/tempfile.rs`, `process_uid` | `getuid`, to check that the folder for launch payloads belongs to the current user |
| `src/keepass/document.rs`, `publish_new_no_clobber` | `renamex_np` with `RENAME_EXCL`, which puts a new vault file in place without replacing a file that appeared in the meantime |
| `src/keepass/document.rs`, `macos_full_sync` | `fcntl` with `F_FULLFSYNC`, so a saved vault is on the disk and not only in the drive's cache |
| `src/sync/icloud.rs`, `platform` | resolving a stored bookmark and using the URL `NSFileCoordinator` hands to its block, for vaults in iCloud Drive; the bindings mark both `unsafe` |
| `src/platform/system_idle.rs` | one CoreGraphics call that reads the seconds since the last input in the login session, for the auto-lock alternative |

## Master-password handling - what FerrisPass does and doesn't do

| | |
|---|---|
| Password held in memory while vault is unlocked | its SHA-256, which is what KDBX uses; the password itself is dropped at unlock |
| Password persisted to disk | **never** |
| Password sent over the network | **never** - the cloud provider only sees ciphertext |
| Password stored in Keychain | **only if you enable Touch ID for that vault** - see below |
| Auto-lock timeout | configurable in Settings; default 4 minutes without input in a FerrisPass window, the quick access panel, the global shortcuts or the menu bar item. Input in other apps does not count, unless you choose to lock after the Mac itself has been idle for 5, 15 or 30 minutes instead; then input in any app counts |
| Clipboard auto-clear after copy | for what you copy with FerrisPass's own copy actions: configurable, default 10 seconds; also at quit and at launch (at launch only with "Paste from Other Apps" set to Always Allow). Text you copy or cut out of an editor field with the standard editing commands is an ordinary copy and is not cleared |

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
