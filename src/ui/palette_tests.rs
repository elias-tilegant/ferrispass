//! Tests for the palette shim: what each accessor returns, that `ds::apply`
//! switches it, and that only the old UI reads it.
//!
//! They sit beside `palette.rs` rather than inside it, because inside the
//! deprecated module the test harness itself would use deprecated items.

use std::path::{Path, PathBuf};

use gpui::{Hsla, TestAppContext};

use crate::app::settings::ThemeChoice;
use crate::ui::ds::tests::lock_mode;
use crate::ui::ds::{self, A11yPrefs, Appearance, Mode, tokens::TEXT_SIZE_STANDARD};
use crate::ui::palette;

type Accessor = fn() -> Hsla;

const ACCESSORS: [(&str, Accessor); 22] = [
    ("bg", palette::bg),
    ("panel", palette::panel),
    ("sidebar", palette::sidebar),
    ("border", palette::border),
    ("border_strong", palette::border_strong),
    ("text", palette::text),
    ("text_muted", palette::text_muted),
    ("text_faint", palette::text_faint),
    ("blue", palette::blue),
    ("blue_hover", palette::blue_hover),
    ("blue_soft", palette::blue_soft),
    ("blue_border", palette::blue_border),
    ("orange", palette::orange),
    ("orange_deep", palette::orange_deep),
    ("orange_soft", palette::orange_soft),
    ("orange_border", palette::orange_border),
    ("green", palette::green),
    ("green_soft", palette::green_soft),
    ("green_border", palette::green_border),
    ("red", palette::red),
    ("yellow", palette::yellow),
    ("transparent_overlay", palette::transparent_overlay),
];

/// The rows of the mapping table in `design/README.md`: accessor and token,
/// both in backticks.
fn readme_table() -> Vec<(String, String)> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("design/README.md");
    let readme = std::fs::read_to_string(path).expect("read design/README.md");
    readme
        .lines()
        .filter_map(|line| {
            let cells: Vec<&str> = line.split('|').map(str::trim).collect();
            let accessor = cells.get(1)?.strip_prefix('`')?.strip_suffix("()`")?;
            let token = cells.get(2)?.strip_prefix('`')?.strip_suffix('`')?;
            Some((accessor.to_string(), token.to_string()))
        })
        .collect()
}

/// In every mode, each of the 22 palette accessors returns the token the
/// table in `design/README.md` names, and the table names all 22.
#[test]
fn palette_follows_the_readme_table() {
    let table = readme_table();
    let named: Vec<&str> = table
        .iter()
        .map(|(accessor, _)| accessor.as_str())
        .collect();
    let accessors: Vec<&str> = ACCESSORS.iter().map(|(name, _)| *name).collect();
    assert_eq!(named, accessors, "the table and the accessors differ");

    let _lock = lock_mode();
    for mode in Mode::ALL {
        palette::set_mode(mode);
        assert_eq!(palette::mode(), mode);
        for ((name, accessor), (_, token)) in ACCESSORS.iter().zip(&table) {
            let expected = mode
                .colors()
                .by_name(token)
                .unwrap_or_else(|| panic!("{token} is not a token"));
            assert_eq!(
                accessor(),
                expected.hsla,
                "{name} in {mode:?} is not {token}"
            );
        }
    }
    palette::set_mode(Mode::Light);
}

/// `ds::apply` switches the accessors to the mode it puts into effect.
#[gpui::test]
fn ds_apply_switches_the_palette(cx: &mut TestAppContext) {
    let _lock = lock_mode();
    cx.update(|cx| {
        gpui_component::init(cx);
        let cases = [
            (ThemeChoice::Dark, false, Mode::Dark),
            (ThemeChoice::Light, true, Mode::LightHc),
            (ThemeChoice::Dark, true, Mode::DarkHc),
            (ThemeChoice::Light, false, Mode::Light),
        ];
        for (choice, increase_contrast, expected) in cases {
            let prefs = A11yPrefs {
                increase_contrast,
                reduce_motion: false,
                voiceover: false,
            };
            ds::apply(
                cx,
                Appearance {
                    choice,
                    prefs,
                    text_size: TEXT_SIZE_STANDARD,
                },
            );
            assert_eq!(palette::mode(), expected);
            assert_eq!(palette::bg(), expected.colors().bg_window.hsla);
        }
    });
}

fn rust_files(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            rust_files(&path, found);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            found.push(path);
        }
    }
}

/// Whether the code part of `line` (before any `//`) names `palette` as a
/// word of its own, which is how every path to the module starts.
fn names_palette(line: &str) -> bool {
    let code = line.split("//").next().unwrap_or_default();
    code.match_indices("palette").any(|(at, word)| {
        let is_name = |c: char| c.is_alphanumeric() || c == '_';
        !code[..at].chars().next_back().is_some_and(is_name)
            && !code[at + word.len()..].chars().next().is_some_and(is_name)
    })
}

/// Files that may name the palette anywhere: the old shell, screens and
/// widgets, the module itself, its declaration and these tests.
fn may_name_palette(relative: &str) -> bool {
    relative.starts_with("src/ui/legacy/")
        || relative.starts_with("src/ui/widgets/")
        || [
            "src/ui/palette.rs",
            "src/ui/palette_tests.rs",
            "src/ui/mod.rs",
        ]
        .contains(&relative)
}

/// The one line anywhere else that may name the palette: the switch in
/// `ds::apply`.
const MODE_SWITCH: (&str, &str) = ("src/ui/ds/mod.rs", "crate::ui::palette::set_mode(mode);");

/// The palette shim serves the old UI only; new code reads `cx.ds()`.
/// Outside the files in [`may_name_palette`], the switch in `ds::apply` is
/// the only line that names the module, and it is there exactly once.
#[test]
fn no_palette_outside_legacy() {
    assert!(names_palette("use crate::ui::palette;"));
    assert!(names_palette(".bg(palette::panel())"));
    assert!(names_palette("use crate::ui::{palette, icons};"));
    assert!(!names_palette("palette_index: 0,"));
    assert!(!names_palette("FAVICON_PALETTE_SIZE"));
    assert!(!names_palette("// the old palette"));

    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for dir in ["src", "tests", "examples"] {
        rust_files(&root.join(dir), &mut files);
    }
    assert!(
        files.len() > 50,
        "the scan found only {} files",
        files.len()
    );
    let mut found = Vec::new();
    let mut switches = 0;
    for file in &files {
        let relative = file
            .strip_prefix(root)
            .expect("under the repo")
            .to_string_lossy()
            .replace('\\', "/");
        if may_name_palette(&relative) {
            continue;
        }
        let text = std::fs::read_to_string(file).expect("read source");
        for (number, line) in text.lines().enumerate() {
            if !names_palette(line) {
                continue;
            }
            if (relative.as_str(), line.trim()) == MODE_SWITCH && switches == 0 {
                switches += 1;
            } else {
                found.push(format!("{relative}:{}: {}", number + 1, line.trim()));
            }
        }
    }
    assert!(
        found.is_empty(),
        "the palette shim is for the old screens only, read cx.ds() instead:\n{}",
        found.join("\n")
    );
    assert_eq!(
        switches, 1,
        "ds::apply no longer switches the palette with `{}`",
        MODE_SWITCH.1
    );
}
