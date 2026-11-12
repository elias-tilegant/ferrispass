//! Motion: how long a change takes on screen, and whether anything moves.
//!
//! Every animation takes its time from [`duration`], with one of five
//! [`Token`]s and the kind of [`Input`] that caused it. Two rules sit on top
//! of the tokens:
//!
//! - After a key press every duration is zero. Someone at the keyboard
//!   waits for nothing.
//! - With Reduce Motion on in macOS, a transition is at most a fade of
//!   [`REDUCED_FADE`], and nothing changes place: no springs, no sliding.
//!   [`springs_allowed`] says whether a spring or any move may run.
//!
//! Rings and threads that count time keep running either way: they are
//! information, and their timers are not durations.
//!
//! Reduce Motion is read from `cx.reduce_motion()`. `ds::apply` sets it from
//! [`A11yPrefs`](super::A11yPrefs) every time it runs, since gpui-base reads
//! the macOS setting only once, at startup.
//!
//! `with_animation` and `with_spring` appear only under `src/ui/ds/`, so
//! every motion in the app passes through here; a test holds the rest of
//! `src/` to that.

use std::collections::HashSet;
use std::time::Duration;

use gpui::{App, Global};
use gpui_base::Spring;

/// The steps of the motion scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Token {
    /// Selection, hover, filtering, the quick access panel: no motion.
    Instant,
    /// A copy by click, a new password from the dice.
    Micro,
    /// Popovers, the HUD, a one-time code changing digits.
    Short,
    /// Unlocking, a version sliding into the history, the shell closing.
    Medium,
    /// Onboarding and Ferris, such as waking up after an unlock.
    Long,
}

impl Token {
    pub const ALL: [Token; 5] = [
        Token::Instant,
        Token::Micro,
        Token::Short,
        Token::Medium,
        Token::Long,
    ];

    /// The time of the step, before input and Reduce Motion have a say.
    pub const fn base(self) -> Duration {
        Duration::from_millis(match self {
            Token::Instant => 0,
            Token::Micro => 120,
            Token::Short => 180,
            Token::Medium => 240,
            Token::Long => 320,
        })
    }
}

/// What caused a change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Input {
    /// A key press, typed text and shortcuts alike.
    Keyboard,
    /// A click, a scroll or the pointer moving.
    Pointer,
    /// Nothing the user did in the window: a timer, the app starting, a
    /// Touch ID prompt that succeeded.
    System,
}

/// The longest a transition takes with Reduce Motion on. It fades, it does
/// not move.
pub const REDUCED_FADE: Duration = Duration::from_millis(120);

/// How long the HUD shows a success.
pub const HOLD_HUD_SUCCESS: Duration = Duration::from_millis(1500);
/// How long a copy button shows its check mark.
pub const HOLD_COPY_CHECK: Duration = Duration::from_millis(1200);
/// How long work runs before a spinner or a progress bar appears.
pub const DELAY_PROGRESS: Duration = Duration::from_millis(400);
/// Frames per second of the copper thread that counts down a copied secret.
pub const TIMER_THREAD_HZ: u32 = 12;
/// Updates per second of the one-time code ring.
pub const TIMER_OTP_HZ: u32 = 10;
/// Updates per second of the menu bar fill.
pub const TIMER_MENUBAR_HZ: u32 = 1;

/// How long a change of `token` takes when `input` caused it: zero after a
/// key press, at most [`REDUCED_FADE`] with Reduce Motion, the token's time
/// otherwise.
pub fn duration(token: Token, input: Input, cx: &App) -> Duration {
    if input == Input::Keyboard {
        return Duration::ZERO;
    }
    if cx.reduce_motion() {
        return token.base().min(REDUCED_FADE);
    }
    token.base()
}

/// The soft spring: sidebar, columns, the strength meter. Critically
/// damped, it settles without passing its target.
pub const fn spring_soft() -> Spring {
    Spring::new(Duration::from_millis(350)).with_damping(1.0)
}

/// The firm spring, for things that snap into place. It passes its target
/// by a little.
pub const fn spring_firm() -> Spring {
    Spring::new(Duration::from_millis(300)).with_damping(0.9)
}

/// Whether a spring or a change of place may run. With Reduce Motion the
/// caller shows the end state at once, or fades to it.
pub fn springs_allowed(cx: &App) -> bool {
    !cx.reduce_motion()
}

/// The kind of input the windows saw last.
#[derive(Clone, Copy)]
struct LastInput(Input);

impl Global for LastInput {}

/// Called by the windows' root handlers for every key press, click, scroll
/// and pointer move. Sets the global only when the kind changes, so a moving
/// pointer does not notify on every event.
pub fn note_input(input: Input, cx: &mut App) {
    if last_input(cx) != input {
        cx.set_global(LastInput(input));
    }
}

/// The kind of input the windows saw last, [`Input::System`] before any.
pub fn last_input(cx: &App) -> Input {
    cx.try_global::<LastInput>()
        .map_or(Input::System, |last| last.0)
}

/// The occasions Ferris has a moment for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Occasion {
    /// Ferris waves on the very first start.
    FirstStart,
    /// A small cheer the first time something succeeds.
    Success,
    /// Ferris wakes up on the lock screen as the vault opens.
    Wake,
}

/// Lets each of Ferris's moments run once: the first start and the success
/// once at all, waking up once per unlock.
#[derive(Debug, Default)]
pub struct Moment {
    ran: HashSet<Occasion>,
    unlock: u64,
}

impl Moment {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the moment for `occasion` runs, and for how long. `input` is
    /// what finished the step the moment belongs to: for waking up, the
    /// input that completed the unlock, so a Touch ID success is
    /// [`Input::System`] even when Return opened the prompt, and Return in
    /// the password field is [`Input::Keyboard`].
    ///
    /// Asking uses the occasion up, whether the moment runs or not: after a
    /// key press or with Reduce Motion the answer is `None`, and Ferris
    /// shows the end of the moment at once. Waking up belongs to an unlock,
    /// so before the first [`Self::begin_unlock`] it never runs.
    pub fn may_run(&mut self, occasion: Occasion, input: Input, cx: &App) -> Option<Duration> {
        if occasion == Occasion::Wake && self.unlock == 0 {
            return None;
        }
        if !self.ran.insert(occasion) {
            return None;
        }
        if input == Input::Keyboard || cx.reduce_motion() {
            return None;
        }
        Some(duration(Token::Long, input, cx))
    }

    /// Called as the vault unlocks, before asking for [`Occasion::Wake`]:
    /// waking up may run once more. The first start and the success stay
    /// used up.
    pub fn begin_unlock(&mut self) {
        self.unlock += 1;
        self.ran.remove(&Occasion::Wake);
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use gpui::TestAppContext;

    use super::super::components::probe;
    use super::*;

    const INPUTS: [Input; 3] = [Input::Keyboard, Input::Pointer, Input::System];

    #[gpui::test]
    fn duration_is_zero_after_keyboard_input(cx: &mut TestAppContext) {
        cx.update(|cx| {
            for reduce_motion in [false, true] {
                cx.set_reduce_motion(reduce_motion);
                for token in Token::ALL {
                    assert_eq!(
                        duration(token, Input::Keyboard, cx),
                        Duration::ZERO,
                        "{token:?}, reduce motion {reduce_motion}"
                    );
                }
            }
        });
    }

    #[gpui::test]
    fn duration_is_a_short_fade_with_reduce_motion(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_reduce_motion(true);
            for input in [Input::Pointer, Input::System] {
                for token in Token::ALL {
                    let time = duration(token, input, cx);
                    assert!(time <= REDUCED_FADE, "{token:?} after {input:?}: {time:?}");
                }
                assert_eq!(duration(Token::Instant, input, cx), Duration::ZERO);
                assert_eq!(duration(Token::Long, input, cx), REDUCED_FADE);
            }
        });
    }

    #[gpui::test]
    fn duration_follows_the_token_for_pointer_input_without_reduce_motion(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_reduce_motion(false);
            for input in [Input::Pointer, Input::System] {
                let times: Vec<u128> = Token::ALL
                    .iter()
                    .map(|&token| duration(token, input, cx).as_millis())
                    .collect();
                assert_eq!(times, [0, 120, 180, 240, 320], "after {input:?}");
            }
        });
    }

    /// The springs are the ones the design names, and with Reduce Motion
    /// none may run. `Spring` has no accessors for its period and damping,
    /// so they are compared through their debug output.
    #[gpui::test]
    fn springs_are_off_with_reduce_motion(cx: &mut TestAppContext) {
        let soft = format!("{:?}", spring_soft());
        let firm = format!("{:?}", spring_firm());
        assert!(
            soft.contains("350ms") && soft.contains("damping: 1.0"),
            "{soft}"
        );
        assert!(
            firm.contains("300ms") && firm.contains("damping: 0.9"),
            "{firm}"
        );
        cx.update(|cx| {
            cx.set_reduce_motion(false);
            assert!(springs_allowed(cx));
            cx.set_reduce_motion(true);
            assert!(!springs_allowed(cx));
        });
    }

    #[gpui::test]
    fn a_moment_runs_once_per_occasion(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_reduce_motion(false);
            let mut moment = Moment::new();
            moment.begin_unlock();
            for occasion in [Occasion::FirstStart, Occasion::Success, Occasion::Wake] {
                assert_eq!(
                    moment.may_run(occasion, Input::Pointer, cx),
                    Some(Token::Long.base()),
                    "{occasion:?} the first time"
                );
                for input in INPUTS {
                    assert_eq!(
                        moment.may_run(occasion, input, cx),
                        None,
                        "{occasion:?} again after {input:?}"
                    );
                }
            }

            // A moment that did not run, after a key press or with Reduce
            // Motion, is used up as well: Ferris showed its end at once.
            let mut moment = Moment::new();
            assert_eq!(
                moment.may_run(Occasion::FirstStart, Input::Keyboard, cx),
                None
            );
            assert_eq!(
                moment.may_run(Occasion::FirstStart, Input::Pointer, cx),
                None
            );
            cx.set_reduce_motion(true);
            assert_eq!(moment.may_run(Occasion::Success, Input::Pointer, cx), None);
            cx.set_reduce_motion(false);
            assert_eq!(moment.may_run(Occasion::Success, Input::Pointer, cx), None);
        });
    }

    #[gpui::test]
    fn the_wake_moment_runs_once_per_unlock(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_reduce_motion(false);
            let mut moment = Moment::new();
            assert_eq!(
                moment.may_run(Occasion::Wake, Input::System, cx),
                None,
                "no unlock yet"
            );
            assert!(
                moment
                    .may_run(Occasion::Success, Input::Pointer, cx)
                    .is_some()
            );
            for unlock in 1..=3 {
                moment.begin_unlock();
                assert_eq!(
                    moment.may_run(Occasion::Wake, Input::System, cx),
                    Some(Token::Long.base()),
                    "unlock {unlock}"
                );
                assert_eq!(
                    moment.may_run(Occasion::Wake, Input::System, cx),
                    None,
                    "a second time in unlock {unlock}"
                );
                assert_eq!(
                    moment.may_run(Occasion::Success, Input::Pointer, cx),
                    None,
                    "the success again after unlock {unlock}"
                );
            }
        });
    }

    #[gpui::test]
    fn the_wake_moment_runs_after_touch_id_but_not_after_return_in_the_password_field(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            cx.set_reduce_motion(false);
            let mut moment = Moment::new();
            // Return opened the Touch ID prompt; the unlock ends with the
            // prompt's success, which is no key press.
            moment.begin_unlock();
            assert_eq!(
                moment.may_run(Occasion::Wake, Input::System, cx),
                Some(Token::Long.base())
            );
            // The master password, sent with Return.
            moment.begin_unlock();
            assert_eq!(moment.may_run(Occasion::Wake, Input::Keyboard, cx), None);
            // Touch ID again, with Reduce Motion on.
            moment.begin_unlock();
            cx.set_reduce_motion(true);
            assert_eq!(moment.may_run(Occasion::Wake, Input::System, cx), None);
        });
    }

    /// The two calls that animate, built from pieces so that this file
    /// does not match them.
    const ANIMATIONS: [&str; 2] = [concat!("with_", "animation"), concat!("with_", "spring")];

    /// Each line of `code` that animates, as `file:line: text`.
    fn animations_in(file: &str, code: &str) -> Vec<String> {
        code.lines()
            .enumerate()
            .filter(|(_, line)| ANIMATIONS.iter().any(|call| line.contains(call)))
            .map(|(at, line)| format!("{file}:{}: {}", at + 1, line.trim()))
            .collect()
    }

    /// Every motion goes through this module: outside `src/ui/ds/`, no code
    /// in `src/` animates. Comments and inline test modules do not count,
    /// and neither do the files that only hold tests. A test that animates
    /// something outside the design system still goes through here.
    #[test]
    fn no_animation_outside_the_design_system() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let design_system = root.join("src/ui/ds");
        let mut files = Vec::new();
        probe::rust_files(&root.join("src"), &mut files);
        files.sort();
        let outside: Vec<_> = files
            .iter()
            .filter(|file| !file.starts_with(&design_system))
            .filter(|file| {
                let name = file.file_name().unwrap_or_default().to_string_lossy();
                name != "tests.rs" && !name.ends_with("_tests.rs")
            })
            .collect();
        assert!(outside.len() > 50, "the scan found {} files", outside.len());
        let found: Vec<String> = outside
            .iter()
            .flat_map(|file| {
                let source = std::fs::read_to_string(file).expect("read source");
                let name = file
                    .strip_prefix(root)
                    .unwrap_or(file)
                    .display()
                    .to_string();
                animations_in(&name, &probe::code_only(&source))
            })
            .collect();
        assert!(
            found.is_empty(),
            "animate through ui::ds::motion, not in place:\n{}",
            found.join("\n")
        );
        // The scan does see a call.
        let sample = format!("div().{}(id, animation, step)", ANIMATIONS[0]);
        assert_eq!(animations_in("sample.rs", &sample).len(), 1);
    }
}
