# P1a.12a: Splitting the app state and the shell

`src/app/state.rs` had grown to about 9,300 lines and `src/ui/app_shell.rs` to about 4,100, each of them mostly one `impl` block. This step cuts both into one file per concern and moves the screens next to the shell under `src/ui/legacy/`. It only moves code: no logic changed, no function was renamed, and nothing inside a function was reordered. The characterization tests from P1a.05b passed before the move and pass after it.

Smaller files let the next steps work on the state and the shell at the same time without editing the same file, and a fix that still has to go into 0.9.x runs into small conflicts at most.

Started from `p1a-05b-characterization` at `facb9c1` on 2026-10-28.

| Commit | What |
|---|---|
| `a686ae6` | `src/app/state.rs` becomes `src/app/state/` |
| `853ba87` | `src/ui/app_shell.rs` becomes `src/ui/legacy/shell/`, `src/ui/screens/` moves to `src/ui/legacy/screens/` |
| the last one | This page and the paths in `docs/architecture.md` |

## What moved where

### `src/app/state/`

| File | Holds |
|---|---|
| `mod.rs` | the types, `AppState` with its fields, `Default`, `Drop`, the free helper functions, the module list |
| `session.rs` | constructors, plain accessors, overlays and the pending notice, switching between unlocked vaults |
| `routing.rs` | session ids, parking, routing save and sync results to the session that started them |
| `open.rs` | the unlock form, the open attempt, recents, the KDF check, biometric unlock |
| `binding.rs` | restoring the sync binding after unlock, backoff, upload baselines |
| `updates.rs` | update check and install, What's New, About |
| `lock.rs` | the deferred lock, quarantine, discarding unsaved work |
| `browse.rs` | sync history, status accessors, the connect flow, TOTP, group expansion, favicons |
| `save.rs` | background saves |
| `mutate.rs` | entry and group changes |
| `select.rs` | selection, search, copying values, the vault browser model |
| `present.rs` | `summary()` |
| `connect.rs` | disconnect, SharePoint and iCloud connect, the file picker, reconnect |
| `sync.rs` | sync runs, remote conflicts, committing a merge |
| `sync_config_probe_tests.rs`, `park_tests.rs`, `biometric_tests.rs`, `selection_tests.rs`, `characterization_tests.rs` | the test modules, one file each, under their old names |

### `src/ui/legacy/`

| File | Holds |
|---|---|
| `mod.rs` | `pub mod screens;` and `pub(super) mod shell;` |
| `screens/` | `src/ui/screens/`, renamed, no file changed; `vault.rs` is not split |
| `shell/mod.rs` | the types, `AppShell` with its fields, `AppShell::new` and the accessors right after it, `BiometricUnlockOutcome`, the free helper functions, the module list |
| `shell/tasks.rs` | the TOTP tick, auto-lock, session lock, auto-sync, the Auto-Type hotkey listener, wiping session secrets |
| `shell/forms.rs` | the unlock form, input accessors, file and iCloud prompts, the entry form and its custom fields, the password generator |
| `shell/handlers.rs` | the `on_action_*` handlers apart from Auto-Type, the group form, entry and group commands, error notices |
| `shell/auto_type.rs` | the Auto-Type actions and the typing run |
| `shell/inputs.rs` | input field events, the vault switcher, picking a vault file |
| `shell/unlock.rs` | password and Touch ID unlock, the KDF cost confirmation |
| `shell/lock.rs` | cancelling an unlock, locking, the search field |
| `shell/secrets.rs` | copying with a timed clipboard clear, launching an entry, revealing a password, the settings |
| `shell/render.rs` | `render_body`, the `Focusable` and `Render` impls, the modal overlays |
| `shell/characterization_tests.rs` | the test module, under its old name |

Each concern file is `use super::*;` and one `impl` block with the methods in their original order (`render.rs` also carries the trait impls).

## Visibility

- Fields stay private. A child module can see its parent's private items, so the `impl` blocks in the new files use the fields, the private types and the free functions of `mod.rs` as before.
- A private method that another file calls is now `pub(super)`: visible inside `app::state` or `ui::legacy::shell` and nowhere else. Only what the compiler asked for was widened. Nothing became `pub` or `pub(crate)`.
- `ui::legacy` is private and `shell` inside it is `pub(super)`, so the shell is exactly as visible as `app_shell` was. `crate::ui::screens` and `crate::ui::AppShell` are re-exported as before.
- The screens import the shell as `crate::ui::app_shell::AppShell`. `src/ui/mod.rs` keeps that path working with `use legacy::shell as app_shell;`, so no import outside `src/ui/mod.rs` changed.
- The test modules keep their names. Production code reaches into two of them in test builds (`characterization_tests::WipeWitness` in the state, `characterization_tests::record_error_notice` in the shell), and both paths still resolve without an edit.

## How to check it

```
git diff --color-moved=dimmed-zebra --color-moved-ws=allow-indentation-change p1a-05b-characterization...p1a-12a-decomposition
```

Moved lines are dimmed. Everything else should be one of these:

- module docs (`//!`), `mod` and `use` lines, and the `impl` header and closing brace of each new file,
- `pub(super)` in front of a private method that another file calls,
- lines rustfmt rewrapped: a signature that got longer than 100 columns with `pub(super)`, or a test line that fits on one line now that its module lost a level of indentation (one of them is in `stale_session_results_change_nothing`),
- the two section banners in the old `state.rs`, whose text went into the module docs of `open.rs` and `connect.rs`.

A move like this can hide one trap. A private method called from another file does not compile, unless a trait in scope has a method of the same name; then the call quietly goes to the trait. Every private method named in a file other than its own was checked: the hits are mentions in comments and the trait methods `render` and `focus_handle`, so no call changed its target.

`cargo test --locked -- --list` lists the same number of tests before and after. The state tests keep their names; the shell test only changed its module path, from `ui::app_shell::` to `ui::legacy::shell::`. `tests/keymap.rs` passes against its unchanged snapshot, and `tests/architecture.rs` finds no new `unsafe` and no new exception.

## What did not change

`src/app/mod.rs`, `src/app/actions.rs`, every screen, `Cargo.toml` and `Cargo.lock`, and every comment apart from the two banners.
