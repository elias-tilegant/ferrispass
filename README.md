# FerrisPass

[![CI](https://github.com/elias-tilegant/ferrispass/actions/workflows/ci.yml/badge.svg)](https://github.com/elias-tilegant/ferrispass/actions/workflows/ci.yml)

A native macOS, KeePass-compatible client built in Rust on top of [GPUI](https://github.com/zed-industries/zed).

Reads and writes KDBX 4 files (AES-256 + Argon2id), interoperable with KeePassXC and KeePass2.

## Features

- **Vault**: open, browse, search, add, edit, delete (with recycle bin + restore), permanent delete
- **Passwords**: zxcvbn strength meter, dedicated password generator (length slider, character classes, wordlist mode)
- **TOTP**: read otpauth URLs, live 1 Hz countdown, click-to-copy code
- **Favorites**: starred entries via the `Favorite` tag (round-trips through KeePassXC)
- **QOL**: click-on-field to copy, password reveal toggle, click-on-URL to open in browser, copy-toast notifications
- **Auto-lock**: idle-timeout configurable in Settings (default 4 min, "Never" supported)
- **Clipboard auto-clear**: configurable wipe after copy (default 10 s, "Never" supported); also wipes on lock
- **Auto-Type**: global hotkey (default ⌃⌥⌘V) types `{USERNAME}{TAB}{PASSWORD}{ENTER}` into the previously-focused window; foreground app is matched to a vault entry by URL hostname. Off by default — enable in Settings → Auto-Type. Requires the macOS Accessibility permission.
- **Resume**: most-recently-opened vault auto-loads at startup; Recents list on the welcome screen
- **Cloud sync**: SharePoint via Microsoft Graph (device-code OAuth, ETag-based conflict detection, three-way merge) — see [Getting Started: SharePoint Sync](./docs/getting-started-sharepoint.md) for the connect walkthrough
- **Theming**: light + dark mode (⌘⇧D)

## Feature matrix

FerrisPass is still young, so this matrix is intentionally honest about what is already there and what is not.

| Capability | FerrisPass | [KeePassXC](https://keepassxc.org/) | [KeePassium](https://keepassium.com/) | [Strongbox](https://strongboxsafe.com/) |
|---|---|---|---|---|
| Platform focus | macOS, Apple Silicon | Windows, macOS, Linux | iPhone, iPad, Mac | iPhone, iPad, Mac, Apple Watch |
| License / source model | GPL-3.0-or-later, open source | GPLv3, open source | GPLv3, open source | Open source, commercial Pro tier |
| KDBX read/write | Yes, KDBX 4 | Yes | Yes, KDB/KDBX | Yes, plus Password Safe |
| Password generator | Yes | Yes | Yes | Yes |
| TOTP / one-time codes | Yes | Yes | Yes | Yes |
| Favorites / tags | Yes, via `Favorite` tag | Yes | Yes | Yes |
| Search and basic entry editing | Yes | Yes | Yes | Yes |
| Browser or OS AutoFill | Not yet | Browser extension | iOS/macOS AutoFill | iOS/macOS AutoFill and browser extensions |
| Auto-Type | Yes, global hotkey | Yes | Not a primary feature | Not a primary feature |
| Cloud sync | SharePoint only | File-based / bring your own sync | Broad Files app and direct cloud support | Broad cloud, WebDAV, SFTP support |
| Conflict handling / merge | Three-way merge for SharePoint | KeeShare / database tools | Merge support | Advanced sync and merge |
| Passkeys | Not yet | Yes | Yes | Yes |
| Hardware keys | Not yet | YubiKey / OnlyKey challenge-response | YubiKey | YubiKey |
| Password auditing | Not yet | Yes | Premium leak audit | Yes |
| SSH agent | Not yet | Yes | No | macOS Pro |
| In-app updates | Yes, signed GitHub Releases | Platform/package dependent | App Store | App Store |

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

## Installation

Download `FerrisPass-X.Y.Z-arm64.dmg` from the [Releases](https://github.com/elias-tilegant/ferrispass/releases) page, open it, drag `FerrisPass.app` into `Applications`.

The app is signed with a Developer ID and Apple-notarized, so first launch opens cleanly with no Gatekeeper override.

Requires Apple Silicon (M1 or newer) and macOS 12 (Monterey) or later. Intel Macs are not supported.

## Auto-updates

FerrisPass checks GitHub Releases on launch (rate-limited to once per day). When a newer build is published, a banner appears on the Welcome screen with an **Install** button — click it and the app downloads the new bundle, verifies its Ed25519 signature against an embedded public key, atomic-replaces itself, and prompts you to restart.

Independent of Apple's Developer ID + notarization (which signs the DMG), every update payload and its complete manifest carry separate [minisign](https://jedisct1.github.io/minisign/) signatures. The signed manifest binds the version, URL, payload size, and payload signature; all checks must pass before an update is applied.

To disable: Settings → General → Updates → toggle to **Off**. The "Check now" button still works manually any time. The preference persists in `~/Library/Application Support/ferrispass/settings.json`.

The auto-updater lives in `src/update/`; the public key it verifies against is `bundle/minisign-pub.txt`, embedded into every build at compile time via `include_str!`.

## Development

For local development:

```sh
cargo check
cargo test
cargo run
```

Tested on macOS only. Linux builds but the SharePoint sync expects the Apple Keychain.

## Building a release DMG

`scripts/build-mac.sh` produces a notarized, stapled, arm64 `.dmg` ready for distribution. Pipeline: `cargo build --release --target aarch64-apple-darwin` → `.app` bundle → `codesign --options runtime` → `create-dmg` → `notarytool submit --wait` → `stapler staple`.

```sh
scripts/build-mac.sh                  # full release build
scripts/build-mac.sh --skip-notarize  # local iteration, no Apple roundtrip
```

Requirements (one-time setup):
- Xcode Command Line Tools (`xcode-select --install`)
- `Developer ID Application` certificate in Keychain (Xcode → Settings → Accounts → Manage Certificates)
- Notarization credentials stored as Keychain profile named `ferrispass-notarize`:
  ```sh
  xcrun notarytool store-credentials "ferrispass-notarize" \
      --apple-id <your-apple-id> --team-id <your-team-id> \
      --password <app-specific-password>
  ```
- `bundle/icon.png` — a 1024×1024 master PNG of the app icon
- Optional: `brew install create-dmg` (prettier DMG window; falls back to plain `hdiutil` if absent)

Forks must edit the `TEAM_ID`, `SIGNING_IDENTITY`, and `BUNDLE_ID` constants at the top of `scripts/build-mac.sh` to match their own Apple Developer account, plus generate their own minisign keypair (`scripts/setup-minisign.sh`) and update the `UPDATE_ENDPOINT` constant in `src/update/mod.rs` to point at their fork's release URL. The embedded public key is unique per fork — users of one fork won't accept update bundles signed by another.

### Automated releases (GitHub Actions)

`.github/workflows/release.yml` runs the full pipeline on every `v*` tag push and attaches the resulting DMG to a GitHub Release. Required repo secrets:

| Secret | Source |
|---|---|
| `APPLE_CERT_BASE64` | `base64 < cert.p12 \| pbcopy` (export Developer ID cert from Keychain Access first) |
| `APPLE_CERT_PASSWORD` | password set when exporting the .p12 |
| `APPLE_ID` | Apple ID email used for notarization |
| `APPLE_TEAM_ID` | 10-char Team ID |
| `APPLE_NOTARIZE_PASSWORD` | app-specific password from [appleid.apple.com](https://appleid.apple.com) |
| `MINISIGN_PRIVATE_KEY` | full content of `~/.ferrispass/minisign.key` — paste **both lines** including the `untrusted comment:` header. Generated by `scripts/setup-minisign.sh`. |
| `MINISIGN_PASSWORD` | passphrase you typed when running `scripts/setup-minisign.sh`. Used by the release pipeline to sign update bundles for the in-app auto-updater. |

See [`docs/release-process.md`](./docs/release-process.md) for the full release workflow, common failure modes, and minisign-key backup strategy.

If you'd rather keep signing material off GitHub, leave the secrets unset and run `scripts/build-mac.sh` locally instead. The release workflow only fires on tag pushes, so until you set the secrets it won't run successfully — that's the intended fail-safe.

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
| Auto-Type selected entry (3 s countdown) | ⌘⇧T |
| Auto-Type matching entry into focused window | ⌃⌥⌘V (configurable, off by default) |
| Quit | ⌘Q |

## Security notes

- Master password is held in memory only while the vault is open; required to re-encrypt on save.
- KDBX writer is pinned to a [forked keepass-rs](https://github.com/elias-tilegant/keepass-rs) (`cc6845a`) carrying three KDBX 4 interop fixes the upstream lacks; without these, written files don't reopen in KeePassXC.
- SharePoint refresh tokens live in the macOS Keychain (`ferrispass-sync` service); access tokens are in-memory and ~1 h TTL.
- Recents file (`~/Library/Application Support/ferrispass/recent.json`) holds **paths only** — no passwords, no tokens.

## Author

Created and maintained by [Elias Tilegant](https://github.com/elias-tilegant).

## License

FerrisPass is licensed under the **GNU General Public License v3.0 or later** (`GPL-3.0-or-later`). See [LICENSE](./LICENSE) for the full text.

The GPL is required because FerrisPass links GPUI, which transitively depends on `ztracing` / `zlog` (both GPL-3.0-or-later). It also matches the convention of the broader KeePass-compatible ecosystem (KeePass2, KeePassXC, Bitwarden are all GPL).

This means: anyone distributing a modified FerrisPass binary must publish their source modifications under the same terms — a deliberate guarantee for a security-critical app.
