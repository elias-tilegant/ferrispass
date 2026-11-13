//! The gallery's tests.
//!
//! `cargo test` builds every example, and runs the tests inside this one
//! because its `[[example]]` entry in `Cargo.toml` says `test = true`. They
//! draw the same view the gallery window shows, headless. They hold that
//! every component module has a page, that every page draws in every mode,
//! at both text sizes and in both languages, that the keys switch them, that
//! the Kit page holds every glyph the app draws again, and, by reading the
//! gallery's source, that it builds no component of its own and reads
//! nothing.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use ferrispass::app::settings::TextSize;
use ferrispass::i18n::Language;
use ferrispass::tr;
use ferrispass::ui::ds::{self, ActiveTokens as _, Mode};
use gpui::{AppContext as _, Entity, KeyContext, Keystroke, TestAppContext, VisualTestContext};
use gpui_component::Root;

use crate::gallery::{self, Gallery, Look};
use crate::pages::{self, Page};

/// This file, which the scans of the gallery's source leave out: it names
/// what they look for.
const THIS_FILE: &str = "tests.rs";

/// The mode of the old screens' colors and the language are one setting
/// for the whole process. The tests that change them hold this.
fn lock() -> MutexGuard<'static, ()> {
    static LOOK: Mutex<()> = Mutex::new(());
    LOOK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The gallery as its window shows it: started as `main` starts it, under
/// the Kit's `Root`.
fn open(cx: &mut TestAppContext) -> (Entity<Gallery>, &mut VisualTestContext) {
    cx.update(gallery::init);
    let mut made = None;
    let (_, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| Gallery::new(window, cx));
        made = Some(view.clone());
        Root::new(view, window, cx)
    });
    cx.run_until_parked();
    (made.expect("the gallery view"), cx)
}

/// Draws a frame and says whether `page` is in it, in `look`: its outer
/// element laid out, the tokens and the window's text size those of the
/// look, the catalogues read in its language.
fn assert_shows(cx: &mut VisualTestContext, page: Page, look: Look) {
    cx.run_until_parked();
    cx.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        cx.debug_bounds(page.selector()).is_some(),
        "{page:?} is not drawn in {look:?}"
    );
    let points = ds::text_size_points(look.text_size);
    let (mode, text_size, rem) = cx.update(|window, cx| {
        (
            cx.ds().mode,
            cx.ds().text_size,
            f32::from(window.rem_size()),
        )
    });
    assert_eq!(mode, look.mode, "{page:?} in {look:?}");
    assert_eq!(text_size, points, "{page:?} in {look:?}");
    assert_eq!(rem, points, "{page:?} in {look:?}");
    assert_eq!(&*rust_i18n::locale(), look.language.code(), "{page:?}");
}

#[gpui::test]
fn every_page_renders_in_every_mode_size_and_language(cx: &mut TestAppContext) {
    let _lock = lock();
    let (gallery, cx) = open(cx);
    let mut cancel = BTreeMap::new();
    for page in Page::ALL.iter().copied() {
        for mode in Mode::ALL {
            for text_size in [TextSize::Standard, TextSize::Large] {
                for language in Language::ALL {
                    let look = Look {
                        mode,
                        text_size,
                        language,
                    };
                    cx.update(|window, cx| {
                        gallery.update(cx, |gallery, cx| {
                            gallery.show(page, cx);
                            gallery.set_look(look, window, cx);
                        })
                    });
                    assert_shows(cx, page, look);
                    cancel.insert(language.code(), tr!("common.cancel"));
                }
            }
        }
    }
    // The text keys follow the language the gallery sets, although the app
    // itself shows English only for now.
    assert_eq!(&*cancel["en"], "Cancel");
    assert_eq!(&*cancel["de"], "Abbrechen");
}

/// ↑ and ↓ move through the pages and stop at the first, 1 to 4 pick the
/// mode, + and - the text size, L the language.
#[gpui::test]
fn the_keys_switch_the_page_the_mode_the_text_size_and_the_language(cx: &mut TestAppContext) {
    let _lock = lock();
    let (_, cx) = open(cx);
    let mut look = Look::FIRST;
    assert_shows(cx, Page::ALL[0], look);
    cx.simulate_keystrokes("down");
    assert_shows(cx, Page::ALL[1], look);
    cx.simulate_keystrokes("up up");
    assert_shows(cx, Page::ALL[0], look);
    for (key, mode) in [("4", Mode::DarkHc), ("3", Mode::LightHc), ("2", Mode::Dark)] {
        cx.simulate_keystrokes(key);
        look.mode = mode;
        assert_shows(cx, Page::ALL[0], look);
    }
    cx.simulate_keystrokes("+");
    look.text_size = TextSize::Large;
    assert_shows(cx, Page::ALL[0], look);
    cx.simulate_keystrokes("-");
    look.text_size = TextSize::Standard;
    assert_shows(cx, Page::ALL[0], look);
    cx.simulate_keystrokes("l");
    look.language = Language::German;
    assert_shows(cx, Page::ALL[0], look);
    cx.simulate_keystrokes("l 1");
    look.language = Language::English;
    look.mode = Mode::Light;
    assert_shows(cx, Page::ALL[0], look);
}

/// A text field takes the same keys as text, so the gallery's keys are off
/// while one has focus.
#[gpui::test]
fn the_keys_stay_off_while_a_text_field_has_focus(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gallery::bind_keys(cx);
        let gallery = [KeyContext::parse(gallery::CONTEXT).expect("context")];
        let typing = [
            KeyContext::parse(gallery::CONTEXT).expect("context"),
            KeyContext::parse("Input").expect("context"),
        ];
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        for keys in ["up", "down", "1", "2", "3", "4", "+", "=", "-", "l"] {
            let keystroke = Keystroke::parse(keys).expect("a keystroke");
            let (bindings, _) =
                keymap.bindings_for_input(std::slice::from_ref(&keystroke), &gallery);
            assert_eq!(bindings.len(), 1, "{keys} in the gallery");
            let (bindings, _) = keymap.bindings_for_input(&[keystroke], &typing);
            assert!(bindings.is_empty(), "{keys} in a text field");
        }
    });
}

// ------------------------------------------------------------ scans --

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// Every `.rs` file under `examples/design_gallery/` but this one, by its
/// path from the repository, with its text.
fn gallery_sources() -> Vec<(String, String)> {
    let gallery = root().join("examples/design_gallery");
    let mut files = Vec::new();
    let mut pending = vec![gallery.clone()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).expect("read the gallery's folder") {
            let path: PathBuf = entry.expect("a directory entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if path == gallery.join(THIS_FILE) {
                continue;
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let name = path
                    .strip_prefix(root())
                    .expect("inside the repository")
                    .display()
                    .to_string();
                let text = std::fs::read_to_string(&path).expect("read a gallery file");
                files.push((name, text));
            }
        }
    }
    files.sort();
    let names: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
    for expected in ["main.rs", "gallery.rs", "pages.rs"] {
        assert!(
            names
                .iter()
                .any(|name| name.ends_with(&format!("design_gallery/{expected}"))),
            "the scan does not see {expected}: {names:?}"
        );
    }
    files
}

/// `source` with its comments and the insides of its string and character
/// literals blanked out, line breaks kept, so that a scan reads code alone
/// and finds it on its line.
fn code_only(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let blank = |c: char| if c == '\n' { '\n' } else { ' ' };
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    let mut out = String::with_capacity(source.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                out.push(' ');
                i += 1;
            }
        } else if c == '/' && next == Some('*') {
            let mut depth = 0;
            while i < chars.len() {
                let pair = (chars[i], chars.get(i + 1).copied());
                if pair == ('/', Some('*')) {
                    depth += 1;
                    out.push_str("  ");
                    i += 2;
                } else if pair == ('*', Some('/')) {
                    depth -= 1;
                    out.push_str("  ");
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    out.push(blank(chars[i]));
                    i += 1;
                }
            }
        } else if c == 'r'
            && (i == 0 || !ident(chars[i - 1]))
            && let Some(quote) = (i + 1..chars.len()).find(|&at| chars[at] != '#')
            && chars[quote] == '"'
        {
            // A raw string: r"..." or r#"..."#.
            let hashes = quote - i - 1;
            out.extend(&chars[i..=quote]);
            i = quote + 1;
            while i < chars.len() {
                let closes =
                    chars[i] == '"' && (1..=hashes).all(|at| chars.get(i + at) == Some(&'#'));
                if closes {
                    out.extend(&chars[i..=i + hashes]);
                    i += hashes + 1;
                    break;
                }
                out.push(blank(chars[i]));
                i += 1;
            }
        } else if c == '"' {
            out.push('"');
            i += 1;
            while i < chars.len() {
                match chars[i] {
                    '\\' => {
                        out.push(' ');
                        out.extend(chars.get(i + 1).map(|&c| blank(c)));
                        i += 2;
                    }
                    '"' => {
                        out.push('"');
                        i += 1;
                        break;
                    }
                    other => {
                        out.push(blank(other));
                        i += 1;
                    }
                }
            }
        } else if c == '\'' && (next == Some('\\') || chars.get(i + 2) == Some(&'\'')) {
            // A character literal, not a lifetime.
            let end = (i + 2..chars.len())
                .find(|&at| chars[at] == '\'')
                .unwrap_or(chars.len() - 1);
            out.push('\'');
            out.extend((i + 1..end).map(|_| ' '));
            out.push('\'');
            i = end + 1;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

/// The identifiers on each line of `code`, with the line's number.
fn identifiers(code: &str) -> impl Iterator<Item = (usize, &str)> {
    code.lines().enumerate().flat_map(|(at, line)| {
        line.split(|c: char| !(c.is_alphanumeric() || c == '_'))
            .filter(|word| !word.is_empty())
            .map(move |word| (at + 1, word))
    })
}

/// The color tokens that belong to a component rather than to the screen:
/// the aliases, the brand tokens and the component tokens of `ColorSet`.
/// Frames and captions may take the semantic tokens above them.
fn component_tokens() -> BTreeSet<String> {
    let source = std::fs::read_to_string(root().join("src/ui/ds/tokens/generated.rs"))
        .expect("read the tokens");
    let body = source
        .split_once("pub struct ColorSet {")
        .and_then(|(_, rest)| rest.split_once("\n}"))
        .map(|(body, _)| body)
        .expect("the ColorSet struct");
    let mut tokens = BTreeSet::new();
    let mut semantic = true;
    for line in body.lines().map(str::trim) {
        if let Some(heading) = line.strip_prefix("// ") {
            semantic = heading.starts_with("Semantic tokens");
            continue;
        }
        if !semantic
            && let Some(name) = line
                .strip_prefix("pub ")
                .and_then(|rest| rest.split_once(": Color,"))
                .map(|(name, _)| name)
        {
            tokens.insert(name.to_string());
        }
    }
    tokens
}

/// What the components draw on, which the gallery must leave to them: the
/// keycap, the unstyled controls under the button, the list, the canvas of
/// the ring, the spinner, and the Kit's input, switch and button.
const COMPONENT_PARTS: &[&str] = &[
    "Keycap",
    "gpui_base",
    "uniform_list",
    "canvas",
    "Spinner",
    "Input",
];

/// The same, as paths into the Kit.
const KIT_CONTROLS: &[&str] = &[
    "gpui_component::switch",
    "gpui_component::button",
    "gpui_component::spinner",
];

/// Every place in `code`, from `file`, that draws with a component's token
/// or builds what a component draws on.
fn components_built(file: &str, source: &str, component_tokens: &BTreeSet<String>) -> Vec<String> {
    let code = code_only(source);
    let mut found: Vec<String> = identifiers(&code)
        .filter(|(_, word)| component_tokens.contains(*word) || COMPONENT_PARTS.contains(word))
        .map(|(line, word)| format!("{file}:{line}: {word}"))
        .collect();
    for (at, line) in code.lines().enumerate() {
        let squeezed: String = line.split_whitespace().collect();
        for path in KIT_CONTROLS {
            if squeezed.contains(path) {
                found.push(format!("{file}:{}: {path}", at + 1));
            }
        }
    }
    found
}

/// The gallery calls the components and draws frames and captions around
/// them: no color token of a component, so no surface, edge or text in a
/// component's color, and nothing a component draws on, the keycap above
/// all, which the compiler keeps out as well. A frame in a semantic token
/// that copies a component is past this scan.
#[test]
fn the_gallery_builds_no_component_of_its_own() {
    let tokens = component_tokens();
    for token in [
        "button_primary_bg",
        "keycap_bg",
        "row_selected_bg",
        "otp_ring",
    ] {
        assert!(tokens.contains(token), "{token} is a component's token");
    }
    for token in ["bg_window", "text_secondary", "border_separator"] {
        assert!(!tokens.contains(token), "{token} is for frames too");
    }
    let found: Vec<String> = gallery_sources()
        .iter()
        .flat_map(|(file, source)| components_built(file, source, &tokens))
        .collect();
    assert!(
        found.is_empty(),
        "the gallery builds what a component draws:\n{}",
        found.join("\n")
    );

    // The scan sees a token and a part in code, and not in a comment or a
    // string. The keycap's name is built from pieces, so that the scan for
    // keycaps outside the design system does not find this file.
    let keycap = concat!("Key", "cap");
    let sample = format!(
        "div().bg(cx.ds().colors.button_primary_bg.hsla)\n\
         // a keycap_bg in a comment\n\
         .child(\"uniform_list\")\n\
         .child({keycap}::new(&keystroke))\n\
         .child(gpui_component :: switch::Switch::new(\"s\"))"
    );
    assert_eq!(
        components_built("sample.rs", &sample, &tokens),
        [
            "sample.rs:1: button_primary_bg",
            "sample.rs:4: Keycap",
            "sample.rs:5: gpui_component::switch",
        ]
    );
}

/// What the gallery must not read, as it appears in code: settings, the
/// app's state, a vault, files, the environment, the clipboard, and the
/// accessibility settings of macOS, which `ds::apply` takes from the
/// gallery instead.
const READS: &[&str] = &[
    "settings::load",
    "AppSettings",
    "AppState",
    "keepass",
    "KeePass",
    "fs::",
    "File::",
    "env::var",
    "a11y_prefs",
    "Appearance::current",
    "apply_choice",
];

/// The same, in any case.
const READS_ANY_CASE: &[&str] = &["clipboard", "pasteboard"];

fn reads_in(file: &str, source: &str) -> Vec<String> {
    let code = code_only(source);
    let mut found = Vec::new();
    for (at, line) in code.lines().enumerate() {
        let squeezed: String = line.split_whitespace().collect();
        let lower = squeezed.to_lowercase();
        for read in READS {
            if squeezed.contains(read) {
                found.push(format!("{file}:{}: {read}", at + 1));
            }
        }
        for read in READS_ANY_CASE {
            if lower.contains(read) {
                found.push(format!("{file}:{}: {read}", at + 1));
            }
        }
    }
    found
}

/// The gallery reads no settings, no vault and no clipboard, and leaves the
/// accessibility settings of macOS to `ds::init`, which the app runs the
/// same way at start. A read hidden behind a function of the app that the
/// gallery calls is past this scan.
#[test]
fn the_gallery_reads_nothing() {
    let found: Vec<String> = gallery_sources()
        .iter()
        .flat_map(|(file, source)| reads_in(file, source))
        .collect();
    assert!(
        found.is_empty(),
        "the gallery reads something:\n{}",
        found.join("\n")
    );

    let sample = "let settings = ferrispass::app::settings::load();\n\
                  /* AppState */ let text = std::fs::read_to_string(path);\n\
                  let label = \"clipboard\";\n\
                  ClipboardService::write_secret(value);";
    assert_eq!(
        reads_in("sample.rs", sample),
        [
            "sample.rs:1: settings::load",
            "sample.rs:2: fs::",
            "sample.rs:4: clipboard",
        ]
    );
}

/// Every module in a file of its own that `src/ui/ds/components/mod.rs`
/// declares `pub mod` has a page, and no page names a module that is not
/// there. The keycap is no `pub mod`: it lives inside a shortcut pair or a
/// button and shows on their pages. The copper count, a `pub mod` written
/// inline, is no component.
#[test]
fn every_component_module_has_a_page() {
    let source = std::fs::read_to_string(root().join("src/ui/ds/components/mod.rs"))
        .expect("read the component modules");
    let modules: BTreeSet<String> = code_only(&source)
        .lines()
        .filter_map(|line| line.trim().strip_prefix("pub mod ")?.strip_suffix(';'))
        .map(|name| name.trim().to_string())
        .collect();
    for expected in ["button", "panel_footer", "vault_badge"] {
        assert!(modules.contains(expected), "the scan reads {expected}");
    }
    assert!(!modules.contains("keycap"), "the keycap is not public");

    let shown: BTreeSet<&str> = Page::ALL
        .iter()
        .flat_map(|page| page.modules())
        .copied()
        .collect();
    let missing: Vec<&String> = modules
        .iter()
        .filter(|module| !shown.contains(module.as_str()))
        .collect();
    assert!(missing.is_empty(), "no page shows {missing:?}");
    let unknown: Vec<&&str> = shown
        .iter()
        .filter(|module| !modules.contains(**module))
        .collect();
    assert!(
        unknown.is_empty(),
        "pages name modules that are gone: {unknown:?}"
    );
}

/// The Kit page holds the glyphs the app draws again in place of the Kit's,
/// which are the files directly under `assets/icons/`, and next to each the
/// Kit's own drawing, not the app's.
#[test]
fn the_kit_page_holds_every_glyph_the_app_draws_again() {
    let mut files: Vec<String> = std::fs::read_dir(root().join("assets/icons"))
        .expect("read assets/icons")
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "svg"))
        .map(|path| {
            let name = path.file_name().expect("a file name").to_string_lossy();
            format!("icons/{name}")
        })
        .collect();
    files.sort();
    assert!(
        files.iter().any(|file| file == "icons/eye.svg"),
        "the scan reads the app's eye: {files:?}"
    );

    let glyphs = pages::kit_glyphs();
    let shown: Vec<&str> = glyphs.iter().map(|glyph| &*glyph.path).collect();
    assert_eq!(
        shown, files,
        "the Kit page and the files under assets/icons/"
    );
    for glyph in &glyphs {
        let app =
            std::fs::read(root().join("assets").join(&*glyph.path)).expect("the app's drawing");
        assert_ne!(app, glyph.kit.as_ref(), "{} beside itself", glyph.path);
    }
}
