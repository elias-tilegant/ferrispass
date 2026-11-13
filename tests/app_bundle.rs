//! The app bundle: where the icon comes from, how `scripts/build-mac.sh`
//! compiles it, the release job that runs the script, and the check that
//! keeps a binary out of the bundle that the build did not produce.
//!
//! The build itself needs Xcode, a signing identity and several minutes, so
//! the script and the workflow are read as text here, the way
//! `tests/architecture.rs` reads the other workflows. The bundle check is
//! the exception: it runs for real, with `sh`, against a release folder with
//! dep-info files like cargo's and a source tree, made up in a temporary
//! folder.

#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

const BUILD_SCRIPT: &str = "scripts/build-mac.sh";
const BUNDLE_CHECK: &str = "scripts/check-bundle-binaries.sh";
const ICON_DOCUMENT: &str = "bundle/icon/AppIcon.icon";
const LAYER_DRAWINGS: &str = "design/brand/icon";
const INFO_PLIST: &str = "bundle/macos/Info.plist";
const RELEASE_WORKFLOW: &str = ".github/workflows/release.yml";

/// What the bundle check says when a copy is not the binary in the release
/// folder or the dep-info of that binary names no file of the source tree,
/// and when a file compiled into the binary is newer than it.
const FOREIGN: &str = "the bundle does not carry the binary this build produced; \
                       CARGO_TARGET_DIR points somewhere else";
const STALE: &str = "a source file is newer than the binary; the build did not rebuild it";
/// What it says when a file compiled into the binary has been deleted since.
const GONE: &str = "a source file the binary was built from no longer exists; \
                    the build did not rebuild it";

/// What the bundle check says when cargo left no dep-info next to `binary`.
fn no_dep_info(binary: &str) -> String {
    format!("cargo wrote no dep-info for {binary}; the build did not come from cargo")
}

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &str) -> String {
    fs::read_to_string(root().join(path)).unwrap_or_else(|err| panic!("{path}: {err}"))
}

/// The code of a shell script, one command per entry: trimmed, comments and
/// blank lines left out, and a line ending in `\` joined to the next.
fn code_lines(script: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for line in script.lines() {
        let line = line.trim();
        if current.is_empty() && (line.is_empty() || line.starts_with('#')) {
            continue;
        }
        match line.strip_suffix('\\') {
            Some(head) => {
                current.push_str(head.trim_end());
                current.push(' ');
            }
            None => {
                current.push_str(line);
                lines.push(std::mem::take(&mut current));
            }
        }
    }
    lines
}

/// Where `line` is in the build script, which must have it exactly once.
fn line_of(lines: &[String], line: &str) -> usize {
    let found: Vec<usize> = (0..lines.len()).filter(|&i| lines[i] == line).collect();
    match found[..] {
        [at] => at,
        [] => panic!("{BUILD_SCRIPT} has no line `{line}`"),
        _ => panic!("{BUILD_SCRIPT} has `{line}` {} times", found.len()),
    }
}

/// Where the one line starting with `start` is in the build script.
fn line_starting(lines: &[String], start: &str) -> usize {
    let found: Vec<usize> = (0..lines.len())
        .filter(|&i| lines[i].starts_with(start))
        .collect();
    match found[..] {
        [at] => at,
        [] => panic!("{BUILD_SCRIPT} has no line starting `{start}`"),
        _ => panic!(
            "{BUILD_SCRIPT} has {} lines starting `{start}`",
            found.len()
        ),
    }
}

/// Requires that the `if` at `at` ends the script with `exit 1` before its
/// `fi`.
fn exits(lines: &[String], at: usize) {
    let body: Vec<&String> = lines[at + 1..].iter().take_while(|l| *l != "fi").collect();
    assert!(
        body.iter().any(|l| *l == "exit 1"),
        "`{}` in {BUILD_SCRIPT} does not stop the build with `exit 1`",
        lines[at]
    );
}

/// The string value after `<key>{key}</key>` in a property list.
fn plist_string(plist: &str, key: &str) -> Option<String> {
    let mut lines = plist.lines().map(str::trim);
    let key = format!("<key>{key}</key>");
    lines.find(|line| *line == key)?;
    let value = lines
        .next()?
        .strip_prefix("<string>")?
        .strip_suffix("</string>")?;
    Some(value.to_string())
}

#[test]
fn the_icon_source_is_the_composer_document() {
    assert!(
        !root().join("bundle/icon.png").exists(),
        "bundle/icon.png is back; the app icon is {ICON_DOCUMENT}"
    );
    let document: serde_json::Value =
        serde_json::from_str(&read(&format!("{ICON_DOCUMENT}/icon.json")))
            .expect("icon.json is JSON");

    // The layers from top to bottom, one group each. The final drawing may
    // replace what they hold, not how many there are or what they are.
    let layers: Vec<&str> = document["groups"]
        .as_array()
        .expect("icon.json has groups")
        .iter()
        .map(|group| {
            let layers = group["layers"].as_array().expect("a group has layers");
            assert_eq!(layers.len(), 1, "one layer per group: {group}");
            layers[0]["image-name"]
                .as_str()
                .expect("a layer names its image")
        })
        .collect();
    assert_eq!(layers, ["key.svg", "face.svg", "body.svg"]);

    // The document draws the drawings kept in design/brand/icon, and nothing
    // else.
    let mut assets: Vec<String> = fs::read_dir(root().join(ICON_DOCUMENT).join("Assets"))
        .expect("the icon document has Assets/")
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assets.sort();
    assert_eq!(assets, ["body.svg", "face.svg", "key.svg"]);
    for layer in layers {
        assert_eq!(
            read(&format!("{ICON_DOCUMENT}/Assets/{layer}")),
            read(&format!("{LAYER_DRAWINGS}/{layer}")),
            "{ICON_DOCUMENT}/Assets/{layer} differs from {LAYER_DRAWINGS}/{layer}"
        );
    }

    // A fill of its own for the dark appearance; without one macOS makes
    // the background a neutral grey there.
    let fills = document["fill-specializations"]
        .as_array()
        .expect("icon.json has fill-specializations");
    assert!(
        fills.iter().any(|fill| fill.get("appearance").is_none()),
        "icon.json has no default fill"
    );
    assert!(
        fills.iter().any(|fill| fill["appearance"] == "dark"),
        "icon.json has no fill for the dark appearance"
    );
}

#[test]
fn info_plist_names_the_icon() {
    let plist = read(INFO_PLIST);
    let name = plist_string(&plist, "CFBundleIconName").unwrap_or_else(|| {
        panic!("{INFO_PLIST} has no CFBundleIconName; macOS 26 then ignores Assets.car")
    });
    assert_eq!(name, "AppIcon");
    assert!(
        root().join(format!("bundle/icon/{name}.icon")).is_dir(),
        "CFBundleIconName is {name}, but there is no bundle/icon/{name}.icon"
    );
    let lines = code_lines(&read(BUILD_SCRIPT));
    let compile = line_starting(&lines, "xcrun actool ");
    assert!(
        lines[compile].contains(&format!(" --app-icon {name} ")),
        "actool must compile the icon named {name}: {}",
        lines[compile]
    );
    // The fallback for older systems, if it is named at all, names the .icns
    // that actool writes next to Assets.car.
    if let Some(file) = plist_string(&plist, "CFBundleIconFile") {
        assert_eq!(file, name, "CFBundleIconFile");
    }
}

#[test]
fn the_build_script_guards_actool_and_the_car() {
    let text = read(BUILD_SCRIPT);
    for gone in ["icon.png", "ICON_PNG", "sips", "iconutil", "iconset"] {
        assert!(
            !text.contains(gone),
            "{BUILD_SCRIPT} still mentions `{gone}`"
        );
    }
    let lines = code_lines(&text);

    line_of(
        &lines,
        r#"ICON_SRC="${PROJECT_ROOT}/bundle/icon/AppIcon.icon""#,
    );
    exits(
        &lines,
        line_of(&lines, r#"if [ ! -d "${ICON_SRC}" ]; then"#),
    );

    // An actool whose version cannot be read counts as too old, and one
    // older than 26 stops the build before it compiles anything.
    let unreadable = line_of(&lines, r#"''|*[!0-9]*) ACTOOL_MAJOR=0 ;;"#);
    let guard = line_of(&lines, r#"if [ "${ACTOOL_MAJOR}" -lt 26 ]; then"#);
    exits(&lines, guard);
    let compile = line_starting(
        &lines,
        r#"xcrun actool "${ICON_SRC}" --compile "${ICON_OUT}" "#,
    );
    assert!(
        unreadable < guard && guard < compile,
        "the version guard comes before actool"
    );
    for option in [
        "--app-icon AppIcon",
        "--include-all-app-icons",
        "--platform macosx",
        "--target-device mac",
        r#"--minimum-deployment-target "${MIN_MACOS}""#,
        "--output-partial-info-plist",
    ] {
        assert!(
            lines[compile].contains(option),
            "actool runs without `{option}`: {}",
            lines[compile]
        );
    }

    // actool can end with 0 without writing anything.
    let car = line_of(&lines, r#"if ! test -s "${ICON_OUT}/Assets.car"; then"#);
    exits(&lines, car);

    // Into the bundle before it is signed, so the seal covers it, and the
    // temporary folder gone.
    let copy = line_of(
        &lines,
        r#"cp "${ICON_OUT}/Assets.car" "${APP_BUNDLE}/Contents/Resources/Assets.car""#,
    );
    let cleanup = lines
        .iter()
        .rposition(|l| l == r#"rm -rf "${ICON_OUT}""#)
        .expect("the temporary icon folder is removed");
    let appearances = line_starting(&lines, r#"if ! jq -e "${ICON_APPEARANCES} | length > 1" "#);
    exits(&lines, appearances);
    let signing = lines
        .iter()
        .position(|l| l.starts_with("codesign --force"))
        .expect("the build script signs the bundle");
    assert!(
        compile < car && car < copy && copy < cleanup && copy < appearances,
        "Assets.car is checked, then copied, then the temporary folder removed"
    );
    assert!(
        cleanup < signing && appearances < signing,
        "the icon is in the bundle and checked before the bundle is signed"
    );
}

#[test]
fn the_build_script_calls_the_bundle_check() {
    let lines = code_lines(&read(BUILD_SCRIPT));
    // The check looks for the two binaries by these names, and the bundle
    // takes them from these copies.
    line_of(&lines, r#"BINARY_NAME="ferrispass""#);
    line_of(&lines, r#"CLI_BINARY_NAME="ferrispass-cli""#);
    line_of(&lines, r#"BIN_PATH="${DIST_DIR}/${BINARY_NAME}""#);
    line_of(&lines, r#"CLI_BIN_PATH="${DIST_DIR}/${CLI_BINARY_NAME}""#);

    let build = line_of(
        &lines,
        "cargo build --release --target aarch64-apple-darwin",
    );
    let copy = line_of(
        &lines,
        r#"cp "${RELEASE_DIR}/${BINARY_NAME}" "${BIN_PATH}""#,
    );
    assert!(build < copy);
    assert_eq!(
        lines[copy + 1],
        r#"cp "${RELEASE_DIR}/${CLI_BINARY_NAME}" "${CLI_BIN_PATH}""#
    );
    assert_eq!(
        lines[copy + 2],
        r#"sh "${PROJECT_ROOT}/scripts/check-bundle-binaries.sh" "${RELEASE_DIR}" "${DIST_DIR}" "${PROJECT_ROOT}""#,
        "the bundle check has to run right after the two copies, and its result has to count"
    );
    let into_bundle = line_of(
        &lines,
        r#"mv "${BIN_PATH}" "${APP_BUNDLE}/Contents/MacOS/${BINARY_NAME}""#,
    );
    let cli_into_bundle = line_of(
        &lines,
        r#"cp "${CLI_BIN_PATH}" "${APP_BUNDLE}/Contents/MacOS/${CLI_BINARY_NAME}""#,
    );
    assert!(copy + 2 < into_bundle && copy + 2 < cli_into_bundle);
}

/// The lines of the job in `workflow` that builds the release, the one that
/// runs `scripts/build-mac.sh` without `--sign-update`, trimmed.
fn build_job(workflow: &str) -> Vec<&str> {
    let mut jobs: Vec<Vec<&str>> = Vec::new();
    let mut in_jobs = false;
    for line in workflow.lines() {
        if !line.starts_with(' ') && !line.is_empty() {
            in_jobs = line == "jobs:";
            continue;
        }
        if !in_jobs {
            continue;
        }
        let is_job = line.starts_with("  ")
            && !line.starts_with("   ")
            && !line.trim_start().starts_with('#')
            && line.trim_end().ends_with(':');
        if is_job {
            jobs.push(Vec::new());
        } else if let Some(job) = jobs.last_mut() {
            job.push(line.trim());
        }
    }
    let mut builds = jobs
        .into_iter()
        .filter(|job| job.contains(&"run: scripts/build-mac.sh"));
    let job = builds
        .next()
        .unwrap_or_else(|| panic!("no job in {RELEASE_WORKFLOW} runs scripts/build-mac.sh"));
    assert!(
        builds.next().is_none(),
        "more than one job in {RELEASE_WORKFLOW} builds the release"
    );
    job
}

#[test]
fn the_release_workflow_checks_the_icon_folder_and_pins_xcode() {
    let workflow = read(RELEASE_WORKFLOW);
    assert!(
        !workflow.contains("icon.png"),
        "{RELEASE_WORKFLOW} still looks for bundle/icon.png"
    );
    let job = build_job(&workflow);

    // The macOS 26 image, not macos-latest: each image carries one major
    // version of Xcode.
    assert!(
        job.contains(&"runs-on: macos-26"),
        "the release build does not run on the macos-26 image"
    );
    // One fixed Xcode 26 release, selected before the build.
    let pin = regex::Regex::new(r"^sudo xcode-select -s /Applications/Xcode_26\.\d+(\.\d+)?\.app$")
        .unwrap();
    let selects: Vec<usize> = (0..job.len())
        .filter(|&i| job[i].contains("xcode-select"))
        .collect();
    let [select] = selects[..] else {
        panic!("the release build selects Xcode {} times", selects.len());
    };
    assert!(
        pin.is_match(job[select]),
        "the release build selects no fixed Xcode 26 release: {}",
        job[select]
    );

    let icon = job
        .iter()
        .position(|l| *l == "if [ ! -d bundle/icon/AppIcon.icon ]; then")
        .expect("the release build checks that bundle/icon/AppIcon.icon is there");
    assert!(
        job[icon + 1..]
            .iter()
            .take_while(|l| **l != "fi")
            .any(|l| *l == "exit 1"),
        "a missing icon folder does not stop the release build"
    );
    let build = job
        .iter()
        .position(|l| *l == "run: scripts/build-mac.sh")
        .unwrap();
    assert!(select < build && icon < build);
}

/// The two binaries the bundle carries.
const BINARIES: [&str; 2] = ["ferrispass", "ferrispass-cli"];

/// The source tree. A checkout may have a space in its path, and the
/// dep-info then escapes it.
const SOURCE: &str = "source tree";

/// What cargo compiled into both binaries, as their dep-info names it: Rust
/// sources, and files they take in with `include_str!` or `include_bytes!`,
/// also outside `src/`.
const COMPILED: [&str; 5] = [
    "src/lib.rs",
    "src/app/mod.rs",
    "locales/app.yml",
    "assets/fonts/Mono Regular.ttf",
    "bundle/minisign-pub.txt",
];

/// A file compiled into one binary only, deleted after that binary was built.
const REMOVED: &str = "src/app/removed.rs";

/// Files in the source tree that go into no binary.
const NOT_COMPILED: [&str; 3] = [
    "src/app/kinds/tests.rs",
    "assets/licenses/OFL.txt",
    "docs/notes.md",
];

/// `file` in the source tree.
fn source(file: &str) -> String {
    format!("{SOURCE}/{file}")
}

/// A point in time, `seconds` after an arbitrary start.
fn time(seconds: u64) -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000 + seconds)
}

/// A finished cargo build in a temporary folder: `release/` with the two
/// binaries and their dep-info, `copies/` with what was copied out of it,
/// and the source tree they were built from.
struct Build {
    dir: tempfile::TempDir,
}

impl Build {
    /// Every source last changed at 100, both binaries and their dep-info
    /// written at 200, and the copies, taken at 300, the same bytes.
    fn new() -> Self {
        let build = Build {
            dir: tempfile::tempdir().expect("temporary folder"),
        };
        for file in COMPILED.iter().chain(&NOT_COMPILED) {
            build.write(&source(file), file, 100);
        }
        for binary in BINARIES {
            let bytes = format!("{binary} as cargo built it");
            build.write(&format!("release/{binary}"), &bytes, 200);
            let dep_info = build.dep_info(binary, SOURCE, &COMPILED);
            build.write(&format!("release/{binary}.d"), &dep_info, 200);
            build.write(&format!("copies/{binary}"), &bytes, 300);
        }
        build
    }

    /// The dep-info cargo writes next to `binary` when it builds it from
    /// `files` in the tree in `tree`: one rule, the binary, a colon and the
    /// files, each by its full path with spaces escaped. Cargo writes it on
    /// one line; here the last two files go on a continued line, which the
    /// format allows as well.
    fn dep_info(&self, binary: &str, tree: &str, files: &[&str]) -> String {
        // Cargo names the files with symbolic links resolved, and the
        // temporary folder can be behind one (/var is /private/var on
        // macOS). The check is handed the path as it is.
        let real = fs::canonicalize(self.dir.path()).expect("temporary folder");
        let escaped = |path: PathBuf| path.to_str().expect("a UTF-8 path").replace(' ', "\\ ");
        let files: Vec<String> = files
            .iter()
            .map(|file| escaped(real.join(tree).join(file)))
            .collect();
        let (first, last) = files.split_at(files.len() - 2);
        format!(
            "{}: {} \\\n    {}\n",
            escaped(real.join("release").join(binary)),
            first.join(" "),
            last.join(" ")
        )
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.dir.path().join(relative)
    }

    fn write(&self, relative: &str, contents: &str, at: u64) {
        let path = self.path(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, contents).unwrap();
        self.touch(relative, at);
    }

    fn touch(&self, relative: &str, at: u64) {
        fs::File::options()
            .write(true)
            .open(self.path(relative))
            .unwrap_or_else(|err| panic!("{relative}: {err}"))
            .set_modified(time(at))
            .unwrap();
    }

    /// Runs the check as the build script does and returns its exit code
    /// and everything it printed.
    fn check(&self) -> (Option<i32>, String) {
        let out = Command::new("sh")
            .arg(root().join(BUNDLE_CHECK))
            .arg(self.path("release"))
            .arg(self.path("copies"))
            .arg(self.path(SOURCE))
            .output()
            .expect("run the bundle check");
        let output = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        (out.status.code(), output)
    }
}

#[test]
fn the_bundle_check_refuses_a_stale_or_foreign_binary() {
    let build = Build::new();
    let (code, output) = build.check();
    assert_eq!(
        code,
        Some(0),
        "the copies are the binaries cargo built:\n{output}"
    );

    // A test module, a license text or the docs may be newer: cargo compiles
    // none of them into the binaries and has nothing to rebuild after a
    // change to them.
    for file in NOT_COMPILED {
        build.touch(&source(file), 400);
    }
    let (code, output) = build.check();
    assert_eq!(
        code,
        Some(0),
        "only the files the dep-info names count:\n{output}"
    );

    for (binary, other) in [
        ("ferrispass", "ferrispass-cli"),
        ("ferrispass-cli", "ferrispass"),
    ] {
        // A file compiled into `binary` that changed after it was built,
        // though not after the other one: cargo would have built `binary`
        // again.
        for file in COMPILED {
            let build = Build::new();
            build.touch(&format!("release/{other}"), 400);
            build.touch(&source(file), 300);
            let (code, output) = build.check();
            assert_eq!(code, Some(1), "{file} is newer than {binary}:\n{output}");
            assert!(output.contains(STALE), "{file}, {binary}:\n{output}");
            assert!(
                output.contains(&format!("{SOURCE}/{file} is newer than "))
                    && output.contains(&format!("release/{binary}\n")),
                "the check names {file} and {binary}:\n{output}"
            );
        }

        // A file compiled into `binary`, though not into the other one, that
        // has been deleted since: cargo would have built `binary` again, or
        // failed.
        let build = Build::new();
        build.write(&source(REMOVED), REMOVED, 100);
        let mut named = COMPILED.to_vec();
        named.push(REMOVED);
        let dep_info = build.dep_info(binary, SOURCE, &named);
        build.write(&format!("release/{binary}.d"), &dep_info, 200);
        fs::remove_file(build.path(&source(REMOVED))).unwrap();
        let (code, output) = build.check();
        assert_eq!(
            code,
            Some(1),
            "{REMOVED} was deleted after {binary} was built:\n{output}"
        );
        assert!(output.contains(GONE), "{binary}:\n{output}");
        assert!(
            output.contains(&format!("release/{binary}.d names "))
                && output.contains(&format!("{SOURCE}/{REMOVED}, which no longer exists")),
            "the check names the dep-info of {binary} and {REMOVED}:\n{output}"
        );

        // No dep-info next to `binary`.
        let build = Build::new();
        fs::remove_file(build.path(&format!("release/{binary}.d"))).unwrap();
        let (code, output) = build.check();
        assert_eq!(code, Some(1), "{binary} has no dep-info:\n{output}");
        assert!(output.contains(&no_dep_info(binary)), "{binary}:\n{output}");

        // A dep-info of a build from another checkout, whose files are all
        // older than `binary`.
        let build = Build::new();
        for file in COMPILED {
            build.write(&format!("other checkout/{file}"), file, 100);
        }
        let elsewhere = build.dep_info(binary, "other checkout", &COMPILED);
        build.write(&format!("release/{binary}.d"), &elsewhere, 200);
        let (code, output) = build.check();
        assert_eq!(
            code,
            Some(1),
            "{binary} was built from another checkout:\n{output}"
        );
        assert!(output.contains(FOREIGN), "{binary}:\n{output}");
        assert!(
            output.contains(&format!("release/{binary}.d names no file under ")),
            "the check names the dep-info of {binary}:\n{output}"
        );

        // A copy with other bytes than the binary in the release folder.
        let build = Build::new();
        build.write(&format!("copies/{binary}"), "an older build", 300);
        let (code, output) = build.check();
        assert_eq!(code, Some(1), "the copy of {binary} is foreign:\n{output}");
        assert!(output.contains(FOREIGN), "{binary}:\n{output}");
        assert!(
            output.contains(&format!("copies/{binary} ")),
            "the check names {binary}:\n{output}"
        );

        // No copy at all.
        let build = Build::new();
        fs::remove_file(build.path(&format!("copies/{binary}"))).unwrap();
        let (code, output) = build.check();
        assert_eq!(code, Some(1), "{binary} was not copied:\n{output}");
        assert!(output.contains(FOREIGN), "{binary}:\n{output}");
    }
}
