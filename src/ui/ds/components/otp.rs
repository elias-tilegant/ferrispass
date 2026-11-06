//! The one-time code ring: how long the current one-time code still holds,
//! next to the code.
//!
//! The ring is drawn on a canvas: the track in `otp_track` and over it,
//! clockwise from the top, the time left in `otp_ring`, or in
//! `otp_ring_warn` below five seconds. Before it stand the code in `Code`,
//! in groups of three, and the seconds left in `Callout` with digits of equal
//! width. Masked, as in the panel, the code is "••• •••" whatever it is;
//! unmasked, in its last five seconds, the next code stands beside it. The
//! ring keeps no timer of its own: the caller draws it again, ten times a
//! second, with the time left.
//!
//! For VoiceOver the ring is a timer named "One-time code, expires in 20
//! seconds", the seconds rounded up to a multiple of five, so the name
//! changes at most every five seconds. The code reaches the tree only as the
//! timer's value, and only while it is unmasked and assistive technology is
//! running, which the caller says through [`OtpRing::assistive`]. The texts
//! are plain text without an id, which writes no node of its own.

use std::f32::consts::TAU;
use std::time::Duration;

use gpui::{
    App, Bounds, Canvas, Div, ElementId, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, Path, PathBuilder, Pixels, RenderOnce, Role, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, Window, canvas, div, point, px,
};
use zeroize::Zeroizing;

use super::counted;
use crate::tr;
use crate::ui::ds::ActiveTokens as _;
use crate::ui::ds::tokens::{ColorSet, dimension};
use crate::ui::ds::typography::{TextStyle, TextStyled as _, tabular_figures, u};

/// What a masked code shows, whatever its digits.
const MASKED_CODE: &str = "••• •••";

/// A code is shown in groups of this many digits.
const CODE_GROUP: usize = 3;

/// Below this much time left the ring warns, and an unmasked ring shows the
/// next code.
const LAST_SECONDS: Duration = Duration::from_secs(5);

/// The name VoiceOver reads rounds the seconds up to a multiple of this.
const ANNOUNCE_STEP: u64 = 5;

/// The unit after the seconds left, the symbol for seconds, which reads the
/// same in English and German.
const SECONDS: &str = "s";

/// How large the ring is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OtpSize {
    /// 16 points, in a row.
    #[default]
    Row16,
    /// 18 points, in an entry's details.
    Detail18,
}

impl OtpSize {
    /// The diameter in points at text size 13.
    pub fn diameter(self) -> f32 {
        match self {
            Self::Row16 => 16.0,
            Self::Detail18 => 18.0,
        }
    }
}

/// The ring of a one-time code. It has no `Debug`, so it cannot be printed
/// with its code.
#[derive(Clone, IntoElement)]
pub struct OtpRing {
    id: ElementId,
    remaining: Duration,
    period: Duration,
    code: Option<Zeroizing<String>>,
    masked: bool,
    assistive: bool,
    next_code: Option<Zeroizing<String>>,
    size: OtpSize,
}

impl OtpRing {
    /// A ring with `remaining` of `period` left, masked until the caller
    /// says otherwise.
    pub fn new(id: impl Into<ElementId>, remaining: Duration, period: Duration) -> Self {
        Self {
            id: id.into(),
            remaining,
            period,
            code: None,
            masked: true,
            assistive: false,
            next_code: None,
            size: OtpSize::default(),
        }
    }

    /// The current code. Without one the ring shows only the time left.
    pub fn code(mut self, code: Zeroizing<String>) -> Self {
        self.code = Some(code);
        self
    }

    /// The code after the current one, shown in the last five seconds while
    /// the ring is unmasked.
    pub fn next_code(mut self, code: Zeroizing<String>) -> Self {
        self.next_code = Some(code);
        self
    }

    /// Whether the code shows as "••• •••", which is the default. The panel
    /// keeps it masked.
    pub fn masked(mut self, masked: bool) -> Self {
        self.masked = masked;
        self
    }

    /// Whether assistive technology such as VoiceOver is running. Only then,
    /// and only while unmasked, does the code go into the tree, as the
    /// timer's value. The ring never finds this out itself.
    pub fn assistive(mut self, assistive: bool) -> Self {
        self.assistive = assistive;
        self
    }

    pub fn size(mut self, size: OtpSize) -> Self {
        self.size = size;
        self
    }

    /// Whether the next code stands beside the current one.
    fn shows_next_code(&self) -> bool {
        !self.masked && self.next_code.is_some() && in_last_seconds(self.remaining)
    }
}

/// The whole seconds left, as the ring shows them: 17.3 seconds is 18.
pub fn shown_seconds(remaining: Duration) -> u64 {
    remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0)
}

/// The seconds VoiceOver hears: the seconds shown, rounded up to a multiple
/// of five, so the timer's name changes at most every five seconds.
pub fn announced_seconds(remaining: Duration) -> u64 {
    shown_seconds(remaining).div_ceil(ANNOUNCE_STEP) * ANNOUNCE_STEP
}

/// Whether the code is in its last five seconds.
fn in_last_seconds(remaining: Duration) -> bool {
    remaining < LAST_SECONDS
}

/// `code` in groups of three: "739114" is "739 114".
fn grouped(code: &str) -> Zeroizing<String> {
    let mut text = Zeroizing::new(String::with_capacity(code.len() + code.len() / CODE_GROUP));
    for (index, c) in code.chars().enumerate() {
        if index > 0 && index % CODE_GROUP == 0 {
            text.push(' ');
        }
        text.push(c);
    }
    text
}

/// What the code shows: "••• •••" while masked, the code in groups of three
/// otherwise.
fn shown_code(code: &str, masked: bool) -> SharedString {
    if masked {
        SharedString::new_static(MASKED_CODE)
    } else {
        grouped(code).as_str().to_string().into()
    }
}

/// The paths of the ring in `bounds`, with their colors: the track, a full
/// circle in `otp_track`, and over it the time left, clockwise from the top,
/// in `otp_ring`, or in `otp_ring_warn` in the last five seconds. Both are
/// strokes of `STROKE_OTP` inside `bounds`.
fn ring_paths(
    bounds: Bounds<Pixels>,
    remaining: Duration,
    period: Duration,
    colors: &ColorSet,
) -> Vec<(Path<Pixels>, Hsla)> {
    let stroke = px(dimension::STROKE_OTP);
    let side = bounds.size.width.min(bounds.size.height);
    let radius = (side - stroke) / 2.;
    let center = bounds.center();
    // The point `turn` of a full turn clockwise from the top.
    let at = |turn: f32| {
        let angle = turn * TAU;
        point(
            center.x + radius * angle.sin(),
            center.y - radius * angle.cos(),
        )
    };
    let arc = |turn: f32| -> Option<Path<Pixels>> {
        if turn <= 0. {
            return None;
        }
        let radii = point(radius, radius);
        let mut path = PathBuilder::stroke(stroke);
        path.move_to(at(0.));
        if turn >= 1. {
            // One arc cannot close a circle; two halves do.
            path.arc_to(radii, px(0.), false, true, at(0.5));
            path.arc_to(radii, px(0.), false, true, at(0.));
        } else {
            path.arc_to(radii, px(0.), turn > 0.5, true, at(turn));
        }
        path.build().ok()
    };
    let left = if period.is_zero() {
        0.
    } else {
        (remaining.as_secs_f32() / period.as_secs_f32()).clamp(0., 1.)
    };
    let running = if in_last_seconds(remaining) {
        colors.otp_ring_warn.hsla
    } else {
        colors.otp_ring.hsla
    };
    let mut paths: Vec<(Path<Pixels>, Hsla)> = arc(1.)
        .map(|track| (track, colors.otp_track.hsla))
        .into_iter()
        .collect();
    paths.extend(arc(left).map(|time| (time, running)));
    paths
}

/// Every element the ring builds, before `assemble` puts them in place. The
/// code, the next code and the seconds each sit in a box that sets their
/// text style; the box holds the text.
struct Built {
    timer: Stateful<Div>,
    code: Option<(Div, SharedString)>,
    next_code: Option<(Div, SharedString)>,
    seconds: (Div, SharedString),
    ring: Canvas<()>,
}

impl OtpRing {
    fn build(self, cx: &mut App) -> Built {
        let colors = cx.ds().colors;
        // The one place the code may reach the tree.
        let in_tree: Option<SharedString> = (!self.masked && self.assistive)
            .then(|| self.code.as_ref().map(|code| shown_code(code, false)))
            .flatten();
        let code = self.code.as_ref().map(|code| {
            let shown = counted(div(), "otp-code")
                .debug_selector(|| "fp-otp-code".into())
                .flex_none()
                .text_style(TextStyle::Code)
                .text_color(colors.text_primary.hsla);
            (shown, shown_code(code, self.masked))
        });
        let next_code = self
            .next_code
            .as_ref()
            .filter(|_| self.shows_next_code())
            .map(|code| {
                let shown = counted(div(), "otp-next-code")
                    .debug_selector(|| "fp-otp-next-code".into())
                    .flex_none()
                    .text_style(TextStyle::Code)
                    .text_color(colors.text_tertiary.hsla);
                (shown, shown_code(code, false))
            });
        let seconds = (
            counted(div(), "otp-seconds")
                .debug_selector(|| "fp-otp-seconds".into())
                .flex_none()
                .text_style(TextStyle::Callout)
                .font_features(tabular_figures())
                .text_color(colors.text_secondary.hsla),
            format!("{} {SECONDS}", shown_seconds(self.remaining)).into(),
        );
        let (remaining, period) = (self.remaining, self.period);
        let ring = canvas(
            |_, _, _| {},
            move |bounds, (), window, _| {
                for (path, color) in ring_paths(bounds, remaining, period, colors) {
                    window.paint_path(path, color);
                }
            },
        )
        .flex_none()
        .size(u(self.size.diameter()));
        let timer = counted(div(), "otp-timer")
            .id(self.id)
            .role(Role::Timer)
            .aria_label(tr!(
                "a11y.one_time_code_expires",
                seconds = announced_seconds(self.remaining)
            ))
            .when_some(in_tree, |this, code| this.aria_value(code))
            .flex()
            .flex_none()
            .items_center()
            .gap(u(dimension::SPACE_8));
        Built {
            timer,
            code,
            next_code,
            seconds,
            ring,
        }
    }

    /// Every element `render` builds, each read as the tests read it: the
    /// timer, the boxes of the code, the next code and the seconds with the
    /// text each holds, and the ring.
    #[cfg(test)]
    pub(super) fn parts(&self, _: &mut Window, cx: &mut App) -> Vec<super::probe::ComponentPart> {
        use super::probe::part;
        let built = self.clone().build(cx);
        let mut parts = vec![part("timer", built.timer)];
        for (name, text_name, shown) in [
            ("code", "code_text", built.code),
            ("next_code", "next_code_text", built.next_code),
            ("seconds", "seconds_text", Some(built.seconds)),
        ] {
            if let Some((shown, text)) = shown {
                parts.push(part(name, shown));
                parts.push(part(text_name, text));
            }
        }
        parts.push(part("ring", built.ring));
        parts
    }
}

use gpui::prelude::FluentBuilder as _;

/// Puts what the ring builds in place: the code, the next code, the seconds
/// left and the ring.
fn assemble(built: Built) -> Stateful<Div> {
    let shown = |(shown, text): (Div, SharedString)| shown.child(text);
    built
        .timer
        .children(built.code.map(shown))
        .children(built.next_code.map(shown))
        .child(shown(built.seconds))
        .child(built.ring)
}

impl RenderOnce for OtpRing {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        assemble(self.build(cx))
    }
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::super::probe::{self, ComponentPart as Part};
    use super::*;
    use crate::ui::ds::tokens::LIGHT;

    const CODE: &str = "739114";
    const NEXT: &str = "205881";
    const PERIOD: Duration = Duration::from_secs(30);

    fn ring(remaining: u64, masked: bool, assistive: bool) -> OtpRing {
        OtpRing::new("otp", Duration::from_secs(remaining), PERIOD)
            .code(Zeroizing::new(CODE.to_string()))
            .next_code(Zeroizing::new(NEXT.to_string()))
            .masked(masked)
            .assistive(assistive)
    }

    fn parts_of(cx: &mut TestAppContext, build: impl Fn() -> OtpRing + 'static) -> Vec<Part> {
        probe::in_frame(cx, move |window, cx| build().parts(window, cx))
    }

    fn named<'a>(parts: &'a [Part], name: &str) -> Option<&'a Part> {
        parts.iter().find(|part| part.name == name)
    }

    /// The ring is a timer named with the seconds left, rounded up to a
    /// multiple of five, so the name changes at most every five seconds.
    #[gpui::test]
    fn the_ring_is_a_timer_named_with_rounded_seconds(cx: &mut TestAppContext) {
        for (seconds, announced) in [(18, 20), (13, 15), (4, 5), (0, 0), (20, 20), (21, 25)] {
            assert_eq!(
                announced_seconds(Duration::from_secs(seconds)),
                announced,
                "{seconds} s"
            );
        }
        let almost = Duration::from_millis(17_300);
        assert_eq!(shown_seconds(almost), 18, "the seconds shown round up");
        assert_eq!(announced_seconds(almost), 20);
        // The name stays the same from 20 down to 16 seconds left.
        let names: Vec<u64> = (16..=20)
            .map(|seconds| announced_seconds(Duration::from_secs(seconds)))
            .collect();
        assert_eq!(names, [20; 5]);

        let parts = parts_of(cx, || ring(18, true, false));
        let timer = named(&parts, "timer").expect("the timer");
        assert_eq!(timer.role, Some(Role::Timer));
        assert!(timer.id.is_some(), "an id, so it reaches the tree");
        // The English of `a11y.one_time_code_expires`, written out.
        assert_eq!(
            timer.node.label(),
            Some("One-time code, expires in 20 seconds")
        );
    }

    /// Every point of a path, as the turn clockwise from the top of
    /// `bounds`' center and the distance from it.
    fn turns_and_distances(path: &Path<Pixels>, bounds: Bounds<Pixels>) -> Vec<(f32, f32)> {
        let center = bounds.center();
        path.vertices
            .iter()
            .map(|vertex| {
                let x = f32::from(vertex.xy_position.x - center.x);
                let y = f32::from(vertex.xy_position.y - center.y);
                let turn = x.atan2(-y).rem_euclid(TAU) / TAU;
                (turn, x.hypot(y))
            })
            .collect()
    }

    /// The time left runs in `otp_ring`, and in `otp_ring_warn` below five
    /// seconds, over the track in `otp_track`. The test reads the paths the
    /// canvas paints, as `ring_paths` builds them, since the test platform
    /// keeps the paths of a frame to itself: every point of both lies on the
    /// ring, the track goes all the way round, and the time left covers as
    /// much of the turn as is left, clockwise from the top.
    #[gpui::test]
    fn the_ring_warns_below_five_seconds(cx: &mut TestAppContext) {
        let colors = &LIGHT;
        let bounds = Bounds::new(point(px(10.), px(20.)), gpui::size(px(36.), px(36.)));
        let stroke = dimension::STROKE_OTP;
        let radius = (36. - stroke) / 2.;
        for (millis, running) in [
            (6_000, colors.otp_ring.hsla),
            (5_000, colors.otp_ring.hsla),
            (4_900, colors.otp_ring_warn.hsla),
            (1_000, colors.otp_ring_warn.hsla),
        ] {
            let remaining = Duration::from_millis(millis);
            let paths = ring_paths(bounds, remaining, PERIOD, colors);
            assert_eq!(paths.len(), 2, "{millis} ms: the track and the time left");
            assert_eq!(paths[0].1, colors.otp_track.hsla, "{millis} ms: the track");
            assert_eq!(paths[1].1, running, "{millis} ms: the time left");
            let left = remaining.as_secs_f32() / PERIOD.as_secs_f32();
            for (index, (path, _)) in paths.iter().enumerate() {
                let points = turns_and_distances(path, bounds);
                assert!(!points.is_empty(), "{millis} ms: path {index} has points");
                for (turn, distance) in &points {
                    assert!(
                        (distance - radius).abs() <= stroke / 2. + 0.5,
                        "{millis} ms: path {index} leaves the ring at {distance} px"
                    );
                    let end = if index == 0 { 1. } else { left };
                    assert!(
                        *turn <= end + 0.01 || *turn >= 0.99,
                        "{millis} ms: path {index} runs past {end} of the turn, to {turn}"
                    );
                }
                // A point just before the top belongs to the start.
                let mut turns: Vec<f32> = points
                    .iter()
                    .map(|(turn, _)| if *turn >= 0.99 { turn - 1. } else { *turn })
                    .collect();
                turns.sort_by(f32::total_cmp);
                let (first, last) = (turns[0], turns[turns.len() - 1]);
                let mut gaps: Vec<f32> = turns.windows(2).map(|pair| pair[1] - pair[0]).collect();
                if index == 0 {
                    // All the way round, across the top as well.
                    gaps.push(1. + first - last);
                } else {
                    assert!(first.abs() <= 0.01, "{millis} ms: starts at {first}");
                    assert!(
                        (last - left).abs() <= 0.01,
                        "{millis} ms: ends at {last}, not {left}"
                    );
                }
                let widest = gaps.iter().copied().fold(0., f32::max);
                assert!(
                    widest < 0.05,
                    "{millis} ms: path {index} has a gap of {widest} of the turn"
                );
            }
        }
        assert_eq!(
            ring_paths(bounds, Duration::ZERO, PERIOD, colors).len(),
            1,
            "nothing left: the track alone"
        );

        // The canvas draws the ring in a window, 16 points across in a row.
        let cx = probe::window(cx, |_, _| ring(3, false, false).into_any_element());
        assert!(cx.debug_bounds("fp-otp-seconds").is_some(), "it draws");
    }

    /// Masked, the code shows as "••• •••" whatever it is, the next code
    /// does not show, and no node holds either, with or without assistive
    /// technology.
    #[gpui::test]
    fn a_masked_code_never_enters_the_tree(cx: &mut TestAppContext) {
        assert_eq!(shown_code(CODE, true), MASKED_CODE);
        assert_eq!(shown_code("12345678", true), MASKED_CODE);
        assert_eq!(shown_code(CODE, false), "739 114");
        assert_eq!(shown_code("12345678", false), "123 456 78");
        for assistive in [false, true] {
            for remaining in [18, 3] {
                let parts = parts_of(cx, move || ring(remaining, true, assistive));
                assert!(
                    named(&parts, "next_code").is_none(),
                    "assistive: {assistive}, {remaining} s: no next code"
                );
                for part in &parts {
                    for (property, said) in part.texts() {
                        let said = said.unwrap_or_default();
                        for code in [CODE, "739 114", "739", "114", NEXT, "205 881", "205", "881"] {
                            assert!(
                                !said.contains(code),
                                "assistive: {assistive}: the {property} of the {} holds {code}",
                                part.name
                            );
                        }
                    }
                }
            }
        }
    }

    /// The code is the timer's value only while unmasked and while
    /// assistive technology runs; in the three other cases the timer has no
    /// value.
    #[gpui::test]
    fn a_code_enters_the_tree_only_revealed_with_assistive(cx: &mut TestAppContext) {
        for (masked, assistive, value) in [
            (true, false, None),
            (true, true, None),
            (false, false, None),
            (false, true, Some("739 114")),
        ] {
            let parts = parts_of(cx, move || ring(18, masked, assistive));
            let timer = named(&parts, "timer").expect("the timer");
            assert_eq!(
                timer.node.value(),
                value,
                "masked: {masked}, assistive: {assistive}"
            );
        }
    }

    /// Unmasked, the next code stands beside the current one in the last
    /// five seconds, and not before.
    #[gpui::test]
    fn the_next_code_appears_in_the_last_five_seconds(cx: &mut TestAppContext) {
        for (millis, shows) in [(18_000, false), (5_000, false), (4_900, true), (0, true)] {
            let remaining = Duration::from_millis(millis);
            let build = move || {
                OtpRing::new("otp", remaining, PERIOD)
                    .code(Zeroizing::new(CODE.to_string()))
                    .next_code(Zeroizing::new(NEXT.to_string()))
                    .masked(false)
            };
            let parts = parts_of(cx, build);
            assert_eq!(
                named(&parts, "next_code").is_some(),
                shows,
                "{millis} ms left"
            );
            let cx = probe::window(cx, move |_, _| build().into_any_element());
            assert_eq!(
                cx.debug_bounds("fp-otp-next-code").is_some(),
                shows,
                "{millis} ms left, drawn"
            );
        }
        let parts = parts_of(cx, || {
            OtpRing::new("otp", Duration::from_secs(3), PERIOD)
                .code(Zeroizing::new(CODE.to_string()))
                .masked(false)
        });
        assert!(named(&parts, "next_code").is_none(), "no next code given");
    }
}
