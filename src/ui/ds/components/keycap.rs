//! The keycap: one keystroke as a Mac prints it.
//!
//! A keycap never stands alone. Only `ui::ds` can build one, and it does so
//! in two places: in a [`Shortcut`](super::Shortcut), next to the word for
//! what the keys do, and in a [`Button`](super::Button), whose name says it.
//! For VoiceOver the keys go into `aria_keyshortcuts` on the control they act
//! on, never into a name: on the button itself, and for a shortcut pair on
//! the control the screen binds the keys to. The keycap has no node of its
//! own.
//!
//! The glyphs are set here rather than taken from GPUI's `Display` for
//! `Keystroke`, which writes `^` for Control, puts ⌘ before ⇧ and spells out
//! `enter`. Modifiers come in the order ⌃⌥⇧⌘, Escape is the word "Esc", and
//! a letter is shown as the capital printed on the key.

use gpui::{
    App, InteractiveElement as _, IntoElement, Keystroke, ParentElement as _, RenderOnce,
    SharedString, Styled as _, Window, div, prelude::FluentBuilder as _, px,
};

use crate::ui::ds::ActiveTokens as _;
use crate::ui::ds::tokens::dimension;
use crate::ui::ds::typography::{TextStyle, TextStyled as _, u};

/// Space between the key and the edge of the cap, in points.
const PADDING: f32 = 5.0;

/// One keystroke on a cap.
#[derive(IntoElement)]
pub(in crate::ui::ds) struct Keycap {
    text: SharedString,
    on_accent: bool,
}

impl Keycap {
    pub(in crate::ui::ds) fn new(keystroke: &Keystroke) -> Self {
        Self {
            text: keycap_text(keystroke).into(),
            on_accent: false,
        }
    }

    /// A cap on the copper surface: no surface of its own, only an edge.
    pub(in crate::ui::ds) fn on_accent(mut self, on_accent: bool) -> Self {
        self.on_accent = on_accent;
        self
    }

    #[cfg(test)]
    pub(in crate::ui::ds) fn text(&self) -> &str {
        &self.text
    }
}

impl RenderOnce for Keycap {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.ds().colors;
        let (surface, edge, text) = if self.on_accent {
            (
                None,
                colors.keycap_on_accent_border,
                colors.keycap_on_accent_text,
            )
        } else {
            (
                Some(colors.keycap_bg),
                colors.keycap_border,
                colors.keycap_text,
            )
        };
        div()
            .debug_selector(|| "fp-keycap".into())
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .h(u(dimension::HEIGHT_KEYCAP))
            .min_w(u(dimension::HEIGHT_KEYCAP))
            .px(u(PADDING))
            .rounded(px(dimension::RADIUS_XS))
            .border_1()
            .border_color(edge.hsla)
            .when_some(surface, |this, surface| this.bg(surface.hsla))
            .text_style(TextStyle::Caption)
            .text_color(text.hsla)
            .child(self.text)
    }
}

/// The keystrokes in `keys`, GPUI key strings such as `cmd-shift-c`, one per
/// keystroke, separated by spaces. A string GPUI cannot read is a mistake in
/// the code: loud in a debug build, left out in a release build.
pub(in crate::ui::ds) fn parse_keys(keys: &str) -> Vec<Keystroke> {
    keys.split_whitespace()
        .filter_map(|source| match Keystroke::parse(source) {
            Ok(keystroke) => Some(keystroke),
            Err(error) => {
                debug_assert!(false, "not a key string: {error}");
                None
            }
        })
        .collect()
}

/// A keystroke as its keycap shows it: the modifiers in the order ⌃⌥⇧⌘,
/// then the key.
pub(in crate::ui::ds) fn keycap_text(keystroke: &Keystroke) -> String {
    let modifiers = &keystroke.modifiers;
    let mut text = String::new();
    if modifiers.control {
        text.push('⌃');
    }
    if modifiers.alt {
        text.push('⌥');
    }
    if modifiers.shift {
        text.push('⇧');
    }
    if modifiers.platform {
        text.push('⌘');
    }
    text.push_str(&key_text(&keystroke.key));
    text
}

/// The keys for `aria_keyshortcuts`: the keycap texts, separated by spaces.
/// Whether VoiceOver reads the glyphs in the user's language is unconfirmed
/// until the macOS adapter of AccessKit passes the value on and a device
/// session hears it.
pub(in crate::ui::ds) fn keys_text(keys: &[Keystroke]) -> SharedString {
    keys.iter()
        .map(keycap_text)
        .collect::<Vec<_>>()
        .join(" ")
        .into()
}

/// The key without its modifiers. GPUI names keys in lowercase.
fn key_text(key: &str) -> String {
    let named = match key {
        "enter" => "⏎",
        "escape" => "Esc",
        "tab" => "⇥",
        "backspace" => "⌫",
        "delete" => "⌦",
        "space" => "␣",
        "up" => "↑",
        "down" => "↓",
        "left" => "←",
        "right" => "→",
        "pageup" => "⇞",
        "pagedown" => "⇟",
        "home" => "↖",
        "end" => "↘",
        _ => "",
    };
    if !named.is_empty() {
        return named.to_string();
    }
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        // A letter key: the cap shows the capital, as on the keyboard.
        (Some(letter), None) => letter.to_ascii_uppercase().to_string(),
        // Any other key as GPUI names it. The shortcuts in the app use none.
        _ => key.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::super::probe;
    use super::*;

    fn cap(source: &str) -> String {
        keycap_text(&Keystroke::parse(source).expect("a key string"))
    }

    #[test]
    fn keycap_order_is_control_option_shift_command() {
        assert_eq!(cap("cmd-shift-alt-ctrl-k"), "⌃⌥⇧⌘K");
        assert_eq!(cap("shift-cmd-c"), "⇧⌘C");
        assert_eq!(cap("ctrl-cmd-q"), "⌃⌘Q");
        assert_eq!(cap("alt-shift-enter"), "⌥⇧⏎");
        let keycap = Keycap::new(&Keystroke::parse("cmd-alt-c").expect("a key string"));
        assert_eq!(keycap.text(), "⌥⌘C");
    }

    #[test]
    fn keys_show_as_a_mac_prints_them() {
        for (source, shown) in [
            ("enter", "⏎"),
            ("escape", "Esc"),
            ("tab", "⇥"),
            ("shift-tab", "⇧⇥"),
            ("backspace", "⌫"),
            ("cmd-backspace", "⌘⌫"),
            ("delete", "⌦"),
            ("space", "␣"),
            ("up", "↑"),
            ("down", "↓"),
            ("left", "←"),
            ("right", "→"),
            ("pageup", "⇞"),
            ("pagedown", "⇟"),
            ("home", "↖"),
            ("end", "↘"),
            ("cmd-1", "⌘1"),
            ("cmd-/", "⌘/"),
            ("cmd-,", "⌘,"),
            ("cmd-f", "⌘F"),
        ] {
            assert_eq!(cap(source), shown, "{source}");
        }
        assert_eq!(keys_text(&parse_keys("cmd-k cmd-s")), "⌘K ⌘S");
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "not a key string")]
    fn a_key_string_gpui_cannot_read_is_loud() {
        parse_keys("cmd-k-x");
    }

    /// `Keycap` is visible inside `ui::ds` only, so code elsewhere cannot
    /// name it. This scan backs that up for every folder with Rust in it,
    /// in case the visibility is ever widened. Like the other scans over the
    /// source it leaves out comments and inline test modules, and reads the
    /// rest of a line after a `//` inside a string.
    /// The lines of `code` that name `Keycap::new`, called or passed on as
    /// a function, and not as part of a longer name.
    fn keycap_constructions(code: &str) -> Vec<(usize, String)> {
        // Built from pieces so this file does not match its own pattern.
        let constructor = concat!("Keycap", "::new");
        let joins = |c: char| c.is_alphanumeric() || c == '_';
        let mut found = Vec::new();
        for (number, line) in code.lines().enumerate() {
            for (at, _) in line.match_indices(constructor) {
                let longer = line[..at].ends_with(joins)
                    || line[at + constructor.len()..].starts_with(joins);
                if !longer {
                    found.push((number + 1, line.trim().to_string()));
                }
            }
        }
        found
    }

    /// A keycap never stands alone. Inside the design system two places
    /// build one: the shortcut pair, next to the word for what its keys do,
    /// and the button, whose name says it. The test reads every file under
    /// `src/ui/ds` through `code_only` and finds `Keycap::new`, called or
    /// passed on as a function, in `shortcut.rs` and `button.rs` only, and
    /// in both; outside the design system `Keycap` cannot be named at all.
    /// A keycap built through a renamed import or a macro is past it.
    #[test]
    fn keycap_never_alone() {
        let ds = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui/ds");
        let allowed = [
            ds.join("components/shortcut.rs"),
            ds.join("components/button.rs"),
        ];
        let mut files = Vec::new();
        probe::rust_files(&ds, &mut files);
        assert!(
            files.len() > 10,
            "the scan found only {} files",
            files.len()
        );
        let mut alone = Vec::new();
        let mut paired = Vec::new();
        for file in &files {
            let text = std::fs::read_to_string(file).expect("read source");
            for (number, line) in keycap_constructions(&probe::code_only(&text)) {
                if allowed.contains(file) {
                    paired.push(file.clone());
                } else {
                    alone.push(format!("{}:{number}: {line}", file.display()));
                }
            }
        }
        assert!(
            alone.is_empty(),
            "a keycap stands in a Shortcut or a Button only:\n{}",
            alone.join("\n")
        );
        for file in &allowed {
            assert!(paired.contains(file), "{} builds no keycap", file.display());
        }
    }

    #[test]
    fn keycap_cannot_be_built_outside_ds() {
        // Built from pieces so this file does not match its own pattern.
        let constructor = concat!("Keycap", "::");
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut files = Vec::new();
        for dir in ["src", "tests", "examples"] {
            probe::rust_files(&root.join(dir), &mut files);
        }
        assert!(
            files.len() > 50,
            "the scan found only {} files",
            files.len()
        );
        let ds = root.join("src/ui/ds");
        let mut found = Vec::new();
        for file in files.iter().filter(|file| !file.starts_with(&ds)) {
            let text = std::fs::read_to_string(file).expect("read source");
            let code = probe::code_only(&text);
            for (number, line) in code.lines().enumerate() {
                if line.contains(constructor) {
                    found.push(format!(
                        "{}:{}: {}",
                        file.display(),
                        number + 1,
                        line.trim()
                    ));
                }
            }
        }
        assert!(
            found.is_empty(),
            "keycaps are built inside ui::ds only, use a Shortcut or Button::shortcut:\n{}",
            found.join("\n")
        );
    }
}
