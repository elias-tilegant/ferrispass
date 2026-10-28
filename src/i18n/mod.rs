//! Text keys and the UI language.
//!
//! Every text the new UI shows comes from a key in `locales/*.yml`, where
//! English and German stand side by side. `tr!` looks a key up, `tp!` picks
//! the singular or plural form for a count. rust-i18n compiles the catalogues
//! into the crate but only notices a missing key at runtime, by showing the
//! key itself. The tests in `tests.rs` close that gap: every key has both
//! languages with the same placeholders, and every key the source asks for
//! exists. Both macros take the key as a literal so the tests can find it.
//!
//! German stays off until the old main window is translated as well. Until
//! then the app is English throughout, whatever macOS asks for, so the two
//! languages never appear side by side. The command line tool never uses
//! text keys and never calls [`init`]: its output is an interface for scripts
//! and stays English.

#![forbid(unsafe_code)]

pub mod fmt;
#[cfg(test)]
mod tests;

use std::sync::Once;

use gpui::SharedString;

/// Whether the app follows macOS into German. Off until every screen,
/// including the old main window, has its German text.
pub const GERMAN_ENABLED: bool = false;

/// A language the catalogues carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    English,
    German,
}

impl Language {
    pub const ALL: [Language; 2] = [Language::English, Language::German];

    /// The locale code the catalogues and rust-i18n use.
    pub fn code(self) -> &'static str {
        match self {
            Language::English => "en",
            Language::German => "de",
        }
    }
}

/// The language the UI shows: German only when macOS asks for it and German
/// is switched on, English otherwise.
pub fn ui_language() -> Language {
    choose(GERMAN_ENABLED, macos_language())
}

fn choose(german_enabled: bool, asked: Language) -> Language {
    match asked {
        Language::German if german_enabled => Language::German,
        Language::English | Language::German => Language::English,
    }
}

/// Which of English and German macOS asks for. A language change in System
/// Settings applies from the next launch, as in any Mac app.
#[cfg(target_os = "macos")]
fn macos_language() -> Language {
    use objc2_foundation::{NSArray, NSBundle, NSLocale, NSString};

    let tags = if NSBundle::mainBundle().bundleIdentifier().is_some() {
        // The bundle's view, which includes the per-app language from
        // System Settings.
        let offered = NSArray::from_retained_slice(&[
            NSString::from_str(Language::English.code()),
            NSString::from_str(Language::German.code()),
        ]);
        NSBundle::preferredLocalizationsFromArray(&offered)
    } else {
        // `cargo run` and the tests have no bundle, so only the system list.
        NSLocale::preferredLanguages()
    };
    first_supported(tags.iter().map(|tag| tag.to_string()))
}

#[cfg(not(target_os = "macos"))]
fn macos_language() -> Language {
    Language::English
}

/// The first language in the user's order that the app has catalogues for,
/// English when there is none.
fn first_supported(tags: impl IntoIterator<Item = String>) -> Language {
    tags.into_iter()
        .find_map(|tag| language_of(&tag))
        .unwrap_or(Language::English)
}

/// `de`, `de-DE`, `de_CH` and the like are German; the region does not
/// matter, the catalogues have one German.
fn language_of(tag: &str) -> Option<Language> {
    let primary = tag.split(['-', '_']).next().unwrap_or_default();
    Language::ALL
        .into_iter()
        .find(|language| primary.eq_ignore_ascii_case(language.code()))
}

/// The language the catalogues are read in right now.
fn current_language() -> Language {
    if &*rust_i18n::locale() == Language::German.code() {
        Language::German
    } else {
        Language::English
    }
}

static INIT: Once = Once::new();

/// Hands the Kit's own texts (context menus, dialog buttons) to the app's
/// catalogues and sets the UI language for both. Called once at app start.
/// A second call does nothing, because rust-i18n panics when a crate is
/// extended twice.
pub fn init() {
    INIT.call_once(|| {
        rust_i18n::extend!(gpui_component);
        rust_i18n::set_locale(ui_language().code());
    });
}

/// rust-i18n reads `locales/` while it compiles the crate but does not tell
/// Cargo, so an edited catalogue alone would not trigger a rebuild. Including
/// each file here does. A test checks that the list matches the directory.
#[cfg_attr(not(test), allow(dead_code))]
const CATALOGUES: &[(&str, &str)] = &[
    ("a11y.yml", include_str!("../../locales/a11y.yml")),
    ("common.yml", include_str!("../../locales/common.yml")),
    ("glossary.yml", include_str!("../../locales/glossary.yml")),
    ("kit.yml", include_str!("../../locales/kit.yml")),
    ("menubar.yml", include_str!("../../locales/menubar.yml")),
    ("msg.yml", include_str!("../../locales/msg.yml")),
    ("quick.yml", include_str!("../../locales/quick.yml")),
];

/// The text for `key` in the current language. Use [`tr!`](crate::tr).
#[doc(hidden)]
pub fn text(key: &str, args: &[(&str, String)]) -> SharedString {
    text_in(current_language(), key, args).into()
}

/// The singular or plural text for `count`. Use [`tp!`](crate::tp).
#[doc(hidden)]
pub fn plural(key: &str, count: usize, args: &[(&str, String)]) -> SharedString {
    plural_in(current_language(), key, count, args).into()
}

fn text_in(language: Language, key: &str, args: &[(&str, String)]) -> String {
    fill(key, &lookup(language, key), args)
}

fn plural_in(language: Language, key: &str, count: usize, args: &[(&str, String)]) -> String {
    let key = format!("{key}.{}", plural_form(count));
    let count = u64::try_from(count).unwrap_or(u64::MAX);
    let mut args = args.to_vec();
    args.push(("count", fmt::integer(count, language)));
    text_in(language, &key, &args)
}

/// English and German share the plural rule for whole numbers: one for 1,
/// other for everything else, 0 included.
fn plural_form(count: usize) -> &'static str {
    if count == 1 { "one" } else { "other" }
}

fn lookup(language: Language, key: &str) -> String {
    match crate::_rust_i18n_try_translate(language.code(), key) {
        Some(text) => text.into_owned(),
        None => {
            // `used_keys_exist` checks every key the source names, so only a
            // key that slipped past it gets here: loud in a debug build, the
            // key itself in a release build, which is what rust-i18n shows.
            if cfg!(debug_assertions) {
                panic!("text key `{key}` is missing from locales/");
            }
            key.to_string()
        }
    }
}

fn fill(key: &str, template: &str, args: &[(&str, String)]) -> String {
    if cfg!(debug_assertions) {
        for name in placeholders(template) {
            assert!(
                args.iter().any(|(arg, _)| *arg == name),
                "text key `{key}` needs a value for %{{{name}}}"
            );
        }
    }
    let names: Vec<&str> = args.iter().map(|(name, _)| *name).collect();
    let values: Vec<String> = args.iter().map(|(_, value)| value.clone()).collect();
    rust_i18n::replace_patterns(template, &names, &values)
}

/// The names of the `%{name}` placeholders in `text`, in order.
fn placeholders(text: &str) -> Vec<&str> {
    let mut names = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("%{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            break;
        };
        names.push(&after[..end]);
        rest = &after[end + 1..];
    }
    names
}

/// The text for a key in the current language, as a `SharedString`.
///
/// The key is a string literal, so the tests can check that it exists.
/// Placeholders are filled by name, `vault = name` for `%{vault}`, with any
/// value that implements `Display`.
#[macro_export]
macro_rules! tr {
    ($key:literal $(,)?) => {
        $crate::i18n::text($key, &[])
    };
    ($key:literal, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::i18n::text(
            $key,
            &[$((
                ::core::stringify!($name),
                ::std::string::ToString::to_string(&$value),
            )),+],
        )
    };
}

/// The singular or plural text for a count, as a `SharedString`.
///
/// Reads `<key>.one` for 1 and `<key>.other` for any other count, and fills
/// `%{count}` with the count written for the current language (1,234 or
/// 1.234). The count is a `usize`; further placeholders work as in `tr!`.
#[macro_export]
macro_rules! tp {
    ($key:literal, $count:expr $(, $name:ident = $value:expr)* $(,)?) => {
        $crate::i18n::plural(
            $key,
            $count,
            &[$((
                ::core::stringify!($name),
                ::std::string::ToString::to_string(&$value),
            )),*],
        )
    };
}
