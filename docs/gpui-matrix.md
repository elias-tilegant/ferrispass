# GPUI capability matrix

What the redesign needs from GPUI, how each piece looks in the old Zed revision and in the new snapshot, and what the spikes found out. Started with P0.02 on 2026-10-15.

- **Zed 1870e26** is what `master` built through a git dependency up to v0.9.2.
- **gpui-pre 0.3.8** is the crates.io snapshot of zed@279fe07 that FerrisPass has used since P0.01 (`Cargo.toml`, GPUI section). GPUI Kit (gpui-component and gpui-base 0.7.1) sits on top of it.

Paths in the "Source" column are relative to `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`. Paths in the "Zed 1870e26" column are relative to `~/.cargo/git/checkouts/zed-a70e2ad075855582/1870e26/crates/`. A bare `:line` refers to the last file named in the same cell. All line numbers were checked against the sources on 2026-10-15. They shift with every gpui-pre bump, so the matrix needs rechecking after each one.

Anything that needs a window or the real AppKit platform gets tested in a spike on its own branch (`spike/p0-NN-name`). Until that spike has run, the row says "pending spike". Its result then goes into the row and into [Spike results](#spike-results).

## Matrix

| Capability | Zed 1870e26 | gpui-pre 0.3.8 | Source | Runtime check | Spike |
|---|---|---|---|---|---|
| `WindowKind::PopUp`: window class | NSPanel subclass `GPUIPanel` with style `NonactivatingPanel` (`gpui_macos/src/window.rs:134`, `:945-947`) | same | `gpui-pre-macos-0.3.8/src/window.rs:129` (`GPUIPanel` from `NSPanel`), `:95-96` (mask `1 << 7`), `:1019-1022` (PopUp: mask plus `PANEL_CLASS`) | pending spike | SP03, SP05 |
| `WindowKind::PopUp`: level | `NSPopUpWindowLevel` = 101 (`gpui_macos/src/window.rs:94`, `:1159`) | same | `gpui-pre-macos-0.3.8/src/window.rs:103`, `:1231` | pending spike | SP03 |
| `WindowKind::PopUp`: Spaces | `CanJoinAllSpaces` and `FullScreenAuxiliary` (`gpui_macos/src/window.rs:1164-1167`) | same | `gpui-pre-macos-0.3.8/src/window.rs:1236-1239` | pending spike | SP05, SP03 |
| `WindowKind::PopUp`: mouse without focus | tracking area with `ActiveAlways`, set up in the PopUp branch itself (`gpui_macos/src/window.rs:1145-1157`) | same, only moved into the helper `add_mouse_tracking_area` with the same four options | `gpui-pre-macos-0.3.8/src/window.rs:1229`, `:2488-2508` | pending spike | SP03 |
| `WindowKind::AnchoredPopup` | rejected on macOS (`gpui_macos/src/platform.rs:659`) | same | `gpui-pre-macos-0.3.8/src/platform.rs:772` | not needed, the source is clear | none |
| Key status: `canBecomeKeyWindow` | always `YES` for `GPUIWindow` and `GPUIPanel`, both from `build_window_class` (`gpui_macos/src/window.rs:375-378`, `:133-134`) | **unchanged, always `YES`.** A PopUp can become key as soon as it gets focus, so SP03 is still needed | `gpui-pre-macos-0.3.8/src/window.rs:438-441` (`canBecomeKeyWindow`), `:434-437` (`canBecomeMainWindow`), `:128-129` (both classes) | pending spike | SP03 |
| Key status: `focus: false` with `orderFront` | `focus && show` calls `makeKeyAndOrderFront`, otherwise only `orderFront` (`gpui_macos/src/window.rs:1216-1220`) | same. `PlatformWindow::activate`, however, always makes the window key | `gpui-pre-macos-0.3.8/src/window.rs:1288-1292`, `activate` `:1788`, `:1797` | pending spike | SP03, SP01 |
| `acceptsFirstMouse` | registered, returns `YES` (`gpui_macos/src/window.rs:288`, `:3195`) | same, and also remembers `first_mouse` | `gpui-pre-macos-0.3.8/src/window.rs:282-285`, `:3521-3526` | pending spike | SP03 (first click into the panel) |
| `window.prompt` | `NSAlert` as a sheet on the window (`gpui_macos/src/window.rs:1510`, `:1531`, `:1586`; `gpui/src/window.rs:6244`) | same. GPUI Kit doesn't install its own `PromptBuilder` (`set_prompt_builder` appears nowhere in `gpui-component-0.7.1` or `gpui-base-0.7.1`), so prompts still use the native `NSAlert` | `gpui-pre-macos-0.3.8/src/window.rs:1704`, `:1725` (`NSAlert::new`), `:1780` (`beginSheetModalForWindow`); `gpui-pre-0.3.8/src/window.rs:6529`, `:6550-6553` (`PromptBuilder::Default` goes to the platform) | pending spike | none |
| Dock menu: `set_dock_menu` | present (`gpui/src/app.rs:2435`; `gpui_macos/src/platform.rs:1061`, delegate `applicationDockMenu` `:139`) | same | `gpui-pre-0.3.8/src/app.rs:2679-2681`; `gpui-pre-macos-0.3.8/src/platform.rs:1172-1182`, delegate `:147-150`, handler `:1644` | pending spike | none |
| `on_open_urls`: delivery | callback on the `Application` builder, delegate `application:openURLs:` (`gpui/src/app.rs:270`; `gpui_macos/src/platform.rs:143`, `:777`, `:1421-1443`) | same. **Without a registered callback the delegate drops the URLs** (`take()`, no buffer). For a cold start, `on_open_urls` has to be set before `run` | `gpui-pre-0.3.8/src/app.rs:287-293`; `gpui-pre-macos-0.3.8/src/platform.rs:151-154`, `:891-893`, `:1568`, `:1585` | pending spike | SP06 |
| `on_app_quit` | present, can't cancel the quit, handlers get 200 ms (`gpui/src/app.rs:2352`, `:78`; `gpui/src/app/context.rs:208`; delegate `applicationWillTerminate` `gpui_macos/src/platform.rs:98`) | same | `gpui-pre-0.3.8/src/app.rs:2594-2612`, `SHUTDOWN_TIMEOUT` `:78`; `gpui-pre-0.3.8/src/app/context.rs:200`; `gpui-pre-macos-0.3.8/src/platform.rs:107`, `:1472-1479` | pending spike | SP14 |
| `observe_window_activation` | present (`gpui/src/app/context.rs:444`) | same | `gpui-pre-0.3.8/src/app/context.rs:420` | pending spike | SP02, SP03 |
| `remove_window` | present (`gpui/src/window.rs:2187`) | same. It only sets `removed`, and the window closes at the end of the update | `gpui-pre-0.3.8/src/window.rs:2324-2326` | pending spike | SP02 |
| `KeyBinding::load` with a mapper | `use_key_equivalents` and `PlatformKeyboardMapper` (`gpui/src/keymap/binding.rs:48`) | same. The mapper only swaps punctuation, using Apple's localized table (German: `/` becomes `ß`, `[` becomes `ö`). Letters aren't remapped and still match by character | `gpui-pre-0.3.8/src/keymap/binding.rs:48-66`; `App::keyboard_mapper` `gpui-pre-0.3.8/src/app.rs:1139`; `gpui-pre-macos-0.3.8/src/platform.rs:1144`; tables `gpui-pre-macos-0.3.8/src/keyboard.rs:111` | tested headless (SP10): German, Swiss and US read through `UCKeyTranslate` without switching the input source, dispatch in a headless window. The German table matches the live mapper on my Mac | SP10 |
| Layout change at runtime | `on_keyboard_layout_change` (`gpui/src/app.rs:1016`; `gpui_macos/src/platform.rs:1359`) | same. **It only replaces the mapper. Bindings that are already loaded keep the old table**, and neither GPUI Kit nor gpui-base rebinds them. The shortcut registry has to reload in the callback | `gpui-pre-0.3.8/src/app.rs:1144`, `:966-977`; `gpui-pre-macos-0.3.8/src/platform.rs:156-159`, `:1482-1486` | not needed, the source is clear (SP10) | SP10 |
| `HasWindowHandle` | `Window` and `MacWindow` (`gpui/src/window.rs:7098`; `gpui_macos/src/window.rs:2205`) | same. The handle holds the `NSView`, and the `NSWindow` (or `NSPanel`) comes from `[view window]` | `gpui-pre-0.3.8/src/window.rs:7434`; `gpui-pre-macos-0.3.8/src/window.rs:2410-2417` | pending spike | SP03, SP12 |
| `sharingType` | not there | not there, and not in GPUI Kit either. Only reachable through `HasWindowHandle` and objc2, in a future `src/platform/panel.rs` | `sharingType` and `NSWindowSharing` appear nowhere in `gpui-pre-0.3.8` or `gpui-pre-macos-0.3.8` | pending spike | SP12 |
| `cx.hide` | `NSApp hide:` (`gpui/src/app.rs:1307`; `gpui_macos/src/platform.rs:598`) | same | `gpui-pre-0.3.8/src/app.rs:1472-1474`; `gpui-pre-macos-0.3.8/src/platform.rs:711-716` | pending spike | SP05 |
| `reduce_motion` | only the flag on `App`, the platform never reads the setting (`gpui/src/app.rs:1059`) | same flag, and the macOS platform still reads nothing. **New with GPUI Kit:** `gpui_base::init`, called from `gpui_component::init`, reads `accessibilityDisplayShouldReduceMotion` once at startup. On macOS it doesn't notice later changes. Calling `apply_system_reduce_motion` picks them up | `gpui-pre-0.3.8/src/app.rs:1222-1233`; `reduce_motion` doesn't appear in `gpui-pre-macos-0.3.8`; `gpui-base-0.7.1/src/reduce_motion.rs:108-112`, `:65`, `gpui-base-0.7.1/src/lib.rs:231`, `gpui-component-0.7.1/src/lib.rs:134` | pending spike | P0.07 |
| AccessKit roles | accesskit 0.24 and accesskit_macos 0.26 (workspace `Cargo.toml:515-516`), `SubclassingAdapter` (`gpui_macos/src/window.rs:2160`), `role`, `aria_label`, `aria_value` (`gpui/src/elements/div.rs:1268`, `:1290`, `:1384`) | same, same versions | `gpui-pre-0.3.8/Cargo.toml:265-267`; `gpui-pre-macos-0.3.8/Cargo.toml:58-63`; `gpui-pre-macos-0.3.8/src/window.rs:2365`; `gpui-pre-0.3.8/src/gpui.rs:97-99`; `gpui-pre-0.3.8/src/elements/div.rs:1310`, `:1332`, `:1341`, `:1426`; `gpui-pre-0.3.8/src/element.rs:114` | pending spike (VoiceOver, Accessibility Inspector) | SP03 (VoiceOver), later UI tests |
| Minimum macOS of gpui-pre | shaders compiled for 10.15.7 (`gpui_apple/build.rs:147`), runtime checks for 12 and 15 | same. Far below FerrisPass's minimum of 26, so nothing to decide | `gpui-pre-apple-0.3.8/build.rs:138`; `gpui-pre-macos-0.3.8/src/platform.rs:847`; `gpui-pre-macos-0.3.8/src/window.rs:365` | not needed | none |

### What this means for the spikes and P0.07

- SP03 is still needed. `canBecomeKeyWindow` is still hard-coded to `YES` in gpui-pre 0.3.8, and `activate` makes every window key. A panel that never becomes key needs either its own subclass or `setBecomesKeyOnlyIfNeeded` through `HasWindowHandle`.
- SP06 has to test a cold start. URLs that arrive before the callback is set are lost, so `on_open_urls` goes on the `Application` builder before `run`, not into the first frame.
- SP12 builds on `HasWindowHandle`, because GPUI has no `sharingType`.
- For P0.07: GPUI Kit reads "Reduce motion" at startup and ignores later changes. What's left is deciding whether FerrisPass calls `apply_system_reduce_motion` again whenever the app becomes active.

## Headless UI tests in CI

Do headless UI tests run on the `macos-latest` runner? Going by the source, FerrisPass's tests need neither Metal nor a window server.

- `#[gpui::test]` builds a `TestAppContext` on `TestPlatform::new`, with `NoopTextSystem` and no renderer (`gpui-pre-0.3.8/src/app/test_context.rs:131`, `gpui-pre-0.3.8/src/platform/test/platform.rs:124-131`).
- Windows then get a `HeadlessAtlas` instead of a Metal atlas (`gpui-pre-0.3.8/src/platform/test/window.rs:90-93`).
- Only `HeadlessAppContext` with `gpui_platform::current_headless_renderer` needs Metal (`gpui-pre-0.3.8/src/app/headless_app_context.rs:82`, `gpui-pre-platform-0.3.8/src/gpui_platform.rs:111`). FerrisPass doesn't use it. Pixel comparisons in later UI tests would need it, and only then does it matter whether the runner has Metal.

The P0.01 tests (`tests/keymap.rs` and the tests in `src/ui/widgets/secret_input.rs`) already pass locally on this test platform. The first CI run on the branch (PR #2, 2026-10-15, image `macos-26-arm64`) confirms it: every test passes there, the headless UI tests included.

SP09 used the Metal path locally. `HeadlessAppContext::with_platform`, with the text system of a headless macOS platform and `gpui_platform::current_headless_renderer()` (feature `test-support` of gpui-pre-platform), renders a view into an image without a window. That works for pixel comparisons on a Mac. Whether the runner has Metal is still open.

## Spike results

Each spike adds a row here and a short report with measurements. The result is "passed", "failed" or "fallback chosen", or "prepared" while the spike is built but hasn't run yet. "Step" is the redesign step that needs the answer.

| No. | Spike | Step | Result | Measured | Fallback | Date, branch |
|---|---|---|---|---|---|---|
| SP01 | Touch ID from the panel, focus return, auto-type while locked | P0.04 (without auto-type), P0.09 (auto-type part) | | | | |
| SP02 | Reactivation within 1 s after closing | P0.03 | | | | |
| SP03 | Panel that never becomes key | P0.08 | | | | |
| SP04 | Reading browsers through the accessibility API | P0.10 | | | | |
| SP05 | Panel when the app is hidden with ⌘H and when its window is closed | P0.03 | prepared | - | - | 2026-10-15, `spike/p0-03-quick-panel` (d96e026), run pending |
| SP06 | URL scheme and document types | P0.11 | prepared | parser tests 3 of 3 | - | 2026-10-15, `spike/p0-11-url-scheme` (9f08037), run pending |
| SP07 | Status item with the notch (a), login item that knows how it was started (b) | P0.05 (a), P0.11 (b) | | | | |
| SP08 | KeePassXC round trip with CustomData | P0.12 | fallback chosen: merge filter (CLI pass done, GUI pass open) | marker values byte-identical with keepassxc-cli 2.6.6, 2.7.12 and 2.8.0-beta1. Item times get lost. Without the filter, 3 entry conflicts and 1 group conflict per KeePassXC save, with the filter 0 (6 of 6 pairs, 10 of 10 rounds) | merge filter as in the report, with each marker's clock stored in its value | 2026-10-15, `spike/p0-12-keepassxc-roundtrip` (ad3209a, d93dfff) |
| SP09 | Font | P0.07 | passed, Medium only at weight 530 | `.SystemUIFont` is within 0.00 to 0.11% of the CoreText system font (11 to 28 pt). The current stack resolves to Helvetica (-4.63% at 13 pt). With `tnum` every digit is 8.112 wide | none needed. Setting "SF Pro" directly doesn't work | 2026-10-15, `spike/p0-07-font` (bf56eea) |
| SP10 | German keyboard layout | P0.14 | passed | 35 of 35 app shortcuts fire in a headless window on each layout (German, Swiss, US). No shortcut swallows any of the 19 Option characters tested | keep ⌘↑ instead of ⌘[ | 2026-10-15, `spike/p0-14-keyboard-layout` (905bf11) |
| SP11 | One LAContext for several vaults | P0.09 | | | | |
| SP12 | `sharingType` against Teams, Zoom, Webex | P0.13 | | | | |
| SP13 | Clipboard history apps (a), privacy prompt at launch (b) | P0.13 (a), P0.06b (b) | b prepared. a open (needs a session with third-party apps) | - | - | 2026-10-15, b: `spike/p0-06b-pasteboard-privacy` (747a94d), run pending |
| SP14 | Secure Event Input in the GPUI password field | P0.06 | prepared | - | - | 2026-10-15, `spike/p0-06-secure-input` (b06bb8a), run pending |

### Reports

One subsection per spike: setup, measurements, decision. Runs that need a window, a global shortcut or a system dialog are prepared and done together in one session on a real Mac. The steps for that session and the test data live next to the repo in `ferrispass-spikes/`, not in it.

#### SP08 KeePassXC round trip with CustomData (P0.12)

**Setup.** `examples/spike_12.rs` uses keepass-rs to write a KDBX 4.1 file with FerrisPass's own CustomData keys. They are called markers here, to keep them apart from KeePass's built-in Tags field. The group `Kunden` gets `FerrisPass.Role`, two entries get `FerrisPass.Type`, `FerrisPass.RotationSince` and `FerrisPass.Via` (one of these markers is written without an item time), and the Meta block gets `FerrisPass.Fav.<vault>.<UUID>`. `keepassxc-cli edit` then changes another entry once and the marked entry once, each with 2.7.12 (installed), 2.6.6 (previous minor) and 2.8.0-beta1. The decrypted XML is compared byte for byte and also run through the app's own `merge::diff` and `apply_picks`. No window is involved, and the GUI pass is still open. Test data: `ferrispass-spikes/sp08/run-20261015-103307`.

What came out:
- Marker values are byte-identical in all three versions, including values that keepass-rs reads as binary.
- The per-item `LastModificationTime` is lost: KeePassXC never writes it for entries and groups and writes it for Meta only in KDBX 4.1, and `keepassxc-cli` saved the 4.1 file as 4.0 in all three versions. KeePassXC 2.6.6 reads and writes the 4.1 file without errors.
- KeePassXC adds a `_LAST_MODIFIED` key with a localized date string to every non-empty CustomData block, and stamps it again on every load without updating the object's modification time. In KeePassXC 2.7.12, `CustomData::set` calls `updateLastModified` for the item it just read as well. It was re-stamped 30 out of 30 times in 10 rounds.
- Merging today (0.9.x) gives 3 entry conflicts and 1 group conflict after every KeePassXC save, on entries nobody changed, in both directions. The causes are `_LAST_MODIFIED` and the lost item times on marked objects, plus one that has nothing to do with markers: keepass-rs creates new entries without an Auto-Type block, and so does FerrisPass's own `create_entry`. KeePassXC writes its default block on the next save without bumping the modification time, and `metadata_differences` compares `autotype` as a whole.
- If changing a marker leaves the entry's or group's modification time alone, the merge asks the user to pick a side. If it bumps the time, the merge resolves it on its own. But if a marker is set with a bumped time and the entry is later edited on the other side, the newer side wins the whole entry and the marker change is silently lost.
- A prototype merge filter (`fdiff`) leaves `_LAST_MODIFIED`, `KPXC_RANDOM_SLUG` and all `FerrisPass.*` keys on entries and groups out of the comparison, ignores item times and treats a missing Auto-Type block as KeePassXC's default. With it there were 0 conflicts in 6 of 6 round-trip pairs, no merge prompts in 10 of 10 rounds of repeated saves, and marker-only changes merged without asking.

**Decision.** Use the merge filter defined above. For the marker work later on, I'd merge markers key by key, with the clock stored in the value (for example `v1|<Unix seconds>|<content>`, and a deletion as a tombstone with empty content), because no item time survives KeePassXC. That format is also never valid Base64. In that design, setting a marker should not bump the entry's modification time, and marker values should be read as XML text (a string, or the Base64 of the binary value).

This already affects 0.9.x. `_LAST_MODIFIED` on entries and groups and the Auto-Type default cause false conflicts as soon as an entry carries CustomData (KeePassXC-Browser keys, for example) or was created in FerrisPass. Whether to fix this before 0.10.0 is still open.

#### SP09 Font (P0.07)

**Setup.** `examples/spike_07.rs` creates the macOS platform headless and never starts it (no `NSApplication`, no window), then measures with its CoreText text system. The reference is a CoreText line in `kCTFontUIFontSystem`, which is `.SFNS-Regular`, the system font TextEdit uses. GPUI's headless renderer also wrote a test image, `sp09/out/spike_07.png` (not committed).

What came out:
- GPUI treats the whole font stack in `src/ui/theme.rs:28-29` as one family name, doesn't find it and falls back to Helvetica (same FontId). `"SF Pro"` and `"SF Pro Text"` don't exist as families and fall back to Helvetica as well. `.SystemUIFont` resolves to `.SFNS`.
- Width of the test sentence compared to the CoreText system font: `.SystemUIFont` -0.10% (11 pt), 0.00% (13 pt), +0.11% (17 pt), +0.11% (20 pt), -0.11% (28 pt). The current stack: -6.89%, -4.63%, -0.61%, +1.95%, +4.49%.
- 400, 600 and 700 are separate faces. 500 lands on Regular, and 800 and 900 both land on Black, although CoreText has all nine faces. The reason is that font-kit maps the weight trait to CSS weights piecewise (`FONT_WEIGHT_MAPPING`), and SF Medium sits at about 530 there. `FontWeight(510)` to `FontWeight(530)` gives Medium, 750 to 780 gives Heavy. With the embedded Atkinson files, 500 gives Medium as expected.
- At 13 pt, the digits 0 to 9 are 5.954 to 8.290 pt wide without `tnum` and all 8.112 with it.
- Atkinson Hyperlegible Mono (OFL 1.1 without a Reserved Font Name, upstream `googlefonts/atkinson-hyperlegible-next-mono` 154d503): the static TTFs load through `add_fonts`, and every glyph is 8.216 wide. The variable `[wght]` file alone only gives one face, so the static files are the ones to embed.
- No missing glyph for ÄÖÜäöüß€„“. Decomposed and precomposed umlauts have the same width and the same number of glyphs.

**Decision.** Passed. When the theme switches to the system font, it sets `font_family` to `.SystemUIFont` and uses `FontWeight(530.0)` for Medium, with a headless test that tells Medium apart from Regular and Semibold.

#### SP10 German keyboard layout (P0.14)

**Setup.** `examples/spike_14.rs` reads the German, Swiss and US layouts through `TISCreateInputSourceList` and `UCKeyTranslate` without selecting an input source. It then uses GPUI's own translation (`parse_keystroke`, `gpui-pre-macos-0.3.8/src/events.rs:421-482`) to work out which key gives which keystroke. The planned shortcut list is loaded with `KeyBinding::load` and GPUI's tables. The tests press every shortcut the way someone with that layout would, through a headless window with the FerrisPass and GPUI Kit keymaps. All 5 tests pass. Adding an extra `alt-l` binding and loading without `use_key_equivalents` made three of them fail, as they should. HIToolbox aborts when TIS isn't called on the main thread, hence `--test-threads=1`.

What came out:
- All 35 app shortcuts fire on all three layouts. Letters bind by character, not by position, so ⌘Y and ⌘Z swap places on QWERTZ, as in every Mac app. ⌃⌥⌘P and ⌃⌥⌘V go through Carbon by keycode, which means by position. P and V sit in the same place on all three layouts.
- With `use_key_equivalents`, ⌘/ is shown and pressed as ⌘ß (German), ⌘' (Swiss) and ⌘/ (US). Without it, German and Swiss users have to press ⇧⌘7. ⌘[ can't be typed on German and Swiss without key equivalents, and with them it's ⌘ö. ⌘↑, which the planned list uses instead, works everywhere.
- No shortcut swallows any of the 19 Option characters tested, and all of them reach the text field. That covers German ⌥L = @, ⌥E = €, ⌥5 to ⌥9 for brackets and |, ⌥⇧7 = \, and Swiss ⌥G = @. Because `Keystroke::should_match` also checks a keystroke with a `key_char` against a binding without ⌥, the shortcut registry must not bind `alt-<letter>` or `alt-<digit>` without ⌘ or ⌃.
- GPUI's `Display` shows ⌃ as `^`, puts ⌘ before ⇧ and spells out `enter`. The keycap component in the new design draws the glyphs itself, in Apple's order ⌃⌥⇧⌘.
- For layout changes, see the matrix. The shortcut registry rebinds in `on_keyboard_layout_change`.

#### SP05, SP06, SP13 b, SP14 (prepared)

Each spike's example, run script and results template are on its branch:
- `spike/p0-03-quick-panel`: `examples/spike_03.rs`, `spike/run_sp05.sh`
- `spike/p0-11-url-scheme`: `examples/spike_11.rs`, `spike/run_sp06.sh`, plus the link parser with unit tests
- `spike/p0-06b-pasteboard-privacy`: `examples/spike_06b.rs`, `spike/run_sp13b.sh`
- `spike/p0-06-secure-input`: `examples/spike_06.rs`, `examples/spike_06_tap.rs`, `spike/run_sp14.sh`

All of them build, and none has run yet. They need a window, the global shortcut ⌃⌥⌘P, LaunchServices or a possible system dialog, and SP14 also needs Input Monitoring for the terminal. SP13 a runs later in a session with the third-party apps.
