# Architecture

A reading guide for someone opening the repo for the first time. Aim: 15 minutes to "I know where to add my change."

## Crate layout

Single-crate workspace, no FFI. Everything compiles via `cargo build`.

```
src/
├── app/        Bootstrap, AppState (the single source of mutable truth),
│               settings + recents persistence, key bindings, time helpers
├── domain/     UI-safe vault types — VaultSnapshot, VaultEntry, VaultGroup.
│               Crucially: zero secret material. Only what the UI needs to render.
├── keepass/    Adapter over the forked keepass-rs crate. Document open/save,
│               three-way merge for conflicts, password generator, snapshot
│               extraction (Database → VaultSnapshot).
├── sync/       SharePoint cloud sync. Device-code OAuth, Microsoft Graph
│               client, per-vault SyncConfig, upload-on-save orchestration,
│               412-conflict handling.
├── ui/         GPUI views, screens, widgets. AppShell is the top-level
│               component that observes AppState and renders the active screen.
├── update/     Auto-update system. Wraps cargo-packager-updater. Handles
│               manifest fetch, version compare, download + verify + install.
├── favicon.rs  DuckDuckGo favicon fetcher (per-entry icon enrichment).
├── lib.rs      Module root — declares the eight pub mods above.
└── main.rs     Entry point — calls `ferrispass::app::run()`.

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

- `VaultStatus` — Welcome, AwaitingPassword, Open, Error
- `SaveStatus` — Idle, Saving, Saved, Failed
- `SyncStatus` — Disconnected, Idle, Connecting, Synced, Conflict, Failed, Reconnect
- `UpdateStatus` — Idle, Checking, Available, Downloading, ReadyToRestart, Failed
- `FaviconDownloadStatus` — Idle, Running, Finished
- `Overlay` — None, Connect, Settings, AddEntry, EditEntry, Conflict, VaultSwitcher

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

Reference implementation: `try_restore_sync_binding` in `state.rs:541`. Copy this pattern for any new async operation.

`AppShell` (in `src/ui/app_shell.rs`) holds UI-local state (input fields, scroll positions, focus handles, debounce tasks) and subscribes to `AppState` via `cx.observe`. AppShell never mutates AppState directly — it dispatches actions or calls public methods on the `Entity<AppState>`.

## Trust boundaries

```
┌─────────────────────────────────────────────────────────┐
│ Process memory (vault unlocked)                         │
│   - VaultDocument (decrypted entries, master password)  │
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
└─────────────────────────────────────────────────────────┘
              │ HTTPS / Microsoft Graph
              ▼
┌─────────────────────────────────────────────────────────┐
│ Cloud (SharePoint via Microsoft Graph)                  │
│   - .kdbx file (encrypted at rest by FerrisPass)        │
│   - never sees the master password                      │
└─────────────────────────────────────────────────────────┘
```

The cloud only ever sees ciphertext. The master password never leaves process memory; it's required to re-encrypt on save and is wiped when the vault locks.

## Why a forked keepass-rs

Pinned to `elias-tilegant/keepass-rs@cc6845a` because upstream's KDBX-4 write path produced files unreadable by KeePassXC. The fork carries three interop fixes: AES-KDF UUID handling, omit-None XML field serialization, and base64-encoded timestamp formatting. The pin in `Cargo.toml` is the single source of truth for the fork commit; bump it deliberately and re-run interop tests against KeePassXC + KeePass2 before shipping.

## Async runtime

GPUI provides its own task scheduler. Two flavors:

- `cx.background_spawn(fut)` — runs on a thread pool. Use for blocking I/O (network, disk, Argon2 KDF). Future is cancelled on drop unless `.detach()`-ed.
- `cx.spawn(fut)` — runs on the foreground render loop. Use to update `Entity` state after a background task completes. Inside the future, call `this.update(cx, |state, cx| ...)` to mutate state safely.

We do NOT pull in `tokio` directly — but `cargo-packager-updater` uses `reqwest` which transitively brings tokio in. Tokio code runs only inside the updater's downloader; everything else stays sync + GPUI-scheduled.

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
