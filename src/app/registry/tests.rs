//! The command table checked against itself, the catalogues and GPUI's key
//! parser.

use std::collections::{BTreeSet, HashMap};

use gpui::Keystroke;

use super::*;
use crate::i18n::{Language, catalogue_text};

fn id(spec: &CommandSpec) -> &'static str {
    spec.id.as_str()
}

fn parsed(spec: &CommandSpec, key: &str) -> Keystroke {
    Keystroke::parse(key)
        .unwrap_or_else(|err| panic!("{}: `{key}` does not parse: {err:?}", id(spec)))
}

// ----------------------------------------------------------- names, words --

#[test]
fn ids_are_unique() {
    let mut ids = BTreeSet::new();
    let mut label_keys = BTreeSet::new();
    for spec in commands() {
        assert!(ids.insert(id(spec)), "{} is in the table twice", id(spec));
        assert!(
            label_keys.insert(spec.label_key),
            "{} is the label of two commands",
            spec.label_key
        );
        assert!(
            command(spec.id).is_some_and(|found| std::ptr::eq(found, spec)),
            "command({}) finds another row",
            id(spec)
        );
    }
}

#[test]
fn ids_name_an_area_and_a_verb() {
    let snake_case = |part: &str| {
        !part.is_empty()
            && part.starts_with(|c: char| c.is_ascii_lowercase())
            && part.chars().all(|c| c.is_ascii_lowercase() || c == '_')
    };
    for spec in commands() {
        let (area, verb) = id(spec)
            .split_once('.')
            .unwrap_or_else(|| panic!("{}: no area", id(spec)));
        assert!(
            snake_case(area) && snake_case(verb),
            "{}: ids are <area>.<verb> in snake_case",
            id(spec)
        );
        assert_eq!(
            spec.label_key,
            format!("action.{verb}.label"),
            "{}: the label key follows from the id",
            id(spec)
        );
    }
}

#[test]
fn every_label_key_exists() {
    for spec in commands() {
        for language in Language::ALL {
            let text = catalogue_text(language, spec.label_key);
            assert!(
                text.is_some_and(|text| !text.trim().is_empty()),
                "{}: `{}` has no {} text in locales/action.yml",
                id(spec),
                spec.label_key,
                language.code()
            );
        }
    }
}

#[test]
fn label_key_matches_label() {
    // `label` reads its key through tr!, which the key scans see; `label_key`
    // is the same key as text. A row whose two disagree would show one word
    // and be checked for another.
    for spec in commands() {
        let shown = (spec.label)().to_string();
        let keyed = catalogue_text(Language::English, spec.label_key);
        assert_eq!(
            Some(shown),
            keyed,
            "{}: `label` does not read `{}`",
            id(spec),
            spec.label_key
        );
    }
}

// ------------------------------------------------------------------ keys --

/// Keys with a name rather than a character, as GPUI writes them.
const NAMED_KEYS: &[&str] = &[
    "enter",
    "escape",
    "tab",
    "space",
    "backspace",
    "delete",
    "up",
    "down",
    "left",
    "right",
    "home",
    "end",
    "pageup",
    "pagedown",
];

#[test]
fn every_keystroke_parses() {
    for spec in commands() {
        for key in spec.keys {
            let keystroke = parsed(spec, key);
            // GPUI takes "shift-cmd-c" and "cmd-shift-c" alike. Writing each
            // key the one way GPUI writes it back keeps the table greppable
            // and the same as the keymap snapshot.
            assert_eq!(
                keystroke.unparse(),
                *key,
                "{}: write `{key}` the way GPUI does",
                id(spec)
            );
            // The parser takes any word as a key name, so a misspelt one
            // would never fire.
            assert!(
                keystroke.key.chars().count() == 1 || NAMED_KEYS.contains(&keystroke.key.as_str()),
                "{}: `{key}` is no key GPUI knows",
                id(spec)
            );
        }
    }
}

#[test]
fn no_shortcut_takes_an_option_character() {
    // Option with a character key and without Cmd or Ctrl types a character
    // on Mac layouts: Option-L is @ on a German keyboard. A shortcut on it
    // would take that character away from every text field.
    for spec in commands() {
        for key in spec.keys {
            let keystroke = parsed(spec, key);
            let modifiers = keystroke.modifiers;
            let types_a_character = modifiers.alt
                && !modifiers.platform
                && !modifiers.control
                && keystroke.key.chars().count() == 1;
            assert!(
                !types_a_character,
                "{}: `{key}` takes an Option character",
                id(spec)
            );
        }
    }
}

#[test]
fn no_keystroke_collision_per_surface() {
    let mut taken: HashMap<(Surface, Keystroke), &str> = HashMap::new();
    for spec in commands() {
        for surface in spec.surfaces {
            for key in spec.keys {
                let keystroke = parsed(spec, key);
                if let Some(other) = taken.insert((*surface, keystroke), id(spec)) {
                    panic!(
                        "`{key}` on {surface:?} is taken twice: {other} and {}",
                        id(spec)
                    );
                }
            }
        }
    }
}

// --------------------------------------------------------------- secrets --

#[test]
fn touches_secret_needs_an_entry_target() {
    let secret: Vec<&CommandSpec> = commands()
        .iter()
        .filter(|spec| spec.touches_secret)
        .collect();
    assert!(!secret.is_empty(), "no command touches a secret");
    for spec in secret {
        assert_eq!(
            spec.target,
            TargetKind::Entry,
            "{}: a command that touches a secret acts on the selected entry",
            id(spec)
        );
    }
}
