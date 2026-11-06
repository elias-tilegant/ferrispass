//! The vault badge: which vault an entry is in, at the end of a row.
//!
//! The personal vault is a ring. A team vault is a dot in the team's color
//! with its initial next to it, always, so that the color is never the only
//! sign. Any other vault, a file FerrisPass does not know the audience of,
//! is a dashed ring with its initial.
//!
//! In a row the badge writes no node: the vault is the last part of the
//! row's name. On its own it is an image named after the vault, through
//! [`VaultBadge::standalone`].

use gpui::{
    App, ElementId, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce,
    Role, SharedString, Styled as _, Window, div, prelude::FluentBuilder as _, px,
};

use super::Standalone;
use crate::ui::ds::ActiveTokens as _;
use crate::ui::ds::typography::{TextStyle, TextStyled as _, u};

/// Diameter of the ring and of the dot, in points.
const MARK_SIZE: f32 = 8.0;

/// Width of the ring, in pixels.
const RING_WIDTH: f32 = 1.5;

/// Space between the dot or the ring and the initial, in points.
const INITIAL_GAP: f32 = 3.0;

/// Which vault an entry is in.
#[derive(Clone, Debug, PartialEq, IntoElement)]
pub enum VaultBadge {
    /// The personal vault: a ring in `vault_personal`.
    Personal,
    /// A team vault: a dot in the team's color and the initial
    /// [`initial_for`] gives it.
    Team { initial: SharedString, color: Hsla },
    /// Any other vault: a dashed ring in `vault_other` and its initial.
    Other { initial: SharedString },
}

impl VaultBadge {
    /// A team vault's badge. `color` is one of the team colors of the
    /// tokens. Panics on an empty initial, in every build: without it the
    /// color would be the only sign of the team.
    pub fn team(initial: impl Into<SharedString>, color: Hsla) -> Self {
        let initial = initial.into();
        assert!(!initial.is_empty(), "a team badge needs its initial");
        Self::Team { initial, color }
    }

    /// The badge of a vault that is neither personal nor a team's. Panics on
    /// an empty initial, in every build.
    pub fn other(initial: impl Into<SharedString>) -> Self {
        let initial = initial.into();
        assert!(!initial.is_empty(), "a vault badge needs its initial");
        Self::Other { initial }
    }

    /// The badge on its own: an image named `name`, such as "Team
    /// Beratung" or "Personal, only you". Panics on an empty name.
    pub fn standalone(self, id: impl Into<ElementId>, name: impl Into<SharedString>) -> Standalone {
        Standalone::new(id.into(), Role::Image, name.into(), self.into_any_element())
    }
}

impl RenderOnce for VaultBadge {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let colors = cx.ds().colors;
        let mark = div()
            .debug_selector(|| "fp-vault-mark".into())
            .flex_none()
            .size(u(MARK_SIZE))
            .rounded_full();
        let (mark, initial) = match self {
            Self::Personal => (
                mark.border(px(RING_WIDTH))
                    .border_color(colors.vault_personal.hsla),
                None,
            ),
            Self::Team { initial, color } => (mark.bg(color), Some(initial)),
            Self::Other { initial } => (
                mark.border(px(RING_WIDTH))
                    .border_dashed()
                    .border_color(colors.vault_other.hsla),
                Some(initial),
            ),
        };
        div()
            .debug_selector(|| "fp-vault-badge".into())
            .flex()
            .flex_none()
            .items_center()
            .gap(u(INITIAL_GAP))
            .child(mark)
            .when_some(initial, |this, initial| {
                this.child(
                    div()
                        .debug_selector(|| "fp-vault-initial".into())
                        .text_style(TextStyle::Caption)
                        .text_color(colors.text_secondary.hsla)
                        .child(initial),
                )
            })
    }
}

/// The initial of the vault called `name`, among the vaults called
/// `others`: the first letter of the first word of its name that is not
/// "Team", as a capital. "Team Beratung" is B, "kunden-alt" is K.
///
/// When the word of another vault starts with the same letter, the initials
/// take a second letter, as written in the word. In the order of their
/// words, each takes the first letter after its first where its word
/// differs from every word before it, and that no word before it took as
/// its second letter: "Team Vertrieb" next to "Team Verwaltung" is Ve, and
/// Verwaltung is Vw. Only letters and digits count. A name with neither
/// gives no initial; two vaults with the same word keep the same initial.
pub fn initial_for(name: &str, others: &[String]) -> String {
    let word = initial_word(name);
    let Some(&first) = word.first() else {
        return String::new();
    };
    // An initial is one capital letter, as the design draws it; that is no
    // text set in capitals.
    let mut initial: String = char::to_uppercase(first).collect();
    let mut words: Vec<Vec<char>> = others
        .iter()
        .map(|other| initial_word(other))
        .filter(|other| other.first().is_some_and(|letter| same(*letter, first)))
        .collect();
    if words.is_empty() {
        return initial;
    }
    words.push(word.clone());
    words.sort_by_key(|word| folded(word));
    let mut seconds: Vec<char> = Vec::new();
    for (index, current) in words.iter().enumerate() {
        let before = &words[..index];
        let second = (1..current.len())
            .map(|at| (at, current[at]))
            .find(|(at, letter)| {
                let apart = before
                    .iter()
                    .all(|word| word.get(*at).is_none_or(|other| !same(*other, *letter)));
                apart && !seconds.iter().any(|taken| same(*taken, *letter))
            })
            .map(|(_, letter)| letter)
            .or_else(|| current.get(1).copied());
        if *current == word {
            initial.extend(second);
            return initial;
        }
        seconds.extend(second);
    }
    initial
}

/// The letters and digits of the word an initial comes from: the first
/// word of `name` that is not "Team", or "Team" when it has no other.
fn initial_word(name: &str) -> Vec<char> {
    let mut words = name.split_whitespace();
    let word = words
        .clone()
        .find(|word| !word.eq_ignore_ascii_case("team"))
        .or_else(|| words.next())
        .unwrap_or_default();
    word.chars().filter(|c| c.is_alphanumeric()).collect()
}

/// Whether two letters are the same, whatever their case.
fn same(a: char, b: char) -> bool {
    a.to_lowercase().eq(b.to_lowercase())
}

/// `word` in small letters, for sorting.
fn folded(word: &[char]) -> String {
    word.iter().flat_map(|c| c.to_lowercase()).collect()
}

#[cfg(test)]
mod tests {
    use gpui::{TestAppContext, VisualTestContext};

    use super::super::probe;
    use super::*;
    use crate::ui::ds::tokens::LIGHT;

    fn text(value: &'static str) -> SharedString {
        SharedString::new_static(value)
    }

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    /// The surfaces a window painted in its last frame.
    fn surfaces(cx: &mut VisualTestContext) -> Vec<Hsla> {
        cx.update(|window, _| {
            window
                .painted_quads()
                .into_iter()
                .filter_map(|quad| quad.background.as_solid())
                .filter(|color| !color.is_transparent())
                .collect()
        })
    }

    /// A team badge is a dot in the team's color with the initial next to
    /// it, 3 points away, so the color is never the only sign. Another
    /// vault's badge shows its initial too; the personal vault's ring has
    /// none.
    #[gpui::test]
    fn a_team_badge_always_shows_its_initial(cx: &mut TestAppContext) {
        let teal = LIGHT.team_teal.hsla;
        let cx_team = probe::window(cx, move |_, _| {
            VaultBadge::team(text("B"), teal).into_any_element()
        });
        let mark = cx_team.debug_bounds("fp-vault-mark").expect("the dot");
        let initial = cx_team
            .debug_bounds("fp-vault-initial")
            .expect("the initial");
        assert!(initial.size.width > px(0.), "the initial has a width");
        let gap = cx_team.update(|window, _| u(INITIAL_GAP).to_pixels(window.rem_size()));
        assert!(
            ((initial.left() - mark.right()) - gap).abs() <= px(1.),
            "the initial follows the dot, 3 points away"
        );
        let painted = surfaces(cx_team);
        assert!(
            painted.contains(&teal),
            "the dot in the team's color: {painted:?}"
        );

        let cx_other = probe::window(cx, |_, _| VaultBadge::other(text("K")).into_any_element());
        assert!(cx_other.debug_bounds("fp-vault-initial").is_some());

        let cx_personal = probe::window(cx, |_, _| VaultBadge::Personal.into_any_element());
        assert!(cx_personal.debug_bounds("fp-vault-mark").is_some());
        assert_eq!(cx_personal.debug_bounds("fp-vault-initial"), None);
    }

    #[test]
    #[should_panic(expected = "a team badge needs its initial")]
    fn a_team_badge_needs_its_initial_in_every_build() {
        let _ = VaultBadge::team(SharedString::default(), LIGHT.team_teal.hsla);
    }

    /// In a row the badge writes no node; on its own it is an image named
    /// after the vault.
    #[gpui::test]
    fn a_badge_is_an_image_on_its_own_only(cx: &mut TestAppContext) {
        let (in_row, alone, id) = probe::in_frame(cx, |window, cx| {
            let teal = LIGHT.team_teal.hsla;
            let in_row = [
                VaultBadge::Personal,
                VaultBadge::team(text("B"), teal),
                VaultBadge::other(text("K")),
            ]
            .map(|badge| probe::node(RenderOnce::render(badge, window, cx)).0);
            let badge =
                || VaultBadge::team(text("B"), teal).standalone("vault", text("Team Beratung"));
            let alone = probe::node(RenderOnce::render(badge(), window, cx));
            let id = probe::element_id(RenderOnce::render(badge(), window, cx));
            (in_row, alone, id)
        });
        assert_eq!(in_row, [None, None, None], "no node in a row");
        let (role, node) = alone;
        assert_eq!(role, Some(Role::Image));
        assert_eq!(node.label(), Some("Team Beratung"));
        assert_eq!(id, Some(ElementId::from("vault")));
    }

    #[test]
    fn initial_for_skips_the_word_team_and_resolves_collisions() {
        for (name, others, initial) in [
            ("Team Beratung", names(&[]), "B"),
            ("Team Vertrieb", names(&["Team Verwaltung"]), "Ve"),
            ("Team Verwaltung", names(&["Team Vertrieb"]), "Vw"),
            ("Persönlich", names(&[]), "P"),
            ("kunden-alt", names(&[]), "K"),
            // The rest of the names do not change an initial without a
            // collision.
            (
                "Team Vertrieb",
                names(&["Team Beratung", "kunden-alt"]),
                "V",
            ),
            ("team beratung", names(&[]), "B"),
            ("Team", names(&[]), "T"),
            ("team österreich", names(&[]), "Ö"),
            ("Team Beratung", names(&["Team Buchhaltung"]), "Be"),
            ("Team Buchhaltung", names(&["Team Beratung"]), "Bu"),
            // Three that start alike: each differs from the words before it.
            (
                "Team Vermietung",
                names(&["Team Vertrieb", "Team Verwaltung"]),
                "Ve",
            ),
            (
                "Team Vertrieb",
                names(&["Team Vermietung", "Team Verwaltung"]),
                "Vt",
            ),
            (
                "Team Verwaltung",
                names(&["Team Vermietung", "Team Vertrieb"]),
                "Vw",
            ),
            // A word of one letter keeps it.
            ("Team X", names(&["Team Xylo"]), "X"),
            ("Team Xylo", names(&["Team X"]), "Xy"),
            ("", names(&["Team Beratung"]), ""),
        ] {
            assert_eq!(
                initial_for(name, &others),
                initial,
                "{name} next to {others:?}"
            );
        }
    }
}
