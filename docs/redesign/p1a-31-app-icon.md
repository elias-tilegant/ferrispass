# P1a.31: App icon from Icon Composer

The app icon is Ferris holding a key, on cream, and it is built the way macOS 26 expects: a layered Icon Composer document that `actool` compiles into `Assets.car`, named in `Info.plist` with `CFBundleIconName`. macOS draws the dark, tinted and clear appearances from it. The release build runs on a fixed Xcode 26, and the build script now refuses to package binaries that the build did not produce.

| Commit | What |
|---|---|
| `843fa7d` | The icon document `bundle/icon/AppIcon.icon` and the layer drawings in `design/brand/icon/` |
| `4a16ac6` | `scripts/build-mac.sh` compiles the document with `actool`; `CFBundleIconName` in `Info.plist`; `bundle/icon.png` removed |
| `4b0bef1` | `scripts/check-bundle-binaries.sh` and its call right after the binaries are copied |
| `3a7b32d` | The release build on the `macos-26` image with Xcode 26.6 |
| `fd6b02e`, `bd855de` | `tests/app_bundle.rs` |
| `6d3f91d` | README, `docs/architecture.md`, `docs/release-process.md` |
| `2785fe2` | The bundle check judges a stale binary by the dep-info cargo writes next to it, with the test and `docs/release-process.md` |
| `076bed1` | The bundle check also stops when the dep-info names a file that no longer exists, with the test and `docs/release-process.md` |
| `d8b7027` | The test of the call requires `set -euo pipefail` before it |

## Where the icon's sources live

- `bundle/icon/AppIcon.icon` is what the build reads: `icon.json` and three SVG layers under `Assets/`. There is one group per layer, from top to bottom `key`, `face` and `body`. `key` and `face` are opaque, `body` is slightly glassy (translucency 0.25), and all three have a neutral shadow. The background is the document's fill, a cream gradient, and a graphite one for the dark appearance. The document's name has to match `--app-icon` in the build script and `CFBundleIconName`.
- `design/brand/icon/` holds the same three layers, `build.py`, which draws them from hand-set geometry, and a README on how to change them. `python3 -I design/brand/icon/build.py <folder>` writes the three layers byte for byte as they are in the repo (checked with `cmp`), plus two previews on cream and on graphite.
- Nothing is rendered in CI, and no raster image of the icon is in the repo.

## How the build compiles it

Step 2 of `scripts/build-mac.sh`:

1. Reads the version from `xcrun actool --version` and stops when it is older than 26 or cannot be read. Only a full Xcode has `actool`; the Command Line Tools alone do not.
2. Runs `xcrun actool` on the document with `--app-icon AppIcon --include-all-app-icons --platform macosx --target-device mac`, the deployment target from `MIN_MACOS` and a partial `Info.plist` into a temporary folder in `dist/`.
3. Stops unless `Assets.car` is there and not empty: `actool` can end with 0 without writing it.

Step 3 copies `Assets.car` into `Contents/Resources`, and `AppIcon.icns` too when `actool` wrote one, then removes the temporary folder. Before the bundle is signed, `xcrun assetutil --info` writes `dist/assets-car.json`, and the build stops unless that report lists `AppIcon` in more than one appearance. A catalog with the icon in one appearance only would show the default icon in dark, tinted and clear as well. The signature in step 4 seals both files.

`Info.plist` keeps `CFBundleIconFile` next to `CFBundleIconName`, since the build takes along the `.icns` that `actool` writes. The partial `Info.plist` from `actool` names the same two keys.

The release job runs on the `macos-26` image and selects `/Applications/Xcode_26.6.app` right after checkout, the default and newest Xcode 26 on that image on 13.11.2026. Each image carries one major version of Xcode, so `macos-latest` would bring a newer `actool` once it moves on. If the image drops 26.6, the step fails and the release stops before it builds.

## The bundle check

`scripts/check-bundle-binaries.sh <release_dir> <copies_dir> <source_root>` runs right after the build script copies `ferrispass` and `ferrispass-cli` out of `RELEASE_DIR`. For each of the two it stops the build

- when the copy has other bytes than the binary in `RELEASE_DIR` ("the bundle does not carry the binary this build produced; CARGO_TARGET_DIR points somewhere else"),
- when the dep-info file cargo writes next to that binary, `ferrispass.d` or `ferrispass-cli.d`, is missing or cannot be read ("cargo wrote no dep-info for ferrispass; the build did not come from cargo", with the binary's name),
- when that dep-info names no file under the source root (the first sentence again),
- when a file the dep-info names no longer exists ("a source file the binary was built from no longer exists; the build did not rebuild it"), and
- when a file the dep-info names is newer than that binary ("a source file is newer than the binary; the build did not rebuild it").

The dep-info is in make's format, `<binary>: <file> <file> ...`, with a space in a path written as `\ ` and a line that ends in `\` continued on the next. It lists the files cargo compiled into the binary: the Rust sources and what they take in with `include_str!` and `include_bytes!`, such as the locales, the assets and `bundle/minisign-pub.txt`. Test modules, license texts and the docs are not in it, so a change to them alone does not stop a build that cargo rightly left alone. A binary that cargo did not have to rebuild is older than the build but newer than every file it was made from, so it passes. Binaries left in a target folder that cargo did not write to fail as soon as one of their files has changed or been deleted since they were built. Cargo takes a deleted file for a change as well: it rebuilds, or fails without the file, so after a build that cargo just ran every file the dep-info names exists. The call in the build script has no exit of its own; the check's exit code stops the build through the script's `set -euo pipefail`. The check runs on the release runner too, which is macOS with the same `awk`, `find` and `cmp` as a local build.

## What the tests show

`tests/app_bundle.rs` reads the files as text, except for the last test:

- `the_icon_source_is_the_composer_document`: the document has the three groups in their order, one layer each, its layers are the drawings in `design/brand/icon/`, it has a fill for the dark appearance, and `bundle/icon.png` is gone.
- `info_plist_names_the_icon`: `CFBundleIconName` is `AppIcon`, the document of that name exists, and `actool` is called with `--app-icon` and that name.
- `the_build_script_guards_actool_and_the_car`: the source path and the precondition on it; from the version guard, the `case` line that turns a version that is not a number into 0 and the comparison with 26, in that order and before `actool`; the `actool` options; the `test -s` on `Assets.car` after `actool`; then the copy, followed by the removal of the temporary folder and by the `jq -e` line that stops unless `${ICON_APPEARANCES}` has more than one entry, both before signing. Each of these `if` lines has an `exit 1` before its `fi`. No `sips`, `iconutil` or `icon.png` is left. The lines that feed the guards are not held: how the version is read from `actool --version` and cut to its major part, the `assetutil --info` run that writes the report, and the filter in `ICON_APPEARANCES` that picks the appearances of `AppIcon` out of it.
- `the_build_script_calls_the_bundle_check`: the call is the line right after the two copies, word for word, and the bundle takes the binaries from those copies. `set -euo pipefail` comes before the call, and no line from there up to the call has `set +e`, `set +o errexit` or `|| true`.
- `the_release_workflow_checks_the_icon_folder_and_pins_xcode`: the job that runs the build script runs on `macos-26`, selects one fixed Xcode 26 release before the build, and stops when `bundle/icon/AppIcon.icon` is missing.
- `the_bundle_check_refuses_a_stale_or_foreign_binary` runs the script with `sh` in a temporary folder that holds a release folder with a dep-info file for each binary, the copies and a source tree with a space in its path, with modification times set by the test. Each dep-info names five files, under `src/`, `locales/`, `assets/` (one with a space in its name) and `bundle/`, two of them on a continued line. Same bytes and older files pass, and so do a newer test module, license text and doc that the dep-info does not name. For each binary on its own: each named file that is newer than that binary, but not the other one, fails with the stale sentence and names the file and the binary; a file named in the dep-info of that binary only and deleted since fails with its own sentence and names the dep-info and the file; a missing dep-info fails with the dep-info sentence; a dep-info that names the files of another checkout, a copy with other bytes and a missing copy fail with the first sentence. No cargo runs in the test.

## Mutation probes

Each probe changed one thing on a clean tree, ran `cargo test --locked --test app_bundle`, and was undone with `git checkout`.

| What was changed | Test that failed | Failing line |
|---|---|---|
| The `test -s` on `Assets.car` removed | `the_build_script_guards_actool_and_the_car` | ``scripts/build-mac.sh has no line `if ! test -s "${ICON_OUT}/Assets.car"; then` `` |
| `CFBundleIconName` removed from `Info.plist` | `info_plist_names_the_icon` | `bundle/macos/Info.plist has no CFBundleIconName; macOS 26 then ignores Assets.car` |
| `bundle/icon.png` put back | `the_icon_source_is_the_composer_document` | `bundle/icon.png is back; the app icon is bundle/icon/AppIcon.icon` |
| The "Select Xcode 26" step removed from `release.yml` | `the_release_workflow_checks_the_icon_folder_and_pins_xcode` | `the release build selects Xcode 0 times` |
| The call of the bundle check removed from the build script | `the_build_script_calls_the_bundle_check` | `the bundle check has to run right after the two copies` |
| `set -euo pipefail` turned into `set -uo pipefail` | `the_build_script_calls_the_bundle_check` | ``scripts/build-mac.sh has no line `set -euo pipefail` `` |
| `set +e` added right after `set -euo pipefail` | `the_build_script_calls_the_bundle_check` | ``a line between `set -euo pipefail` and the bundle check has `set +e`: set +e`` |
| The bundle check judges `ferrispass` only | `the_bundle_check_refuses_a_stale_or_foreign_binary` | `src/lib.rs is newer than ferrispass-cli:` followed by the check's own success line |
| `-newer` turned into `! -newer` | `the_bundle_check_refuses_a_stale_or_foreign_binary` | `the copies are the binaries cargo built:` followed by "a source file is newer than the binary" |
| The version guard at 25 instead of 26 | `the_build_script_guards_actool_and_the_car` | ``scripts/build-mac.sh has no line `if [ "${ACTOOL_MAJOR}" -lt 26 ]; then` `` |
| The bundle check skips the dep-info after the byte comparison and accepts anything | `the_bundle_check_refuses_a_stale_or_foreign_binary` | `src/lib.rs is newer than ferrispass:` followed by the check's own success line |
| The bundle check no longer tests that the named files exist | `the_bundle_check_refuses_a_stale_or_foreign_binary` | `src/app/removed.rs was deleted after ferrispass was built:` followed by the check's own success line |

## What was checked on this Mac

macOS 27 with Xcode 27 (`actool` 27.0, macOS SDK 27.0), `scripts/build-mac.sh --skip-notarize` with its own `CARGO_TARGET_DIR`, signed with the local Developer ID, not notarized. The app was not started.

- The build ran through all steps. `actool` wrote `Assets.car` (2,016,232 bytes), `AppIcon.icns` (66,070 bytes) and a partial `Info.plist` with `CFBundleIconFile` and `CFBundleIconName`, both `AppIcon`. With a deployment target of 26.0 it still writes the `.icns`.
- `assetutil --info` lists `AppIcon` as an `IconImageStack` in three appearances: `NSAppearanceNameAqua`, `NSAppearanceNameDarkAqua` and `ISAppearanceTintable`. Clear and tinted are drawn from the tintable stack. The report names the catalog's platform version as 26.0 and its storage version as Xcode 27.0.
- `codesign --verify --deep --strict` passes, and the seal lists `Resources/Assets.car` and `Resources/AppIcon.icns`. `plutil` shows both icon keys and `LSMinimumSystemVersion` 26.0 in the bundle's `Info.plist`.
- The bundled `ferrispass` contains "Search saved", a text only recent sources have. With the signatures removed, the bundled `ferrispass-cli` is byte for byte the one in the release folder, and `ferrispass` differs only in two bytes of the `__LINKEDIT` size that its signature leaves behind.
- With the version guard fed `ACTOOL_MAJOR=25` by hand, the build stops in step 2 with "actool 27.0 cannot compile .icon files (needs full Xcode 26 or newer)" and exit 1. The version in the message is the real one; only the major version the guard compares was replaced.
- That second build found nothing to rebuild: cargo left both binaries untouched, older than the build, and the bundle check passed them.
- With the bundle check reading the dep-info, the build ran again from an empty target folder and passed. Cargo wrote `ferrispass.d` and `ferrispass-cli.d` into `target/aarch64-apple-darwin/release`, next to the binaries, which is where the check reads them. Each is one line naming 258 files, all in this checkout: the Rust sources, the locales, the assets and `bundle/minisign-pub.txt`, no test module, and neither `Cargo.toml` nor `Cargo.lock`.
- After touching only `src/app/kinds/tests.rs`, cargo rebuilt nothing and the check passed. After touching `src/app/mod.rs`, cargo rebuilt both binaries and the check passed.
- A deleted source file, on a small crate with the same two binaries and a target folder of its own: with the module removed from the code and its file deleted, cargo rebuilt, the new dep-info no longer named the file, and the check passed. With the file of `ferrispass-cli` deleted outright, cargo stopped with an error, the old binary stayed in the release folder, and the check stopped with the sentence for a file that no longer exists.
- `ictool` 2.0 renders of the six appearances with `--design-generation 26` and `27` side by side: generation 27 looks like the renders from October. Generation 26 draws more light on the edges of the glassy body, a rounder outline, and in Tinted Dark a darker, bluer background. Clear Light is faint in both, a light figure on light glass. At 16 px both keep the silhouette with the claws, the two eyes and the key as a green spot.

## What waits

- The look in the Dock, checked by hand on the built bundle: light, dark, tinted and clear, the Finder list at 16 px, the app switcher, Spotlight, and no grey squircle around the icon. Clear Light needs the closest look.
- A notarized build from the release workflow, on macOS 26 with Xcode 26.6, and the same look on macOS 26. That build is the first time `actool` and `assetutil` from Xcode 26 run on this document: whether they write the same three appearances, under the same names, and an `.icns`. The release workflow only runs on a tag, so this waits for the first test build.

## Limits

- The dep-info names files, not settings. A change to `Cargo.toml` or `Cargo.lock` alone (a version or a dependency), to `.cargo/config.toml` or to `RUSTFLAGS` makes cargo rebuild, but when the release folder holds binaries from an earlier build, the check does not see it.
- It compares times. A file that changes but keeps an older time is not noticed, by cargo nor by the check.
- With a target folder shared by several checkouts, the check cannot tell whose build a binary is. When another checkout's build is newer than every file here, cargo leaves that binary in the release folder and rewrites the dep-info to name this checkout's files, and the check passes it. A release build uses a target folder of its own: CI does, and so does the bundle build as `docs/release-process.md` describes it.
- It needs the dep-info to name files by their full path, as cargo does by default. With `build.dep-info-basedir` set to a folder above the sources, cargo writes their paths relative to it, and the check stops every build.
- `cmp` runs on the copies before signing. The binaries in the signed bundle differ from the release folder by their signature.
- The `assetutil` check relies on the field names that Xcode 27's `assetutil` prints (`Name`, `Appearance`). If Xcode 26's report names them differently, the release build stops at that check; it does not let an icon with one appearance through.
- `ci.yml` still says that the release build runs on the same image as CI. CI stays on `macos-latest`, which is the macOS 26 image today.
- The tests read the build script and the workflow as text. They hold the lines listed under "What the tests show" and their order, not what the shell or the runner does with them. They do not hold the lines that feed the version guard and the appearance check: the tests stay green with the major version set to a constant, without the `assetutil --info` run, and with a filter that counts every entry in the catalog rather than the appearances of `AppIcon`. Only a build runs those lines, as the one on this Mac did.

The agents spent about 3 hours on this, including review and fixes.
