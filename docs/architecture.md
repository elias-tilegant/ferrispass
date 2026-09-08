# Architecture

A reading guide for someone opening the repo for the first time. Aim: 15 minutes to "I know where to add my change."

## Crate layout

Single crate, no build script, everything compiles via `cargo build`. The
`unsafe_code` lint is `deny` rather than `forbid`, because six modules reach
macOS APIs the safe wrappers do not cover: the Touch ID prompt, the
session-lock notifications, iCloud file coordination, the staged-file
permissions, the update installer and two POSIX calls in the save path. Each
carries the allow at its own top or on the one function that needs it, so
`grep -rn 'allow(unsafe_code)' src/` is the complete list; nothing else in the
tree may use it.

```
src/
├── app/        Bootstrap, AppState (the single source of mutable truth),
│               settings + recents persistence, key bindings, time helpers
├── domain/     Vault types the UI renders - VaultSnapshot, VaultEntry,
│               VaultGroup. Decrypted, and not secret-free: a protected custom
│               field's value is cleartext here, same trust zone as the
│               entry password. Debug impls redact; the data does not.
├── keepass/    Adapter over the forked keepass-rs crate. Document open/save,
│               two-file merge for conflicts, password generator, snapshot
│               extraction (Database → VaultSnapshot).
├── sync/       Provider-neutral cloud sync. Registry, bindings, SharePoint
│               Graph adapter, and Apple iCloud Drive file adapter.
│               client, per-vault SyncConfig, upload-on-save orchestration,
│               412-conflict handling.
├── ui/         GPUI views, screens, widgets. AppShell is the top-level
│               component that observes AppState and renders the active screen.
├── update/     Auto-update system. Wraps cargo-packager-updater. Handles
│               manifest fetch, version compare, download + verify + install.
├── cli.rs      Headless command tree, stable JSON envelope, secret-output
│               policy and two-step SharePoint sync.
├── cli_install.rs  macOS registration of the bundled CLI in /usr/local/bin.
├── favicon.rs  DuckDuckGo favicon fetcher (per-entry icon enrichment).
├── lib.rs      Module root.
└── main.rs     Entry point - calls `ferrispass::app::run()`.

src/bin/
└── ferrispass-cli.rs  Thin CLI entry point that delegates to `cli::run()`.

bundle/
├── icon.png            App icon master (1024×1024).
├── minisign-pub.txt    Update-signing public key, embedded via include_str!.
└── macos/
    ├── Info.plist          App-bundle metadata (rendered with __VERSION__ substitution).
    └── entitlements.plist  Hardened Runtime entitlements (intentionally minimal).

scripts/
├── build-mac.sh         9-stage release pipeline (build → sign → notarize → manifest).
└── setup-minisign.sh    One-time keypair generation for update signing.
```

## Data flow: opening a vault

```
User picks file.kdbx (Welcome screen, or recents list, or ⌘O)
      │
      ▼
  AppState.set_vault_status(AwaitingPassword)
      │
      ▼
User submits password (Unlock screen)
      │
      ▼
  cx.background_spawn:
    KeePassRepository::open(path, password)
      → keepass::Database
      → snapshot_from_database()
      → VaultSnapshot
      │
      ▼
  cx.spawn → AppState.update():
    state.vault = VaultStatus::Open { document, snapshot }
    cx.notify()
      │
      ▼
  AppShell observes AppState change → re-renders → vault screen visible
```

## State pattern

`AppState` (in `src/app/state.rs`) holds *all* mutable application state in a single `gpui::Entity`. Status is encoded in enums per concern:

- `VaultStatus` - Empty, AwaitingPassword, Opening, Open, LockedPendingSave,
  Error
- `SaveStatus` - Idle, Saving, Saved, Failed
- `SyncStatus` - Disconnected, Idle, Connecting, Restoring, Syncing, Synced,
  Conflict, Failed, Reconnect.
  `Conflict` carries three kinds: entries whose fields, placement or expiry
  diverged, groups whose content or placement diverged, and the database's own
  settings. Each is decided on the clock KDBX keeps for it, and each reaches
  the screen when that clock cannot rank the two sides, because it ties, is
  missing, or claims a time nobody could have written yet
- `UpdateStatus` - Idle, Checking, Available, Downloading, ReadyToRestart, Failed
- `FaviconDownloadStatus` - Idle, Running, Finished
- `Overlay` - None, Connect, Settings, AddEntry, EditEntry, AddGroup,
  RenameGroup, Conflict, VaultSwitcher, AddVault, WhatsNew, About

Mutations always flow through `AppState` methods. The pattern is:

```rust
pub fn start_some_async_thing(&mut self, cx: &mut Context<Self>) {
    self.status = Status::InFlight;
    cx.notify();                           // UI re-paints with the loading state

    let task = cx.background_spawn(async {
        do_blocking_io()                   // network, disk, KDF, etc.
    });
    cx.spawn(async move |this, cx| {
        let result = task.await;
        this.update(cx, |state, cx| {
            state.status = match result { /* terminal state */ };
            cx.notify();                   // UI re-paints with the result
        }).ok();
    }).detach();
}
```

Reference implementation: `try_restore_sync_binding` in `app/state.rs`, whose completion body is split out as `apply_sync_binding_restore` so it can be tested. Copy that shape for any new async operation. No line number here: it would be wrong by the next commit.

`AppShell` (in `src/ui/app_shell.rs`) holds UI-local state (input fields, scroll positions, focus handles, debounce tasks) and subscribes to `AppState` via `cx.observe`. AppShell never mutates AppState directly - it dispatches actions or calls public methods on the `Entity<AppState>`.

## Trust boundaries

```
┌─────────────────────────────────────────────────────────┐
│ Process memory (vault unlocked)                         │
│   - VaultDocument (decrypted entries, composite key)    │
│     The master password is reduced to its SHA-256 at    │
│     unlock; the plaintext is not kept                   │
│   - a 0600 launch file while an entry is launching,     │
│     holding that entry's password until it is unlinked  │
│   - SyncBinding (in-flight access token, ~1h TTL)       │
└─────────────────────────────────────────────────────────┘
              │ atomic write (fsync + rename)
              ▼
┌─────────────────────────────────────────────────────────┐
│ Disk: ~/Library/Application Support/ferrispass/         │
│   - settings.json    plain JSON, no secrets             │
│   - recent.json      paths only, no passwords           │
│   - sync/<hash>.json site/drive/item ids, no tokens     │
└─────────────────────────────────────────────────────────┘
              │ Security framework (keyring crate)
              ▼
┌─────────────────────────────────────────────────────────┐
│ macOS Keychain                                          │
│   - service: ferrispass-sync                            │
│   - account: user-email                                 │
│   - secret:  OAuth refresh token (long-lived)           │
│                                                         │
│   - service: ferrispass-biometric   (Touch ID only)     │
│   - account: enrolment id                               │
│   - secret:  that vault's master password               │
│     Written only when the user enables Touch ID for a   │
│     vault. See SECURITY.md for what guards it.          │
└─────────────────────────────────────────────────────────┘
              │ HTTPS / Microsoft Graph
              ▼
┌─────────────────────────────────────────────────────────┐
│ Cloud (SharePoint via Microsoft Graph)                  │
│   - .kdbx file (encrypted at rest by FerrisPass)        │
│   - never sees the master password                      │
└─────────────────────────────────────────────────────────┘
```

The cloud only ever sees ciphertext. The master password is consumed once to
build the `DatabaseKey`, which keeps only its SHA-256, the single form KDBX
uses, and zeroizes on drop. The plaintext is not retained. Two exceptions are
deliberate and documented: Touch ID writes the password to the login keychain,
described in SECURITY.md, and launching an entry stages a 0600 file holding
that entry's password until the launcher has read it.

## CLI trust boundary

The CLI opens KDBX files through the same `KeePassRepository` and saves through
the same atomic `VaultDocument` publication path as the GUI. It does not run the
GPUI application or expose a local service.

Human output is intended for terminals. `--format json` wraps every result in
the versioned `ferrispass-cli/v1` envelope for scripts and agents. Entry output
never contains passwords, TOTP seeds or protected custom values. An explicit
entry UUID, field name and `--reveal` are required to read one secret.

CLI mutations are applied in memory first and only reach disk with `--commit`.
Sync adds another guard, for either provider: a read-only plan binds the local
ciphertext hash and the remote revision into a token, and the commit
invocation revalidates both before saving or uploading, then checks that the
vault's binding still describes the same relationship before it writes the
config back. The app is a second process holding that file.

## Why a forked keepass-rs

Pinned to the `elias-tilegant/keepass-rs` fork because upstream's KDBX-4 write path produced files unreadable by KeePassXC. The fork carries three interop fixes: AES-KDF UUID handling, omit-None XML field serialization, and base64-encoded timestamp formatting. The pin in `Cargo.toml` is the single source of truth for the exact fork commit; bump it deliberately and re-run interop tests against KeePassXC + KeePass2 before shipping.

### Custom-icon-aware database merge

Icon references travel with entries and groups during a merge while the images
live in a database-local table, so the fork copies the images a merged
reference needs and drops those nothing points at any more. Where both
databases use one `CustomIconUUID` for different images, which happens when two
clients each add an icon offline, the source's id is renamed before any
reference is copied: afterwards there is no way left to tell which side meant
which image.

The fork reports what a merge could not do cleanly as a `MergeWarning` enum
rather than as sentences. FerrisPass decides from those whether a merge lost
anything, and that decision must not depend on wording it does not control.

### Attachment-aware database merge

The fork's database merge translates database-local attachment IDs, copies new blobs, preserves attachment names and current/history references, and reuses equal protected or unprotected values. FerrisPass can therefore resolve local/remote attachment divergence without dropping binaries or leaving dangling references.

Coverage for the fork and FerrisPass proves that the merge preserves:

- attachment bytes and protected/unprotected value state;
- attachment names and per-entry references;
- deduplication and stable reference remapping when database-local attachment IDs differ;
- current and historical entry versions;
- additions, removals, renames, and concurrent conflicts without dangling blobs or lost data.

Continue exercising round trips through FerrisPass, KeePassXC, and KeePass2 with divergent local/remote fixtures when changing this path.

## Async runtime

GPUI provides its own task scheduler. Two flavors:

- `cx.background_spawn(fut)` - runs on a thread pool. Use for blocking I/O (network, disk, Argon2 KDF). Future is cancelled on drop unless `.detach()`-ed.
- `cx.spawn(fut)` - runs on the foreground render loop. Use to update `Entity` state after a background task completes. Inside the future, call `this.update(cx, |state, cx| ...)` to mutate state safely.

All HTTP goes through `sync::http`, which owns a small Tokio runtime and two
reqwest clients: `metadata_client` for short request/response calls (sign-in,
Graph metadata, the update manifest, favicons) and `transfer_client` for
vault-sized bodies. They differ only in their deadline, so the system proxy and
the native trust store cannot apply to one and not the other. Background tasks
enter the runtime synchronously. That Tokio runtime is the only one: everything else asynchronous in the app runs on GPUI's own scheduler, described above.

| Call class | Connect | Total |
|---|---|---|
| Metadata (sign-in, Graph, manifest, favicon) | 10 s | 30 s |
| Transfer (vault, update bundle) | 10 s | 1 h, plus a 120 s idle cap |

## UI rendering

GPUI is element-based, not retained-tree. Every render of a screen produces a fresh element tree. State changes trigger re-renders via `cx.notify()` on the relevant `Entity`. The `gpui-component` crate provides h_flex/v_flex layout, theme-aware styling, and ready-made widgets (Input, Slider, Icon).

Screens live in `src/ui/screens/` and follow the convention:

```rust
pub fn render(shell: &AppShell, cx: &mut Context<AppShell>) -> AnyElement {
    // Read state synchronously up front
    let state = shell.state().read(cx);
    let snapshot = state.snapshot();
    
    // Build the element tree
    div().child(...).into_any_element()
}
```

Listeners use `cx.listener(|shell, event, window, cx| ...)` to capture mutations + dispatch back through AppShell or AppState methods.

## Security-critical files

If you're touching one of these, get a second pair of eyes:

| File | Why |
|---|---|
| `src/keepass/document.rs` | Vault save path. Bug here = corrupted .kdbx files. |
| `src/keepass/repository.rs` | Vault open + snapshot extraction. Bug here = entries showing wrong data or password leakage. |
| `src/sync/auth.rs` | OAuth device-code flow. Bug = users sign in to attacker-controlled apps. |
| `src/sync/tokens.rs` | Keychain interaction. Bug = refresh tokens written to disk in plaintext. |
| `src/update/client.rs` | Update install path. Bug = unsigned updates accepted, RCE potential. |
| `bundle/minisign-pub.txt` | The trust anchor for auto-updates. Touching this without intent invalidates every existing install's update path. |

See [`SECURITY.md`](../SECURITY.md) for the threat model and reporting policy.
