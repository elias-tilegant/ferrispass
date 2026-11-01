//! The pre-release channel: `scripts/update-channel.sh`, run as the release
//! workflow runs it, against a stand-in for `gh` that serves releases from a
//! temporary folder and records every upload.
//!
//! The workflow test in `tests/architecture.rs` checks that the script runs
//! after every release; this file checks what it decides. The channel
//! `kanal-vorab` must end up holding exactly the manifest pair of the highest
//! version among the published `v*` releases that carry one, whatever an
//! earlier run left behind, must never move back, must receive every upload
//! the script makes, and must be left alone, with the run failing, whenever
//! its state or a version cannot be trusted.

#![cfg(unix)]

use std::cell::Cell;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const CHANNEL: &str = "kanal-vorab";

/// A stand-in for the `gh` commands the script uses, as close to the real
/// ones as the script can tell.
///
/// Each release is a folder `$GH_STUB/<tag>` with its files under `assets/`,
/// an optional `state/<name>` for a file that has not finished uploading
/// (GitHub's `starter`), an optional `meta.json` (`{"prerelease": false}`,
/// `{"draft": true}`), and `created`, a number that orders releases newest
/// first, as GitHub lists them.
///
/// - `gh api repos/.../releases` answers in REST's shape (`tag_name`,
///   `draft`, `prerelease`, `assets[].name` and `.state`), three releases
///   per page, and only the first page without `--paginate`.
/// - `gh release view --json a,b` returns those fields only (`tagName`,
///   `isDraft`, `isPrerelease`, `assets`), and rejects unknown ones.
/// - `gh release upload --clobber` deletes every file it is about to replace
///   before it uploads any, as gh does; `GH_STUB_BREAK_AFTER=n` breaks the
///   upload off after n files. Without `--clobber` an existing file is an
///   error. Every finished upload is logged to `$GH_STUB/../uploads.log`.
const STUB_GH: &str = r#"#!/bin/bash
set -eo pipefail
root="$GH_STUB"
meta() {
    if [ -f "$root/$1/meta.json" ]; then cat "$root/$1/meta.json"; else echo '{}'; fi
}
assets() {
    local dir="$root/$1" file name state
    for file in "$dir"/assets/*; do
        [ -e "$file" ] || continue
        name="$(basename "$file")"
        state=uploaded
        [ -f "$dir/state/$name" ] && state="$(cat "$dir/state/$name")"
        jq -n --arg name "$name" --arg state "$state" '{name: $name, state: $state}'
    done | jq -s .
}
release() {
    jq -n --arg tag "$1" --argjson meta "$(meta "$1")" --argjson assets "$(assets "$1")" '{
        tag: $tag, draft: ($meta.draft // false),
        prerelease: (if $meta | has("prerelease") then $meta.prerelease else true end),
        assets: $assets}'
}
newest_first() {
    local dir
    for dir in "$root"/*/; do
        echo "$(cat "$dir/created" 2>/dev/null || echo 0) $(basename "$dir")"
    done | sort -rn | cut -d' ' -f2
}
group="$1"; shift
action=""
if [ "$group" = release ]; then action="$1"; shift; fi
filter=""; fields=""; dir="."; clobber=""; paginate=""; patterns=""; args=""
while [ $# -gt 0 ]; do
    case "$1" in
        --jq) filter="$2"; shift 2 ;;
        --json) fields="$2"; shift 2 ;;
        --repo) shift 2 ;;
        --dir) dir="$2"; shift 2 ;;
        --pattern) patterns="$patterns $2"; shift 2 ;;
        --clobber) clobber=yes; shift ;;
        --paginate) paginate=yes; shift ;;
        *) args="$args $1"; shift ;;
    esac
done
set -- $args
case "$group $action" in
    "api ")
        case "$1" in repos/*/releases\?*) ;; *) echo "stub gh: unexpected api $1" >&2; exit 64 ;; esac
        newest_first | paste -d' ' - - - | while IFS= read -r page; do
            for tag in $page; do
                release "$tag" | jq '{tag_name: .tag, draft, prerelease,
                    assets: [.assets[] | {name, state}]}'
            done | jq -s . | jq -r "$filter"
            [ -n "$paginate" ] || break
        done
        ;;
    "release view")
        [ -d "$root/$1" ] || { echo "release not found" >&2; exit 1; }
        [ -n "$fields" ] || { echo "stub gh: view without --json" >&2; exit 64; }
        release "$1" | jq --arg fields "$fields" '
            {tagName: .tag, isDraft: .draft, isPrerelease: .prerelease, assets} as $all
            | ($fields | split(",")) as $want
            | if ($want - ($all | keys)) != [] then
                  error("Unknown JSON field: \(($want - ($all | keys))[0])")
              else reduce $want[] as $key ({}; . + {($key): $all[$key]}) end' \
            | jq -r "$filter"
        ;;
    "release download")
        for pattern in $patterns; do
            if [ ! -f "$root/$1/assets/$pattern" ] || [ -f "$root/$1/state/$pattern" ]; then
                echo "no asset $pattern in $1" >&2; exit 1
            fi
            cp "$root/$1/assets/$pattern" "$dir/"
        done
        ;;
    "release upload")
        tag="$1"; shift
        [ -d "$root/$tag" ] || { echo "release not found" >&2; exit 1; }
        for file in "$@"; do
            name="$(basename "$file")"
            if [ -e "$root/$tag/assets/$name" ] && [ -z "$clobber" ]; then
                echo "asset $name already exists" >&2; exit 1
            fi
        done
        for file in "$@"; do
            rm -f "$root/$tag/assets/$(basename "$file")" "$root/$tag/state/$(basename "$file")"
        done
        uploaded=0
        for file in "$@"; do
            if [ -n "${GH_STUB_BREAK_AFTER:-}" ] && [ "$uploaded" -ge "$GH_STUB_BREAK_AFTER" ]; then
                echo "upload broke off" >&2; exit 1
            fi
            mkdir -p "$root/$tag/assets"
            cp "$file" "$root/$tag/assets/"
            echo "$tag $(basename "$file")" >> "$root/../uploads.log"
            uploaded=$((uploaded + 1))
        done
        ;;
    *) echo "stub gh: unexpected $group $action" >&2; exit 64 ;;
esac
"#;

/// A set of releases and a run of the channel script against them.
struct Releases {
    dir: tempfile::TempDir,
    created: Cell<u32>,
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
        Releases {
            dir,
            created: Cell::new(0),
        }
    }

    fn folder(&self, tag: &str) -> PathBuf {
        self.dir.path().join("releases").join(tag)
    }

    fn assets(&self, tag: &str) -> PathBuf {
        self.folder(tag).join("assets")
    }

    /// A release with these files added; a new one is the newest so far.
    fn release(&self, tag: &str, files: &[(&str, &str)]) -> &Self {
        let assets = self.assets(tag);
        if !self.folder(tag).exists() {
            fs::create_dir_all(&assets).unwrap();
            self.created.set(self.created.get() + 1);
            fs::write(
                self.folder(tag).join("created"),
                self.created.get().to_string(),
            )
            .unwrap();
        }
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
        fs::write(self.folder(tag).join("meta.json"), json).unwrap();
        self
    }

    /// Marks a file of `tag` as not finished uploading.
    fn unfinished(&self, tag: &str, name: &str) -> &Self {
        fs::create_dir_all(self.folder(tag).join("state")).unwrap();
        fs::write(self.folder(tag).join("state").join(name), "starter").unwrap();
        self
    }

    fn remove(&self, tag: &str, name: &str) -> &Self {
        fs::remove_file(self.assets(tag).join(name)).unwrap();
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

    /// The channel's files and their contents.
    fn channel_files(&self) -> Vec<(String, String)> {
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

    fn run(&self) -> Run {
        self.run_with(&[])
    }

    /// Runs the script, and requires that every upload it made went to the
    /// channel: it reads the other releases and never writes to them.
    fn run_with(&self, env: &[(&str, &str)]) -> Run {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let path = format!(
            "{}:{}",
            self.dir.path().join("bin").display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let log = self.dir.path().join("uploads.log");
        let _ = fs::remove_file(&log);
        let out = Command::new("bash")
            .arg(root.join("scripts/update-channel.sh"))
            .arg("elias-tilegant/ferrispass")
            .env("PATH", path)
            .env("GH_STUB", self.dir.path().join("releases"))
            .envs(env.iter().copied())
            .output()
            .expect("run scripts/update-channel.sh");
        let uploads: Vec<String> = fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect();
        let output = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        for upload in &uploads {
            assert!(
                upload.starts_with(&format!("{CHANNEL} ")),
                "an upload outside the channel: {upload}\n{output}"
            );
        }
        Run {
            code: out.status.code(),
            output,
            uploads,
        }
    }
}

fn manifest(version: &str) -> String {
    format!("{{\"version\": \"{version}\", \"platforms\": {{}}}}\n")
}

/// Runs the script and requires that it succeeded and left the channel
/// holding `tag`'s pair and nothing else, uploading only if `moved`.
fn converges(releases: &Releases, tag: &str, moved: bool) -> Run {
    let run = releases.run();
    assert_eq!(run.code, Some(0), "{}", run.output);
    assert_eq!(
        releases.channel_holds().as_deref(),
        Some(tag),
        "{}",
        run.output
    );
    assert_eq!(releases.channel_files().len(), 2, "{}", run.output);
    assert_eq!(
        !run.uploads.is_empty(),
        moved,
        "{:?}\n{}",
        run.uploads,
        run.output
    );
    run
}

/// Runs the script and requires that it failed and changed nothing.
fn refuses(releases: &Releases, why: &str) {
    let before = releases.channel_files();
    let run = releases.run();
    assert_ne!(run.code, Some(0), "{why}: the run passed\n{}", run.output);
    assert!(run.uploads.is_empty(), "{why}: {:?}", run.uploads);
    assert_eq!(releases.channel_files(), before, "{why}");
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
    fs::remove_dir_all(releases.folder("v0.10.0-pre.3")).unwrap();
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
fn an_upload_that_broke_off_is_put_right_by_the_next_run() {
    // gh deletes both old files first. Broken off before the first upload,
    // the channel is empty; after the first, it holds the new manifest only.
    for (break_after, left) in [("0", vec![]), ("1", vec!["update.json"])] {
        let releases = Releases::new();
        releases
            .published("v0.9.4", "0.9.4")
            .published("v0.10.0-pre.1", "0.10.0-pre.1")
            .channel_from("v0.9.4");
        let run = releases.run_with(&[("GH_STUB_BREAK_AFTER", break_after)]);
        assert_ne!(run.code, Some(0), "{}", run.output);
        let names: Vec<String> = releases
            .channel_files()
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        assert_eq!(names, left, "after breaking off at {break_after}");
        converges(&releases, "v0.10.0-pre.1", true);
    }
}

#[test]
fn a_lone_file_left_by_a_broken_upload_is_put_right() {
    // A lone manifest at the highest version, and one below it.
    for held in ["v0.10.0-pre.1", "v0.9.4"] {
        let releases = Releases::new();
        releases
            .published("v0.9.4", "0.9.4")
            .published("v0.10.0-pre.1", "0.10.0-pre.1")
            .channel_from(held)
            .remove(CHANNEL, "update.json.minisig");
        let run = converges(&releases, "v0.10.0-pre.1", true);
        assert!(run.output.contains("::warning::"), "{}", run.output);
    }

    // A lone signature says no version; the highest release's pair goes in.
    let releases = Releases::new();
    releases
        .published("v0.9.4", "0.9.4")
        .published("v0.10.0-pre.1", "0.10.0-pre.1")
        .channel_from("v0.10.0-pre.1")
        .remove(CHANNEL, "update.json");
    converges(&releases, "v0.10.0-pre.1", true);

    // A signature that did not finish uploading counts as missing.
    let releases = Releases::new();
    releases
        .published("v0.9.4", "0.9.4")
        .published("v0.10.0-pre.1", "0.10.0-pre.1")
        .channel_from("v0.10.0-pre.1")
        .unfinished(CHANNEL, "update.json.minisig");
    converges(&releases, "v0.10.0-pre.1", true);
}

#[test]
fn a_pair_that_does_not_match_its_release_is_put_right() {
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
fn every_page_of_releases_is_read() {
    // GitHub lists the newest first; the highest version is the oldest
    // release here, on the last page.
    let releases = Releases::new();
    releases.published("v0.10.0-pre.2", "0.10.0-pre.2");
    for patch in 4..=9 {
        releases.published(&format!("v0.9.{patch}"), &format!("0.9.{patch}"));
    }
    releases.release(CHANNEL, &[]);
    converges(&releases, "v0.10.0-pre.2", true);
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
        .published("v0.10.0-pre.2", "0.10.0-pre.2")
        .unfinished("v0.10.0-pre.2", "update.json.minisig")
        .published("nightly", "9.9.9")
        .channel_from("v0.10.0-pre.1");
    converges(&releases, "v0.10.0-pre.1", false);
}

#[test]
fn a_release_whose_tag_is_not_a_version_is_skipped() {
    let releases = Releases::new();
    releases
        .published("v0.9.4", "0.9.4")
        .published("v0.10", "0.10")
        .release(CHANNEL, &[]);
    let run = converges(&releases, "v0.9.4", true);
    assert!(
        run.output.contains("::warning::skipping release v0.10"),
        "{}",
        run.output
    );
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
    releases
        .channel_from("v0.9.4")
        .release(CHANNEL, &[("notes.txt", "stray")]);
    refuses(&releases, "an extra file");

    let releases = base();
    releases.release(CHANNEL, &[("update.json", &manifest("0.10.0-pre.3"))]);
    refuses(&releases, "a lone manifest above every release");

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

        let releases = base();
        releases.release(CHANNEL, &[("update.json", content)]);
        refuses(&releases, &format!("{why}, lone"));
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
        .release("v0.9.4", &[("FerrisPass.dmg", "image")])
        .release(CHANNEL, &[]);
    refuses(&releases, "no release with a manifest");
}
