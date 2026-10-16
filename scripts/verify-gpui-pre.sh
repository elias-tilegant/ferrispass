#!/usr/bin/env bash
#
# Supply-chain check for the GPUI crates FerrisPass takes from crates.io.
#
# GPUI reaches FerrisPass as `gpui-pre`, a crates.io republication of Zed's
# GPUI crates by a single maintainer, and GPUI Kit (gpui-component and the
# crates around it) is published by Longbridge. Neither comes from a pinned git
# source any more, so every bump of those pins is checked with this script
# before the lock change is committed:
#
#   1. Every gpui-pre* crate in Cargo.lock is downloaded from static.crates.io
#      and its SHA-256 compared with the lock.
#   2. Its sources are compared with the Zed commit it names in
#      [package.metadata.gpui-pre] (zed-rev), which must be EXPECTED_ZED_REV
#      and on Zed's main branch. The crates carry no .cargo_vcs_info.json, so
#      that metadata is the only provenance. Accepted without review: the
#      rename to gpui-pre-*, path dependencies turned into registry
#      dependencies, git dependencies replaced by the crates.io release that
#      Zed's own manifest names next to them (those releases are compared with
#      their git commit as well), and exact pins relaxed to a caret while
#      Cargo.lock still holds the pinned version. Everything else, source
#      patches included, must be on REVIEWED_FINDINGS below.
#   3. The GPUI Kit crates must come from EXPECTED_KIT_REV, the commit of the
#      release tag v<version> of longbridge/gpui-kit. They are compared with
#      it, manifests included, and scanned for build scripts, network and
#      process APIs, embedded files and unsafe code. The scan is a list to
#      read, not a verdict.
#   4. The resolved feature graph must not enable FORBIDDEN_FEATURES or pull
#      in FORBIDDEN_CRATES.
#
# A file or directory upstream that a crate does not ship is a finding too
# (leaving out a check changes behaviour as much as adding one), and so is a
# difference in the published manifest of a replacement crate. Source
# findings carry the full SHA-256 of both sides, so a reviewed finding stops
# matching as soon as either side changes. A reviewed finding that no longer
# shows up fails the run until its line is deleted.
#
# Everything downloaded is untrusted data. Each archive and each checkout gets
# a fresh, empty directory, nothing in them is built or executed, and the
# Python helper that parses manifests runs isolated (-I) from a directory of
# its own.
#
# Usage: scripts/verify-gpui-pre.sh [--keep] [--work-dir DIR] [--also NAME@VERSION]...
#   --keep            keep the work directory (archives, checkouts, report/)
#   --work-dir DIR    use DIR, which must not exist yet; implies --keep
#   --also NAME@VER   also compare and scan a GPUI Kit crate that is not in
#                     Cargo.lock; its checksum comes from the crates.io index
#
# Exit status: 0 no unexpected difference, 1 unexpected difference found,
# 2 the check could not run (missing tool, network, malformed input).
#
# On a bump: update EXPECTED_ZED_REV and EXPECTED_KIT_REV, run the script, read every new
# UNEXPECTED finding in report/ (the .diff files hold the full diffs) and,
# once understood, add it to REVIEWED_FINDINGS with the reason. Delete the
# lines it reports as stale.
#
# Needs cargo (one `cargo metadata --locked` call), curl, git 2.32 or newer,
# diff, shasum and python3 3.11 or newer (tomllib). GITHUB_TOKEN, when
# set, is used for the one GitHub API call per repository (rate limits).

set -euo pipefail

# ---------------------------------------------------------------------------
# Reviewed inputs. Change these only as part of a reviewed bump.
# ---------------------------------------------------------------------------

ZED_REPO="https://github.com/zed-industries/zed.git"
ZED_GITHUB="zed-industries/zed"
ZED_BRANCH="main"
KIT_REPO="https://github.com/longbridge/gpui-kit.git"

# The Zed commit every gpui-pre crate must snapshot (gpui-pre 0.3.8).
EXPECTED_ZED_REV="279fe070bb389b79652e52065b2f001edcc0b11b"

# GPUI Kit crates published by Longbridge; each must come from tag v<version>,
# and that tag must be this commit (v0.7.1). A moved tag fails the check.
KIT_CRATES="gpui-component gpui-base gpui-component-macros gpui-kit-assets gpui-kit"
EXPECTED_KIT_REV="87d10ae5e1299d1be18670a37570c5402f952e34"

# Features that must stay off anywhere in the resolved graph (crate:feature),
# and crates that must not be in it at all. cargo metadata resolves for every
# platform and includes dev-dependencies, so this is the widest view.
FORBIDDEN_FEATURES="gpui-pre:inspector gpui-base:inspector gpui-component:inspector gpui-component:speech gpui-pre-http-client:github-download"
FORBIDDEN_CRATES="gpui-fast gpui-shell gpui-webview cpal objc2-speech"

# gpui-pre* crates that are not Zed snapshots, reviewed by hand, one per line:
# name, version, repository, commit. A gpui-pre crate that names no Zed commit
# and is not listed here fails the check.
#   gpui-pre-reqwest: zed-industries/reqwest branch prep-for-crates-io (PR #4).
#   Only gpui-kit-assets uses it, and only on wasm.
OTHER_PRE_CRATES="
gpui-pre-reqwest 0.12.15 https://github.com/zed-industries/reqwest.git 68a26f46bb6ed5131f56517a574041d2c4d2aec8
"

# Differences that were read and accepted, one per line:
#   crate | finding, exactly as the check prints it | why it is acceptable
# A finding on a file carries the full SHA-256 of the archive's file and, for a
# changed file, of the upstream one, so a change on either side fails again.
# A listed finding that no longer shows up fails the run as stale; delete its
# line then. The source patches come from the public republish script
# script/bump-gpui.ts in longbridge/gpui-kit.
read -r -d '' REVIEWED_FINDINGS <<'REVIEWED' || true
gpui-pre | [dependencies] proptest: Zed proptest (git https://github.com/proptest-rs/proptest rev 3dca198a8fef1b32e3a66f1e1897c955b4dc5b5b, version None), published proptest (crates.io 1) | optional and only behind test-support, which FerrisPass enables for its own tests only; crates.io proptest 1 stands in for Zed's git pin
gpui-pre-http-client | feature github-download: Zed ['dep:async-fs', 'dep:async-tar', 'dep:sha2', 'dep:tempfile', 'dep:util'], published [] | emptied because async-tar is a git-only dependency that crates.io rejects; FerrisPass never enables github-download
gpui-pre-http-client | [target.cfg(not(target_family="wasm")).dependencies] async-fs: only in Zed, async-fs (crates.io 2.1) | optional dependency of the emptied github-download feature, removed with it
gpui-pre-http-client | [target.cfg(not(target_family="wasm")).dependencies] async-tar: only in Zed, async-tar (git https://github.com/zed-industries/async-tar rev bd3ad6f89df9a9da7a8535958756d6bf465936a0, version None) | optional dependency of the emptied github-download feature, removed with it
gpui-pre-http-client | [target.cfg(not(target_family="wasm")).dependencies] sha2: only in Zed, sha2 (crates.io 0.10) | optional dependency of the emptied github-download feature, removed with it
gpui-pre-http-client | [target.cfg(not(target_family="wasm")).dependencies] tempfile: only in Zed, tempfile (crates.io 3.20.0) | optional dependency of the emptied github-download feature, removed with it
gpui-pre-http-client | [target.cfg(not(target_family="wasm")).dependencies] util: only in Zed, util (path crates/util) | optional dependency of the emptied github-download feature, removed with it
gpui-pre-macros | [dependencies] proc-macro-crate: only in the published crate, proc-macro-crate (crates.io 3) | needed by the facade rewrite, compile time only; 3.5.0 is in Cargo.lock anyway (gpui-component-macros, zbus_macros)
gpui-pre-sum-tree | [dependencies] proptest: Zed proptest (git https://github.com/proptest-rs/proptest rev 3dca198a8fef1b32e3a66f1e1897c955b4dc5b5b, version None), published proptest (crates.io 1) | optional and only behind test-support, as in gpui-pre
gpui-pre-web | [dependencies] unicode-properties: Zed pins =0.1.3, published 0.1.3, Cargo.lock resolves to 0.1.4 | gpui-pre-web is a wasm-only dependency of gpui-pre-platform and never built for macOS
gpui-pre | content differs: src/action.rs (sha256 78bd18baf94baf08637d99a34a3fd147b5784ccbbfefcad3a4bb9a6577b18c12, upstream 6566c34d4f4cf7f6a5280cbc4c07c747c5c6fc4c6554abd42c73fefe5716fc20) | actions! derives $crate::Action instead of gpui::Action, so the macro resolves under the gpui-pre name; nothing else changes
gpui-pre-apple | content differs: build.rs (sha256 e68855230185a01ad2355b2139a47b4285ecda44789b4f297356f6c957bd6bbf, upstream f1dcbed46932e6687961f41f51379df0306d03a57b0c7237506c7bb7c7ccc45d) | the shader-binding build script reads the gpui sources from vendor/gpui instead of ../gpui; the vendored files are compared byte for byte
gpui-pre-macros | content differs: src/gpui_macros.rs (sha256 62c50a2b15b39e51623304c3557cc897b5fd77ea84a415e617ceb060b0506663, upstream 3fa61f05bb72dfac77efcbbb079923acb3ac9440fe6e42057bf94954f6ac8d3b) | every proc-macro entry point passes its output through gpui_pre_facade_paths::rewrite, which changes nothing unless the crate being compiled names gpui-kit in its manifest
gpui-pre-macros | only in the archive: src/gpui_pre_facade_paths.rs (sha256 f929a8d8169407fa91e588b2768ad564f2200281e63ccdac775458c3d3bc2a5e) | the facade rewrite, read in full: when the manifest of the crate being compiled names gpui-kit, it rewrites gpui:: and gpui_platform:: path heads and string literals containing ::gpui:: or ::gpui_platform:: to the facade; otherwise every token passes through unchanged. Its one lookup is proc-macro-crate reading that manifest
gpui-pre-refineable | only upstream, not packaged: derive_refineable (a separate package, directory, tree sha256 e382af00567187f71e8f7f5d3013c51b8364a43062e2ed424692a4eefcf4f234) | published on its own as gpui-pre-derive-refineable and compared with Zed separately
gpui-pre-web | only upstream, not packaged: examples (directory, tree sha256 f484fb748e15ead42a0e675a6a0a4f38d9a38eaf9324bfad99bf479bf9911508) | example programs, not part of the library; gpui-pre-web is wasm-only and never built for macOS
gpui-pre-reqwest | only upstream, not packaged: examples/wasm_github_fetch (a separate package, directory, tree sha256 92031ee1310ec83a6bd1e6417ef06e5b0d4506aacdc6aafbefc1d40cc918425c) | a separate example package, not part of the library; gpui-pre-reqwest is wasm-only
zed-font-kit | only upstream, not packaged: c (a separate package, directory, tree sha256 f1958bd9439cd31a27e539d5862d8944b04e3f03dc35de74f1ca74943d9472b5) | font-kit-c, the separate C bindings package (staticlib), not part of the library
zed-font-kit | only upstream, not packaged: resources (directory, tree sha256 f43d2a182b4bb104ee0886d545b468846416d7983545c135fc8ac93c984aafd6) | test fonts, excluded by font-kit's own manifest (exclude = ["resources/**"]) and read only by a FreeType test
zed-font-kit | [target.cfg(not(any(target_arch="wasm32",target_family="windows",target_os="android",target_env="ohos"))).dependencies] dirs: version upstream '6.0', published '5.0' | on macOS font-kit calls only dirs::home_dir (src/sources/fs.rs, default_font_directories); dirs 5.0.1 with dirs-sys 0.4.1 and dirs 6.0.0 with dirs-sys 0.5.0 have the same mac.rs and the same home_dir ($HOME, else getpwuid_r), and dirs 5.0.1 was in the lock before
gpui-base | only upstream, not packaged: examples (directory, tree sha256 bcecdcec8204ac6be183def928410a910a59dbd89e262734c1506269afe4f4c0) | example programs, not part of the library
REVIEWED

# ---------------------------------------------------------------------------

die() {
    printf 'verify-gpui-pre: %s\n' "$*" >&2
    exit 2
}

KEEP=0
WORK=""
ALSO=""
while [ $# -gt 0 ]; do
    case "$1" in
        --keep) KEEP=1 ;;
        --work-dir)
            [ $# -ge 2 ] || die "--work-dir needs a directory"
            WORK=$2
            KEEP=1
            shift
            ;;
        --also)
            [ $# -ge 2 ] || die "--also needs NAME@VERSION"
            case "$2" in *@*) ;; *) die "--also needs NAME@VERSION, got: $2" ;; esac
            ALSO="$ALSO $2"
            shift
            ;;
        -h | --help)
            awk 'NR > 2 && /^#/ { sub(/^# ?/, ""); print; next } NR > 2 { exit }' "$0"
            exit 0
            ;;
        *) die "unknown argument: $1 (see --help)" ;;
    esac
    shift
done

for tool in cargo curl git diff shasum python3 awk; do
    command -v "$tool" >/dev/null 2>&1 || die "missing tool: $tool"
done
python3 -I -c 'import sys, tomllib; sys.exit(0 if sys.version_info >= (3, 11) else 1)' 2>/dev/null ||
    die "python3 3.11 or newer is required (tomllib)"

ROOT=$(cd "$(dirname "$0")/.." && pwd)
[ -f "$ROOT/Cargo.toml" ] && [ -f "$ROOT/Cargo.lock" ] || die "no Cargo.toml and Cargo.lock in $ROOT"

if [ -n "$WORK" ]; then
    [ ! -e "$WORK" ] || die "--work-dir must not exist yet: $WORK"
    mkdir -p "$WORK"
    WORK=$(cd "$WORK" && pwd)
else
    WORK=$(mktemp -d "${TMPDIR:-/tmp}/verify-gpui-pre.XXXXXX")
fi
cleanup() {
    if [ "$KEEP" -eq 0 ]; then
        rm -rf "$WORK"
    fi
}
trap cleanup EXIT
trap 'exit 2' INT TERM

# tool/ holds only our own helper. dl/ holds the archives, src/ one fresh
# directory per extracted archive, git/ one fresh directory per checkout.
mkdir "$WORK/tool" "$WORK/meta" "$WORK/dl" "$WORK/src" "$WORK/git" "$WORK/report"
mkdir "$WORK/report/pre" "$WORK/report/subst" "$WORK/report/kit"
SUMMARY="$WORK/report/summary.txt"
: >"$SUMMARY"

# Git must not pick up user or system configuration (credential helpers,
# filters, hooks) and must never prompt.
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null GIT_TERMINAL_PROMPT=0 GIT_LFS_SKIP_SMUDGE=1

UNEXPECTED=0
say() { printf '%s\n' "$*" | tee -a "$SUMMARY"; }
flag() {
    UNEXPECTED=$((UNEXPECTED + 1))
    say "  UNEXPECTED: $*"
}

HELPER="$WORK/tool/helper.py"
cat >"$HELPER" <<'PY'
"""Helper for scripts/verify-gpui-pre.sh.

Reads manifests, lock files and diff listings as data. Nothing it reads is
imported or executed. Values that end up in paths or URLs are validated.
"""
import hashlib
import json
import os
import re
import shutil
import sys
import tarfile
import tomllib

CRATES_IO = "registry+https://github.com/rust-lang/crates.io-index"
TOKEN = re.compile(r"^[A-Za-z0-9_.+-]+$")
SHA = re.compile(r"^[0-9a-f]{40}$")
REL_PATH = re.compile(r"^[A-Za-z0-9_.-]+(/[A-Za-z0-9_.-]+)*$")


def die(msg):
    print(f"helper: {msg}", file=sys.stderr)
    sys.exit(2)


def load_toml(path):
    with open(path, "rb") as f:
        return tomllib.load(f)


def token(value, what):
    if not isinstance(value, str) or not TOKEN.match(value):
        die(f"malformed {what}: {value!r}")
    return value


def safe_rel(value, what):
    if value == "":
        return "."
    if (
        not isinstance(value, str)
        or not REL_PATH.match(value)
        or any(part in (".", "..") for part in value.split("/"))
    ):
        die(f"malformed {what}: {value!r}")
    return value


def one_line(value):
    text = re.sub(r"\s+", " ", str(value)).strip()
    return text or "-"


def is_pre(name):
    return name == "gpui-pre" or name.startswith("gpui-pre-")


def lock_sums(lock_path):
    sums = {}
    for p in load_toml(lock_path).get("package", []):
        if p.get("source") == CRATES_IO:
            sums[(p["name"], p["version"])] = p.get("checksum")
    return sums


def lock_versions(lock_path):
    versions = {}
    for p in load_toml(lock_path).get("package", []):
        versions.setdefault(p["name"], set()).add(p["version"])
    return versions


def semver(text):
    m = re.match(r"^\s*[=^]?\s*(\d+)\.(\d+)\.(\d+)", text or "")
    return tuple(int(x) for x in m.groups()) if m else None


def caret_compatible(a, b):
    """Would a caret requirement on version a accept version b?"""
    if a[0] != b[0]:
        return False
    if a[0] > 0:
        return True
    if a[1] != b[1]:
        return False
    return a[1] > 0 or a[2] == b[2]


def sha256_file(path):
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


def sha256_tree(root):
    digest = hashlib.sha256()
    for here, dirs, files in os.walk(root):
        dirs.sort()
        for name in sorted(files):
            path = os.path.join(here, name)
            digest.update(f"{os.path.relpath(path, root)}\0{sha256_file(path)}\n".encode())
    return digest.hexdigest()


class Findings:
    """Prints LEVEL<TAB>message lines; UNEXPECTED ones that are on the
    reviewed list print as REVIEWED<TAB>message<TAB>reason instead."""

    def __init__(self, crate, reviewed_path, used_path):
        self.crate = crate
        self.used_path = used_path
        self.reviewed = {}
        with open(reviewed_path, encoding="utf-8") as f:
            for line in f:
                parts = [p.strip() for p in line.split(" | ")]
                if len(parts) == 3 and not line.lstrip().startswith("#"):
                    self.reviewed[(parts[0], parts[1])] = parts[2]
        self.bad = 0

    def out(self, level, message):
        if level == "UNEXPECTED" and (self.crate, message) in self.reviewed:
            print(f"REVIEWED\t{message}\t{self.reviewed[(self.crate, message)]}")
            with open(self.used_path, "a", encoding="utf-8") as used:
                used.write(f"{self.crate} | {message}\n")
            return
        if level == "UNEXPECTED":
            self.bad += 1
        print(f"{level}\t{message}")

    def exit(self):
        sys.exit(1 if self.bad else 0)


def cmd_crates(meta_path, lock_path, kit_names):
    """List gpui-pre* and GPUI Kit crates: kind, name, version, checksum."""
    kit = set(kit_names.split())
    with open(meta_path, encoding="utf-8") as f:
        meta = json.load(f)
    sums = lock_sums(lock_path)
    rows = set()
    for p in meta.get("packages", []):
        name, version = p["name"], p["version"]
        if is_pre(name):
            kind = "pre"
        elif name in kit:
            kind = "kit"
        else:
            continue
        if p.get("source") != CRATES_IO:
            die(f"{name} {version} does not come from crates.io (source {p.get('source')!r})")
        checksum = sums.get((name, version))
        if not checksum or not re.match(r"^[0-9a-f]{64}$", checksum):
            die(f"no valid checksum for {name} {version} in Cargo.lock")
        rows.add((kind, token(name, "name"), token(version, "version"), checksum))
    listed = {(r[1], r[2]) for r in rows}
    for name, version in sums:
        if is_pre(name) and (name, version) not in listed:
            die(f"Cargo.lock has {name} {version}, cargo metadata does not")
    if not any(r[0] == "pre" for r in rows):
        die("no gpui-pre crate in cargo metadata")
    for row in sorted(rows):
        print("\t".join(row))


def cmd_lock_checksum(lock_path, name, version):
    print(lock_sums(lock_path).get((name, version)) or "-")


def cmd_index_checksum(index_path, version):
    with open(index_path, encoding="utf-8") as f:
        for line in f:
            entry = json.loads(line)
            if entry.get("vers") == version:
                checksum = entry.get("cksum", "")
                if not re.match(r"^[0-9a-f]{64}$", checksum):
                    die("malformed checksum in the crates.io index")
                print(f"{checksum}\t{'yanked' if entry.get('yanked') else 'live'}")
                return
    die(f"version {version} not in the crates.io index")


def cmd_provenance(crate_root):
    """zed-rev, zed-crate, vcs sha1, path_in_vcs, dirty, description."""
    manifest = load_toml(os.path.join(crate_root, "Cargo.toml"))
    package = manifest.get("package", {})
    snap = package.get("metadata", {}).get("gpui-pre", {})
    rev = snap.get("zed-rev", "-")
    if rev != "-" and not SHA.match(rev):
        die(f"malformed zed-rev: {rev!r}")
    crate = snap.get("zed-crate", "-")
    if crate != "-":
        token(crate, "zed-crate")
    sha, path, dirty = "-", "-", "-"
    vcs_path = os.path.join(crate_root, ".cargo_vcs_info.json")
    if os.path.isfile(vcs_path):
        with open(vcs_path, encoding="utf-8") as f:
            vcs = json.load(f)
        git = vcs.get("git", {})
        sha = git.get("sha1", "-")
        if sha != "-" and not SHA.match(sha):
            die(f"malformed vcs sha1: {sha!r}")
        path = safe_rel(vcs.get("path_in_vcs", ""), "path_in_vcs")
        dirty = "true" if git.get("dirty") else "false"
    print("\t".join([rev, crate, sha, path, dirty, one_line(package.get("description", "-"))]))


def cmd_zed_paths(workspace_path, crates_tsv):
    """Map each Zed crate name (4th column of crates_tsv) to its directory."""
    deps = load_toml(workspace_path).get("workspace", {}).get("dependencies", {})
    with open(crates_tsv, encoding="utf-8") as f:
        for line in f:
            name = line.rstrip("\n").split("\t")[3]
            spec = deps.get(name)
            if not isinstance(spec, dict) or "path" not in spec:
                die(f"Zed workspace has no path dependency named {name}")
            print(f"{name}\t{safe_rel(spec['path'], 'Zed crate path')}")


DEP_KINDS = ("dependencies", "dev-dependencies", "build-dependencies")


def dep_tables(manifest):
    tables = {}
    for kind in DEP_KINDS:
        table = manifest.get(kind) or manifest.get(kind.replace("-", "_"))
        if table:
            tables[kind] = table
    for cfg, target in (manifest.get("target") or {}).items():
        for kind in DEP_KINDS:
            table = target.get(kind) or target.get(kind.replace("-", "_"))
            if table:
                tables[f"target.{re.sub(r'[ ]+', '', cfg)}.{kind}"] = table
    return tables


def spec_of(raw):
    return {"version": raw} if isinstance(raw, str) else dict(raw)


KNOWN_DEP_FIELDS = {
    "package", "version", "path", "git", "rev", "tag", "branch",
    "features", "default-features", "default_features", "optional",
}


def norm(key, spec):
    """A dependency as compared. Fields outside KNOWN_DEP_FIELDS (registry,
    registry-index, artifact, lib, target, public, workspace...) are kept in
    `other`, so any of them on either side is a difference."""
    return {
        "package": spec.get("package", key),
        "version": spec.get("version"),
        "path": spec.get("path"),
        "git": spec.get("git"),
        "rev": spec.get("rev") or spec.get("tag") or spec.get("branch"),
        "features": tuple(sorted(set(spec.get("features", [])))),
        "default_features": bool(spec.get("default-features", spec.get("default_features", True))),
        "optional": bool(spec.get("optional", False)),
        "other": tuple(sorted((k, json.dumps(v, sort_keys=True)) for k, v in spec.items() if k not in KNOWN_DEP_FIELDS)),
    }


def resolve_zed(key, raw, ws_deps):
    """Apply Cargo's workspace inheritance to a dependency of a Zed crate."""
    spec = spec_of(raw)
    if spec.get("workspace"):
        if key not in ws_deps:
            die(f"Zed workspace does not define dependency {key}")
        base = spec_of(ws_deps[key])
        merged = {k: v for k, v in base.items() if k not in ("features", "optional")}
        merged["features"] = list(base.get("features", [])) + list(spec.get("features", []))
        merged["optional"] = spec.get("optional", False)
        if spec.get("default-features") is True or spec.get("default_features") is True:
            merged["default-features"] = True
        spec = merged
    return norm(key, spec)


def effective_build(pkg, root):
    """The build script Cargo runs: the declared one, else build.rs if present."""
    value = pkg.get("build")
    if value is None:
        return "build.rs" if os.path.isfile(os.path.join(root, "build.rs")) else False
    return value


def effective_lib(manifest, name):
    """(path, name, proc-macro) of the library target, with Cargo's defaults
    filled in; published manifests spell them out, source manifests do not."""
    table = manifest.get("lib", {})
    return (
        table.get("path", "src/lib.rs"),
        table.get("name", name.replace("-", "_")),
        bool(table.get("proc-macro", table.get("proc_macro", False))),
    )


def describe(dep):
    if dep["path"]:
        source = f"path {dep['path']}"
    elif dep["git"]:
        source = f"git {dep['git']} rev {dep['rev']}, version {dep['version']}"
    else:
        source = f"crates.io {dep['version']}"
    return f"{dep['package']} ({source})"


def cmd_manifest(crate_root, workspace_path, zed_dir, mapping_tsv, subst_out, lock_path, crate, reviewed, used, label="Zed"):
    """Compare a published manifest with the Zed manifest it came from.

    The published Cargo.toml is the one Cargo uses when it builds the crate as
    a dependency, so that is the side compared. Allowed without review: the
    package rename, path dependencies on other republished Zed crates, git
    dependencies replaced by the crates.io release Zed names next to them
    (written to subst_out for a separate check), exact pins relaxed to a caret
    requirement while Cargo.lock still holds the pinned version, and optional
    dependencies whose package is not in Cargo.lock at all (never built).
    Dev-dependencies are never built for FerrisPass and only noted. The GPUI
    Kit crates go through the same comparison against their own workspace
    (label "upstream", the mapping maps each crate to itself).
    """
    published = load_toml(os.path.join(crate_root, "Cargo.toml"))
    zed = load_toml(os.path.join(zed_dir, "Cargo.toml"))
    ws = load_toml(workspace_path).get("workspace", {})
    ws_deps = ws.get("dependencies", {})
    locked = lock_versions(lock_path)
    mapping = {}
    with open(mapping_tsv, encoding="utf-8") as f:
        for line in f:
            zed_name, pub_name = line.rstrip("\n").split("\t")
            mapping[zed_name] = pub_name
    findings = Findings(crate, reviewed, used)
    out = findings.out

    pp, zp = published.get("package", {}), zed.get("package", {})

    def inherited(field):
        value = zp.get(field)
        if isinstance(value, dict) and value.get("workspace"):
            return ws.get("package", {}).get(field)
        return value

    if mapping.get(zp.get("name")) != pp.get("name"):
        out("UNEXPECTED", f"package {pp.get('name')} does not map to {label} crate {zp.get('name')}")
    else:
        out("ALLOWED", f"package {zp.get('name')} {inherited('version')} published as {pp.get('name')} {pp.get('version')}")
    for field in ("edition", "links"):
        if inherited(field) != pp.get(field):
            out("UNEXPECTED", f"package.{field}: {label} {inherited(field)!r}, published {pp.get(field)!r}")
    if inherited("rust-version") != pp.get("rust-version"):
        out("INFO", f"package.rust-version: {label} {inherited('rust-version')!r}, published {pp.get('rust-version')!r}")

    zb, pb = effective_build(zp, zed_dir), effective_build(pp, crate_root)
    if zb != pb:
        out("UNEXPECTED", f"build script: {label} {zb!r}, published {pb!r}")

    zl, pl = effective_lib(zed, zp.get("name", "")), effective_lib(published, pp.get("name", ""))
    if (zl[0], zl[2]) != (pl[0], pl[2]):
        out("UNEXPECTED", f"lib target: {label} path {zl[0]} proc-macro {zl[2]}, published path {pl[0]} proc-macro {pl[2]}")
    if zl[1] != pl[1]:
        out("INFO", f"lib name: {label} {zl[1]}, published {pl[1]} (dependents import it by their dependency key)")

    zf = {k: tuple(sorted(v)) for k, v in (zed.get("features") or {}).items()}
    pf = {k: tuple(sorted(v)) for k, v in (published.get("features") or {}).items()}
    for name in sorted(set(zf) | set(pf)):
        if zf.get(name) != pf.get(name):
            out("UNEXPECTED", f"feature {name}: {label} {list(zf.get(name, ()))}, published {list(pf.get(name, ()))}")

    for section in ("patch", "replace"):
        if section in published:
            out("INFO", f"[{section}] present (Cargo ignores it in a dependency)")

    def relaxed_pin(z, p):
        """Zed pins =X, the crate asks for ^X: fine while the lock holds X."""
        if not (z["version"] or "").lstrip().startswith("="):
            return None
        pinned = semver(z["version"])
        if pinned is None or p["package"] != z["package"]:
            return None
        exact = z["version"].lstrip("= ")
        # Only "X" or "^X" is the caret form of "=X"; ">=X", "~X" or a range
        # starting with X is a different requirement and stays unexpected.
        if not re.fullmatch(r"\^?\s*" + re.escape(exact), (p["version"] or "").strip()):
            return None
        compatible = sorted(v for v in locked.get(p["package"], ()) if semver(v) and caret_compatible(pinned, semver(v)))
        if compatible == [exact]:
            return ("RELAXED", f"{label} pins {z['version']}, published {p['version']}, Cargo.lock holds {exact}")
        if not compatible:
            return ("INFO", f"{label} pins {z['version']}, published {p['version']}, not in Cargo.lock (not built)")
        return ("UNEXPECTED", f"{label} pins {z['version']}, published {p['version']}, Cargo.lock resolves to {', '.join(compatible)}")

    z_tables, p_tables = dep_tables(zed), dep_tables(published)
    with open(subst_out, "a", encoding="utf-8") as subst:
        for table in sorted(set(z_tables) | set(p_tables)):
            miss = "INFO" if table.endswith("dev-dependencies") else "UNEXPECTED"
            zt, pt = z_tables.get(table, {}), p_tables.get(table, {})
            for key in sorted(set(zt) | set(pt)):
                where = f"[{table}] {key}"
                if key not in pt:
                    out(miss, f"{where}: only in {label}, {describe(resolve_zed(key, zt[key], ws_deps))}")
                    continue
                p = norm(key, spec_of(pt[key]))
                unbuilt = miss == "UNEXPECTED" and p["optional"] and p["package"] not in locked
                level = "INFO" if unbuilt else miss
                suffix = " (optional, not in Cargo.lock, never built)" if unbuilt else ""
                if p["path"] or p["git"]:
                    out(level, f"{where}: published dependency is not a crates.io dependency, {describe(p)}{suffix}")
                if key not in zt:
                    out(level, f"{where}: only in the published crate, {describe(p)}{suffix}")
                    continue
                z = resolve_zed(key, zt[key], ws_deps)
                if z["path"]:
                    want = mapping.get(z["package"])
                    if want is None:
                        out(level, f"{where}: {label} path crate {z['package']} is not a republished crate in Cargo.lock, published {describe(p)}{suffix}")
                    elif p["package"] != want:
                        out(level, f"{where}: {label} {z['package']} should be {want}, published {describe(p)}{suffix}")
                    else:
                        out("ALLOWED", f"{where}: path dependency, published {p['package']} {p['version']}")
                elif z["git"]:
                    if p["package"] == z["package"] and z["version"] and p["version"] == z["version"]:
                        out("SUBST", f"{where}: {label} {describe(z)}, published crates.io {p['package']} {p['version']}")
                        subst.write(f"{p['package']}\t{p['version'].lstrip('=')}\t{z['git']}\t{z['rev']}\n")
                    else:
                        out(level, f"{where}: {label} {describe(z)}, published {describe(p)}{suffix}")
                elif (p["package"], p["version"]) != (z["package"], z["version"]):
                    relaxed = relaxed_pin(z, p)
                    if relaxed and miss == "UNEXPECTED":
                        out(relaxed[0], f"{where}: {relaxed[1]}")
                    else:
                        out(level, f"{where}: {label} {describe(z)}, published {describe(p)}{suffix}")
                for field in ("features", "default_features", "optional", "other"):
                    if z[field] != p[field]:
                        out(level, f"{where}: {field} {label} {z[field]!r}, published {p[field]!r}{suffix}")
    findings.exit()


def cmd_manifest_plain(crate_root, up_dir, crate, reviewed, used):
    """Compare the published manifest of a crate that is not a Zed snapshot
    (a crates.io replacement for a Zed git dependency, or gpui-pre-reqwest)
    with the manifest at its upstream commit. The published Cargo.toml is the
    one Cargo builds with. Name, version and description are notes; every
    other difference in what gets built is unexpected unless reviewed."""
    published = load_toml(os.path.join(crate_root, "Cargo.toml"))
    upstream = load_toml(os.path.join(up_dir, "Cargo.toml"))
    findings = Findings(crate, reviewed, used)
    out = findings.out
    pp, up = published.get("package", {}), upstream.get("package", {})
    for field in ("name", "version"):
        if up.get(field) != pp.get(field):
            out("INFO", f"package.{field}: upstream {up.get(field)!r}, published {pp.get(field)!r}")
    for field in ("edition", "links"):
        if up.get(field) != pp.get(field):
            out("UNEXPECTED", f"package.{field}: upstream {up.get(field)!r}, published {pp.get(field)!r}")
    ub, pb = effective_build(up, up_dir), effective_build(pp, crate_root)
    if ub != pb:
        out("UNEXPECTED", f"build script: upstream {ub!r}, published {pb!r}")
    ul, pl = effective_lib(upstream, up.get("name", "")), effective_lib(published, pp.get("name", ""))
    if (ul[0], ul[2]) != (pl[0], pl[2]):
        out("UNEXPECTED", f"lib target: upstream path {ul[0]} proc-macro {ul[2]}, published path {pl[0]} proc-macro {pl[2]}")
    if ul[1] != pl[1]:
        out("INFO", f"lib name: upstream {ul[1]}, published {pl[1]} (follows the package name)")
    for field in ("crate-type", "crate_type"):
        if (upstream.get("lib") or {}).get(field) != (published.get("lib") or {}).get(field):
            out("UNEXPECTED", f"lib {field}: upstream {(upstream.get('lib') or {}).get(field)!r}, published {(published.get('lib') or {}).get(field)!r}")
    uf = {k: sorted(v) for k, v in (upstream.get("features") or {}).items()}
    pf = {k: sorted(v) for k, v in (published.get("features") or {}).items()}
    for name in sorted(set(uf) | set(pf)):
        if uf.get(name) != pf.get(name):
            out("UNEXPECTED", f"feature {name}: upstream {uf.get(name)}, published {pf.get(name)}")
    u_tables, p_tables = dep_tables(upstream), dep_tables(published)
    for table in sorted(set(u_tables) | set(p_tables)):
        level = "INFO" if table.endswith("dev-dependencies") else "UNEXPECTED"
        ut, pt = u_tables.get(table, {}), p_tables.get(table, {})
        for key in sorted(set(ut) | set(pt)):
            where = f"[{table}] {key}"
            if key not in pt:
                out(level, f"{where}: only upstream, {describe(norm(key, spec_of(ut[key])))}")
                continue
            p = norm(key, spec_of(pt[key]))
            if p["path"] or p["git"]:
                out(level, f"{where}: published dependency is not a crates.io dependency, {describe(p)}")
            if key not in ut:
                out(level, f"{where}: only in the published crate, {describe(p)}")
                continue
            u = norm(key, spec_of(ut[key]))
            for field in ("package", "version", "features", "default_features", "optional", "other"):
                if u[field] != p[field]:
                    out(level, f"{where}: {field} upstream {u[field]!r}, published {p[field]!r}")
    findings.exit()


def under(path, root):
    return path == root or path.startswith(root.rstrip("/") + "/")


def cmd_classify_diff(listing, crate_root, up_root, repo_root, crate, reviewed, used):
    """Sort `diff -rq` output into unexpected differences and notes.

    Unexpected: a file whose content differs, and anything only in the
    archive. Two archive-only cases are notes instead: a license or readme
    file Cargo copied into the package root from a parent directory of the
    upstream crate, and a vendor/<crate>/ tree whose files are identical to
    the sibling crate next to the upstream one. Both are checked byte for
    byte. Anything only upstream is unexpected too: leaving out a file can
    change behaviour as much as adding one. Findings carry the full SHA-256 of
    what they are about (both sides for a changed file), so a reviewed finding
    stops matching when either side changes.
    """
    findings = Findings(crate, reviewed, used)
    out = findings.out
    with open(listing, encoding="utf-8", errors="replace") as f:
        lines = [line.rstrip("\n") for line in f if line.strip()]
    for line in lines:
        m = re.match(r"^Only in (.*): (.*)$", line)
        if m:
            parent, name = m.group(1), m.group(2)
            full = os.path.join(parent, name)
            if under(parent, crate_root):
                rel = os.path.relpath(full, crate_root)
                if os.path.isfile(full):
                    origin = None
                    if "/" not in rel:
                        here = os.path.dirname(up_root.rstrip("/"))
                        while under(here, repo_root):
                            candidate = os.path.join(here, rel)
                            if os.path.isfile(candidate) and sha256_file(candidate) == sha256_file(full):
                                origin = os.path.relpath(candidate, repo_root)
                                break
                            if here == repo_root:
                                break
                            here = os.path.dirname(here)
                    if origin:
                        out("INFO", f"only in the archive: {rel}, copied by cargo package from {origin} (identical)")
                    else:
                        out("UNEXPECTED", f"only in the archive: {rel} (sha256 {sha256_file(full)})")
                elif os.path.isdir(full) and rel.split("/")[0] == "vendor":
                    vendor = os.path.join(crate_root, "vendor")
                    sibling_root = os.path.dirname(up_root.rstrip("/"))
                    same = 0
                    for here, dirs, files in os.walk(full):
                        dirs.sort()
                        for fname in sorted(files):
                            path = os.path.join(here, fname)
                            sibling = os.path.join(sibling_root, os.path.relpath(path, vendor))
                            if os.path.isfile(sibling) and sha256_file(sibling) == sha256_file(path):
                                same += 1
                            else:
                                out("UNEXPECTED", f"vendored file differs from ../{os.path.relpath(path, vendor)}: {os.path.relpath(path, crate_root)} (sha256 {sha256_file(path)})")
                    out("INFO", f"only in the archive: {rel}/, {same} vendored files identical to the sibling Zed crates")
                else:
                    out("UNEXPECTED", f"only in the archive: {rel}/ (tree sha256 {sha256_tree(full)})")
            else:
                rel = os.path.relpath(full, up_root)
                if os.path.isdir(full):
                    kind = f"directory, tree sha256 {sha256_tree(full)}"
                    if os.path.isfile(os.path.join(full, "Cargo.toml")):
                        kind = "a separate package, " + kind
                else:
                    kind = f"file, sha256 {sha256_file(full)}"
                out("UNEXPECTED", f"only upstream, not packaged: {rel} ({kind})")
            continue
        m = re.match(r"^Files (.*) and (.*) differ$", line)
        if m and under(m.group(1), crate_root):
            rel = os.path.relpath(m.group(1), crate_root)
            out("UNEXPECTED", f"content differs: {rel} (sha256 {sha256_file(m.group(1))}, upstream {sha256_file(m.group(2))})")
        else:
            out("UNEXPECTED", line)
    findings.exit()


def cmd_check_tree(root, mode):
    """crate: only regular files and directories. checkout: links stay inside."""
    real_root = os.path.realpath(root)
    bad = []
    for here, dirs, files in os.walk(root):
        if mode == "checkout" and ".git" in dirs and here == root:
            dirs.remove(".git")
        for name in dirs + files:
            path = os.path.join(here, name)
            if os.path.islink(path):
                if mode == "crate" or not under(os.path.realpath(path), real_root):
                    bad.append(path)
            elif name in files and not os.path.isfile(path):
                bad.append(path)
    for path in bad:
        print(path, file=sys.stderr)
    sys.exit(1 if bad else 0)


def cmd_extract_crate(archive, dest, prefix):
    """Check every member of a .crate archive, then extract it into dest.

    Only regular files and directories under prefix/ pass: no links, devices
    or fifos, no absolute names, no empty, "." or ".." components, no name
    twice. Nothing is written before every member has passed, and the files
    are written here one by one (exclusive create, no overwrite), so nothing
    in the archive decides where data lands. Exit 1 lists the problems.
    """
    token(prefix, "crate directory")
    try:
        with tarfile.open(archive, "r:gz") as tar:
            members = tar.getmembers()
            problems, seen = [], set()
            for m in members:
                parts = m.name.split("/")
                if m.isdir() and len(parts) > 1 and parts[-1] == "":
                    parts = parts[:-1]
                if m.name.startswith("/") or parts[0] != prefix or any(x in ("", ".", "..") for x in parts):
                    problems.append(f"name outside {prefix}/: {m.name!r}")
                elif not (m.isreg() or m.isdir()):
                    problems.append(f"neither file nor directory (type {m.type!r}): {m.name!r}")
                elif "/".join(parts) in seen:
                    problems.append(f"listed twice: {m.name!r}")
                seen.add("/".join(parts))
            if problems:
                for problem in problems:
                    print(problem, file=sys.stderr)
                sys.exit(1)
            os.makedirs(dest)
            files = 0
            for m in members:
                target = os.path.join(dest, *[x for x in m.name.split("/") if x])
                if m.isdir():
                    os.makedirs(target, exist_ok=True)
                    continue
                os.makedirs(os.path.dirname(target), exist_ok=True)
                with tar.extractfile(m) as src, open(target, "xb") as dst:
                    shutil.copyfileobj(src, dst)
                files += 1
    except (tarfile.TarError, OSError, EOFError) as e:
        print(f"cannot extract {archive}: {e}", file=sys.stderr)
        sys.exit(1)
    print(files)


def cmd_features(meta_path, forbidden_features, forbidden_crates):
    """Print every forbidden feature that is on and every forbidden crate that
    is in the resolved graph, one per line; exit 1 if there is any."""
    with open(meta_path, encoding="utf-8") as f:
        meta = json.load(f)
    packages = {p["id"]: p for p in meta.get("packages", [])}
    nodes = (meta.get("resolve") or {}).get("nodes")
    if not nodes:
        die("cargo metadata has no resolved graph")
    banned = {tuple(item.split(":", 1)) for item in forbidden_features.split()}
    banned_crates = set(forbidden_crates.split())
    hits = []
    for node in nodes:
        name = packages[node["id"]]["name"]
        if name in banned_crates:
            hits.append(f"crate {name} {packages[node['id']]['version']} is in the graph")
        for feature in node.get("features", []):
            if (name, feature) in banned:
                hits.append(f"feature {name}:{feature} is on")
    for hit in sorted(hits):
        print(hit)
    sys.exit(1 if hits else 0)


def cmd_compare_status(path):
    with open(path, encoding="utf-8") as f:
        data = json.load(f)
    print(f"{data.get('status', '?')}\t{data.get('ahead_by', '?')}\t{data.get('behind_by', '?')}")


SCAN = [
    ("network", r"\bstd::net\b|\bTcpStream\b|\bTcpListener\b|\bUdpSocket\b|\bToSocketAddrs\b"),
    ("http-client", r"\breqwest\b|\bureq\b|\bhyper\b|\bisahc\b|\bsurf::|\bcurl::"),
    # A bare `Command::new` only counts in a file that imports a process API:
    # GPUI Kit has a UI component of that name.
    ("process", r"\bstd::process\b|\bprocess::(?:Command|Stdio|exit|abort)\b|\b(?:tokio|smol|async_process)::process\b"),
    ("include_bytes", r"\binclude_bytes!\s*\("),
    ("include_str", r"\binclude_str!\s*\("),
    ("ffi", r"extern\s+\"C\"|#\[\s*link\b|no_mangle|link_section"),
    ("unsafe-block", r"\bunsafe\s*\{"),
    ("unsafe-fn", r"\bunsafe\s+(?:extern\s+\"[^\"]*\"\s+)?fn\b"),
    ("unsafe-impl", r"\bunsafe\s+impl\b"),
]
LINE_COMMENT = re.compile(r"(^|\s)//.*$")


def cmd_scan(crate_root, report_path):
    """Count risky constructs in the library sources and list every hit."""
    hits = {name: [] for name, _ in SCAN}
    for here, dirs, files in os.walk(crate_root):
        dirs.sort()
        for name in sorted(files):
            if not name.endswith(".rs"):
                continue
            path = os.path.join(here, name)
            rel = os.path.relpath(path, crate_root)
            with open(path, encoding="utf-8", errors="replace") as f:
                lines = f.readlines()
            spawns = any(re.search(r"\bprocess\b", LINE_COMMENT.sub("", t)) for t in lines)
            for number, text in enumerate(lines, 1):
                code = LINE_COMMENT.sub("", text)
                for label, pattern in SCAN:
                    if re.search(pattern, code) or (
                        label == "process" and spawns and re.search(r"\bCommand::new\b", code)
                    ):
                        hits[label].append(f"{rel}:{number}: {text.strip()[:160]}")
    manifest = load_toml(os.path.join(crate_root, "Cargo.toml"))
    package = manifest.get("package", {})
    has_build = os.path.isfile(os.path.join(crate_root, "build.rs")) and package.get("build", "build.rs") is not False

    def library(hit):
        return hit.startswith("src/") or hit.startswith("build.rs:")

    with open(report_path, "w", encoding="utf-8") as report:
        report.write(f"build.rs: {'yes' if has_build else 'no'}; links: {package.get('links', '-')}\n")
        for table, deps in sorted(dep_tables(manifest).items()):
            report.write(f"[{table}] {', '.join(sorted(norm(k, spec_of(v))['package'] for k, v in deps.items()))}\n")
        for label, _ in SCAN:
            found = hits[label]
            report.write(f"\n## {label}: {sum(map(library, found))} in src/ and build.rs, {len(found)} in all .rs files\n")
            report.writelines(f"{hit}\n" for hit in found)
    summary = " ".join(f"{label}={sum(map(library, hits[label]))}" for label, _ in SCAN)
    print(f"build.rs={'yes' if has_build else 'no'} {summary}")


COMMANDS = {
    "crates": cmd_crates,
    "lock-checksum": cmd_lock_checksum,
    "index-checksum": cmd_index_checksum,
    "provenance": cmd_provenance,
    "zed-paths": cmd_zed_paths,
    "manifest": cmd_manifest,
    "manifest-plain": cmd_manifest_plain,
    "classify-diff": cmd_classify_diff,
    "extract-crate": cmd_extract_crate,
    "features": cmd_features,
    "check-tree": cmd_check_tree,
    "compare-status": cmd_compare_status,
    "scan": cmd_scan,
}

if __name__ == "__main__":
    if len(sys.argv) < 2 or sys.argv[1] not in COMMANDS:
        die("unknown command")
    COMMANDS[sys.argv[1]](*sys.argv[2:])
PY

# Run the helper isolated: no script directory, no cwd and no user site on
# sys.path, no PYTHON* environment.
py() { python3 -I "$HELPER" "$@"; }

REVIEWED="$WORK/meta/reviewed.txt"
REVIEWED_USED="$WORK/meta/reviewed-used.txt"
printf '%s\n' "$REVIEWED_FINDINGS" | grep -v '^[[:space:]]*$' >"$REVIEWED" || true
: >"$REVIEWED_USED"
awk -F ' [|] ' '/^[[:space:]]*#/ { next } NF != 3 { print "malformed REVIEWED_FINDINGS line: " $0; bad = 1 } END { exit bad }' "$REVIEWED" >&2 ||
    die "fix REVIEWED_FINDINGS"

gitc() {
    git -c core.hooksPath=/dev/null -c core.fsmonitor=false -c protocol.file.allow=never \
        -c advice.detachedHead=false "$@"
}

GITHUB_HEADER="$WORK/meta/github-header"
if [ -n "${GITHUB_TOKEN:-}" ]; then
    (umask 077 && printf 'Authorization: Bearer %s\n' "$GITHUB_TOKEN" >"$GITHUB_HEADER")
else
    : >"$GITHUB_HEADER"
fi

# github_compare OWNER/REPO BASE HEAD: prints "status ahead behind".
github_compare() {
    curl -q -fsSL --proto '=https' --tlsv1.2 -H 'Accept: application/vnd.github+json' -H "@$GITHUB_HEADER" \
        -o "$WORK/meta/compare.json" "https://api.github.com/repos/$1/compare/$2...$3?per_page=1" ||
        return 1
    py compare-status "$WORK/meta/compare.json"
}

# git_fetch DEST URL REV: shallow, blobless fetch of exactly REV into a new repo.
git_fetch() {
    local dest=$1 url=$2 rev=$3 got
    case "$rev" in *[!0-9a-f]* | "") die "not a full commit id: $rev" ;; esac
    [ ${#rev} -eq 40 ] || die "not a full commit id: $rev"
    mkdir "$dest"
    gitc init -q "$dest"
    gitc -C "$dest" remote add origin "$url"
    gitc -C "$dest" fetch -q --depth 1 --filter=blob:none origin "$rev" || die "cannot fetch $rev from $url"
    got=$(gitc -C "$dest" rev-parse FETCH_HEAD)
    [ "$got" = "$rev" ] || die "fetched $got instead of $rev from $url"
}

# git_checkout DEST [PATH...]: check out FETCH_HEAD, only PATHs when given
# (cone mode, which also keeps the files at the repository root).
git_checkout() {
    local dest=$1
    shift
    if [ $# -gt 0 ]; then
        gitc -C "$dest" sparse-checkout set --cone "$@"
    fi
    gitc -C "$dest" checkout -q --detach FETCH_HEAD || die "checkout failed in $dest"
    # A link that leaves the checkout would let diff read files outside it.
    py check-tree "$dest" checkout || die "checkout $dest has links that leave it"
}

# reachable URL REV: is REV a branch or tag head, or on the default branch?
reachable() {
    local url=$1 rev=$2 refs slug status
    refs=$(gitc ls-remote "$url" | awk -v r="$rev" '$1 == r { print $2 }' | tr '\n' ' ')
    if [ -n "$refs" ]; then
        printf 'head of %s\n' "${refs% }"
        return 0
    fi
    case "$url" in
        https://github.com/*)
            slug=${url#https://github.com/}
            slug=${slug%.git}
            status=$(github_compare "$slug" HEAD "$rev") || return 1
            case "$status" in
                behind* | identical*)
                    printf 'on the default branch (%s)\n' "$status"
                    return 0
                    ;;
            esac
            ;;
    esac
    return 1
}

# fetch_crate NAME VERSION SHA256: download, verify and extract into a fresh
# directory. Sets CRATE_DIR. Returns 1 after flagging a bad archive.
# Callers run it as `fetch_crate ... || continue`, which switches errexit off
# inside the function, so every step here checks its own status.
fetch_crate() {
    local name=$1 version=$2 sum=$3 archive dest actual
    dest="$WORK/src/$name-$version"
    CRATE_DIR="$dest/$name-$version"
    [ ! -e "$dest.rejected" ] || return 1
    [ ! -d "$dest" ] || return 0
    archive="$WORK/dl/$name-$version.crate"
    curl -q -fsSL --proto '=https' --tlsv1.2 --retry 2 -o "$archive" \
        "https://static.crates.io/crates/$name/$name-$version.crate" || die "download failed: $name $version"
    actual=$(shasum -a 256 "$archive" | awk '{ print $1 }') || die "cannot hash $archive"
    if [ "$actual" != "$sum" ]; then
        flag "$name $version: SHA-256 $actual, Cargo.lock says $sum"
        return 1
    fi
    # The helper checks every member before it writes anything and then
    # writes the files itself: no links, devices, absolute or ".." names.
    if ! py extract-crate "$archive" "$dest" "$name-$version" >/dev/null 2>"$WORK/meta/$name-$version.rejected"; then
        flag "$name $version: archive rejected ($WORK/meta/$name-$version.rejected)"
        if [ -e "$dest" ]; then
            mv "$dest" "$dest.rejected" || die "cannot move $dest aside"
        else
            mkdir "$dest.rejected" || die "cannot mark $dest as rejected"
        fi
        return 1
    fi
    py check-tree "$dest" crate 2>"$WORK/meta/$name-$version.links" ||
        die "$name $version: extracted tree has links or special files ($WORK/meta/$name-$version.links)"
    [ -d "$CRATE_DIR" ] || die "$name $version: archive has no $name-$version/ directory"
}

# report LABEL FILE: print the helper's unexpected and reviewed findings and
# add the unexpected ones to the total.
report() {
    local label=$1 file=$2 level message reason
    while IFS=$'\t' read -r level message reason; do
        case "$level" in
            UNEXPECTED) say "  UNEXPECTED: $label: $message" ;;
            REVIEWED) say "  reviewed: $label: $message [$reason]" ;;
        esac
    done <"$file"
    UNEXPECTED=$((UNEXPECTED + $(count UNEXPECTED "$file")))
}

# count LEVEL FILE: number of findings of one level.
count() { grep -c "^$1"$'\t' "$2" || true; }

# compare_tree LABEL CRATE_ROOT UPSTREAM_DIR REPO_ROOT REPORT_BASE CRATE_NAME
compare_tree() {
    local label=$1 crate=$2 up=$3 repo=$4 out=$5 name=$6 rc=0 reviewed
    [ -d "$up" ] || die "$label: upstream directory missing: $up"
    set -- -x Cargo.toml -x Cargo.toml.orig -x Cargo.lock -x .cargo_vcs_info.json -x .git
    diff -rq "$@" "$crate" "$up" >"$out.list" 2>&1 || rc=$?
    [ "$rc" -le 1 ] || die "$label: diff failed, see $out.list"
    diff -ru "$@" "$crate" "$up" >"$out.diff" 2>&1 || true
    rc=0
    py classify-diff "$out.list" "$crate" "$up" "$repo" "$name" "$REVIEWED" "$REVIEWED_USED" \
        >"$out.classified" || rc=$?
    [ "$rc" -le 1 ] || die "$label: could not classify $out.list"
    report "$label" "$out.classified"
    reviewed=$(count REVIEWED "$out.classified")
    if [ "$rc" -eq 0 ] && [ "$reviewed" -eq 0 ]; then
        say "  $label: sources identical ($(count INFO "$out.classified") notes)"
    elif [ "$rc" -eq 0 ]; then
        say "  $label: sources identical apart from $reviewed reviewed difference(s) ($(count INFO "$out.classified") notes)"
    fi
}

# compare_manifest_plain NAME CRATE_ROOT UPSTREAM_DIR REPORT_BASE: the
# published manifest of a crate that is not a Zed snapshot against upstream.
compare_manifest_plain() {
    local name=$1 crate=$2 up=$3 out=$4 rc=0
    py manifest-plain "$crate" "$up" "$name" "$REVIEWED" "$REVIEWED_USED" >"$out.manifest" || rc=$?
    [ "$rc" -le 1 ] || die "$name: manifest comparison failed"
    report "$name manifest" "$out.manifest"
    say "  $name manifest: $(count REVIEWED "$out.manifest") reviewed difference(s), $(count INFO "$out.manifest") notes (text diff: $out.Cargo.toml.orig.diff)"
}

say "verify-gpui-pre: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
say "FerrisPass HEAD $(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo '?'), Cargo.lock sha256 $(shasum -a 256 "$ROOT/Cargo.lock" | awk '{ print $1 }')"
say "work directory: $WORK"

cargo metadata --locked --format-version 1 --manifest-path "$ROOT/Cargo.toml" \
    >"$WORK/meta/metadata.json" 2>"$WORK/meta/metadata.err" ||
    die "cargo metadata --locked failed, see $WORK/meta/metadata.err"
py crates "$WORK/meta/metadata.json" "$ROOT/Cargo.lock" "$KIT_CRATES" >"$WORK/meta/crates.tsv"

say ""
say "== feature graph"
rc=0
py features "$WORK/meta/metadata.json" "$FORBIDDEN_FEATURES" "$FORBIDDEN_CRATES" >"$WORK/meta/forbidden.txt" || rc=$?
[ "$rc" -le 1 ] || die "cannot read the resolved feature graph"
if [ "$rc" -eq 0 ]; then
    say "  none of $FORBIDDEN_FEATURES is on; none of $FORBIDDEN_CRATES is in the graph"
else
    while IFS= read -r line; do flag "$line"; done <"$WORK/meta/forbidden.txt"
fi

# ---------------------------------------------------------------------------
# 1 and 2: gpui-pre crates against Zed
# ---------------------------------------------------------------------------

say ""
say "== gpui-pre crates against zed@$EXPECTED_ZED_REV"
: >"$WORK/meta/zed-crates.tsv"
: >"$WORK/meta/other-crates.tsv"
while IFS=$'\t' read -r kind name version sum <&3; do
    [ "$kind" = pre ] || continue
    fetch_crate "$name" "$version" "$sum" || continue
    prov=$(py provenance "$CRATE_DIR")
    IFS=$'\t' read -r zrev zcrate vsha vpath vdirty desc <<<"$prov"
    if [ "$zrev" != "-" ]; then
        if [ "$zrev" != "$EXPECTED_ZED_REV" ]; then
            flag "$name $version snapshots zed@$zrev, expected $EXPECTED_ZED_REV"
            continue
        fi
        [ "$vsha" = "-" ] || say "  note: $name $version also carries .cargo_vcs_info.json ($vsha, $vpath)"
        printf '%s\t%s\t%s\t%s\n' "$name" "$version" "$CRATE_DIR" "$zcrate" >>"$WORK/meta/zed-crates.tsv"
    else
        other=$(printf '%s\n' "$OTHER_PRE_CRATES" | awk -v n="$name" -v v="$version" '$1 == n && $2 == v { print $3 "\t" $4 }')
        if [ -z "$other" ]; then
            flag "$name $version names no Zed commit and is not in OTHER_PRE_CRATES (description: $desc)"
            continue
        fi
        printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$name" "$version" "$CRATE_DIR" "$other" "$vsha" "$vpath" "$vdirty" \
            >>"$WORK/meta/other-crates.tsv"
    fi
done 3<"$WORK/meta/crates.tsv"

: >"$WORK/meta/subst.tsv"
if [ -s "$WORK/meta/zed-crates.tsv" ]; then
    say "  $(wc -l <"$WORK/meta/zed-crates.tsv" | tr -d ' ') crates name zed@$EXPECTED_ZED_REV, checksums match Cargo.lock"
    status=$(github_compare "$ZED_GITHUB" "$ZED_BRANCH" "$EXPECTED_ZED_REV") ||
        die "cannot ask GitHub whether $EXPECTED_ZED_REV is on zed $ZED_BRANCH (set GITHUB_TOKEN if rate-limited)"
    case "$status" in
        behind* | identical*) say "  zed@${EXPECTED_ZED_REV:0:7} is on $ZED_BRANCH (status ahead behind: $(echo "$status" | tr '\t' ' '))" ;;
        *) flag "zed@$EXPECTED_ZED_REV is not on $ZED_BRANCH (status ahead behind: $(echo "$status" | tr '\t' ' '))" ;;
    esac

    ZED="$WORK/git/zed"
    git_fetch "$ZED" "$ZED_REPO" "$EXPECTED_ZED_REV"
    gitc -C "$ZED" show "FETCH_HEAD:Cargo.toml" >"$WORK/meta/zed-workspace.toml"
    py zed-paths "$WORK/meta/zed-workspace.toml" "$WORK/meta/zed-crates.tsv" >"$WORK/meta/zed-paths.tsv"
    # Paths are validated by the helper: no spaces, no "..", relative.
    # shellcheck disable=SC2046
    git_checkout "$ZED" $(cut -f2 "$WORK/meta/zed-paths.tsv")
    awk -F '\t' '{ print $4 "\t" $1 }' "$WORK/meta/zed-crates.tsv" >"$WORK/meta/mapping.tsv"

    while IFS=$'\t' read -r name version dir zcrate <&3; do
        zpath=$(awk -F '\t' -v c="$zcrate" '$1 == c { print $2; exit }' "$WORK/meta/zed-paths.tsv")
        out="$WORK/report/pre/$name"
        compare_tree "$name $version vs zed/$zpath" "$dir" "$ZED/$zpath" "$ZED" "$out" "$name"
        diff -u "$ZED/$zpath/Cargo.toml" "$dir/Cargo.toml.orig" >"$out.Cargo.toml.orig.diff" 2>&1 || true
        rc=0
        py manifest "$dir" "$WORK/meta/zed-workspace.toml" "$ZED/$zpath" "$WORK/meta/mapping.tsv" \
            "$WORK/meta/subst.tsv" "$ROOT/Cargo.lock" "$name" "$REVIEWED" "$REVIEWED_USED" >"$out.manifest" || rc=$?
        [ "$rc" -le 1 ] || die "$name: manifest comparison failed"
        report "$name manifest" "$out.manifest"
        say "  $name manifest: $(count ALLOWED "$out.manifest") allowed, $(count RELAXED "$out.manifest") exact pins relaxed (Cargo.lock holds Zed's version), $(count SUBST "$out.manifest") git dependencies substituted, $(count REVIEWED "$out.manifest") reviewed, $(count INFO "$out.manifest") notes"
    done 3<"$WORK/meta/zed-crates.tsv"
fi

# gpui-pre crates that are not Zed snapshots, against their reviewed commit.
while IFS=$'\t' read -r name version dir repo rev vsha vpath vdirty <&3; do
    say ""
    say "== $name $version against $repo@$rev"
    if [ "$vsha" != "$rev" ]; then
        flag "$name $version was published from ${vsha}, the reviewed commit is $rev"
        continue
    fi
    [ "$vdirty" != true ] || say "  note: published from a dirty tree (expected for a renamed package)"
    where=$(reachable "$repo" "$rev") || {
        flag "$name: $rev is neither a branch or tag head nor on the default branch of $repo"
        where="unconfirmed"
    }
    say "  $rev: $where"
    dest="$WORK/git/other-$name"
    git_fetch "$dest" "$repo" "$rev"
    if [ "$vpath" = "." ]; then
        updir=$dest
        git_checkout "$dest"
    else
        updir="$dest/$vpath"
        git_checkout "$dest" "$vpath"
    fi
    out="$WORK/report/pre/$name"
    compare_tree "$name $version vs ${repo#https://github.com/}/$vpath" "$dir" "$updir" "$dest" "$out" "$name"
    diff -u "$updir/Cargo.toml" "$dir/Cargo.toml.orig" >"$out.Cargo.toml.orig.diff" 2>&1 || true
    compare_manifest_plain "$name" "$dir" "$updir" "$out"
done 3<"$WORK/meta/other-crates.tsv"

# Git dependencies of Zed that the published manifests replaced with the
# crates.io release Zed names next to them.
if [ -s "$WORK/meta/subst.tsv" ]; then
    say ""
    say "== crates.io releases that replace Zed git dependencies"
    sort -u "$WORK/meta/subst.tsv" >"$WORK/meta/subst-unique.tsv"
    while IFS=$'\t' read -r name version repo rev <&3; do
        token_ok=$(printf '%s\n%s\n' "$name" "$version" | awk '!/^[A-Za-z0-9_.+-]+$/ { bad = 1 } END { print bad ? "no" : "yes" }')
        [ "$token_ok" = yes ] || die "malformed substitution: $name $version"
        case "$repo" in https://github.com/*) ;; *) die "substitution $name points to $repo, not GitHub" ;; esac
        sum=$(py lock-checksum "$ROOT/Cargo.lock" "$name" "$version")
        if [ "$sum" = "-" ]; then
            say "  $name $version (for $repo@${rev:0:7}): not in Cargo.lock, never built"
            continue
        fi
        fetch_crate "$name" "$version" "$sum" || continue
        prov=$(py provenance "$CRATE_DIR")
        IFS=$'\t' read -r _ _ vsha vpath _ _ <<<"$prov"
        where=$(reachable "$repo" "$rev") || {
            flag "$name: Zed's pin $rev is neither a branch or tag head nor on the default branch of $repo"
            where="unconfirmed"
        }
        say "  $name $version: Zed pins $repo@$rev ($where)"
        if [ "$vsha" != "$rev" ]; then
            say "  note: $name $version was published from ${vsha}, not from Zed's pin; the sources below decide"
        fi
        [ "$vpath" != "-" ] || vpath="."
        dest="$WORK/git/subst-$name"
        git_fetch "$dest" "$repo" "$rev"
        if [ "$vpath" = "." ]; then
            updir=$dest
            git_checkout "$dest"
        else
            updir="$dest/$vpath"
            git_checkout "$dest" "$vpath"
        fi
        out="$WORK/report/subst/$name"
        compare_tree "$name $version vs ${repo#https://github.com/}@${rev:0:7}" "$CRATE_DIR" "$updir" "$dest" "$out" "$name"
        diff -u "$updir/Cargo.toml" "$CRATE_DIR/Cargo.toml.orig" >"$out.Cargo.toml.orig.diff" 2>&1 || true
        compare_manifest_plain "$name" "$CRATE_DIR" "$updir" "$out"
    done 3<"$WORK/meta/subst-unique.tsv"
fi

# ---------------------------------------------------------------------------
# 3: GPUI Kit crates against their release tag, plus the scan
# ---------------------------------------------------------------------------

say ""
say "== GPUI Kit crates against $KIT_REPO tags"
awk -F '\t' '$1 == "kit" { print $2 "\t" $3 "\t" $4 }' "$WORK/meta/crates.tsv" >"$WORK/meta/kit.tsv"
for spec in $ALSO; do
    name=${spec%@*}
    version=${spec#*@}
    printf '%s\n%s\n' "$name" "$version" | awk '!/^[A-Za-z0-9_.+-]+$/ { exit 1 }' || die "malformed --also: $spec"
    case " $KIT_CRATES " in *" $name "*) ;; *) die "--also takes GPUI Kit crates only: $name" ;; esac
    lower=$(printf '%s' "$name" | tr '[:upper:]' '[:lower:]')
    case ${#lower} in
        1) ipath="1/$lower" ;;
        2) ipath="2/$lower" ;;
        3) ipath="3/${lower:0:1}/$lower" ;;
        *) ipath="${lower:0:2}/${lower:2:2}/$lower" ;;
    esac
    curl -q -fsSL --proto '=https' --tlsv1.2 -o "$WORK/meta/index-$name" "https://index.crates.io/$ipath" ||
        die "cannot read the crates.io index for $name"
    entry=$(py index-checksum "$WORK/meta/index-$name" "$version")
    IFS=$'\t' read -r sum state <<<"$entry"
    say "  --also $name $version: not in Cargo.lock, checksum from the crates.io index ($state)"
    printf '%s\t%s\t%s\n' "$name" "$version" "$sum" >>"$WORK/meta/kit.tsv"
done

: >"$WORK/meta/kit-crates.tsv"
while IFS=$'\t' read -r name version sum <&3; do
    fetch_crate "$name" "$version" "$sum" || continue
    prov=$(py provenance "$CRATE_DIR")
    IFS=$'\t' read -r _ _ vsha vpath _ _ <<<"$prov"
    if [ "$vsha" = "-" ] || [ "$vpath" = "." ] || [ "$vpath" = "-" ]; then
        flag "$name $version has no usable .cargo_vcs_info.json"
        continue
    fi
    tag=$(gitc ls-remote "$KIT_REPO" "refs/tags/v$version" "refs/tags/v$version^{}" | awk '
        $2 ~ /\^\{\}$/ { peeled = $1 } $2 !~ /\^\{\}$/ { plain = $1 }
        END { print (peeled != "" ? peeled : plain) }')
    if [ -z "$tag" ]; then
        flag "$name $version: no tag v$version in $KIT_REPO"
        continue
    fi
    if [ "$tag" != "$vsha" ]; then
        flag "$name $version was published from $vsha, tag v$version is $tag"
        continue
    fi
    if [ "$vsha" != "$EXPECTED_KIT_REV" ]; then
        flag "$name $version was published from $vsha, EXPECTED_KIT_REV is $EXPECTED_KIT_REV"
        continue
    fi
    printf '%s\t%s\t%s\t%s\t%s\n' "$name" "$version" "$CRATE_DIR" "$vsha" "$vpath" >>"$WORK/meta/kit-crates.tsv"
done 3<"$WORK/meta/kit.tsv"

cut -f4 "$WORK/meta/kit-crates.tsv" | sort -u >"$WORK/meta/kit-revs.txt"
while IFS= read -r rev <&3; do
    dest="$WORK/git/kit-$rev"
    git_fetch "$dest" "$KIT_REPO" "$rev"
    # Paths come from validated path_in_vcs values: no spaces, no "..".
    # shellcheck disable=SC2046
    git_checkout "$dest" $(awk -F '\t' -v r="$rev" '$4 == r { print $5 }' "$WORK/meta/kit-crates.tsv" | sort -u)
    say "  checkout of $KIT_REPO at $rev (the tag of the crate version)"
done 3<"$WORK/meta/kit-revs.txt"

# Each GPUI Kit crate maps to itself in the manifest comparison.
cut -f1 "$WORK/meta/kit-crates.tsv" | awk '{ print $1 "\t" $1 }' >"$WORK/meta/kit-mapping.tsv"

while IFS=$'\t' read -r name version dir rev vpath <&3; do
    dest="$WORK/git/kit-$rev"
    out="$WORK/report/kit/$name"
    compare_tree "$name $version vs gpui-kit/$vpath" "$dir" "$dest/$vpath" "$dest" "$out" "$name"
    if cmp -s "$dir/Cargo.toml.orig" "$dest/$vpath/Cargo.toml"; then
        say "  $name manifest: Cargo.toml.orig identical to the tag"
    else
        diff -u "$dest/$vpath/Cargo.toml" "$dir/Cargo.toml.orig" >"$out.Cargo.toml.orig.diff" 2>&1 || true
        flag "$name manifest: Cargo.toml.orig differs from the tag ($out.Cargo.toml.orig.diff)"
    fi
    # Cargo builds with the normalized Cargo.toml, not with Cargo.toml.orig,
    # so that one is compared too, after the workspace inheritance of the tag.
    rc=0
    py manifest "$dir" "$dest/Cargo.toml" "$dest/$vpath" "$WORK/meta/kit-mapping.tsv" \
        /dev/null "$ROOT/Cargo.lock" "$name" "$REVIEWED" "$REVIEWED_USED" upstream >"$out.manifest" || rc=$?
    [ "$rc" -le 1 ] || die "$name: manifest comparison failed"
    report "$name manifest" "$out.manifest"
    say "  $name normalized manifest: $(count ALLOWED "$out.manifest") allowed, $(count RELAXED "$out.manifest") exact pins relaxed, $(count REVIEWED "$out.manifest") reviewed, $(count INFO "$out.manifest") notes"
    scan=$(py scan "$dir" "$out.scan")
    say "  $name scan: $scan (details: $out.scan)"
done 3<"$WORK/meta/kit-crates.tsv"

# A reviewed finding that did not show up any more is stale: the patch went
# away or changed (then it is reported above as unexpected).
awk -F ' [|] ' '/^[[:space:]]*#/ { next } { sub(/^[[:space:]]+/, "", $1); print $1 " | " $2 }' "$REVIEWED" |
    sort -u >"$WORK/meta/reviewed-keys.txt"
sort -u "$REVIEWED_USED" >"$WORK/meta/reviewed-used-keys.txt"
comm -23 "$WORK/meta/reviewed-keys.txt" "$WORK/meta/reviewed-used-keys.txt" >"$WORK/meta/reviewed-stale.txt"
if [ -s "$WORK/meta/reviewed-stale.txt" ]; then
    say ""
    say "== stale REVIEWED_FINDINGS lines (no longer seen, delete them)"
    while IFS= read -r line; do flag "stale reviewed finding: $line"; done <"$WORK/meta/reviewed-stale.txt"
fi

say ""
if [ "$UNEXPECTED" -eq 0 ]; then
    say "RESULT: no unexpected difference"
else
    say "RESULT: $UNEXPECTED unexpected difference(s), see above and $WORK/report"
fi
[ "$KEEP" -eq 0 ] || say "report kept in $WORK/report"
[ "$UNEXPECTED" -eq 0 ] || exit 1
