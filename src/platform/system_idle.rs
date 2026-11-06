//! How long the Mac has been idle: the seconds since the last input of any
//! kind in the login session. The auto-lock can go by this instead of
//! FerrisPass's own idle clock, for people who keep FerrisPass open while they
//! work in other apps (`lock_is_due_with` in `src/app/services/activity.rs`).
//!
//! The number comes from CoreGraphics,
//! `CGEventSourceSecondsSinceLastEventType`, asked for the combined session
//! state and any input event type. It needs no permission. No crate we depend
//! on offers it as a safe function, so the `core_graphics` module below
//! declares it itself, and calling a foreign function is `unsafe`. That one
//! declaration and its one call are the whole reason this file allows
//! `unsafe_code`: two integer constants go in, an `f64` comes out. There is no
//! pointer, nothing is retained or released, there is no callback and no
//! state. The header puts no thread rule on the function; it reads a value
//! the window server keeps for the session, and any thread may ask.
//!
//! What counts as input: every key press, click, mouse and trackpad movement
//! and scroll in the login session, in any app, and also input that a program
//! posts to the session as if it were typed or moved. That includes
//! FerrisPass's own Auto-Type, a presentation remote, remote-control software
//! and a "mouse jiggler" that keeps the Mac awake: while one of those runs,
//! the Mac never looks idle. That is the limit of this clock, and the reason
//! it is an alternative the user picks and not the default. What does not
//! count: anything outside this login session, such as input in another
//! user's session after a fast user switch.
//!
//! The reading is checked before it is used. A value that is not a number,
//! negative or infinite is [`IdleError::Invalid`]; where there is no
//! CoreGraphics the answer is [`IdleError::Unsupported`]. The caller then
//! falls back to its own clock with the same threshold; an error never means
//! "not idle".
//!
//! Code that has a GPUI `App` asks [`idle_for`]. A test puts a fake there as a
//! global on its own `App` (`fake::install`). Without one, a test build
//! answers `Unsupported`, so no unit test depends on whether someone is using
//! the Mac that runs it.

#![allow(unsafe_code)]

use std::time::Duration;

use gpui::App;

/// Why [`idle_for`] has no idle time to give.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum IdleError {
    #[error("the idle time of the Mac is not available on this platform")]
    Unsupported,
    #[error("macOS reported an idle time that is not a number of seconds")]
    Invalid,
}

/// What this module needs from the system: CoreGraphics on macOS, the no-op
/// elsewhere, and a fake in tests.
pub(crate) trait SystemIdleApi: Sync {
    /// The seconds since the last input in the login session, as the system
    /// reports them, before any check.
    fn seconds_since_last_input(&self) -> Result<f64, IdleError>;
}

/// How long the login session has gone without input. An error says why
/// there is no answer; the caller falls back to its own clock and does not
/// read an error as "not idle".
pub fn idle_for(cx: &App) -> Result<Duration, IdleError> {
    checked(system(cx).seconds_since_last_input()?)
}

/// A reading as a `Duration`. NaN, a negative value, infinity and anything
/// too large for a `Duration` are `Invalid`; `Duration::from_secs_f64` would
/// panic on them.
fn checked(seconds: f64) -> Result<Duration, IdleError> {
    if !seconds.is_finite() || seconds < 0.0 {
        return Err(IdleError::Invalid);
    }
    Duration::try_from_secs_f64(seconds).map_err(|_| IdleError::Invalid)
}

/// What `idle_for` reads when no test has put a fake in place.
#[cfg(all(target_os = "macos", not(test)))]
const SYSTEM: &dyn SystemIdleApi = &core_graphics::CoreGraphics;
/// Test builds read the no-op unless a test installs a fake, so the real idle
/// time of the Mac never reaches a unit test through the `App`.
/// `the_real_call_answers_on_macos` calls CoreGraphics directly instead.
#[cfg(any(test, not(target_os = "macos")))]
const SYSTEM: &dyn SystemIdleApi = &noop::Noop;

fn system(#[cfg_attr(not(test), allow(unused_variables))] cx: &App) -> &dyn SystemIdleApi {
    #[cfg(test)]
    if let Some(installed) = cx.try_global::<fake::Installed>() {
        return installed;
    }
    SYSTEM
}

/// The real call through the library as the app builds it, outside the test
/// build, so `idle_for` reads CoreGraphics and not the no-op. Whoever runs the
/// tests has used the Mac within the last day.
///
/// ```
/// # #[cfg(target_os = "macos")]
/// # {
/// use std::time::Duration;
///
/// let cx = gpui::TestAppContext::single();
/// let idle = cx
///     .update(|cx| ferrispass::platform::system_idle::idle_for(cx))
///     .expect("CoreGraphics reports an idle time");
/// assert!(idle < Duration::from_secs(24 * 60 * 60), "idle for {idle:?}");
/// # }
/// ```
#[cfg(doctest)]
pub fn idle_for_reads_core_graphics() {}

/// CoreGraphics' idle time, declared in `CGEventSource.h` as
/// `CFTimeInterval CGEventSourceSecondsSinceLastEventType(CGEventSourceStateID
/// stateID, CGEventType eventType)`, where `CFTimeInterval` is a `double`,
/// `CGEventSourceStateID` a 32-bit signed and `CGEventType` a 32-bit unsigned
/// integer. `kCGEventSourceStateCombinedSessionState` is 0 and
/// `kCGAnyInputEventType` is `~0`.
#[cfg(target_os = "macos")]
mod core_graphics {
    use super::IdleError;

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventSourceSecondsSinceLastEventType(state: i32, event_type: u32) -> f64;
    }

    /// `kCGEventSourceStateCombinedSessionState`: every source that posts to
    /// this login session, the hardware and programs alike.
    const COMBINED_SESSION_STATE: i32 = 0;
    /// `kCGAnyInputEventType`: keyboard, mouse and tablet events.
    const ANY_INPUT_EVENT_TYPE: u32 = u32::MAX;

    /// The raw reading. It is never an error here; `checked` decides whether
    /// the number can be used.
    pub(super) fn seconds_since_last_input() -> Result<f64, IdleError> {
        // SAFETY: a plain call with two integer constants that the header
        // defines, and a `double` back. The declaration above matches the C
        // signature. Nothing is passed by pointer, nothing is retained or
        // released, no callback is registered, and the function has no
        // thread requirement.
        let seconds = unsafe {
            CGEventSourceSecondsSinceLastEventType(COMBINED_SESSION_STATE, ANY_INPUT_EVENT_TYPE)
        };
        Ok(seconds)
    }

    #[cfg(not(test))]
    pub(super) struct CoreGraphics;

    #[cfg(not(test))]
    impl super::SystemIdleApi for CoreGraphics {
        fn seconds_since_last_input(&self) -> Result<f64, IdleError> {
            seconds_since_last_input()
        }
    }
}

/// Every platform other than macOS, and test builds without a fake. There is
/// no idle time to read, so the answer is `Unsupported`, never a made-up
/// number.
#[cfg(any(test, not(target_os = "macos")))]
mod noop {
    use super::{IdleError, SystemIdleApi};

    pub(super) struct Noop;

    impl SystemIdleApi for Noop {
        fn seconds_since_last_input(&self) -> Result<f64, IdleError> {
            Err(IdleError::Unsupported)
        }
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use super::*;

    /// The reading a test gave its `App`. A global on that `App` and nowhere
    /// else, so tests that run side by side, each with its own context, never
    /// see each other's value.
    pub(super) struct Installed(Result<f64, IdleError>);

    impl gpui::Global for Installed {}

    impl SystemIdleApi for Installed {
        fn seconds_since_last_input(&self) -> Result<f64, IdleError> {
            self.0
        }
    }

    /// From here on, `idle_for(cx)` reads `reading` as if CoreGraphics had
    /// returned it, checks included. Installing again replaces the reading.
    pub(crate) fn install(cx: &mut App, reading: Result<f64, IdleError>) {
        cx.set_global(Installed(reading));
    }

    /// The same for a whole `Duration`.
    pub(crate) fn install_idle(cx: &mut App, idle: Duration) {
        install(cx, Ok(idle.as_secs_f64()));
    }
}

#[cfg(test)]
mod tests {
    use gpui::TestAppContext;

    use super::*;

    const FIFTEEN_MINUTES: Duration = Duration::from_secs(15 * 60);

    #[gpui::test]
    fn a_nan_or_negative_reading_is_an_error(cx: &mut TestAppContext) {
        let read = |cx: &mut TestAppContext, reading: f64| {
            cx.update(|cx| fake::install(cx, Ok(reading)));
            cx.update(|cx| idle_for(cx))
        };
        for reading in [f64::NAN, -1.0, f64::INFINITY, f64::NEG_INFINITY, 1e300] {
            assert_eq!(
                read(cx, reading),
                Err(IdleError::Invalid),
                "{reading} from the system was taken as an idle time"
            );
        }
        assert_eq!(read(cx, 0.0), Ok(Duration::ZERO));
        assert_eq!(read(cx, 90.5), Ok(Duration::from_millis(90_500)));
    }

    #[gpui::test]
    fn system_idle_is_unsupported_off_macos(cx: &mut TestAppContext) {
        // The no-op is what every build other than macOS reads, and what a
        // test build reads without a fake.
        assert_eq!(
            noop::Noop.seconds_since_last_input(),
            Err(IdleError::Unsupported)
        );
        assert_eq!(cx.update(|cx| idle_for(cx)), Err(IdleError::Unsupported));
    }

    /// CoreGraphics itself, on a libtest worker thread. Whoever runs the
    /// tests has used the Mac within the last day.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_real_call_answers_on_macos() {
        let seconds =
            core_graphics::seconds_since_last_input().expect("CoreGraphics reports an idle time");
        let idle = checked(seconds).unwrap_or_else(|error| panic!("{seconds}: {error}"));
        assert!(
            idle < Duration::from_secs(24 * 60 * 60),
            "idle for {idle:?}"
        );
    }

    #[gpui::test]
    fn the_fake_is_read_through_the_app(first: &mut TestAppContext, second: &mut TestAppContext) {
        assert_eq!(
            first.update(|cx| idle_for(cx)),
            Err(IdleError::Unsupported),
            "a test build without a fake read something other than the no-op"
        );
        first.update(|cx| fake::install_idle(cx, Duration::ZERO));
        second.update(|cx| fake::install_idle(cx, FIFTEEN_MINUTES));
        assert_eq!(first.update(|cx| idle_for(cx)), Ok(Duration::ZERO));
        assert_eq!(second.update(|cx| idle_for(cx)), Ok(FIFTEEN_MINUTES));

        first.update(|cx| fake::install(cx, Err(IdleError::Unsupported)));
        assert_eq!(first.update(|cx| idle_for(cx)), Err(IdleError::Unsupported));
        assert_eq!(second.update(|cx| idle_for(cx)), Ok(FIFTEEN_MINUTES));
    }

    /// The one foreign function, word for word as it is meant to be, with
    /// the `extern` block around it written out here.
    const DECLARATION: &str =
        "fn CGEventSourceSecondsSinceLastEventType(state: i32, event_type: u32) -> f64;";
    /// The arguments of the one call.
    const CALL_ARGUMENTS: &str = "(COMBINED_SESSION_STATE, ANY_INPUT_EVENT_TYPE)";

    #[test]
    fn system_idle_declares_one_foreign_function_and_one_unsafe_call() {
        let tokens = code_tokens(include_str!("system_idle.rs"));
        let unsafe_at: Vec<usize> = positions(&tokens, |token| *token == Tok::Ident("unsafe"));
        assert_eq!(
            unsafe_at.len(),
            2,
            "`unsafe` has to appear twice, for the extern block and the call"
        );

        // `unsafe extern "C" { <DECLARATION> }`
        let block = unsafe_at[0];
        assert_eq!(tokens.get(block + 1), Some(&Tok::Ident("extern")));
        assert_eq!(tokens.get(block + 2), Some(&Tok::Str("C")));
        assert_eq!(tokens.get(block + 3), Some(&Tok::Punct('{')));
        let declaration = code_tokens(DECLARATION);
        let body = &tokens[block + 4..];
        assert_eq!(
            body.get(..declaration.len()),
            Some(declaration.as_slice()),
            "the extern block declares something other than {DECLARATION}"
        );
        assert_eq!(body.get(declaration.len()), Some(&Tok::Punct('}')));

        // `unsafe { CGEventSourceSecondsSinceLastEventType(<CALL_ARGUMENTS>) }`
        let call = unsafe_at[1];
        let arguments = code_tokens(CALL_ARGUMENTS);
        let expected: Vec<Tok> = [
            Tok::Punct('{'),
            Tok::Ident("CGEventSourceSecondsSinceLastEventType"),
        ]
        .into_iter()
        .chain(arguments)
        .chain([Tok::Punct('}')])
        .collect();
        assert_eq!(
            tokens.get(call + 1..call + 1 + expected.len()),
            Some(expected.as_slice()),
            "the unsafe block does more than the one call with the two constants"
        );

        for constant in [
            "const COMBINED_SESSION_STATE: i32 = 0;",
            "const ANY_INPUT_EVENT_TYPE: u32 = u32::MAX;",
        ] {
            assert_eq!(
                runs(&tokens, &code_tokens(constant)),
                1,
                "the constant is not `{constant}`"
            );
        }
        let mentions = |name: &str| runs(&tokens, &[Tok::Ident(name)]);
        assert_eq!(mentions("extern"), 1, "a second extern block");
        assert_eq!(
            mentions("CGEventSourceSecondsSinceLastEventType"),
            2,
            "the function is declared once and called once"
        );
        for name in ["into_raw", "from_raw", "transmute"] {
            assert_eq!(mentions(name), 0, "`{name}` has no place here");
        }
        let retains = tokens.iter().any(|token| {
            matches!(token, Tok::Ident(name)
                if name.contains("Retain") || name.contains("Release") || *name == "retain")
        });
        assert!(!retains, "nothing here retains or releases an object");
        for (pair, what) in [
            (
                [Tok::Ident("as"), Tok::Punct('*')],
                "a cast to a raw pointer",
            ),
            ([Tok::Punct('*'), Tok::Ident("const")], "a raw pointer type"),
            ([Tok::Punct('*'), Tok::Ident("mut")], "a raw pointer type"),
            ([Tok::Ident("static"), Tok::Ident("mut")], "a `static mut`"),
        ] {
            assert_eq!(runs(&tokens, &pair), 0, "{what}");
        }
    }

    #[test]
    fn the_scan_skips_comments_and_strings() {
        let tokens = code_tokens(
            "// unsafe\n/* unsafe /* nested */ unsafe */ \"unsafe\\\" { \" r#\"unsafe\"# 'x' b'\\'' \
             fn f<'a>(x: &'a u8) {}",
        );
        assert!(!tokens.contains(&Tok::Ident("unsafe")), "{tokens:?}");
        assert_eq!(tokens.first(), Some(&Tok::Str("unsafe\\\" { ")));
        assert!(tokens.contains(&Tok::Ident("fn")));
        assert!(tokens.contains(&Tok::Punct('{')));
    }

    /// A token of code, for the scan above: a name, a string's contents, a
    /// single punctuation character, or anything else as written (numbers,
    /// characters, lifetimes). Comments are dropped, so a word in one is
    /// never code; a string becomes one token, so a word in one is never a
    /// name.
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Tok<'s> {
        Ident(&'s str),
        Str(&'s str),
        Punct(char),
        Other(&'s str),
    }

    fn positions(tokens: &[Tok<'_>], wanted: impl Fn(&Tok<'_>) -> bool) -> Vec<usize> {
        (0..tokens.len()).filter(|&i| wanted(&tokens[i])).collect()
    }

    /// How often `run` appears in `tokens`, token for token.
    fn runs(tokens: &[Tok<'_>], run: &[Tok<'_>]) -> usize {
        tokens
            .windows(run.len())
            .filter(|window| *window == run)
            .count()
    }

    /// The tokens of `source`, after the tokenizer in `tests/architecture.rs`
    /// cut down to what this file contains.
    fn code_tokens(source: &str) -> Vec<Tok<'_>> {
        let bytes = source.as_bytes();
        let at = |i: usize| bytes.get(i).copied().unwrap_or(0);
        let ident = |c: u8| c == b'_' || c.is_ascii_alphanumeric();
        let mut tokens = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if c.is_ascii_whitespace() {
                i += 1;
            } else if c == b'/' && at(i + 1) == b'/' {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            } else if c == b'/' && at(i + 1) == b'*' {
                let mut depth = 0;
                loop {
                    match (at(i), at(i + 1)) {
                        (0, _) => panic!("a block comment that never ends"),
                        (b'/', b'*') => {
                            depth += 1;
                            i += 2;
                        }
                        (b'*', b'/') => {
                            depth -= 1;
                            i += 2;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => i += 1,
                    }
                }
            } else if c == b'r' && at(i + 1) == b'#' && at(i + 2) == b'"' {
                let from = i + 3;
                let end = source[from..]
                    .find("\"#")
                    .map(|n| from + n)
                    .expect("a raw string that never ends");
                tokens.push(Tok::Str(&source[from..end]));
                i = end + 2;
            } else if c == b'"' {
                let from = i + 1;
                i = from;
                while at(i) != b'"' {
                    assert!(i < bytes.len(), "a string that never ends");
                    i += if at(i) == b'\\' { 2 } else { 1 };
                }
                tokens.push(Tok::Str(&source[from..i]));
                i += 1;
            } else if c == b'\'' || (c == b'b' && at(i + 1) == b'\'') {
                let from = i;
                if c == b'b' {
                    i += 1;
                }
                if at(i + 1) == b'\\' {
                    i += 3;
                    while at(i) != b'\'' {
                        assert!(i < bytes.len(), "a character literal that never ends");
                        i += 1;
                    }
                    i += 1;
                } else if at(i + 2) == b'\'' {
                    i += 3;
                } else {
                    // A lifetime or a label.
                    i += 1;
                    while ident(at(i)) {
                        i += 1;
                    }
                }
                tokens.push(Tok::Other(&source[from..i]));
            } else if c == b'_' || c.is_ascii_alphabetic() {
                let from = i;
                while ident(at(i)) {
                    i += 1;
                }
                tokens.push(Tok::Ident(&source[from..i]));
            } else if c.is_ascii_digit() {
                let from = i;
                while ident(at(i)) || (at(i) == b'.' && at(i + 1).is_ascii_digit()) {
                    i += 1;
                }
                tokens.push(Tok::Other(&source[from..i]));
            } else if c.is_ascii() {
                tokens.push(Tok::Punct(c as char));
                i += 1;
            } else {
                // Outside comments and strings, this file is ASCII.
                let width = source[i..].chars().next().map_or(1, char::len_utf8);
                tokens.push(Tok::Other(&source[i..i + width]));
                i += width;
            }
        }
        tokens
    }
}
