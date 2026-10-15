# P0.01: Moving to GPUI Kit

First step of the redesign, going into 0.9.3. GPUI now comes from the crates.io snapshot `gpui-pre 0.3.8` instead of a git dependency on Zed, and the components come from GPUI Kit (`gpui-component 0.7.1`). Before, 0.9.2 used gpui-component 0.5.2 from git, at commit `b004e59`.

The app should look and behave like 0.9.2, with two intended exceptions: macOS 26 is the new minimum, and ⌘F now also works from inside a text field. The toast position may also shift, which gets compared by hand.

Branch `p0-01-gpui-kit`, started from `master` at `d3a7e6d` (v0.9.2) on 2026-10-14. Everything that can only be checked by hand is in [Manual check before release](#manual-check-before-release) at the end.

## Checks

| Check | Result |
|---|---|
| `cargo fmt --all --check` | pass |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | pass (the only output is the known future-incompat note for `block` 0.1.6, same as v0.9.2) |
| `cargo test --locked` | pass, no new `#[ignore]` |
| `cargo build --locked --bin ferrispass-cli` | pass |
| Em and en dash check over tracked files and the commit messages in `master..HEAD` | no hits |
| `cargo audit` | failed on vulnerabilities that `master` already had: rustls 0.23.45 (RUSTSEC-2026-0346 to 0349, in the release binary) and quinn-proto 0.11.17 (RUSTSEC-2026-0338, 0340, 0343, not in the macOS release build). Fixed on this branch in `8368310` (rustls 0.23.46, quinn-proto 0.11.18), see [P0.02](p0-02-matrix-and-tests.md#security-update). |

Both dash checks in CI (tracked files and commit messages) had never worked. The runners use BSD grep, which has no `-P`, so both exited with code 2 and CI read that as clean. `ci.yml` now builds the pattern from UTF-8 bytes, searches with `-E` and runs a self-test first. If the check doesn't find the dash it planted itself, the job fails.

## Release build

Measured with `/usr/bin/time -l cargo build --release --locked` (fat LTO, `codegen-units = 1`) on 12 cores and 36 GB, macOS 27.0.1, SDK 27.0. The "before" numbers come from a fresh build of `d3a7e6d`, because the existing `target/release` was older than v0.9.2.

| | before (v0.9.2, warm) | after, cold (new Kit crates) | after, warm |
|---|---|---|---|
| Build time | 123.7 s | 147.4 s | 122.9 s |
| Peak memory (max RSS) | 3.15 GiB | 3.52 GiB | 3.33 GiB (+5.5%) |
| `ferrispass` | 16,482,752 B | 17,960,112 B (+9.0%) | 17,960,112 B |
| `ferrispass-cli` | 4,202,416 B | 4,218,848 B (+0.4%) | 4,218,848 B |

GPUI's inspector is not in the release build. No `inspector` feature shows up in the release feature graph (`cargo tree -e features,normal,build`), and `strings` finds no `ToggleInspector`, `inspector::` or `DivInspector`, only words from the zxcvbn list and the path `icons/inspector.svg`. The debug binary does contain `inspector::ToggleInspector`, so the check would catch it.

Without `MACOSX_DEPLOYMENT_TARGET`, rustc writes `minos 11.0` into the binary. `scripts/build-mac.sh` sets the target from `MIN_MACOS`, which is now 26.0, and now also stops if the selected SDK is older than that. `release.yml` runs on the same `macos-latest` image as CI, and the first CI run showed that image to be `macos-26-arm64` with SDK 26.5 (see P0.02), so release builds meet the new minimum.

With Apple's toolchain 27 (Xcode 27 beta and the Command Line Tools 27 alike), the local release build stopped with `E0463 can't find crate for ztracing_macro`. rustc strips the proc-macro dylibs, and once the deployment target is 12.0 or later, dyld refuses the stripped files ("mis-aligned LINKEDIT string pool"). This has nothing to do with the new minimum, 12.0 fails the same way. `[profile.release.build-override] strip = "none"` in `Cargo.toml` keeps the symbols in build scripts and proc macros, which only run on the build machine. With it, `scripts/build-mac.sh --skip-notarize` builds again after a clean rebuild of both proc macros, and the shipped binaries are still stripped (`nm -U` lists only `__mh_execute_header`). The `macos-latest` image still comes with SDK 26.5, and the setting keeps `release.yml` working once the image moves to toolchain 27.

## Supply chain

How the check works is described in [release-process.md](../release-process.md#gpui-supply-chain-on-every-gpui-bump). The script is `scripts/verify-gpui-pre.sh`, and its final run exited with 0. In short:

- All checksums match `Cargo.lock`. All 22 Zed crates point to zed@279fe07, and that commit is on Zed's `main`.
- 19 crates are identical to the Zed source. Three carry patches from Longbridge's public republish script `script/bump-gpui.ts`:
  - `actions!` derives `$crate::Action`.
  - `gpui-pre-apple` reads the shader sources from `vendor/`, which is byte-identical to Zed.
  - `gpui-pre-macros` rewrites generated paths to the `gpui_kit::` facade, but only when the crate being compiled lists `gpui-kit` as a dependency.
- The gpui-pre crates have no `.cargo_vcs_info.json`, so the script reads the Zed commit from `[package.metadata.gpui-pre]`.
- The GPUI Kit crates are byte-identical to tag v0.7.1. They only touch the network on wasm. On macOS they add no `unsafe` compared to 0.5.2, and the speech module sits behind a feature that is off.
- gpui-pre and gpui-component are published by the same crates.io account.

## Behavior compared with 0.9.2

### Key bindings

`tests/keymap.rs` compares every binding, in registration order, against `tests/snapshots/keymap.txt`. I collected the old bindings twice, once by reading the 0.5.2 source together with `actions.rs` and once at runtime through a temporary example. Both gave the same list.

| | old | new |
|---|---|---|
| Bindings without the inspector | 130 | 154 |
| FerrisPass's own (`actions::init`) | 19 | 19, same keys, contexts and order |

- Removed (2), both just renamed: in the `Dialog` context, `escape` moved from `dialog::CancelDialog` to `ui::Cancel` and `enter` from `dialog::ConfirmDialog` to `ui::Confirm`. FerrisPass doesn't use the Kit dialog.
- Added (24):
  - In the `Input` context, `alt-cmd-up` and `alt-cmd-down` for multiple cursors, which only matter in multi-line fields (FerrisPass has none), and `cmd-shift-f` for Replace. Fields that aren't searchable pass it on, and FerrisPass doesn't bind that key.
  - In the `Popover` context, `enter` and `space` on `ui::Confirm`. FerrisPass renders no popover. The group context menu uses `PopupMenu`.
  - `escape` in the ColorPicker, plus the bindings for TimeField (8), Command (4) and Carousel (6). FerrisPass uses none of these components.
- The Kit modules register in a different order, but that doesn't change which binding wins. Across contexts GPUI goes by context depth, and within the same context the relative order is the same as before.
- Debug builds bind `alt-cmd-i` to `inspector::ToggleInspector`, in the old and the new version. The test leaves out exactly that line, so any other inspector binding makes it fail.

### ⌘F inside a text field

This is the one change the snapshot can't show. I compared the old and new handlers for Enter, Escape, ⌘C, ⌘F and ⌘Backspace, and four of them behave as before. ⌘F changed: up to 0.5.2, a field that isn't searchable swallowed `input::Search`. Since 0.7 it passes the action on with `cx.propagate()`, so the `cmd-f` binding to `FocusSearch` fires. I'm keeping that (decided 2026-10-15). Outside overlays, ⌘F now focuses the search field from anywhere, as in other Mac apps.

It did open one gap. With an overlay open (entry editor, group, conflict, settings, switcher and so on), the search field is either not drawn or sits behind a modal, so the focus would have jumped out of the editor into a field you can't see. `AppShell::focus_search` now does nothing while an overlay is open (`Overlay::is_active`). The test `cmd_f_focuses_the_search_from_the_shell_and_from_a_text_field` covers the new behavior. The overlay case is in the manual check, because the test setup has no overlays. The 0.9.3 release note mentions the change in one sentence.

Both dispatch tests run headless, without Metal:
- `cmd_c_copies_the_password_from_the_list_but_not_from_a_text_field`: with the focus on the shell, ⌘C fires `CopyPassword`. In a focused field, ⌘C only copies the field's selection, and `CopyPassword` never fires there, with or without a selection.
- `cmd_f_focuses_the_search_from_the_shell_and_from_a_text_field`: see above.

### Protected fields

Tests in `src/ui/widgets/secret_input.rs`, headless on `TestPlatform`:
- A control run on an unprotected field first checks that each way of copying actually reaches the field: ⌘A ⌘C, ⌘A ⌘X, and Select All followed by Copy or Cut, sent the way the Edit menu sends them. The field must still have focus after each one. Without this, the protected cases could pass just because the action never arrived.
- Protected and masked: none of these puts anything on the clipboard, and the value stays.
- Protected and revealed with the eye button (`toggle_masked`, which is what `mask_toggle` calls): same result.
- Removing the copy and cut interception in `SecretInput::render` makes the revealed case fail, because the secret lands on the clipboard. The masked case still passes, since the library already blocks copying while masked.
- The protected field's context menu comes from `protected_context_menu()` and has only Paste and Select All. Whether `render` really hands it to `Input::context_menu` can't be observed headless, because `NativeMenu::show` goes straight to AppKit. That part is in the manual check.

### Other things checked

- Toasts: the defaults are unchanged (top right, same margins, shown inside the window), and the toast layer is still above the overlay. The position is compared by hand in the manual check.
- Theme: `apply_choice` first calls `Theme::change` or `Theme::sync_system_appearance`, then `Theme::update` with the palette in a separate call. Font families that are set explicitly stay as they are.
- Context menu: it already went through `NativeMenu` before. A custom builder replaces the default menu completely.

## Review

I had Codex (`gpt-5.6-sol`, reasoning effort `xhigh`) do three separate read-only reviews.

### Clipboard and protected fields

No leak, whether masked or revealed. While masked, `is_copyable()` returns false, so nothing is written to the clipboard. While revealed, the `capture_action` handlers of `SecretInput` stop Copy and Cut before the library's `on_action` handlers run. The context menu has only Paste and Select All. The touch overlay doesn't offer Copy and Cut while masked, and when revealed they go through the same actions and get stopped the same way. Dragging only extends the selection inside the field, and you can't drag text out of it. gpui-pre has no Services path (`writeSelectionToPasteboard`) for the input field. No findings.

### Supply chain

The three source patches change nothing that matters for security. In the script itself, Codex found several ways it could exit with 0 without having checked what it claims to check. All of them are fixed:
- Archives are checked entry by entry before extraction, and the script's Python helper writes each file itself. Only regular files and directories under `<crate>-<version>/` are allowed: no links, devices, absolute paths, `..` or duplicate entries. Every step in `fetch_crate` checks its own exit status, because the call sits inside `||`, where `errexit` doesn't apply.
- A file that exists upstream but is missing from the published crate is now a finding instead of a note.
- Findings on files carry the full SHA-256 of both sides, so a reviewed patch stops matching as soon as upstream changes.
- Dependencies are compared on every field (`registry`, `artifact`, `lib` and so on). A published dependency has to come from crates.io. The only accepted difference in a version requirement is a relaxed pin, meaning `X` or `^X` in place of `=X`. Anything else, such as `>=`, is a finding.
- The script now also compares the manifests of the replacement crates (zed-font-kit, zed-scap, gpui-pre-reqwest) and of the GPUI Kit crates, the same way it compares the gpui-pre manifests. Before, it skipped them.
- Stale entries in `REVIEWED_FINDINGS` fail the run.
- New: the feature graph from `cargo metadata` must not turn on the inspector, speech or GitHub download, and gpui-fast, gpui-shell and gpui-webview must not be in the graph at all. GPUI Kit has to come from `EXPECTED_KIT_REV` (87d10ae5), so a moved tag gets noticed. `curl` runs with `-q`, which ignores `~/.curlrc`.

I also fed the script bad input on purpose. It rejects archives with `..`, an absolute path, a symlink, a hard link, a FIFO, a duplicate entry or a name outside the crate directory, and writes nothing in those cases. It also flags an extra dependency in a replacement crate, a `registry` field, a `>=` pin, a changed upstream file under a reviewed patch, and a file that only exists upstream.

Codex left two questions open:
- zed-font-kit depends on `dirs` 5 instead of 6. That's harmless: on macOS, font-kit only calls `dirs::home_dir` (`src/sources/fs.rs`, `default_font_directories`). `mac.rs` is identical in dirs 5.0.1 and 6.0.0, and so is `home_dir` in dirs-sys 0.4.1 and 0.5.0 (`$HOME`, otherwise `getpwuid_r`). dirs 5.0.1 was already in `Cargo.lock` on `master`. The finding is in `REVIEWED_FINDINGS` with this reasoning.
- Does GPUI Kit 0.7.1 behave like 0.5.2? Comparing sources can't prove that. The evidence is the tests above, a review of every place where the migration had to change FerrisPass, and the manual check. A full diff from 0.5.2 to 0.7.1 is too large to review in any useful way. For reference, `cargo tree -e features,normal,build` shows these features in the macOS release build: `gpui-component` and `gpui-base` without features, `gpui-pre` with `default`, `font-kit`, `wayland`, `windows-manifest` and `x11`. The `wayland` and `x11` features are Linux platform code and aren't compiled on macOS.

I rejected one finding. Codex pointed out that the macro patch also rewrites string literals. That only happens when the manifest lists `gpui-kit`, and FerrisPass's doesn't. Without `gpui-kit` the patch passes every token through unchanged, and I couldn't change that crate's code anyway. The rewrite is now documented in detail in release-process.md.

### Theme

Same colors, radii and font families, both at startup without a window and when switching live. The assignments are the same as on `master`. In both versions `Theme::change` reloads the registered theme, and only after that does `apply` set the palette. The second `Theme::update` doesn't load any defaults. It reconciles the tokens, syncs the base theme and refreshes all windows. Explicitly set font families stay. The only question Codex left open was whether the built-in themes contain gradients that would survive the reconcile step. `default-theme.json` and `default-colors.json` contain none, and FerrisPass doesn't register theme files of its own. No findings.

## What turned out differently

- Dev dependencies: I meant to add `gpui-kit =0.7.1` with `test-support`. That doesn't work. `gpui-pre-macros` 0.3.8 rewrites every generated `gpui::` path to `gpui_kit::` as soon as the manifest lists `gpui-kit`, dev dependencies included, and the normal library build then fails. So `[dev-dependencies]` now has `gpui` (gpui-pre) and `gpui-component`, each with `test-support`, which are the features `gpui-kit/test-support` would turn on. If FerrisPass ever depends on `gpui-kit` directly, all gpui macros will go through the facade. I haven't decided yet whether to keep it this way.
- proptest is already in `Cargo.lock`, pulled in by `gpui-pre/test-support`, and only appears in test builds.
- gpui-base and gpui-component-macros are held at 0.7.1 with `cargo update -p ... --precise 0.7.1`, which only `Cargo.lock` records (there's a comment in `Cargo.toml`). A plain `cargo update` would raise them, and gpui-base 0.7.2 pulls gpui-fast into `Cargo.lock`.
- The password field's context menu is built by `protected_context_menu() -> gpui::Menu` and converted with `NativeMenu::from`, so a test can read the entries. The entries haven't changed.
- The README's license section says the GPL is required because GPUI depends on "ztracing / zlog (both GPL-3.0-or-later)". That's wrong: both are Apache-2.0, in Zed 1870e26 and in gpui-pre 0.3.8. I still have to rewrite that sentence. The license stays GPL-3.0-or-later.

## Manual check before release

"Old" is the installed v0.9.2, "new" is a build of this branch (`scripts/build-mac.sh --skip-notarize`, output `dist/FerrisPass.app`). Quit the old app before starting the new one, because both use the same bundle ID and the same settings.

First run on 2026-10-15 on macOS 27.0.1, with a test vault of 3,501 entries in six groups and the same settings for old and new. A tick means the item passed in the new build. Items without a tick are still open, and the note says how far they got.

**Minimum version**
- [x] `plutil -extract LSMinimumSystemVersion raw dist/FerrisPass.app/Contents/Info.plist` prints `26.0`
- [ ] Starts on macOS 26 (VM or a colleague's Mac) and on macOS 27. Starts on 27, not tried on 26 yet.
- [x] `dist/FerrisPass.app/Contents/Resources/Licenses/` contains `ISC-Lucide.txt` and `OFL-JetBrainsMono.txt`

**Unlock and lock**
- [x] Unlock with the password
- [ ] Unlock with Touch ID, including the automatic prompt after auto-lock (`f956864`)
- [ ] The Argon2 cost prompt when opening a KeePassXC vault with expensive KDF settings
- [ ] Auto-lock, and locking on screen lock and on sleep. Auto-lock after one minute works, and with an edited entry open it shows the error that the unsaved changes were discarded. Screen lock and sleep not tried yet.

**Toasts**
- [x] An error toast stays, a success toast fades out. Toasts stack, so the error toast is still there after a later one has faded.
- [x] Compare the position, old vs. new, and write it down. Same corner, same top edge, same right margin. The new toast is narrower, about 381 pt instead of 447 pt, so longer messages wrap earlier.

**Appearance**
- [x] Theme System, Light and Dark, each switched live while the app runs. ⌘⇧D toggles as before (Light, Dark, System). System only tried with macOS in light mode.
- [x] Font unchanged (the system font fix is a separate change, see SP09 in [gpui-matrix.md](../gpui-matrix.md))
- [ ] Icons: Check, Plus, Folder, Chevron up, down and right, Settings, Close, Star, StarFill, Eye, EyeOff. All unchanged except Chevron up, Chevron right and StarFill, which didn't come up.
- [ ] Scrollbars in the list and in the settings as before. Both scroll, the scrollbars themselves not compared yet.

**Password field**
- [x] Right-click in the protected field shows only Paste and Select All, masked and revealed
- [x] ⌘C and ⌘X copy nothing, masked and revealed (paste into TextEdit to check). Checked with a marker string on the clipboard, which stayed. The master password field doesn't copy either.
- [x] Edit > Copy in the revealed protected field copies nothing

**Edit menu and Kit components**
- [x] Undo, Redo, Cut, Copy and Paste in normal fields, from the keyboard and from the Edit menu
- [ ] Generator sliders and checkboxes, spinner, tooltips. They look right but haven't been used yet.
- [x] Group context menu with the arrow keys, Enter and Esc

**Keyboard**
- [x] ⌘C in the entry list copies the password. ⌘C in the search field with a selection copies the text
- [ ] ⌘F with no field focused, and from a field in the vault view, jumps to the search field. Works with no field focused, the second case is still open.
- [ ] ⌘F in the entry editor and in every other overlay does nothing, and the focus stays in the field. Checked in the entry editor only.
- [ ] Tab and ⇧Tab as before. Enter in the master password field unlocks, Esc cancels. Tab, Enter, and Esc in the editor (it asks before discarding) work. ⇧Tab still open.

**Auto-Type, launcher, sync**
- [ ] Hotkey ⌃⌥⌘V with the window hidden, and ⌘⇧T with the countdown
- [ ] SAP launch, and the temp file is deleted afterwards
- [ ] SharePoint sync and the conflict view
- [ ] iCloud watcher (notify 8): change the file from outside, the event arrives

**Lists, windows, updates, VoiceOver**
- [ ] Vault switcher (⌘O). A list with about 3,500 entries scrolls smoothly. ⌘O works and the list scrolls through all entries. Smoothness needs a look by eye.
- [ ] ⌘W hides and locks, a click on the Dock icon brings the window back. ⌘Q is refused while a save is running. Driven by UI automation, ⌘W, File > Close Window and the red close button did nothing, in 0.9.2 as well. Most likely the window wasn't key, so this needs a try by hand. ⌘Q quits.
- [x] The update check runs (the updater doesn't check the minimum version). "Check now" reports "You're on the latest version."
- [ ] VoiceOver spot check: labels unchanged or better. In 0.9.2 the accessibility API sees only the window title and the three window buttons, none of the content.

**Screenshots, old vs. new**, each in light and dark, same window size, same vault, stored outside the repo: Welcome, Unlock, vault browser, editor, settings (all tabs), conflict, switcher, About, toast, group context menu, password field context menu. Apart from the toast position, write down every difference here instead of quietly fixing it.

All taken except the conflict view, which needs a sync setup. Differences:
- Text in input fields starts about 2 pt further left, and the eye button in password fields sits about 5 pt further right.
- The generator checkboxes are blue instead of black. In dark mode they now look checked; in 0.9.2 they looked empty there.
- The clear button in the Auto-Type sequence field is a plain × instead of a circled one.
- In dark mode the group context menu is a slightly lighter gray.
- The native context menu of the password field stays light in dark mode, in both versions.
- Settings > General shows "Registration blocked" for the CLI in the new build. That's only because it runs from `dist/` while `/usr/local/bin/ferrispass-cli` points into `/Applications`.

Already in 0.9.2, not caused by the migration: the password health card showed 91 bits for an entry and later 45 bits for the same entry. I haven't looked into why yet.

- Checked on: 2026-10-15 on macOS 27.0.1, macOS 26 still open
- Differences: see above
- Migration approved for 0.9.3: [x] yes  [ ] no (2026-10-15)
