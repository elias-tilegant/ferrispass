# STC KeePass

A native macOS KeePass client built in Rust on top of [GPUI](https://github.com/zed-industries/zed).

Reads and writes KDBX 4 files (AES-256 + Argon2id), interoperable with KeePassXC and KeePass2.

## Features

- **Vault**: open, browse, search, add, edit, delete (with recycle bin + restore), permanent delete
- **Passwords**: zxcvbn strength meter, dedicated password generator (length slider, character classes, wordlist mode)
- **TOTP**: read otpauth URLs, live 1 Hz countdown, click-to-copy code
- **Favorites**: starred entries via the `Favorite` tag (round-trips through KeePassXC)
- **QOL**: click-on-field to copy, password reveal toggle, click-on-URL to open in browser, copy-toast notifications
- **Auto-lock**: idle-timeout configurable in Settings (default 4 min, "Never" supported)
- **Clipboard auto-clear**: configurable wipe after copy (default 10 s, "Never" supported); also wipes on lock
- **Resume**: most-recently-opened vault auto-loads at startup; Recents list on the welcome screen
- **Cloud sync**: SharePoint via Microsoft Graph (device-code OAuth, ETag-based conflict detection, three-way merge)
- **Theming**: light + dark mode (⌘⇧D)

## Structure

```text
src/
  app/        application bootstrap, AppState, recents, settings, time helpers
  domain/     UI-safe vault snapshot types (no secrets in visible models)
  keepass/    keepass-rs adapter, document, password generator, three-way merge
  sync/       SharePoint device-code auth, Graph API, sync service, keychain tokens
  ui/         GPUI views, screens, widgets, palette, theme
examples/
  dump_xml.rs interop diagnostic — prints the decoded KDBX inner XML
```

## Build & run

```sh
cargo check
cargo test
cargo run
```

Tested on macOS only. Linux builds but the SharePoint sync expects the Apple Keychain.

## Keyboard shortcuts

| Action | Shortcut |
|---|---|
| Open vault | ⌘O |
| Lock vault | ⌘L |
| Save vault | ⌘S |
| New entry | ⌘N |
| Edit selected entry | ⌘E |
| Delete to trash | ⌘⌫ |
| Search | ⌘F |
| Settings | ⌘, |
| Settings → Sync tab | ⌘⇧, |
| Toggle theme | ⌘⇧D |
| Copy password | ⌘⇧P |
| Copy username | ⌘⇧U |
| Copy URL | ⌘⇧L |
| Quit | ⌘Q |

## Security notes

- Master password is held in memory only while the vault is open; required to re-encrypt on save.
- KDBX writer is pinned to a [forked keepass-rs](https://github.com/elias-tilegant/keepass-rs) (`cc6845a`) carrying three KDBX 4 interop fixes the upstream lacks; without these, written files don't reopen in KeePassXC.
- SharePoint refresh tokens live in the macOS Keychain (`stc-keepass-sync` service); access tokens are in-memory and ~1 h TTL.
- Recents file (`~/Library/Application Support/stc-keepass/recent.json`) holds **paths only** — no passwords, no tokens.
