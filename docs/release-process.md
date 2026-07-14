# Release Process

End-to-end checklist for cutting a `v0.x.y` release. Read once before your first release; use as a runbook for subsequent ones.

## Pre-release checklist

Run before bumping the version:

```sh
cargo check                                      # warnings ok, errors not
cargo test                                       # all 82+ green
cargo clippy --all-targets                       # informational
git status                                       # working tree clean
git pull --rebase origin master                  # in sync with remote
```

If any of these fail, fix before tagging — the CI will refuse the release otherwise.

## Commit message conventions

The Release-page body is auto-generated from commit messages by [git-cliff](https://git-cliff.org), driven by `cliff.toml` at the repo root. Prefix each commit with a Conventional-Commits-style tag so it lands in the right section.

| Prefix | Section | Example |
|---|---|---|
| `feat:` | Features | `feat(sync): per-vault sync intervals` |
| `fix:` | Bug Fixes | `fix(merge): preserve UUIDs` |
| `perf:` | Performance | `perf(ui): debounce search input` |
| `docs:` | Documentation | `docs: add architecture diagram` |
| `refactor:` | Refactoring | `refactor: extract sync helper` |
| `build:` | Build & Tooling | `build: bump cargo-packager-updater to 0.3` |
| (no prefix or unknown) | Other Changes | `add Author section to README` |
| `chore:` / `ci:` / `test:` / `style:` / `release:` | **skipped** | not surfaced in release notes |

Optional scope in parentheses (`fix(merge):`, `feat(ui):`) — purely documentary; doesn't affect rendering, but useful for `git log --grep`.

**Preview before tagging:**
```sh
brew install git-cliff      # one-time
git-cliff --unreleased --tag vX.Y.Z
```
Prints the exact markdown that will appear on the Release page. If the output looks empty or stuffs everything into "Other Changes", check that recent commits use the prefixes above.

## Bumping the version

Single source of truth is `Cargo.toml`'s `[package].version`. Everything else (Welcome footer, About box, .app Info.plist, update manifest) reads it via `env!("CARGO_PKG_VERSION")` or `sed`-substitution at build time.

```sh
# Patch (bug fixes only):     0.2.0 → 0.2.1
# Minor (new features):       0.2.x → 0.3.0
# Major (API breaks):         0.x.y → 1.0.0     (don't until you mean it)

# Edit Cargo.toml manually, then:
cargo check                                      # regenerate Cargo.lock with new version
git add Cargo.toml Cargo.lock
git commit -m "release: v0.x.y"
```

## Tagging + pushing

```sh
git tag -a v0.x.y -m "FerrisPass 0.x.y"
git push origin master
git push origin v0.x.y
```

The `v` prefix matters — `release.yml` matches `tags: ['v*']`. Tagging without push to master means CI runs against a commit nobody else can see; not catastrophic but messy.

## What the pipeline does

`release.yml` uses separate build, notes, signing, and publishing jobs. The
build and notes jobs never receive the updater private key. The isolated
signing job treats the generated notes as bounded data, verifies the payload
size, and only then signs the payload and complete manifest.

Stages, with rough timing:

| # | Stage | Time | Failure modes |
|---|---|---|---|
| 1 | `cargo build --release --target aarch64-apple-darwin` | 3-7 min | code errors (caught by CI before tagging if you ran `cargo test` first) |
| 2 | Generate `.icns` from `bundle/icon.png` | <5 s | icon missing → script exits early with clear error |
| 3 | Assemble `.app` bundle, render `Info.plist` | <1 s | — |
| 4 | Codesign with Developer ID + Hardened Runtime | <5 s | cert not in Keychain (CI: `APPLE_CERT_BASE64` invalid) |
| 5 | Build DMG via `create-dmg` | 10-30 s | — |
| 6 | Codesign the DMG | <5 s | same as step 4 |
| 7 | Notarize via `xcrun notarytool submit --wait` | 1-5 min | Apple's queue. Rejection = read the notarization log carefully |
| 8 | Staple notarization ticket | <5 s | — |
| 9 | Generate `.app.tar.gz` + unsigned `update.json` with exact payload size | <10 s | `jq` missing or archive creation fails |
| Sign | Inject notes, sign payload, then sign the complete manifest | <10 s | minisign key missing, malformed, or payload size mismatch |
| End | `softprops/action-gh-release@v2` uploads to a GitHub Release | <30 s | `permissions: contents: write` not granted |

Total wall-clock: usually 8-12 minutes.

## Common failure modes

### "base64 conversion failed - was an actual secret key given?"

The signing job's `MINISIGN_PRIVATE_KEY` GitHub Secret is malformed. Most often: the user pasted only the second line of `~/.ferrispass/minisign.key` (the base64 blob) without the `untrusted comment:` header line.

**Fix:**
```sh
cat ~/.ferrispass/minisign.key | pbcopy
```
Then update the GitHub Secret with the full clipboard content. Re-run the workflow via Actions → failed run → "Re-run failed jobs."

The pre-flight check in `release.yml`'s "Stage minisign private key" step now catches this case before the build runs and prints a clear remediation message.

### "errSecInternalComponent" during codesign

Step 4 or 6. The Developer ID cert was imported but the keychain password isn't unlocked, or the cert lacks the matching private key.

**Fix:** verify locally that `security find-identity -v -p codesigning | grep "Developer ID Application"` returns the cert. If empty, re-export the .p12 from your Mac's Keychain Access **with the private key included** (cmd-click both items before exporting).

### Notarization rejected

Step 7. `xcrun notarytool` returns status `Invalid`. Read the log:

```sh
xcrun notarytool log <submission-id> --keychain-profile ferrispass-notarize
```

Common causes:
- Hardened Runtime not enabled (we always pass `--options runtime`, so unlikely)
- Binary not signed with the right cert (check Step 4)
- Embedded helper binary unsigned (we have none, but watch for this if you add Sparkle/etc later)

### "permission denied" creating GitHub Release

Step End. The workflow lacks `contents: write` permission. Already set in `release.yml`'s `permissions:` block; only an issue if someone removed it.

### Users report duplicate entries after sync

Symptom: a single entry created in KeePass2 (or another KeePass client) shows up two, three, or more times in FerrisPass after a sync round-trip — and the counts grow with each cycle. The duplication is real (in the .kdbx file), not a UI artefact.

Root cause: pre-v0.2.1 builds had a bug in `src/keepass/merge.rs::add_entry_under` that re-randomised UUIDs on remote-only entry imports. Other clients then saw the entry as "new on the cloud" on every cycle and kept their own original copy alongside, producing exponential duplication. Fixed in commit XXX (visibility flips in the keepass-rs fork plus deep-replace logic in merge.rs).

Recovery for users on a corrupted vault is documented in §"v0.2.1 release: cleanup recipe for affected users" below — they need a one-time manual deduplication; the code fix only stops the bleeding.

## Recovery: re-tagging a botched release

If the pipeline failed before creating the GitHub Release, no cleanup needed — just fix the issue, re-run the failed job. The tag stays put.

If the pipeline succeeded but the build was bad (e.g., regression slipped through), don't move the tag in place. Push a patch:

```sh
# Edit Cargo.toml: 0.x.y → 0.x.(y+1)
git commit -am "release: v0.x.(y+1) — fix <thing>"
git tag -a v0.x.(y+1) -m "FerrisPass 0.x.(y+1)"
git push origin master --tags
```

Existing v0.x.y installs will see the new release on their next auto-update check.

If the tag was pushed but no Release was ever created (workflow failed early), it's safe to delete + re-push:

```sh
git tag -d v0.x.y
git push --delete origin v0.x.y
git tag -a v0.x.y -m "FerrisPass 0.x.y"
git push origin v0.x.y
```

Don't do this once any user has installed `v0.x.y` — re-tagging the same name on a different commit makes future bisects miserable.

## Minisign-key backup strategy

The minisign private key at `~/.ferrispass/minisign.key` is **irreplaceable**. If you lose it:

- Every installed FerrisPass refuses to apply updates from a new key (the public key is embedded at compile time; old installs only trust the old key)
- The fix is "reinstall from scratch" for every user — which, for an auto-updating app, defeats the purpose

Store at least two copies, in different physical locations, encrypted at rest:

| Location | Form | Why |
|---|---|---|
| Password manager (1Password / Bitwarden / KeePass itself) | `~/.ferrispass/minisign.key` content + passphrase as a "secure note" | Always with you, encrypted by your master password |
| Encrypted external drive | `minisign.key` file + passphrase in a separate file | Survives laptop loss / drive failure / OS reinstall |

Rotate **only** when there's evidence of compromise. Rotation is a release-blocker for old installs, so it's an emergency-only operation.

## Manual local build (skip-notarize)

For testing changes without burning ~5 minutes on Apple's notarization queue:

```sh
scripts/build-mac.sh --skip-notarize
```

Outputs `dist/FerrisPass-X.Y.Z-arm64.dmg` that opens with a Gatekeeper warning (signed but not notarized). Useful for verifying the icon, app start-up flow, etc., before committing to a real release.

`--skip-notarize` stops after the DMG and deliberately does not create updater
artefacts. A normal notarized build produces `.app.tar.gz` and an unsigned
`update.json`; `scripts/build-mac.sh --sign-update` then signs those existing
artefacts. Unsigned manifests are rejected by the application.

## v0.2.1 release: cleanup recipe for affected users

The duplicate-entries sync bug (see §Common failure modes) was fixed in v0.2.1, but already-corrupted vaults won't auto-heal — the duplicates are real bytes in the .kdbx file, the fix only stops new ones from appearing. Paste the following into the GitHub Release body for v0.2.1 so users running the affected v0.2.0 know what to do:

> **⚠️ One-time cleanup needed if you hit duplicate entries**
>
> Earlier FerrisPass builds (v0.2.0 and before) had a sync bug that accumulated duplicate entries on cross-client merges with KeePass2 / KeePassXC. The bug is fixed in this release, but existing vault files may already contain duplicates that won't disappear on their own. To clean up:
>
> 1. In KeePass2 or KeePassXC, open your `.kdbx` file
> 2. Sort entries by Title and identify duplicates (typically same title, possibly identical fields)
> 3. Delete the older copies (keep the most recent per logical entry; "Last Modified" column helps)
> 4. Save and let the change propagate to the cloud
> 5. In FerrisPass: Settings → Sync → Disconnect, then reconnect from scratch (Welcome screen → Connect OneDrive → pick the cleaned file)
> 6. Subsequent syncs will not produce new duplicates
>
> Vaults that were never synced cross-client are unaffected.

## See also

- [`SECURITY.md`](../SECURITY.md) — threat model, vulnerability reporting
- [`docs/architecture.md`](./architecture.md) — module layout, data flow, state pattern
- [`scripts/build-mac.sh`](../scripts/build-mac.sh) — the actual pipeline implementation
- [`.github/workflows/release.yml`](../.github/workflows/release.yml) — CI orchestration
