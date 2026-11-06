//! What you type into the quick access panel while it is still locked.
//!
//! The panel lets you start a search before Touch ID has unlocked the vault.
//! Those keystrokes go into a [`TypedAhead`] buffer, and the panel shows only
//! how many there are, as dots. The rules:
//!
//! - The text sits in a `Zeroizing<String>`, which is overwritten when it is
//!   wiped or dropped, spare capacity included. Growing it never leaves a
//!   copy behind: a full buffer is copied into a bigger one and the old one
//!   is overwritten as it drops.
//! - Secure event input is held for as long as the buffer is live, so other
//!   programs on the Mac don't see the keystrokes. If macOS refuses it,
//!   [`TypedAhead::begin`] fails and there is no buffer; the panel then sends
//!   you to the main window. Where the platform has no secure event input at
//!   all, the buffer works without it and claims nothing.
//! - It is wiped 30 seconds after it was started ([`EXPIRY`]), when you
//!   cancel, and when it looks like a password: more than one word and not a
//!   single match. The two checks that can wipe it,
//!   [`TypedAhead::expire_if_due`] and [`TypedAhead::wipe_if_password_like`],
//!   do the wiping themselves. There is no variant that only reports and
//!   leaves the wiping to the caller.
//! - The 30 seconds are wall-clock time, like the idle clock behind the
//!   auto-lock: a Mac that slept through them finds the buffer expired. A
//!   clock that went backwards counts as expired too.
//! - A wiped buffer stays empty, has let go of secure event input and takes
//!   no more input. The next keystroke needs a new buffer.
//! - [`TypedAhead::take_query`] hands the text over once, as the search the
//!   unlocked panel runs, and the buffer is gone with it. A search never
//!   marks an entry as recently used; only copying a value or launching an
//!   entry does.
//! - Nothing in this file logs, prints or writes the text, and `Debug` shows
//!   only the number of characters. `typed_ahead_never_logs_or_writes` in
//!   `tests/architecture.rs` reads this file to keep it that way.
//!
//! Who owns the buffer and calls these methods is the panel: it starts one
//! with the first keystroke, checks [`TypedAhead::expire_if_due`] on every
//! tick and keystroke and [`TypedAhead::wipe_if_password_like`] after every
//! search, and wipes it on Esc.

use std::fmt;
use std::time::{Duration, SystemTime};

use gpui::App;
use zeroize::{Zeroize as _, Zeroizing};

use crate::platform::secure_input::{self, HoldError, SecureInputGuard};

/// How long typed-ahead text is kept, counted from the first keystroke.
pub const EXPIRY: Duration = Duration::from_secs(30);

/// Room for a search of ordinary length before the buffer has to grow.
const INITIAL_CAPACITY: usize = 64;

/// What was typed into the locked panel. Not `Send`: it holds a secure input
/// guard, which stays on the main thread.
pub struct TypedAhead {
    text: Zeroizing<String>,
    started: SystemTime,
    guard: Option<SecureInputGuard>,
    wiped: bool,
}

impl TypedAhead {
    /// An empty buffer that holds secure event input from now on.
    ///
    /// A refusal by macOS, or a call off the main thread, is an error and
    /// there is no buffer: the panel must not collect keystrokes it cannot
    /// shield. Where the platform has no secure event input at all
    /// ([`HoldError::Unsupported`]), the buffer starts without a guard.
    pub fn begin(cx: &App) -> Result<Self, HoldError> {
        let guard = match secure_input::hold(cx) {
            Ok(guard) => Some(guard),
            Err(HoldError::Unsupported) => None,
            Err(error) => return Err(error),
        };
        Ok(Self {
            text: Zeroizing::new(String::with_capacity(INITIAL_CAPACITY)),
            started: SystemTime::now(),
            guard,
            wiped: false,
        })
    }

    /// Adds `c` at the end. Does nothing once the buffer is wiped.
    pub fn push(&mut self, c: char) {
        if self.wiped {
            return;
        }
        let needed = self.text.len() + c.len_utf8();
        if needed > self.text.capacity() {
            // `String::push` would reallocate and free the old allocation
            // without overwriting it. Copy into a bigger buffer instead; the
            // old one is overwritten as it drops.
            let mut grown =
                Zeroizing::new(String::with_capacity(needed.max(2 * self.text.capacity())));
            grown.push_str(&self.text);
            self.text = grown;
        }
        self.text.push(c);
    }

    /// Removes the last character. Its bytes stay in the spare capacity
    /// until the buffer is wiped or dropped, which overwrites all of it.
    pub fn backspace(&mut self) {
        self.text.pop();
    }

    /// How many characters were typed: the number of dots the panel shows.
    pub fn len(&self) -> usize {
        self.text.chars().count()
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Whether [`EXPIRY`] has passed at `now`. A `now` before the start,
    /// which only a clock that went backwards produces, counts as expired.
    pub fn expired(&self, now: SystemTime) -> bool {
        match now.duration_since(self.started) {
            Ok(elapsed) => elapsed >= EXPIRY,
            Err(_) => true,
        }
    }

    /// Wipes the buffer if it has expired at `now`, and says whether it did.
    /// The panel calls this on every tick and every keystroke; a caller that
    /// ignores the answer still leaves nothing behind.
    pub fn expire_if_due(&mut self, now: SystemTime) -> bool {
        if self.wiped || !self.expired(now) {
            return false;
        }
        self.wipe();
        true
    }

    /// Wipes the buffer if it looks like a password rather than a search:
    /// more than one word, and the search found nothing (`hits` is the number
    /// of matches). Says whether it wiped.
    pub fn wipe_if_password_like(&mut self, hits: usize) -> bool {
        if self.wiped || !is_password_like(&self.text, hits) {
            return false;
        }
        self.wipe();
        true
    }

    /// Overwrites the text and lets go of secure event input. Afterwards the
    /// buffer is empty and takes no more input.
    pub fn wipe(&mut self) {
        self.text.zeroize();
        self.guard = None;
        self.wiped = true;
    }

    /// Hands the text over as the search to run, and ends the buffer with it:
    /// secure event input goes as the guard drops. A wiped buffer hands over
    /// an empty string.
    pub fn take_query(self) -> Zeroizing<String> {
        let Self { text, guard, .. } = self;
        drop(guard);
        text
    }
}

impl fmt::Debug for TypedAhead {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TypedAhead({} chars)", self.len())
    }
}

/// More than one word, and nothing matched. A search is usually one or two
/// words that find something; a passphrase typed into the wrong window is
/// several words that find nothing.
fn is_password_like(query: &str, hits: usize) -> bool {
    hits == 0 && query.split_whitespace().nth(1).is_some()
}

/// The buffer holds a secure input guard, which has to stay on the main
/// thread, so the buffer can't leave it either. This must not compile:
///
/// ```compile_fail,E0277
/// fn assert_send<T: Send>() {}
/// assert_send::<ferrispass::app::unlock::typed_ahead::TypedAhead>();
/// ```
#[cfg(doctest)]
pub fn typed_ahead_is_not_send() {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::secure_input::OsStatus;
    use crate::platform::secure_input::fake::{self, Fake, FakeApi};
    use gpui::TestAppContext;

    fn install(cx: &mut TestAppContext, api: FakeApi) -> Fake {
        cx.update(|cx| fake::install(cx, api))
    }

    fn typed(cx: &mut TestAppContext, text: &str) -> TypedAhead {
        let mut buffer = cx.update(|cx| TypedAhead::begin(cx)).expect("begin");
        for c in text.chars() {
            buffer.push(c);
        }
        buffer
    }

    #[gpui::test]
    fn typed_ahead_holds_secure_input_while_it_lives(cx: &mut TestAppContext) {
        let fake = install(cx, FakeApi::new());

        let mut buffer = typed(cx, "nhq");
        assert!(fake.held(), "a live buffer has to hold secure input");
        assert_eq!(fake.api.enables(), 1);
        buffer.wipe();
        assert!(!fake.held(), "wiping the buffer left secure input on");
        assert_eq!(fake.api.disables(), 1);
        drop(buffer);
        assert_eq!(
            fake.api.disables(),
            1,
            "dropping a wiped buffer released a second time"
        );

        let buffer = typed(cx, "nhq");
        assert!(fake.held());
        assert_eq!(fake.api.enables(), 2);
        drop(buffer);
        assert!(!fake.held(), "dropping the buffer left secure input on");
        assert_eq!(fake.api.disables(), 2);
    }

    #[gpui::test]
    fn typed_ahead_expires_after_thirty_seconds(cx: &mut TestAppContext) {
        install(cx, FakeApi::new());
        let buffer = typed(cx, "nhq");
        let start = buffer.started;

        assert!(!buffer.expired(start));
        assert!(!buffer.expired(start + Duration::from_secs(29)));
        assert!(buffer.expired(start + Duration::from_secs(30)));
        assert!(buffer.expired(start + Duration::from_secs(3600)));
    }

    #[gpui::test]
    fn expire_if_due_wipes_the_buffer_and_releases_the_guard(cx: &mut TestAppContext) {
        let fake = install(cx, FakeApi::new());
        let mut buffer = typed(cx, "nhq");
        let start = buffer.started;

        assert!(!buffer.expire_if_due(start + Duration::from_secs(29)));
        assert_eq!(buffer.len(), 3, "a buffer that has not expired lost text");
        assert!(fake.held());

        assert!(buffer.expire_if_due(start + Duration::from_secs(30)));
        assert!(buffer.is_empty(), "an expired buffer still holds text");
        assert!(!fake.held(), "an expired buffer still holds secure input");
        assert_eq!(fake.api.disables(), 1);

        assert!(
            !buffer.expire_if_due(start + Duration::from_secs(31)),
            "a buffer is wiped once"
        );
        buffer.push('x');
        assert!(buffer.is_empty(), "a wiped buffer took more input");
    }

    #[gpui::test]
    fn a_clock_that_went_backwards_counts_as_expired(cx: &mut TestAppContext) {
        let fake = install(cx, FakeApi::new());
        let mut buffer = typed(cx, "nhq");
        let earlier = buffer.started - Duration::from_secs(1);

        assert!(buffer.expired(earlier));
        assert!(buffer.expire_if_due(earlier));
        assert!(buffer.is_empty());
        assert!(!fake.held());
    }

    #[gpui::test]
    fn wipe_leaves_nothing_behind(cx: &mut TestAppContext) {
        let fake = install(cx, FakeApi::new());
        let mut buffer = typed(cx, "nhq 100");

        buffer.wipe();
        assert!(buffer.is_empty());
        assert_eq!(buffer.len(), 0);
        assert!(!fake.held(), "the wiped buffer kept its guard");
        assert!(buffer.guard.is_none());

        buffer.push('x');
        assert!(buffer.is_empty(), "a wiped buffer took more input");
        assert_eq!(
            buffer.take_query().as_str(),
            "",
            "a wiped buffer still gave text back"
        );
    }

    #[gpui::test]
    fn taking_the_query_consumes_the_buffer(cx: &mut TestAppContext) {
        let fake = install(cx, FakeApi::new());
        let mut buffer = typed(cx, "nhq 1000");
        buffer.backspace();
        assert_eq!(buffer.len(), 7);

        let query = buffer.take_query();
        assert_eq!(query.as_str(), "nhq 100");
        assert!(!fake.held(), "the buffer's guard outlived the hand-over");
        assert_eq!(fake.api.disables(), 1);
    }

    #[gpui::test]
    fn typing_past_the_first_allocation_keeps_every_character(cx: &mut TestAppContext) {
        install(cx, FakeApi::new());
        let long: String = "nhq 100 ä ".repeat(20);
        let buffer = typed(cx, &long);
        assert_eq!(buffer.len(), long.chars().count());
        assert_eq!(buffer.take_query().as_str(), long);
    }

    #[gpui::test]
    fn a_multi_word_query_without_a_hit_is_wiped_as_password_like(cx: &mut TestAppContext) {
        install(cx, FakeApi::new());
        // (typed, hits, wiped)
        let cases = [
            ("nhq", 0, false),
            ("nhq 100", 0, true),
            ("nhq 100", 2, false),
            ("  ", 0, false),
            ("correct horse battery", 0, true),
            ("nhq ", 0, false),
        ];
        for (text, hits, wiped) in cases {
            let mut buffer = typed(cx, text);
            assert_eq!(
                buffer.wipe_if_password_like(hits),
                wiped,
                "{text:?} with {hits} hits"
            );
            if wiped {
                assert!(buffer.is_empty(), "{text:?} was reported wiped but stands");
            } else {
                assert_eq!(buffer.len(), text.chars().count(), "{text:?} lost text");
            }
        }
    }

    #[gpui::test]
    fn typed_ahead_debug_shows_no_characters(cx: &mut TestAppContext) {
        install(cx, FakeApi::new());
        let secret = "vWxQ!";
        let buffer = typed(cx, secret);

        for shown in [format!("{buffer:?}"), format!("{buffer:#?}")] {
            assert_eq!(shown, "TypedAhead(5 chars)");
            for c in secret.chars() {
                assert!(!shown.contains(c), "{shown:?} shows {c:?}");
            }
        }
    }

    #[gpui::test]
    fn typed_ahead_refuses_when_macos_refuses_secure_input(cx: &mut TestAppContext) {
        let fake = install(cx, FakeApi::refusing(HoldError::Os(OsStatus(-50))));

        let begun = cx.update(|cx| TypedAhead::begin(cx));
        assert_eq!(
            begun.err(),
            Some(HoldError::Os(OsStatus(-50))),
            "a buffer started although macOS refused secure input"
        );
        assert!(!fake.held());
        assert_eq!(fake.api.enables(), 0);
    }

    #[gpui::test]
    fn typed_ahead_runs_without_a_guard_where_there_is_none(cx: &mut TestAppContext) {
        let fake = install(cx, FakeApi::refusing(HoldError::Unsupported));

        let mut buffer = cx
            .update(|cx| TypedAhead::begin(cx))
            .expect("no secure event input on this platform is not an error");
        assert!(buffer.guard.is_none());
        assert_eq!(fake.api.enables(), 0);
        buffer.push('n');
        assert_eq!(buffer.len(), 1);
    }
}
