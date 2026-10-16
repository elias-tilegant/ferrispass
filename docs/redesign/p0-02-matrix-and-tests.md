# P0.02: Capability matrix and test groundwork

Second step of the redesign, on the same branch as [P0.01](p0-01-gpui-kit.md), on top of `07ac8e9`, finished 2026-10-15. It maps what the redesign needs from GPUI and adds two kinds of tests that later steps rely on. Two more changes came along because 0.9.3 can't ship without them: a security update and an SDK check in CI.

| Commit | What |
|---|---|
| `8368310` | rustls 0.23.46, quinn-proto 0.11.18 |
| `e5add59` | CI checks the macOS SDK against the minimum version 26 |
| `381c46e` | Architecture test, `tests/architecture.rs` |
| `439bbe4` | proptest and two property tests |
| `0cf0bad` | `docs/gpui-matrix.md` |
| `48961e8` | Fixes from the review of the architecture test |

## Checks

| Check | Result |
|---|---|
| `cargo fmt --all --check` | pass |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | pass (only the known future-incompat note for `block` 0.1.6) |
| `cargo test --locked` | pass, no new `#[ignore]` |
| `cargo build --locked --bin ferrispass-cli` | pass |
| `cargo audit` | pass, 7 vulnerabilities before (see below) |
| Em and en dash check over tracked files and the commit messages in `master..HEAD` | no hits |
| CI on PR #2 (run 38711125091) | pass on image `macos-26-arm64` with SDK 26.5. All tests run there as they do locally, the headless UI tests included. Audit and CodeQL pass too. |

## Security update

`cargo audit` reported 7 vulnerabilities, all of them already present on `master`:
- rustls 0.23.45: RUSTSEC-2026-0346 to 0349, fixed in 0.23.46. rustls handles every HTTPS call the app makes.
- quinn-proto 0.11.17: RUSTSEC-2026-0338, 0340 and 0343, fixed in 0.11.18. It's in `Cargo.lock` through reqwest and gpui-pre-reqwest, but `cargo tree --target all -e all --all-features` shows that no build actually compiles it.

After the update it reports none. Five "unmaintained" warnings remain, and I'm leaving them alone for now: instant 0.1.13 (RUSTSEC-2024-0384), paste 1.0.15 (RUSTSEC-2024-0436), rustls-pemfile 2.2.0 (RUSTSEC-2025-0134), rustybuzz 0.20.1 (RUSTSEC-2026-0206), ttf-parser 0.25.1 (RUSTSEC-2026-0192).

The commit only changes the version and checksum lines of the two packages. gpui-base and gpui-component-macros stay at 0.7.1, which keeps gpui-fast out of `Cargo.lock` (gpui-base 0.7.2 would pull it in). Running `cargo update -p ... --precise` would also have re-resolved seven other requirements to other versions already in `Cargo.lock`: the `windows-sys` requirements of errno, rustix 0.38, rustix 1.1, quinn-udp, winapi-util and tempfile, and tempfile's `getrandom` requirement, which would have dropped from 0.4.3 to 0.3.4. These already flip between versions (on `master`, on P0.01 and after each `cargo update`), and neither bump needs them to move. So I took only the version and checksum from cargo's result, and `cargo metadata --locked` confirms that `Cargo.lock` is consistent.

## SDK check in CI

The first step after checkout in `ci.yml` checks that `xcrun --sdk macosx --show-sdk-version` is at least the minimum macOS version. It reads the minimum from `MIN_MACOS` in `scripts/build-mac.sh`, so the number lives in one place. `release.yml` runs on the same image (`macos-latest`), so every PR shows whether a release could build for macOS 26.

I tested the step locally. SDK 27.0 with a minimum of 26.0 passes. SDK 15.5, an empty version, a failing `xcrun`, a missing `MIN_MACOS`, and SDK 27.0 with a minimum of 28 all fail. An empty value would make `[ -lt ]` exit with 2, and an `if` would read that as a pass, so the step checks that both values are numbers first. On the runner the step passes with SDK 26.5.

## Capability matrix

[`docs/gpui-matrix.md`](../gpui-matrix.md) lists the GPUI capabilities the redesign relies on, plus every code location the spikes rely on, with file and line in Zed 1870e26 and in gpui-pre 0.3.8. A script read every cited line back from the sources. The negative entries (no `sharingType`, no custom `PromptBuilder` in GPUI Kit, no reduce-motion lookup in gpui-pre-macos) were checked with grep.

Nothing that matters to FerrisPass changed between the two versions. What it means for the spikes:
- `canBecomeKeyWindow` is still hard-coded to `YES`, and `activate` makes every window key. SP03 is still needed.
- `application:openURLs:` drops URLs while no callback is set. SP06 has to test a cold start, and `on_open_urls` belongs on the `Application` builder before `run`.
- GPUI has no `sharingType`, so SP12 goes through `HasWindowHandle`.
- GPUI Kit reads "Reduce motion" once in `init` and doesn't notice later changes on macOS. So P0.07 can't count on GPUI Kit to pick up a change while the app runs. `apply_system_reduce_motion` in gpui-base re-reads it.
- gpui-pre's own minimum is macOS 10.15.7 (shaders), far below 26.

In this step, everything in the matrix came from reading source. None of it was run, because every row would need the real AppKit platform or a window on a real Mac, so all rows started out as "pending spike". The spikes try them and fill in their results.

### Headless UI tests in CI

Going by the GPUI source, FerrisPass's tests need neither Metal nor a window server. `#[gpui::test]` runs on `TestPlatform` with `NoopTextSystem` and no renderer. Metal only comes in with `HeadlessAppContext` and a real renderer. The first CI run bears that out.

## Architecture test

`tests/architecture.rs` lists the places that already use `unsafe` in `LEGACY_EXCEPTIONS`, by file and item rather than by line:

| File | Item | Line today |
|---|---|---|
| `src/session_lock.rs` | whole file | 8 |
| `src/update/macos_installer.rs` | whole file | 7 |
| `src/biometric/macos.rs` | whole file | 19 |
| `src/launch/tempfile.rs` | `fn process_uid` | 182 |
| `src/keepass/document.rs` | `fn publish_new_no_clobber` | 1695 |
| `src/keepass/document.rs` | `mod macos_full_sync` | 1740 |
| `src/sync/icloud.rs` | `mod platform` | 268 |

New modules that need `unsafe` go under `src/platform/`, get a `//!` header saying why they need it, and are added to `PLATFORM_MODULES`. The test reads every `.rs` file in the repo except `target/` and `.git/`, and fails on:
- any attribute that relaxes the `unsafe_code` lint and isn't on the list (`allow`, `expect`, `warn`, also inside `cfg_attr` or as a raw identifier), and any list entry whose attribute is gone
- any `unsafe` outside the listed items: blocks, functions, `unsafe impl`, `unsafe extern`, `no_mangle`, `export_name`, `link_section`, `allow_internal_unsafe` and `global_asm` in any spelling, also inside macro bodies. This keeps the rule in place even if someone lowers the lint.
- `unsafe_code` outside an attribute, for example as a macro argument that a macro turns into an `allow`
- a lint level below `deny` in `Cargo.toml`. The entry is parsed as TOML, so comments, quoting and the `unsafe-code` spelling are handled, and there must be exactly one entry.
- lint flags outside `Cargo.toml`: `unsafe_code`, `cap-lints`, `--config` or `include` in `.cargo/config.toml`, and `rustflags`, `cap-lints` or `unsafe_code` in the CI workflows. Escape sequences are decoded before matching.
- code the scan doesn't read: `include!` in any spelling, or a `#[path]` or build target in `Cargo.toml` that doesn't point to a `.rs` file in the repo

The scan works on tokens, so comments, doc comments, strings, raw strings and character literals never count. Source the lexer can't close cleanly (an unterminated comment or string, an unbalanced delimiter) fails the test instead of slipping through. The scan has its own unit tests for these cases.

To make sure it really catches things, I broke the rule on purpose three times and reverted each change:
1. `#[allow(unsafe_code)]` with an `unsafe` block in `src/app/recents.rs`, a file that isn't on the list. The test fails and names the file, the line and `fn architecture_probe`.
2. A new opt-in on a different item in `src/keepass/document.rs`, a file that is on the list. Fails.
3. The lint in `Cargo.toml` set to `allow`, plus an `unsafe` block without an attribute in `src/ui/widgets/secret_input.rs`. Both `nothing_lowers_the_unsafe_code_lint` and `unsafe_code_stays_on_the_exception_list` fail, naming the lint level and the location.

The test can't see unsafe code that a macro from another crate generates without the word `unsafe` appearing in FerrisPass's own source. rustc doesn't report `unsafe_code` for those expansions either, and those crates are covered by the [supply-chain check](../release-process.md#gpui-supply-chain-on-every-gpui-bump). `static mut` passes as a declaration, in the scan and in rustc, but every access needs `unsafe`. Flags from the environment or from a Cargo config above the repo are out of reach. CI sets none, and the workflow check makes sure it stays that way.

## Property tests

proptest is now a dev dependency, with two property tests in `src/keepass/password_gen.rs`:
- `generate` over lengths 0 to 200 and all 16 combinations of character classes: the length is clamped to 4 to 128, only characters from active classes appear (lowercase if no class is active), and every active class appears at least once.
- `estimate_bits` and the strength level never go down when the password gets longer.

`Cargo.lock` only gains FerrisPass's own dependency on proptest, because gpui-pre's `test-support` feature already pulls in the same version.

The first version of the property test had a blind spot. As a check, I deleted `required.push(SYMBOLS)`. Only the old example test failed, the property test didn't. Lengths were spread evenly up to 200, so short passwords were rare, and only short ones can miss a class. Now half the cases are at most 8 characters long, and the property test catches the same change and shrinks it to length 4. Real failures end up in `proptest-regressions/` and belong in the repo.

## Review

I had Codex (`gpt-5.6-sol`, reasoning effort `xhigh`) do one read-only review with two questions: can `tests/architecture.rs` miss new `unsafe` outside the list, and do five randomly picked rows of the matrix hold up?

On the matrix, all five rows (PopUp Spaces, `on_open_urls`, `observe_window_activation`, `cx.hide`, `reduce_motion`) have correct line numbers, and the code supports what they say, including the negative entry for gpui-pre-macos.

On the architecture test, Codex found four real gaps, all fixed in `48961e8`. I reproduced each one against the old test first, which passed every time, and the new test fails on all four:
1. Source outside the scanned folders, through `#[path = "x.inc"]` or a target like `tools/helper.rs`. The whole repo is read now, and `#[path]` and targets have to be `.rs` files in the repo. Checked with a `tools/helper.rs` containing `allow` and `unsafe`.
2. Other ways to spell a macro: `r#global_asm!`, `use core::arch::global_asm as ga`, `r#include!`. Macros are now matched by name in any spelling, and `include` also as a path, a rename or a macro argument. These are covered by unit tests rather than a probe in the source tree, because rustc would reject a `global_asm` there before the test even ran.
3. The lint level was found by substring, so `unsafe_code = "allow" # was "deny"` passed. It's parsed as TOML now. Checked with exactly that line in `Cargo.toml`.
4. `.cargo/config.toml` was checked as raw text, so `"--cap\u002dlints=allow"` slipped through. Escapes are decoded now, and the workflows are checked too. Checked with `RUSTFLAGS: "--cap\u002dlints=allow"` in `ci.yml`.

Two smaller fixes: `#[allow_internal_unsafe]` now counts as unsafe, and a shebang line isn't treated as code. I left three things alone on purpose: `static mut` (see above), macros from other crates (covered by the supply-chain check), and the false alarm on `#[cfg(no_mangle)]`, which errs on the safe side.

## Still open

- Spike runs for the matrix rows. Results go into `docs/gpui-matrix.md`.
- The five "unmaintained" warnings from `cargo audit`. Nothing to do about them yet.
- Carried over from P0.01: the open items of the [manual check](p0-01-gpui-kit.md#manual-check-before-release).
