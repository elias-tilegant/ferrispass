//! What rust-i18n does not check on its own.
//!
//! rust-i18n finds a missing key only at runtime, and then shows the key
//! itself, or the English text where the German is missing. These tests read
//! the compiled catalogues and the source instead: every key has English and
//! German with the same placeholders, every key the source asks for exists,
//! the command line tool never asks for one, and the Kit gets its German
//! through the app. Further down, the texts themselves are checked against
//! design/glossary.toml: words to avoid, tone, and the standard messages.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use regex::Regex;
use serde::Deserialize;

use super::*;
use crate::tr;

// ------------------------------------------------------------ catalogues --

/// One language of the app's own catalogues, flattened: `common.cancel`,
/// `gpui_component.Input.Copy`, `quick.results.same_matches.one`.
fn catalogue(language: Language) -> BTreeMap<String, String> {
    crate::_rust_i18n_backend()
        .messages_for_locale(language.code())
        .unwrap_or_default()
        .into_iter()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect()
}

#[test]
fn the_catalogues_carry_english_and_german_only() {
    let mut locales: Vec<String> = crate::_rust_i18n_backend()
        .available_locales()
        .into_iter()
        .map(|locale| locale.into_owned())
        .collect();
    locales.sort();
    assert_eq!(locales, ["de", "en"]);
}

#[test]
fn every_key_has_both_languages() {
    let english = catalogue(Language::English);
    let german = catalogue(Language::German);
    assert!(!english.is_empty(), "no catalogues compiled in");

    let mut problems = Vec::new();
    let keys: BTreeSet<&String> = english.keys().chain(german.keys()).collect();
    for key in keys {
        for (language, texts) in [("en", &english), ("de", &german)] {
            match texts.get(key) {
                None => problems.push(format!("{key}: no {language}")),
                Some(text) if text.trim().is_empty() => {
                    problems.push(format!("{key}: {language} is empty"))
                }
                Some(_) => {}
            }
        }
        // A plural key needs both forms, or `tp!` finds only one of them.
        for (form, other) in [(".one", ".other"), (".other", ".one")] {
            if let Some(base) = key.strip_suffix(form)
                && !english.contains_key(&format!("{base}{other}"))
            {
                problems.push(format!("{base}: has {form} but no {other}"));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn placeholders_match() {
    let english = catalogue(Language::English);
    let german = catalogue(Language::German);
    let mut problems = Vec::new();
    for (language, texts) in [("en", &english), ("de", &german)] {
        for (key, text) in texts {
            for name in placeholders(text) {
                let named = name.starts_with(|c: char| c.is_ascii_lowercase())
                    && name
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
                if !named {
                    problems.push(format!(
                        "{key} ({language}): %{{{name}}} is not a snake_case name"
                    ));
                }
            }
        }
    }
    for (key, text) in &english {
        let Some(german_text) = german.get(key) else {
            continue; // every_key_has_both_languages reports it
        };
        let en: BTreeSet<&str> = placeholders(text).into_iter().collect();
        let de: BTreeSet<&str> = placeholders(german_text).into_iter().collect();
        if en != de {
            problems.push(format!("{key}: en has {en:?}, de has {de:?}"));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn every_catalogue_is_watched() {
    let mut files = Vec::new();
    for entry in fs::read_dir(root().join("locales")).expect("read locales/") {
        let path = entry.expect("directory entry").path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        // rust-i18n reads subdirectories as well, and CATALOGUES would miss
        // them.
        assert!(!path.is_dir(), "locales/{name}: keep the catalogues flat");
        if [".yml", ".yaml", ".json", ".toml"]
            .iter()
            .any(|extension| name.ends_with(extension))
        {
            files.push(name);
        }
    }
    files.sort();
    let mut watched: Vec<String> = CATALOGUES
        .iter()
        .map(|(name, _)| name.to_string())
        .collect();
    watched.sort();
    assert_eq!(
        files, watched,
        "CATALOGUES in src/i18n/mod.rs must list every file in locales/"
    );
    for (name, text) in CATALOGUES {
        assert!(
            text.starts_with("_version: 2\n"),
            "{name}: catalogues keep both languages side by side (_version: 2)"
        );
    }
}

// --------------------------------------------------------------- lookups --

#[test]
fn tr_reads_english_while_german_is_off() {
    init();
    assert_eq!(tr!("common.cancel"), "Cancel");
    // The German is there, just not shown.
    assert_eq!(text_in(Language::German, "common.cancel", &[]), "Abbrechen");
    assert_eq!(text_in(Language::English, "common.why", &[]), "Why?");
}

#[test]
fn placeholders_are_filled_by_name() {
    let filled = fill(
        "test",
        "Sign in to %{sid}, client %{client} as %{user}",
        &[
            ("user", "N.WEISS".to_string()),
            ("sid", "NHP".to_string()),
            ("client", "100".to_string()),
        ],
    );
    assert_eq!(filled, "Sign in to NHP, client 100 as N.WEISS");
    assert_eq!(
        placeholders("%{count} of %{total}, %{count}"),
        ["count", "total", "count"]
    );
    assert!(placeholders("100% sure, {not} one").is_empty());
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "needs a value for %{user}")]
fn a_placeholder_without_a_value_is_loud_in_debug_builds() {
    fill("test", "Signed in as %{user}", &[]);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "text key `no.such.key` is missing")]
fn a_missing_key_is_loud_in_debug_builds() {
    text("no.such.key", &[]);
}

#[test]
fn one_is_singular_everything_else_plural() {
    assert_eq!(plural_form(0), "other");
    assert_eq!(plural_form(1), "one");
    for count in [2, 5, 11, 21, 101, 1_000] {
        assert_eq!(plural_form(count), "other", "{count}");
    }
}

// -------------------------------------------------------------- language --

#[test]
fn german_needs_the_switch_and_macos() {
    for asked in Language::ALL {
        // Switched off, every answer from macOS gives English.
        assert_eq!(choose(false, asked), Language::English, "{asked:?}");
        // Switched on, the UI follows macOS.
        assert_eq!(choose(true, asked), asked, "{asked:?}");
    }
}

#[test]
fn the_ui_is_english_while_german_is_off() {
    init();
    if !GERMAN_ENABLED {
        assert_eq!(ui_language(), Language::English);
        assert_eq!(&*rust_i18n::locale(), "en");
        assert_eq!(current_language(), Language::English);
    }
}

#[test]
fn language_tags_map_to_catalogues() {
    for (tag, expected) in [
        ("de", Some(Language::German)),
        ("de-DE", Some(Language::German)),
        ("de_CH", Some(Language::German)),
        ("DE-at", Some(Language::German)),
        ("en", Some(Language::English)),
        ("en-GB", Some(Language::English)),
        ("fr-FR", None),
        ("deu", None),
        ("", None),
    ] {
        assert_eq!(language_of(tag), expected, "{tag:?}");
    }
    let tags = |list: &[&str]| list.iter().map(|tag| tag.to_string()).collect::<Vec<_>>();
    assert_eq!(
        first_supported(tags(&["fr-FR", "de-DE", "en-US"])),
        Language::German
    );
    assert_eq!(
        first_supported(tags(&["en-US", "de-DE"])),
        Language::English
    );
    assert_eq!(first_supported(tags(&["fr-FR"])), Language::English);
    assert_eq!(first_supported(tags(&[])), Language::English);
}

#[test]
fn init_can_run_twice() {
    // rust-i18n panics on a second extend!, so init() must not reach it again.
    init();
    init();
    assert_eq!(&*rust_i18n::locale(), ui_language().code());
}

// ------------------------------------------------------------------- Kit --

/// The Kit texts the app shows today: the text field's context menu, the
/// dialog buttons, and the empty states of selects and lists.
const KIT_KEYS: &[&str] = &[
    "Input.Cut",
    "Input.Copy",
    "Input.Paste",
    "Input.Select All",
    "Dialog.ok",
    "Dialog.cancel",
    "Select.placeholder",
    "List.search_placeholder",
    "ComboBox.empty",
];

#[test]
fn extend_adds_german_to_kit_strings() {
    // The Kit ships en, it and three Chinese variants, no German.
    let kit = gpui_component::_rust_i18n_backend();
    assert!(kit.available_locales().iter().any(|locale| locale == "en"));
    assert!(!kit.available_locales().iter().any(|locale| locale == "de"));

    init();

    // What the text field's context menu asks for (gpui-component
    // src/input/input.rs, EditMenuItem).
    for (key, german) in [
        ("Input.Cut", "Ausschneiden"),
        ("Input.Copy", "Kopieren"),
        ("Input.Paste", "Einsetzen"),
        ("Input.Select All", "Alles auswählen"),
        ("Dialog.cancel", "Abbrechen"),
    ] {
        assert_eq!(
            gpui_component::_rust_i18n_translate("de", key),
            german,
            "{key}"
        );
    }
    // Every Kit text the app shows has its German in locales/kit.yml.
    let german = catalogue(Language::German);
    for key in KIT_KEYS {
        assert!(
            german.contains_key(&format!("gpui_component.{key}")),
            "locales/kit.yml has no German for the Kit's {key}"
        );
    }
    // A Kit text the app does not translate falls back to the Kit's English.
    assert_eq!(
        gpui_component::_rust_i18n_translate("de", "Input.Replace"),
        "Replace"
    );
    // English stays the Kit's own.
    assert_eq!(
        gpui_component::_rust_i18n_translate("en", "Input.Copy"),
        "Copy"
    );
}

#[test]
fn kit_texts_keep_the_kit_english() {
    let kit = gpui_component::_rust_i18n_backend();
    let english = catalogue(Language::English);
    let mut found = 0;
    for (key, text) in &english {
        let Some(kit_key) = key.strip_prefix("gpui_component.") else {
            continue;
        };
        found += 1;
        match kit.translate("en", kit_key) {
            Some(kit_text) => assert_eq!(
                text, &kit_text,
                "{key}: the app would change the Kit's English"
            ),
            None => panic!("{key}: the Kit has no such text, check the spelling"),
        }
    }
    assert!(
        found >= KIT_KEYS.len(),
        "locales/kit.yml lost entries ({found} left)"
    );
}

// ----------------------------------------------------------- source scan --

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

/// Every `.rs` file under `dir`, as path relative to the repository and text.
fn rust_files(dir: &Path) -> Vec<(String, String)> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let path = entry.expect("directory entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let relative = path
                    .strip_prefix(root())
                    .expect("inside the repository")
                    .to_string_lossy()
                    .replace('\\', "/");
                let text = fs::read_to_string(&path)
                    .unwrap_or_else(|err| panic!("cannot read {relative}: {err}"));
                files.push((relative, text));
            }
        }
    }
    files.sort();
    files
}

/// What a character of Rust source belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Code,
    Comment,
    Literal,
}

/// Marks every character as code, comment or string/char literal, so the
/// scans match calls only where the compiler sees them.
fn classify(chars: &[char]) -> Vec<Part> {
    let mut parts = vec![Part::Code; chars.len()];
    let mut i = 0;
    while i < chars.len() {
        let start = i;
        let part = match (chars[i], chars.get(i + 1).copied()) {
            ('/', Some('/')) => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                Part::Comment
            }
            ('/', Some('*')) => {
                let mut depth = 0usize;
                while i < chars.len() {
                    if chars[i..].starts_with(&['/', '*']) {
                        depth += 1;
                        i += 2;
                    } else if chars[i..].starts_with(&['*', '/']) {
                        depth -= 1;
                        i += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        i += 1;
                    }
                }
                Part::Comment
            }
            ('"', _) => {
                i = string_end(chars, i + 1);
                Part::Literal
            }
            ('\'', next) => {
                // 'x' and '\n' are literals, 'a in a lifetime is code.
                if next == Some('\\') {
                    i += 3;
                    while i < chars.len() && chars[i] != '\'' {
                        i += 1;
                    }
                    i += 1;
                    Part::Literal
                } else if chars.get(i + 2) == Some(&'\'') {
                    i += 3;
                    Part::Literal
                } else {
                    i += 1;
                    Part::Code
                }
            }
            ('r' | 'b' | 'c', _) if !continues_identifier(chars, i) => {
                match prefixed_literal_end(chars, i) {
                    Some(end) => {
                        i = end;
                        Part::Literal
                    }
                    None => {
                        i += 1;
                        Part::Code
                    }
                }
            }
            _ => {
                i += 1;
                Part::Code
            }
        };
        let end = i.min(chars.len());
        parts[start..end].fill(part);
    }
    parts
}

/// The index after the closing quote of a string whose text starts at `from`.
fn string_end(chars: &[char], from: usize) -> usize {
    let mut i = from;
    while i < chars.len() {
        match chars[i] {
            '\\' => i += 2,
            '"' => return i + 1,
            _ => i += 1,
        }
    }
    chars.len()
}

/// `r"…"`, `r#"…"#`, `b"…"`, `br"…"` and `c"…"`: the index after the
/// literal, or `None` when `start` begins an ordinary identifier.
fn prefixed_literal_end(chars: &[char], start: usize) -> Option<usize> {
    let mut i = start;
    if matches!(chars[i], 'b' | 'c') {
        i += 1;
    }
    if chars.get(i) == Some(&'r') {
        i += 1;
        let hashes = chars[i..].iter().take_while(|&&c| c == '#').count();
        i += hashes;
        if chars.get(i) != Some(&'"') {
            return None;
        }
        let closing: Vec<char> = std::iter::once('"')
            .chain(std::iter::repeat_n('#', hashes))
            .collect();
        let mut j = i + 1;
        while j < chars.len() {
            if chars[j..].starts_with(&closing) {
                return Some(j + closing.len());
            }
            j += 1;
        }
        return Some(chars.len());
    }
    if i > start && chars.get(i) == Some(&'"') {
        return Some(string_end(chars, i + 1));
    }
    None
}

/// The lines on which `needle` starts in code, outside comments and strings.
fn code_lines(source: &str, needle: &str) -> Vec<usize> {
    let chars: Vec<char> = source.chars().collect();
    let parts = classify(&chars);
    let needle: Vec<char> = needle.chars().collect();
    parts
        .iter()
        .enumerate()
        .filter(|&(i, part)| *part == Part::Code && chars[i..].starts_with(&needle))
        .map(|(i, _)| line_of(&chars, i))
        .collect()
}

fn line_of(chars: &[char], i: usize) -> usize {
    chars[..i].iter().filter(|&&c| c == '\n').count() + 1
}

fn continues_identifier(chars: &[char], i: usize) -> bool {
    i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_')
}

/// A macro call in the source: its line and its arguments as written, split
/// at the top-level commas.
#[derive(Debug)]
struct Call {
    line: usize,
    args: Vec<String>,
}

/// Every call of the macro `name!` in code, under any path (`tr!`,
/// `crate::tr!`, `ferrispass::tr!`), but not inside comments or strings.
fn calls(source: &str, name: &str) -> Vec<Call> {
    let chars: Vec<char> = source.chars().collect();
    let parts = classify(&chars);
    let pattern: Vec<char> = format!("{name}!").chars().collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if parts[i] == Part::Code
            && chars[i..].starts_with(&pattern)
            && !continues_identifier(&chars, i)
            && let Some((args, end)) = arguments(&chars, &parts, i + pattern.len())
        {
            found.push(Call {
                line: line_of(&chars, i),
                args,
            });
            i = end;
            continue;
        }
        i += 1;
    }
    found
}

/// The arguments of a macro call whose `!` ends just before `from`, and the
/// index after the closing bracket.
fn arguments(chars: &[char], parts: &[Part], from: usize) -> Option<(Vec<String>, usize)> {
    let mut i = from;
    while i < chars.len() && (chars[i].is_whitespace() || parts[i] == Part::Comment) {
        i += 1;
    }
    if !matches!(chars.get(i), Some('(' | '[' | '{')) {
        return None;
    }
    let mut args = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    i += 1;
    while i < chars.len() {
        let c = chars[i];
        if parts[i] == Part::Code {
            match c {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' if depth == 0 => {
                    push_argument(&mut args, &current);
                    return Some((args, i + 1));
                }
                ')' | ']' | '}' => depth -= 1,
                ',' if depth == 0 => {
                    push_argument(&mut args, &current);
                    current.clear();
                    i += 1;
                    continue;
                }
                _ => {}
            }
        }
        if parts[i] != Part::Comment {
            current.push(c);
        }
        i += 1;
    }
    None
}

fn push_argument(args: &mut Vec<String>, text: &str) {
    let text = text.trim();
    if !text.is_empty() {
        args.push(text.to_string());
    }
}

/// One `tr!` or `tp!` in the source.
#[derive(Debug)]
struct KeyUse {
    line: usize,
    /// The key, or what stands where the key literal should be.
    key: Result<String, String>,
    plural: bool,
    /// The placeholder values the call passes, by name.
    names: Vec<String>,
}

fn key_uses(source: &str) -> Vec<KeyUse> {
    let mut uses = Vec::new();
    for (name, plural) in [("tr", false), ("tp", true)] {
        for call in calls(source, name) {
            let first = call.args.first().cloned().unwrap_or_default();
            let key = match first.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                Some(key) if !key.contains(['"', '\\']) => Ok(key.to_string()),
                _ => Err(first),
            };
            // tp! has the count before the named values.
            let skip = if plural { 2 } else { 1 };
            let names = call
                .args
                .iter()
                .skip(skip)
                .map(|arg| arg.split('=').next().unwrap_or_default().trim().to_string())
                .collect();
            uses.push(KeyUse {
                line: call.line,
                key,
                plural,
                names,
            });
        }
    }
    uses.sort_by_key(|key_use| key_use.line);
    uses
}

/// The catalogue keys a use reads: the key itself, or both plural forms.
fn read_keys(key: &str, plural: bool) -> Vec<String> {
    if plural {
        vec![format!("{key}.one"), format!("{key}.other")]
    } else {
        vec![key.to_string()]
    }
}

/// What is wrong with the key uses of one file, checked against the English
/// catalogue (`every_key_has_both_languages` covers the German).
fn check_key_uses(file: &str, uses: &[KeyUse], english: &BTreeMap<String, String>) -> Vec<String> {
    let mut problems = Vec::new();
    for key_use in uses {
        let at = format!("{file}:{}", key_use.line);
        let key = match &key_use.key {
            Ok(key) => key,
            Err(written) => {
                problems.push(format!(
                    "{at}: the key must be a plain string literal, found `{written}`"
                ));
                continue;
            }
        };
        let mut wanted = BTreeSet::new();
        for form in read_keys(key, key_use.plural) {
            match english.get(&form) {
                Some(text) => wanted.extend(placeholders(text)),
                None => problems.push(format!("{at}: `{form}` is not in locales/")),
            }
        }
        let mut given: BTreeSet<&str> = key_use.names.iter().map(String::as_str).collect();
        if key_use.plural {
            given.insert("count");
        }
        for name in wanted.difference(&given) {
            problems.push(format!("{at}: `{key}` needs a value for %{{{name}}}"));
        }
        for name in given.difference(&wanted) {
            if *name != "count" {
                problems.push(format!("{at}: `{key}` has no %{{{name}}}"));
            }
        }
    }
    problems
}

/// Ways to read a catalogue that skip `tr!` and `tp!`, and with them the key
/// check. Only `src/i18n/` may use them.
const LOOKUPS_PAST_THE_SCAN: &[&str] = &[
    "i18n::text(",
    "i18n::plural(",
    "_rust_i18n_translate",
    "_rust_i18n_try_translate",
    "_rust_i18n_backend",
];

/// Problems with how one file outside `src/i18n/` reads texts.
fn lookups_past_the_scan(file: &str, source: &str) -> Vec<String> {
    let mut problems = Vec::new();
    for needle in LOOKUPS_PAST_THE_SCAN {
        for line in code_lines(source, needle) {
            problems.push(format!(
                "{file}:{line}: `{needle}` skips the key check, use tr! or tp!"
            ));
        }
    }
    for call in calls(source, "t") {
        problems.push(format!(
            "{file}:{}: rust-i18n's t! skips the key check, use tr! or tp!",
            call.line
        ));
    }
    problems
}

#[test]
fn used_keys_exist() {
    let english = catalogue(Language::English);
    let mut problems = Vec::new();
    let mut scanned = 0;
    for (file, source) in rust_files(&root().join("src")) {
        scanned += 1;
        // This file holds the scanner's own test sources.
        if file == "src/i18n/tests.rs" {
            continue;
        }
        problems.extend(check_key_uses(&file, &key_uses(&source), &english));
        if !file.starts_with("src/i18n/") {
            problems.extend(lookups_past_the_scan(&file, &source));
        }
    }
    assert!(scanned > 50, "only {scanned} files under src/");
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

const SCAN_SAMPLE: &str = r##"
    // tr!("in.a.comment")
    /* tp!("in.a.block", 2) */
    let s = "tr!(\"in.a.string\")";
    let r = r#"tr!("in.a.raw.string")"#;
    let quote = '"';
    let a = tr!("sample.plain");
    let b = crate::tr!(
        "sample.named",
        entry = name.trim(),
        vault = format!("{}, {}", x, y),
    );
    let c = tp!["sample.count", items.len(), vault = v];
    let d = xtr!("not.this.one");
    let e = tr!(KEY);
"##;

#[test]
fn the_key_scan_reads_code_only() {
    let uses = key_uses(SCAN_SAMPLE);
    type Found<'a> = (usize, Result<&'a str, &'a str>, bool, Vec<&'a str>);
    let found: Vec<Found> = uses
        .iter()
        .map(|key_use| {
            (
                key_use.line,
                key_use.key.as_deref().map_err(String::as_str),
                key_use.plural,
                key_use.names.iter().map(String::as_str).collect(),
            )
        })
        .collect();
    let expected: [Found; 4] = [
        (7, Ok("sample.plain"), false, vec![]),
        (8, Ok("sample.named"), false, vec!["entry", "vault"]),
        (13, Ok("sample.count"), true, vec!["vault"]),
        (15, Err("KEY"), false, vec![]),
    ];
    assert_eq!(found, expected);
}

#[test]
fn the_key_check_finds_missing_keys_and_values() {
    let english: BTreeMap<String, String> = [
        ("sample.plain", "Plain"),
        ("sample.named", "%{entry} in %{vault}"),
        ("sample.count.one", "One match in %{vault}"),
        ("sample.count.other", "%{count} matches in %{vault}"),
        ("sample.half.one", "One"),
    ]
    .into_iter()
    .map(|(key, text)| (key.to_string(), text.to_string()))
    .collect();
    let check = |source: &str| check_key_uses("f.rs", &key_uses(source), &english);

    let without_the_bad_key: String = SCAN_SAMPLE.lines().take(14).collect::<Vec<_>>().join("\n");
    assert!(check(&without_the_bad_key).is_empty());
    assert_eq!(
        check(r#"tr!("sample.gone")"#),
        ["f.rs:1: `sample.gone` is not in locales/"]
    );
    assert_eq!(
        check(r#"tp!("sample.half", n)"#),
        ["f.rs:1: `sample.half.other` is not in locales/"]
    );
    assert_eq!(
        check(r#"tr!("sample.named", entry = e)"#),
        ["f.rs:1: `sample.named` needs a value for %{vault}"]
    );
    assert_eq!(
        check(r#"tr!("sample.plain", user = u)"#),
        ["f.rs:1: `sample.plain` has no %{user}"]
    );
    assert_eq!(
        check("tr!(KEY)"),
        ["f.rs:1: the key must be a plain string literal, found `KEY`"]
    );
}

#[test]
fn the_scan_catches_lookups_that_skip_the_key_check() {
    let clean = "let a = tr!(\"x\"); // i18n::text(\"y\", &[])\nlet t = \"t!(z)\";";
    assert!(lookups_past_the_scan("f.rs", clean).is_empty());
    assert_eq!(
        lookups_past_the_scan("f.rs", "let a = crate::i18n::text(key, &[]);"),
        ["f.rs:1: `i18n::text(` skips the key check, use tr! or tp!"]
    );
    assert_eq!(
        lookups_past_the_scan("f.rs", "\nlet b = rust_i18n::t!(\"x\");"),
        ["f.rs:2: rust-i18n's t! skips the key check, use tr! or tp!"]
    );
    assert_eq!(
        lookups_past_the_scan("f.rs", "crate::_rust_i18n_try_translate(\"de\", key)"),
        ["f.rs:1: `_rust_i18n_try_translate` skips the key check, use tr! or tp!"]
    );
}

/// The command line tool and its installer. Their output is read by scripts
/// and agents and stays English, so they never touch text keys.
const CLI_SOURCES: &[&str] = &[
    "src/cli.rs",
    "src/cli_install.rs",
    "src/bin/ferrispass-cli.rs",
];

#[test]
fn cli_never_uses_text_keys() {
    let mut problems = Vec::new();
    for file in CLI_SOURCES {
        let source = fs::read_to_string(root().join(file))
            .unwrap_or_else(|err| panic!("cannot read {file}: {err}"));
        problems.extend(cli_text_key_uses(file, &source));
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

fn cli_text_key_uses(file: &str, source: &str) -> Vec<String> {
    let mut problems = Vec::new();
    for name in ["tr", "tp", "t"] {
        for call in calls(source, name) {
            problems.push(format!(
                "{file}:{}: {name}! in the command line tool",
                call.line
            ));
        }
    }
    for line in code_lines(source, "i18n") {
        problems.push(format!(
            "{file}:{line}: the command line tool reaches into i18n"
        ));
    }
    problems
}

#[test]
fn the_cli_scan_catches_a_text_key() {
    let clean = "fn main() { println!(\"{}\", \"tr!(x)\"); } // tr!(\"y\")";
    assert!(cli_text_key_uses("cli.rs", clean).is_empty());
    assert_eq!(
        cli_text_key_uses("cli.rs", "let s = crate::tr!(\"common.cancel\");"),
        ["cli.rs:1: tr! in the command line tool"]
    );
    assert_eq!(
        cli_text_key_uses("cli.rs", "\nuse ferrispass::i18n::text;"),
        ["cli.rs:2: the command line tool reaches into i18n"]
    );
}

#[test]
fn only_the_app_starts_i18n() {
    let mut init_callers = Vec::new();
    let mut extend_callers = Vec::new();
    for (file, source) in rust_files(&root().join("src")) {
        if file == "src/i18n/tests.rs" {
            continue;
        }
        if !file.starts_with("src/i18n/") {
            for _ in code_lines(&source, "i18n::init") {
                init_callers.push(file.clone());
            }
        }
        for _ in calls(&source, "extend") {
            extend_callers.push(file.clone());
        }
    }
    assert_eq!(
        init_callers,
        ["src/app/mod.rs"],
        "i18n::init() belongs to app::run alone"
    );
    // rust-i18n panics on a second extend!, so it lives in init() behind its
    // Once.
    assert_eq!(
        extend_callers,
        ["src/i18n/mod.rs"],
        "rust_i18n::extend! runs in i18n::init() and nowhere else"
    );
}

// ------------------------------------------------------- unused keys --

/// One line of `locales/_reserved.txt`: a key no code reads yet.
#[derive(Debug)]
struct Reserved {
    line: usize,
    key: String,
    /// What will read the key.
    reader: String,
}

fn parse_reserved(text: &str) -> Vec<Reserved> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
        .map(|(index, line)| {
            let line_text = line.trim();
            let (key, reader) = line_text
                .split_once(char::is_whitespace)
                .unwrap_or((line_text, ""));
            Reserved {
                line: index + 1,
                key: key.to_string(),
                reader: reader.trim().to_string(),
            }
        })
        .collect()
}

/// Whether a reservation or a `tp!` base key covers a catalogue key: the key
/// itself, or one of its plural forms.
fn covers(base: &str, key: &str) -> bool {
    key == base
        || key
            .strip_prefix(base)
            .is_some_and(|form| form == ".one" || form == ".other")
}

/// Problems with keys that nothing reads. `keys` are the app's own keys,
/// `used` the keys and `tp!` base keys that code reads.
fn unused_key_problems(
    keys: &BTreeSet<String>,
    used: &BTreeSet<String>,
    reserved: &[Reserved],
) -> Vec<String> {
    let mut problems = Vec::new();
    let read = |key: &str| used.iter().any(|base| covers(base, key));
    for key in keys {
        if !read(key) && !reserved.iter().any(|entry| covers(&entry.key, key)) {
            problems.push(format!(
                "{key}: nothing reads it; use it or list it in locales/_reserved.txt"
            ));
        }
    }
    let mut seen = BTreeSet::new();
    for entry in reserved {
        let at = format!("locales/_reserved.txt:{}", entry.line);
        let covered: Vec<&String> = keys.iter().filter(|key| covers(&entry.key, key)).collect();
        if covered.is_empty() {
            problems.push(format!("{at}: `{}` is not in locales/", entry.key));
        }
        if covered.iter().any(|key| read(key)) {
            problems.push(format!(
                "{at}: `{}` is read by code now, remove the line",
                entry.key
            ));
        }
        if entry.reader.is_empty() {
            problems.push(format!("{at}: say what will read `{}`", entry.key));
        }
        if !seen.insert(entry.key.as_str()) {
            problems.push(format!("{at}: `{}` is listed twice", entry.key));
        }
    }
    problems
}

/// The keys code reads: every `tr!` key and every `tp!` base key in `src/`.
fn keys_read_by_code() -> BTreeSet<String> {
    let mut used = BTreeSet::new();
    for (file, source) in rust_files(&root().join("src")) {
        if file == "src/i18n/tests.rs" {
            continue;
        }
        for key_use in key_uses(&source) {
            if let Ok(key) = key_use.key {
                used.insert(key);
            }
        }
    }
    used
}

#[test]
fn no_unused_keys() {
    // The Kit reads its own namespace.
    let keys: BTreeSet<String> = catalogue(Language::English)
        .into_keys()
        .filter(|key| !key.starts_with("gpui_component."))
        .collect();
    let reserved = fs::read_to_string(root().join("locales/_reserved.txt"))
        .expect("read locales/_reserved.txt");
    let problems = unused_key_problems(&keys, &keys_read_by_code(), &parse_reserved(&reserved));
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn the_unused_key_check_finds_each_problem() {
    let keys: BTreeSet<String> = [
        "a.used",
        "a.waiting",
        "a.forgotten",
        "a.count.one",
        "a.count.other",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    let used: BTreeSet<String> = ["a.used", "a.count"]
        .into_iter()
        .map(String::from)
        .collect();
    let reserved = parse_reserved(
        "# comment\n\na.waiting   the waiting screen\na.used  done by now\na.gone  removed\na.waiting   twice\na.bare\n",
    );
    assert_eq!(
        unused_key_problems(&keys, &used, &reserved),
        [
            "a.forgotten: nothing reads it; use it or list it in locales/_reserved.txt",
            "locales/_reserved.txt:4: `a.used` is read by code now, remove the line",
            "locales/_reserved.txt:5: `a.gone` is not in locales/",
            "locales/_reserved.txt:6: `a.waiting` is listed twice",
            "locales/_reserved.txt:7: `a.bare` is not in locales/",
            "locales/_reserved.txt:7: say what will read `a.bare`",
        ]
    );
}

// --------------------------------------------------------- glossary.toml --

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GlossaryFile {
    style: StyleRules,
    term: Vec<Term>,
    phrase: Vec<PhraseRule>,
    scoped: Vec<ScopedRule>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct StyleRules {
    exempt_keys: Vec<String>,
    no_exclamation_keys: Vec<String>,
    forbidden_starts: Vec<String>,
    capitals: Vec<String>,
    formal_de: Vec<String>,
    formal_de_mid_sentence: Vec<String>,
    no_playful_keys: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Term {
    id: String,
    de: Vec<String>,
    en: Vec<String>,
    #[serde(default)]
    avoid_de: Vec<String>,
    #[serde(default)]
    avoid_en: Vec<String>,
    #[serde(default)]
    allow_keys: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PhraseRule {
    lang: String,
    forbid: Vec<String>,
    #[serde(default)]
    allow_keys: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopedRule {
    lang: String,
    word: String,
    only_keys: Vec<String>,
}

fn glossary_file() -> GlossaryFile {
    let text =
        fs::read_to_string(root().join("design/glossary.toml")).expect("read design/glossary.toml");
    toml::from_str(&text).unwrap_or_else(|err| panic!("design/glossary.toml: {err}"))
}

fn language_named(code: &str) -> Language {
    Language::ALL
        .into_iter()
        .find(|language| language.code() == code)
        .unwrap_or_else(|| panic!("design/glossary.toml: no language {code:?}"))
}

/// A key pattern from glossary.toml, where `*` stands for any run of
/// characters.
fn key_pattern(pattern: &str) -> Regex {
    let parts: Vec<String> = pattern.split('*').map(regex::escape).collect();
    Regex::new(&format!("^{}$", parts.join(".+"))).expect("key pattern")
}

fn key_patterns(patterns: &[String]) -> Vec<Regex> {
    patterns
        .iter()
        .map(|pattern| key_pattern(pattern))
        .collect()
}

fn matches_any(patterns: &[Regex], key: &str) -> bool {
    patterns.iter().any(|pattern| pattern.is_match(key))
}

/// `text` without its `%{name}` placeholders, so their names and values do
/// not count as words.
fn without_placeholders(text: &str) -> String {
    Regex::new(r"%\{[^}]*\}")
        .expect("placeholder pattern")
        .replace_all(text, " ")
        .into_owned()
}

/// Where the sentences of `text` start: the beginning, and after `.`, `?`
/// or `!` followed by a space. Opening quotes and brackets are skipped.
fn sentence_starts(text: &str) -> Vec<usize> {
    let opening = |c: char| c.is_whitespace() || "\"'„“‚‘»«([".contains(c);
    let skip = |from: usize| {
        text[from..]
            .char_indices()
            .find(|&(_, c)| !opening(c))
            .map_or(text.len(), |(offset, _)| from + offset)
    };
    let mut starts = vec![skip(0)];
    for (index, c) in text.char_indices() {
        let after = index + c.len_utf8();
        if ".?!".contains(c) && text[after..].starts_with(char::is_whitespace) {
            starts.push(skip(after));
        }
    }
    starts
}

/// A word avoided by a term.
struct Avoided {
    language: Language,
    word: String,
    term: String,
    pattern: Regex,
    allow: Vec<Regex>,
}

struct Phrase {
    language: Language,
    pattern: Regex,
    allow: Vec<Regex>,
}

struct Scoped {
    language: Language,
    word: String,
    pattern: Regex,
    only: Vec<Regex>,
}

/// design/glossary.toml, ready to check one text at a time.
struct Linter {
    exempt: Vec<Regex>,
    no_exclamation: Vec<Regex>,
    forbidden_starts: Vec<String>,
    capitals: BTreeSet<String>,
    formal_de: Vec<Regex>,
    formal_de_mid_sentence: Vec<Regex>,
    /// The words the terms use, per language. They never count as avoided.
    own_words: Vec<(Language, Regex)>,
    avoided: Vec<Avoided>,
    phrases: Vec<Phrase>,
    scoped: Vec<Scoped>,
}

/// A word that matches at the start of a word, in any case.
fn word_start(word: &str) -> Regex {
    Regex::new(&format!(r"(?i)\b{}", regex::escape(word))).expect("word pattern")
}

/// A whole word or phrase, in any case.
fn whole_phrase(words: &str) -> Regex {
    Regex::new(&format!(r"(?i)\b{}\b", regex::escape(words))).expect("phrase pattern")
}

/// A whole word, in exactly this case.
fn exact_word(word: &str) -> Regex {
    Regex::new(&format!(r"\b{}\b", regex::escape(word))).expect("word pattern")
}

impl Linter {
    fn new(file: &GlossaryFile) -> Self {
        let style = &file.style;
        let mut own_words = Vec::new();
        let mut avoided = Vec::new();
        for term in &file.term {
            assert!(
                !term.de.is_empty() && !term.en.is_empty(),
                "design/glossary.toml: term {} needs words in both languages",
                term.id
            );
            for (language, words, avoid) in [
                (Language::German, &term.de, &term.avoid_de),
                (Language::English, &term.en, &term.avoid_en),
            ] {
                own_words.extend(words.iter().map(|word| (language, whole_phrase(word))));
                avoided.extend(avoid.iter().map(|word| Avoided {
                    language,
                    word: word.clone(),
                    term: term.id.clone(),
                    pattern: word_start(word),
                    allow: key_patterns(&term.allow_keys),
                }));
            }
        }
        let phrases = file
            .phrase
            .iter()
            .flat_map(|rule| {
                rule.forbid.iter().map(|pattern| Phrase {
                    language: language_named(&rule.lang),
                    pattern: Regex::new(pattern).unwrap_or_else(|err| {
                        panic!("design/glossary.toml: phrase {pattern:?}: {err}")
                    }),
                    allow: key_patterns(&rule.allow_keys),
                })
            })
            .collect();
        let scoped = file
            .scoped
            .iter()
            .map(|rule| Scoped {
                language: language_named(&rule.lang),
                word: rule.word.clone(),
                pattern: word_start(&rule.word),
                only: key_patterns(&rule.only_keys),
            })
            .collect();
        Linter {
            exempt: key_patterns(&style.exempt_keys),
            no_exclamation: key_patterns(&style.no_exclamation_keys),
            forbidden_starts: style.forbidden_starts.clone(),
            capitals: style.capitals.iter().cloned().collect(),
            formal_de: style
                .formal_de
                .iter()
                .map(|word| exact_word(word))
                .collect(),
            formal_de_mid_sentence: style
                .formal_de_mid_sentence
                .iter()
                .map(|word| exact_word(word))
                .collect(),
            own_words,
            avoided,
            phrases,
            scoped,
        }
    }

    /// Everything wrong with one text, as "key (language): problem".
    fn check(&self, language: Language, key: &str, text: &str) -> Vec<String> {
        let mut found = Vec::new();
        let starts = sentence_starts(text);

        if text.contains(['\u{2013}', '\u{2014}']) {
            found.push("long dash, use a hyphen or a new sentence".to_string());
        }
        if text.contains('!') && matches_any(&self.no_exclamation, key) {
            found.push("no exclamation mark in this kind of text".to_string());
        }
        for start in &starts {
            for label in &self.forbidden_starts {
                if text[*start..].starts_with(label.as_str()) {
                    found.push(format!("starts a sentence with {label:?}"));
                }
            }
        }
        if language == Language::German {
            for pattern in &self.formal_de {
                for hit in pattern.find_iter(text) {
                    found.push(format!("{:?} is the polite form, use du", hit.as_str()));
                }
            }
            for pattern in &self.formal_de_mid_sentence {
                for hit in pattern.find_iter(text) {
                    if !starts.contains(&hit.start()) {
                        found.push(format!(
                            "{:?} in the middle of a sentence is the polite form, use du",
                            hit.as_str()
                        ));
                    }
                }
            }
        }

        if !matches_any(&self.exempt, key) {
            let words = without_placeholders(text);
            for word in words.split(|c: char| !c.is_alphanumeric()) {
                let letters: Vec<char> = word.chars().filter(|c| c.is_alphabetic()).collect();
                if letters.len() >= 2
                    && letters.iter().all(|c| c.is_uppercase())
                    && !self.capitals.contains(word)
                {
                    found.push(format!(
                        "{word:?} in capitals; pass identifiers as placeholders"
                    ));
                }
            }
            let mut plain = words.clone();
            for (own_language, pattern) in &self.own_words {
                if *own_language == language {
                    plain = pattern.replace_all(&plain, " ").into_owned();
                }
            }
            for avoided in &self.avoided {
                if avoided.language == language
                    && avoided.pattern.is_match(&plain)
                    && !matches_any(&avoided.allow, key)
                {
                    found.push(format!(
                        "avoids {:?} (glossary term {})",
                        avoided.word, avoided.term
                    ));
                }
            }
            for phrase in &self.phrases {
                if phrase.language == language
                    && phrase.pattern.is_match(&words)
                    && !matches_any(&phrase.allow, key)
                {
                    found.push(format!("phrase rule {:?}", phrase.pattern.as_str()));
                }
            }
            for scoped in &self.scoped {
                if scoped.language == language
                    && scoped.pattern.is_match(&words)
                    && !matches_any(&scoped.only, key)
                {
                    found.push(format!("{:?} only in SAP texts", scoped.word));
                }
            }
        }

        found
            .into_iter()
            .map(|problem| format!("{key} ({}): {problem}", language.code()))
            .collect()
    }
}

#[test]
fn glossary_lint() {
    let linter = Linter::new(&glossary_file());
    let mut problems = Vec::new();
    for language in Language::ALL {
        for (key, text) in catalogue(language) {
            problems.extend(linter.check(language, &key, &text));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn the_glossary_rules_fire_in_both_languages() {
    let linter = Linter::new(&glossary_file());
    let fires = |language: Language, key: &str, text: &str, expected: &str| {
        let found = linter.check(language, key, text);
        assert!(
            found.len() == 1 && found[0].contains(expected),
            "{text:?} under {key} should break only {expected:?}, found {found:?}"
        );
    };
    let passes = |language: Language, key: &str, text: &str| {
        let found = linter.check(language, key, text);
        assert!(found.is_empty(), "{text:?} under {key}: {found:?}");
    };
    let (de, en) = (Language::German, Language::English);

    // Avoided words, at the start of a word and in any case.
    fires(de, "lock.x", "Die Datenbank ist gesperrt.", "\"Datenbank\"");
    fires(en, "lock.x", "The database is locked.", "\"database\"");
    fires(de, "sync.x", "Zwei Konflikte warten.", "\"Konflikt\"");
    fires(en, "sync.x", "Merged two versions.", "\"merge\"");
    fires(en, "sync.x", "The remote copy is newer.", "\"remote\"");
    // A term's own words are fine, also when they contain an avoided one.
    passes(en, "glossary.x", "Remote access");
    passes(de, "glossary.x", "Fernzugang");
    // Exceptions only through allow_keys, and technical details are exempt.
    passes(de, "msg.security_model", "im lokalen Schlüsselbund");
    fires(de, "msg.other", "im lokalen Schlüsselbund", "\"lokal\"");
    passes(de, "sync.tech.detail", "KDF Argon2id in der Datenbank");
    passes(en, "sync.tech.detail", "KDF Argon2id in the KDBX database");

    // Vaults are unlocked, not opened.
    fires(de, "lock.x", "Tresor öffnen", "phrase rule");
    fires(de, "lock.x", "Öffne den Team-Tresor", "phrase rule");
    fires(de, "lock.x", "Mit Touch ID öffnen", "phrase rule");
    fires(en, "lock.x", "Open the team vault", "phrase rule");
    fires(en, "lock.x", "Your vault won't open.", "phrase rule");
    passes(de, "action.open_vault_file.label", "Tresor-Datei öffnen");
    passes(en, "quick.footer.x", "Open website");

    // "Mandant" and "client" only in SAP texts.
    fires(de, "lock.x", "Mandant 100", "\"Mandant\"");
    fires(en, "lock.x", "client 100", "\"client\"");
    passes(
        de,
        "quick.footer.sap_login",
        "In %{sid}, Mandant %{client} anmelden",
    );
    passes(
        en,
        "quick.footer.sap_login",
        "Sign in to %{sid}, client %{client}",
    );

    // Calm security and error texts.
    fires(de, "lock.x", "Gesperrt!", "exclamation");
    fires(en, "msg.x", "Locked!", "exclamation");
    passes(en, "empty.vault_new.playful", "Fresh start!");
    fires(de, "lock.x", "Fehler: Datei fehlt", "\"Fehler:\"");
    fires(en, "lock.x", "Saved. Error: file missing", "\"Error:\"");

    // Capitals only from the list; identifiers come as placeholders.
    fires(en, "quick.x", "NHP is down", "\"NHP\"");
    fires(de, "quick.x", "NHP ist weg", "\"NHP\"");
    passes(en, "quick.x", "%{sid} is down. Ask IT, then press OK.");
    passes(de, "quick.x", "SAP GUI über VPN, S/4 auf PRD");

    // German says du.
    fires(de, "lock.x", "Bitte geben Sie das Passwort ein.", "\"Sie\"");
    fires(de, "lock.x", "Ihr Passwort passt nicht.", "\"Ihr\"");
    passes(
        de,
        "msg.x",
        "Deine Änderung ist ersetzt. Sie liegt unter Frühere Versionen.",
    );
    passes(
        de,
        "msg.x",
        "Die Datei, ihr Name und wer sie gespeichert hat.",
    );

    // No long dashes, in either language.
    fires(de, "common.x", "A \u{2013} B", "long dash");
    fires(en, "common.x", "A \u{2014} B", "long dash");
}

// ------------------------------------------------------ standard messages --

/// The standard messages, one key per situation. Every screen that meets one
/// of these situations uses the key; none writes its own sentence.
const STANDARD_MESSAGES: [&str; 24] = [
    "msg.wrong_master_password",
    "msg.file_incomplete",
    "msg.offline",
    "msg.signin_expired",
    "msg.consent_missing",
    "msg.no_access_read",
    "msg.no_access_write",
    "msg.decision_needed",
    "msg.concurrent_change",
    "msg.own_version_replaced",
    "msg.folder_no_history",
    "msg.team_password_changed",
    "msg.protection_raised",
    "msg.protection_lowered",
    "msg.sap_missing_sysnr",
    "msg.sap_no_launch_file",
    "msg.sap_quit_immediately",
    "msg.type_nothing_remembered",
    "msg.type_browser_explicit",
    "msg.type_permission_managed",
    "msg.update_needs_admin",
    "msg.window_closed_locked",
    "msg.delete_permanently",
    "msg.security_model",
];

/// How many words two texts may share at their start before they count as
/// the same message written twice.
const SAME_OPENING: usize = 4;

/// The first words of `text`, lower case, placeholders as `%`. `None` for a
/// text too short to compare.
fn opening(text: &str) -> Option<String> {
    let placeholder = Regex::new(r"%\{[^}]*\}").expect("placeholder pattern");
    let text = placeholder.replace_all(text, "%");
    let words: Vec<String> = text
        .split_whitespace()
        .map(|word| {
            word.trim_matches(|c: char| !c.is_alphanumeric() && c != '%')
                .to_lowercase()
        })
        .filter(|word| !word.is_empty())
        .take(SAME_OPENING)
        .collect();
    (words.len() == SAME_OPENING).then(|| words.join(" "))
}

/// Texts that start with the same words under different keys. The two
/// forms of one plural key count as one text.
fn near_duplicates(texts: &BTreeMap<String, String>) -> Vec<String> {
    let mut by_opening: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
    for (key, text) in texts {
        if let Some(opening) = opening(text) {
            let message = key
                .strip_suffix(".one")
                .or_else(|| key.strip_suffix(".other"))
                .unwrap_or(key);
            by_opening.entry(opening).or_default().insert(message);
        }
    }
    by_opening
        .into_iter()
        .filter(|(_, keys)| keys.len() > 1)
        .map(|(opening, keys)| {
            let keys: Vec<&str> = keys.into_iter().collect();
            format!(
                "{} start alike (\"{opening} ...\"); keep one and refer to it",
                keys.join(" and ")
            )
        })
        .collect()
}

#[test]
fn msg_unique() {
    let english = catalogue(Language::English);
    let mut problems = Vec::new();
    for message in STANDARD_MESSAGES {
        let found = english.keys().any(|key| covers(message, key));
        if !found {
            problems.push(format!("{message}: missing from locales/msg.yml"));
        }
    }
    for key in english.keys().filter(|key| key.starts_with("msg.")) {
        if !STANDARD_MESSAGES.iter().any(|message| covers(message, key)) {
            problems.push(format!(
                "{key}: not a standard message; add it to STANDARD_MESSAGES or move it"
            ));
        }
    }
    for language in Language::ALL {
        for duplicate in near_duplicates(&catalogue(language)) {
            problems.push(format!("{}: {duplicate}", language.code()));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn near_duplicates_are_found_by_their_opening() {
    let texts: BTreeMap<String, String> = [
        (
            "msg.offline",
            "You are offline. Your changes stay on this Mac.",
        ),
        ("sync.status", "You are offline. Your edits wait here."),
        ("quick.count.one", "%{count} identical match in %{vault}"),
        ("quick.count.other", "%{count} identical match in %{vault}s"),
        ("common.short", "Too short"),
        ("common.other", "Too short"),
    ]
    .into_iter()
    .map(|(key, text)| (key.to_string(), text.to_string()))
    .collect();
    assert_eq!(
        near_duplicates(&texts),
        [
            "msg.offline and sync.status start alike (\"you are offline your ...\"); \
             keep one and refer to it"
        ]
    );
}

// ------------------------------------------------------------------- tone --

/// Keys with the segment `playful` under a key that stays factual.
fn playful_problems<'a>(
    keys: impl IntoIterator<Item = &'a String>,
    factual: &[Regex],
) -> Vec<String> {
    keys.into_iter()
        .filter(|key| key.split('.').any(|segment| segment == "playful"))
        .filter(|key| matches_any(factual, key))
        .map(|key| format!("{key}: playful text under a key that stays factual"))
        .collect()
}

#[test]
fn playful_only_where_allowed() {
    let factual = key_patterns(&glossary_file().style.no_playful_keys);
    let english = catalogue(Language::English);
    let problems = playful_problems(english.keys(), &factual);
    assert!(problems.is_empty(), "{}", problems.join("\n"));

    let samples: Vec<String> = [
        "empty.vault_new.playful",
        "msg.offline.playful",
        "quick.footer.playful.hint",
        "onb.join.error.playful",
        "onb.join.done.playful",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    assert_eq!(
        playful_problems(&samples, &factual),
        [
            "msg.offline.playful: playful text under a key that stays factual",
            "quick.footer.playful.hint: playful text under a key that stays factual",
            "onb.join.error.playful: playful text under a key that stays factual",
        ]
    );
}
