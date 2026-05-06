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

`release.yml` runs `scripts/build-mac.sh` on a `macos-latest` runner. Stages, with rough timing:

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
| 9 | Generate `.app.tar.gz` + minisign signature + `update.json` | <10 s | minisign key missing or malformed |
| End | `softprops/action-gh-release@v2` uploads to a GitHub Release | <30 s | `permissions: contents: write` not granted |

Total wall-clock: usually 8-12 minutes.

## Common failure modes

### "base64 conversion failed - was an actual secret key given?"

Step 9. The `MINISIGN_PRIVATE_KEY` GitHub Secret is malformed. Most often: the user pasted only the second line of `~/.ferrispass/minisign.key` (the base64 blob) without the `untrusted comment:` header line.

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

If you have minisign installed locally and the keypair at `~/.ferrispass/minisign.key`, the `--skip-notarize` build also produces the `.app.tar.gz` + `.minisig` + `update.json` artefacts. Otherwise it skips Stage 9 with a warning; the DMG alone is still produced.

## See also

- [`SECURITY.md`](../SECURITY.md) — threat model, vulnerability reporting
- [`docs/architecture.md`](./architecture.md) — module layout, data flow, state pattern
- [`scripts/build-mac.sh`](../scripts/build-mac.sh) — the actual pipeline implementation
- [`.github/workflows/release.yml`](../.github/workflows/release.yml) — CI orchestration
