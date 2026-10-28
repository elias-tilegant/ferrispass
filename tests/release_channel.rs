//! The pre-release channel: `scripts/update-channel.sh`, run as the release
//! workflow runs it, against a stand-in for `gh` that serves releases from a
//! temporary folder and records every upload.
//!
//! The workflow test in `tests/architecture.rs` checks that the script runs
//! after every release; this file checks what it decides. The channel
//! `kanal-vorab` must end up holding exactly the manifest pair of the highest
//! version among the published `v*` releases that carry one, whatever an
//! earlier run left behind, must never move back, and must be left alone,
//! with the run failing, whenever its state or a version cannot be trusted.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const CHANNEL: &str = "kanal-vorab";

/// A stand-in for the `gh` commands the script uses. Each release is a
/// folder `$GH_STUB/<tag>` with its files under `assets/` and an optional
/// `meta.json` (`{"prerelease": false}`, `{"draft": true}`). An upload
/// without `--clobber` over an existing file fails, as it does on GitHub.
const STUB_GH: &str = r#"#!/bin/bash
set -eo pipefail
root="$GH_STUB"
release_json() {
    local dir="$root/$1" meta='{}' names='[]'
    [ -f "$dir/meta.json" ] && meta="$(cat "$dir/meta.json")"
    [ -d "$dir/assets" ] && names="$(ls -1 "$dir/assets" | jq -R . | jq -s .)"
    jq -n --arg tag "$1" --argjson meta "$meta" --argjson names "$names" '{
        tag_name: $tag, draft: ($meta.draft // false),
        isPrerelease: (if $meta | has("prerelease") then $meta.prerelease else true end),
        assets: [$names[] | {name: .}]}'
}
group="$1"; shift
action=""
if [ "$group" = release ]; then action="$1"; shift; fi
filter=""; dir="."; clobber=""; patterns=""; args=""
while [ $# -gt 0 ]; do
    case "$1" in
        --jq) filter="$2"; shift 2 ;;
        --json|--repo) shift 2 ;;
        --dir) dir="$2"; shift 2 ;;
        --pattern) patterns="$patterns $2"; shift 2 ;;
        --clobber) clobber=yes; shift ;;
        --paginate) shift ;;
        *) args="$args $1"; shift ;;
    esac
done
set -- $args
case "$group $action" in
    "api ")
        for release in "$root"/*/; do
            release_json "$(basename "$release")"
        done | jq -s . | jq -r "$filter"
        ;;
    "release view")
        [ -d "$root/$1" ] || { echo "release not found" >&2; exit 1; }
        release_json "$1" | jq -r "$filter"
        ;;
    "release download")
        for pattern in $patterns; do
            [ -f "$root/$1/assets/$pattern" ] || { echo "no asset $pattern in $1" >&2; exit 1; }
            cp "$root/$1/assets/$pattern" "$dir/"
        done
        ;;
    "release upload")
        tag="$1"; shift
        for file in "$@"; do
            target="$root/$tag/assets/$(basename "$file")"
            if [ -e "$target" ] && [ -z "$clobber" ]; then
                echo "asset $(basename "$file") already exists" >&2; exit 1
            fi
        done
        for file in "$@"; do
            cp "$file" "$root/$tag/assets/"
            echo "$tag $(basename "$file")" >> "$root/../uploads.log"
        done
        ;;
    *) echo "stub gh: unexpected $group $action" >&2; exit 64 ;;
esac
"#;

/// A set of releases and a run of the channel script against them.
struct Releases {
    dir: tempfile::TempDir,
}

/// What a run did: its exit code, its output, and the uploads it made.
struct Run {
    code: Option<i32>,
    output: String,
    uploads: Vec<String>,
}

impl Releases {
    fn new() -> Self {
        let dir = tempfile::tempdir().expect("temporary folder");
        fs::create_dir_all(dir.path().join("bin")).unwrap();
        fs::create_dir_all(dir.path().join("releases")).unwrap();
        let gh = dir.path().join("bin/gh");
        fs::write(&gh, STUB_GH).unwrap();
        fs::set_permissions(&gh, fs::Permissions::from_mode(0o755)).unwrap();
        Releases { dir }
    }

    fn assets(&self, tag: &str) -> PathBuf {
        self.dir.path().join("releases").join(tag).join("assets")
    }

    /// A release with exactly these files.
    fn release(&self, tag: &str, files: &[(&str, &str)]) -> &Self {
        let assets = self.assets(tag);
        fs::create_dir_all(&assets).unwrap();
        for (name, content) in files {
            fs::write(assets.join(name), content).unwrap();
        }
        self
    }

    /// A release as the workflow publishes it: a disk image and a manifest
    /// pair for `version`, signed for `tag`.
    fn published(&self, tag: &str, version: &str) -> &Self {
        self.release(
            tag,
            &[
                ("FerrisPass.dmg", "image"),
                ("update.json", &manifest(version)),
                ("update.json.minisig", &format!("signature from {tag}")),
            ],
        )
    }

    fn meta(&self, tag: &str, json: &str) -> &Self {
        fs::write(
            self.dir.path().join("releases").join(tag).join("meta.json"),
            json,
        )
        .unwrap();
        self
    }

    /// The channel holding the pair that `tag` was published with.
    fn channel_from(&self, tag: &str) -> &Self {
        let pair = [
            ("update.json", self.read(tag, "update.json")),
            ("update.json.minisig", self.read(tag, "update.json.minisig")),
        ];
        let pair: Vec<(&str, &str)> = pair.iter().map(|(n, c)| (*n, c.as_str())).collect();
        self.release(CHANNEL, &pair)
    }

    fn read(&self, tag: &str, name: &str) -> String {
        fs::read_to_string(self.assets(tag).join(name))
            .unwrap_or_else(|err| panic!("{tag}/{name}: {err}"))
    }

    /// Which release the channel's pair came from, by its signature.
    fn channel_holds(&self) -> Option<String> {
        let signature =
            fs::read_to_string(self.assets(CHANNEL).join("update.json.minisig")).ok()?;
        let tag = signature.strip_prefix("signature from ")?.to_string();
        let manifest = self.read(CHANNEL, "update.json");
        assert_eq!(
            manifest,
            self.read(&tag, "update.json"),
            "the channel's manifest and signature come from different releases"
        );
        Some(tag)
    }

    fn run(&self) -> Run {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let path = format!(
            "{}:{}",
            self.dir.path().join("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let out = Command::new("bash")
            .arg(root.join("scripts/update-channel.sh"))
            .arg("elias-tilegant/ferrispass")
            .env("PATH", path)
            .env("GH_STUB", self.dir.path().join("releases"))
            .output()
            .expect("run scripts/update-channel.sh");
        let uploads = fs::read_to_string(self.dir.path().join("uploads.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect();
        Run {
            code: out.status.code(),
            output: format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
            uploads,
        }
    }
}

fn manifest(version: &str) -> String {
    format!("{{\"version\": \"{version}\", \"platforms\": {{}}}}\n")
}

/// Runs the script and requires that it succeeded and left the channel
/// holding `tag`'s pair, uploading only if `moved`.
fn converges(releases: &Releases, tag: &str, moved: bool) {
    let run = releases.run();
    assert_eq!(run.code, Some(0), "{}", run.output);
    assert_eq!(
        releases.channel_holds().as_deref(),
        Some(tag),
        "{}",
        run.output
    );
    assert_eq!(
        !run.uploads.is_empty(),
        moved,
        "{:?}\n{}",
        run.uploads,
        run.output
    );
}

/// Runs the script and requires that it failed and changed nothing.
fn refuses(releases: &Releases, why: &str) {
    let before = releases.channel_holds_files();
    let run = releases.run();
    assert_ne!(run.code, Some(0), "{why}: the run passed\n{}", run.output);
    assert!(run.uploads.is_empty(), "{why}: {:?}", run.uploads);
    assert_eq!(releases.channel_holds_files(), before, "{why}");
}

impl Releases {
    fn channel_holds_files(&self) -> Vec<(String, String)> {
        let Ok(entries) = fs::read_dir(self.assets(CHANNEL)) else {
            return Vec::new();
        };
        let mut files: Vec<(String, String)> = entries
            .map(|entry| {
                let entry = entry.unwrap();
                let name = entry.file_name().to_string_lossy().into_owned();
                (name, fs::read_to_string(entry.path()).unwrap_or_default())
            })
            .collect();
        files.sort();
        files
    }
}

#[test]
fn an_empty_channel_takes_the_highest_release() {
    let releases = Releases::new();
    releases
        .published("v0.9.3", "0.9.3")
        .published("v0.9.4", "0.9.4")
        .release(CHANNEL, &[]);
    converges(&releases, "v0.9.4", true);
}

#[test]
fn a_higher_release_moves_the_channel() {
    let releases = Releases::new();
    releases
        .published("v0.9.4", "0.9.4")
        .published("v0.10.0-pre.1", "0.10.0-pre.1")
        .channel_from("v0.9.4");
    converges(&releases, "v0.10.0-pre.1", true);
}

#[test]
fn the_channel_at_the_highest_release_stays() {
    let releases = Releases::new();
    releases
        .published("v0.9.4", "0.9.4")
        .published("v0.10.0-pre.1", "0.10.0-pre.1")
        .channel_from("v0.10.0-pre.1");
    converges(&releases, "v0.10.0-pre.1", false);
}

#[test]
fn a_lower_tag_leaves_the_channel_where_it_is() {
    // A 0.9.x fix tagged after a test build.
    let releases = Releases::new();
    releases
        .published("v0.9.4", "0.9.4")
        .published("v0.10.0-pre.1", "0.10.0-pre.1")
        .published("v0.9.5", "0.9.5")
        .channel_from("v0.10.0-pre.1");
    converges(&releases, "v0.10.0-pre.1", false);
}

#[test]
fn the_channel_never_moves_back() {
    // The channel's release is gone; the highest one left is lower.
    let releases = Releases::new();
    releases
        .published("v0.10.0-pre.2", "0.10.0-pre.2")
        .published("v0.10.0-pre.3", "0.10.0-pre.3")
        .channel_from("v0.10.0-pre.3");
    fs::remove_dir_all(releases.dir.path().join("releases/v0.10.0-pre.3")).unwrap();
    let run = releases.run();
    assert_eq!(run.code, Some(0), "{}", run.output);
    assert!(run.uploads.is_empty(), "{:?}", run.uploads);
    assert!(
        releases
            .read(CHANNEL, "update.json")
            .contains("0.10.0-pre.3"),
        "{}",
        run.output
    );
}

#[test]
fn a_later_run_catches_up_on_one_that_was_cancelled() {
    // pre.2's channel job was cancelled while it waited; the run for the
    // next tag, a lower 0.9.x fix, moves the channel to pre.2 all the same.
    let releases = Releases::new();
    releases
        .published("v0.9.4", "0.9.4")
        .published("v0.10.0-pre.1", "0.10.0-pre.1")
        .published("v0.10.0-pre.2", "0.10.0-pre.2")
        .published("v0.9.5", "0.9.5")
        .channel_from("v0.10.0-pre.1");
    converges(&releases, "v0.10.0-pre.2", true);
}

#[test]
fn a_pair_that_does_not_match_its_release_is_put_right() {
    // An upload that broke off after the manifest, before the signature.
    let releases = Releases::new();
    releases
        .published("v0.9.4", "0.9.4")
        .published("v0.10.0-pre.1", "0.10.0-pre.1")
        .channel_from("v0.9.4")
        .release(CHANNEL, &[("update.json", &manifest("0.10.0-pre.1"))]);
    converges(&releases, "v0.10.0-pre.1", true);

    // The release was re-run and re-signed after the channel took its pair.
    let releases = Releases::new();
    releases
        .published("v0.10.0-pre.1", "0.10.0-pre.1")
        .channel_from("v0.10.0-pre.1")
        .release(
            "v0.10.0-pre.1",
            &[(
                "update.json",
                "{\"version\": \"0.10.0-pre.1\", \"pub_date\": \"later\"}",
            )],
        );
    converges(&releases, "v0.10.0-pre.1", true);
}

#[test]
fn versions_are_ordered_by_semver_precedence() {
    for (lower, higher) in [
        ("0.10.0-pre.9", "0.10.0"),
        ("0.10.0-pre.9", "0.10.0-pre.10"),
        ("0.9.10", "0.10.0-pre.1"),
        ("0.10.0-pre.2", "0.10.0-rc.1"),
    ] {
        let releases = Releases::new();
        releases
            .published(&format!("v{higher}"), higher)
            .published(&format!("v{lower}"), lower)
            .channel_from(&format!("v{lower}"));
        converges(&releases, &format!("v{higher}"), true);
    }
}

#[test]
fn releases_without_a_published_pair_do_not_count() {
    let releases = Releases::new();
    releases
        .published("v0.10.0-pre.1", "0.10.0-pre.1")
        .published("v0.10.0-pre.5", "0.10.0-pre.5")
        .meta("v0.10.0-pre.5", r#"{"draft": true}"#)
        .release(
            "v0.10.0-pre.4",
            &[("update.json", &manifest("0.10.0-pre.4"))],
        )
        .release(
            "v0.10.0-pre.3",
            &[("update.json.minisig", "signature from v0.10.0-pre.3")],
        )
        .published("nightly", "9.9.9")
        .channel_from("v0.10.0-pre.1");
    converges(&releases, "v0.10.0-pre.1", false);
}

#[test]
fn a_channel_it_cannot_trust_stops_the_run() {
    let base = || {
        let releases = Releases::new();
        releases
            .published("v0.9.4", "0.9.4")
            .published("v0.10.0-pre.1", "0.10.0-pre.1");
        releases
    };

    let releases = base();
    refuses(&releases, "kanal-vorab missing");

    let releases = base();
    releases
        .channel_from("v0.9.4")
        .meta(CHANNEL, r#"{"prerelease": false}"#);
    refuses(&releases, "kanal-vorab not a pre-release");

    let releases = base();
    releases.release(CHANNEL, &[("update.json", &manifest("0.9.4"))]);
    refuses(&releases, "only the manifest");

    let releases = base();
    releases.release(CHANNEL, &[("update.json.minisig", "signature from v0.9.4")]);
    refuses(&releases, "only the signature");

    let releases = base();
    releases
        .channel_from("v0.9.4")
        .release(CHANNEL, &[("notes.txt", "stray")]);
    refuses(&releases, "an extra file");

    for (why, content) in [
        ("broken JSON", "{\"version\": "),
        ("no version", "{\"versio\": \"0.9.4\"}"),
        ("a malformed version", "{\"version\": \"v0.9.4\"}"),
    ] {
        let releases = base();
        releases.release(
            CHANNEL,
            &[
                ("update.json", content),
                ("update.json.minisig", "signature from v0.9.4"),
            ],
        );
        refuses(&releases, why);
    }
}

#[test]
fn a_release_it_cannot_trust_stops_the_run() {
    let releases = Releases::new();
    releases
        .published("v0.10.0-pre.1", "0.10.0-pre.1")
        .published("v0.10.0-pre.2", "0.10.0-pre.1")
        .channel_from("v0.10.0-pre.1");
    refuses(&releases, "a manifest that names another version");

    let releases = Releases::new();
    releases
        .published("v0.10.0-pre.1", "0.10.0-pre.1")
        .published("v0.10", "0.10")
        .channel_from("v0.10.0-pre.1");
    refuses(&releases, "a tag that is not a version");

    let releases = Releases::new();
    releases
        .release("v0.9.4", &[("FerrisPass.dmg", "image")])
        .release(CHANNEL, &[]);
    refuses(&releases, "no release with a manifest");
}
