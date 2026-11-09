//! What you type into the quick access panel while it is still locked.
//!
//! The panel lets you start a search before Touch ID has unlocked the vault.
//! Those keystrokes go into a [`TypedAhead`] buffer, and the panel shows only
//! how many there are, as dots. The rules:
//!
//! - The text sits in a `Zeroizing<String>`, which is overwritten when it is
//!   wiped or dropped, spare capacity included. Growing it never leaves a
//!   copy behind. `String::push` would reallocate and free the old
//!   allocation as it was, so when a character doesn't fit in the room
//!   left, [`TypedAhead::push`] first copies the text into a bigger buffer
//!   and the old one is overwritten as it drops. In debug builds `push`
//!   asserts that `String::push` then finds room, and the tests here type up
//!   to the last free byte with a character of every UTF-8 length.
//!   `typed_ahead_wipes_by_overwriting` in `tests/architecture.rs` lists
//!   every method called on the text, so a `wipe` that only clears it, or a
//!   `reserve`, `push_str` or `shrink_to_fit`, fails there. That list
//!   allows `String::push` in `push` and can't see whether it grows the
//!   text; the assertion can.
//! - Secure event input is held for as long as the buffer is live and the
//!   panel is key, so other programs on the Mac don't see the keystrokes.
//!   [`TypedAhead::begin`] takes it for an empty buffer, before any key
//!   arrives: the panel begins the buffer as soon as it is key while locked,
//!   so secure input is on before the first keystroke reaches FerrisPass,
//!   not from the first keystroke on. If macOS refuses it, `begin` fails and
//!   there is no buffer; the panel then sends you to the main window. Where
//!   the platform has no secure event input at all, the buffer works without
//!   it and claims nothing.
//! - While the Touch ID dialog has key status, the panel lets go of secure
//!   event input with [`TypedAhead::release_secure_input`] and takes it again
//!   with [`TypedAhead::rearm_secure_input`] once it is key again. The text
//!   and its clock stay through that, and no key goes in while secure input
//!   is off. If macOS refuses it the second time, the text is wiped.
//! - It is wiped 30 seconds after the first keystroke ([`EXPIRY`]), also
//!   while the Touch ID dialog is up, when you cancel, and when it looks like
//!   a password: more than one word and not a single match. With nothing
//!   typed there is nothing to expire, and the buffer keeps secure input for
//!   as long as the panel keeps it. The two checks that can wipe it,
//!   [`TypedAhead::expire_if_due`] and [`TypedAhead::wipe_if_password_like`],
//!   do the wiping themselves. There is no variant that only reports and
//!   leaves the wiping to the caller.
//! - The 30 seconds are wall-clock time, like the idle clock behind the
//!   auto-lock: a Mac that slept through them finds the buffer expired. A
//!   clock that went backwards counts as expired too.
//! - A wiped buffer stays empty, has let go of secure event input and takes
//!   no more input. A panel that is still key and locked begins the next one
//!   at once; `rearm_secure_input` puts a new, empty one in its place.
//! - [`TypedAhead::take_query`] hands the text over once the vault is open,
//!   as the search the unlocked panel runs, and the buffer is gone with it.
//!   Text that has expired by then is wiped instead, and the search starts
//!   empty. A search never marks an entry as recently used; only copying a
//!   value or launching an entry does.
//! - Nothing in this file logs, prints or writes the text, and `Debug` shows
//!   only the number of characters. `typed_ahead_never_logs_or_writes` in
//!   `tests/architecture.rs` reads this file to keep it that way.
//!
//! Who owns the buffer and calls these methods is the panel. It begins one as
//! soon as it is key while locked, before any key arrives, puts every typed
//! key into it, checks [`TypedAhead::expire_if_due`] on every tick and
//! keystroke, the ticks while the Touch ID dialog is up included, and
//! [`TypedAhead::wipe_if_password_like`] after every search, and wipes it on
//! Esc. When the Touch ID dialog takes key status, which the panel knows
//! because its unlock is under way, it calls
//! [`TypedAhead::release_secure_input`], and
//! [`TypedAhead::rearm_secure_input`] as soon as it is key again. When it
//! loses key status any other way, it wipes the buffer, so secure input does
//! not outlast it. Once the vault is open, [`TypedAhead::take_query`] gives
//! the search.

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
    /// When the first key went in; `None` while nothing was typed.
    started: Option<SystemTime>,
    guard: Option<SecureInputGuard>,
    /// The panel let go of secure event input while something else has key
    /// status; no key goes in until it takes it again.
    released: bool,
    wiped: bool,
}

/// A secure input guard, or none where the platform has no secure event
/// input. A refusal by macOS, or a call off the main thread, is an error.
fn shield(cx: &App) -> Result<Option<SecureInputGuard>, HoldError> {
    match secure_input::hold(cx) {
        Ok(guard) => Ok(Some(guard)),
        Err(HoldError::Unsupported) => Ok(None),
        Err(error) => Err(error),
    }
}

impl TypedAhead {
    /// An empty buffer that holds secure event input from now on. The panel
    /// calls this as soon as it is key while locked, before any key arrives,
    /// so the first keystroke is already shielded. The 30 seconds start with
    /// the first key, not here.
    ///
    /// A refusal by macOS, or a call off the main thread, is an error and
    /// there is no buffer: the panel must not collect keystrokes it cannot
    /// shield. Where the platform has no secure event input at all
    /// ([`HoldError::Unsupported`]), the buffer starts without a guard.
    pub fn begin(cx: &App) -> Result<Self, HoldError> {
        let guard = shield(cx)?;
        Ok(Self {
            text: Zeroizing::new(String::with_capacity(INITIAL_CAPACITY)),
            started: None,
            guard,
            released: false,
            wiped: false,
        })
    }

    /// Adds `c` at the end; the first key starts the 30 seconds. Does nothing
    /// once the buffer is wiped, or while the panel has let go of secure
    /// event input.
    pub fn push(&mut self, c: char) {
        if self.wiped || self.released {
            return;
        }
        if self.started.is_none() {
            self.started = Some(SystemTime::now());
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
        // Checked apart from the condition above, so that a test typing up
        // to the edge fails here if the condition misjudged it.
        let room = self.text.capacity() - self.text.len();
        debug_assert!(
            c.len_utf8() <= room,
            "String::push would have to grow the typed text and free the old \
             allocation without overwriting it"
        );
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

    /// Whether [`EXPIRY`] has passed at `now`, counted from the first key. A
    /// `now` before that, which only a clock that went backwards produces,
    /// counts as expired. With nothing typed yet, nothing has expired.
    pub fn expired(&self, now: SystemTime) -> bool {
        let Some(started) = self.started else {
            return false;
        };
        match now.duration_since(started) {
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

    /// The panel lost key status to the Touch ID dialog: lets go of secure
    /// event input and keeps the text and its clock. [`expire_if_due`]
    /// still wipes the text 30 seconds after the first key, and no key goes
    /// in until [`rearm_secure_input`] has taken secure input again.
    ///
    /// [`expire_if_due`]: TypedAhead::expire_if_due
    /// [`rearm_secure_input`]: TypedAhead::rearm_secure_input
    pub fn release_secure_input(&mut self) {
        self.guard = None;
        self.released = true;
    }

    /// The panel is key again: takes secure event input again for the text
    /// and the clock it kept. A buffer that still holds it is left as it is.
    /// One that was wiped meanwhile, because it expired while the dialog was
    /// up, starts over empty under secure input, as a new one from
    /// [`TypedAhead::begin`] does.
    ///
    /// If macOS refuses, or the call is off the main thread, the text is
    /// wiped and the error comes back, as from `begin`: the panel must not
    /// collect keystrokes it cannot shield.
    pub fn rearm_secure_input(&mut self, cx: &App) -> Result<(), HoldError> {
        if self.wiped {
            *self = Self::begin(cx)?;
            return Ok(());
        }
        if !self.released {
            return Ok(());
        }
        match shield(cx) {
            Ok(guard) => {
                self.guard = guard;
                self.released = false;
                Ok(())
            }
            Err(error) => {
                self.wipe();
                Err(error)
            }
        }
    }

    /// Hands the text over as the search to run once the vault is open, and
    /// ends the buffer with it: secure event input goes as the guard drops.
    /// Text that has expired at `now` is wiped first, so the search starts
    /// empty, as it does for a wiped buffer.
    pub fn take_query(mut self, now: SystemTime) -> Zeroizing<String> {
        self.expire_if_due(now);
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

    /// The panel begins the buffer as soon as it is key while locked, so
    /// secure input is on before the first key reaches FerrisPass. With
    /// nothing typed there is nothing to expire, and the buffer keeps secure
    /// input for as long as the panel keeps it. The keys that follow go in
    /// under the same guard, which goes when the buffer is wiped, expires,
    /// hands its text over or is dropped, and only once.
    #[gpui::test]
    fn secure_input_is_on_before_the_first_key(cx: &mut TestAppContext) {
        let fake = install(cx, FakeApi::new());
        for ending in ["wipe", "expiry", "hand-over", "drop"] {
            let enables = fake.api.enables();
            let disables = fake.api.disables();

            let mut buffer = cx.update(|cx| TypedAhead::begin(cx)).expect("begin");
            assert!(
                fake.held(),
                "{ending}: secure input is off before the first key"
            );
            assert_eq!(fake.api.enables(), enables + 1, "{ending}");
            let an_hour_on = SystemTime::now() + Duration::from_secs(3600);
            assert!(
                !buffer.expire_if_due(an_hour_on),
                "{ending}: a buffer with nothing typed in it expired"
            );
            assert!(
                fake.held(),
                "{ending}: secure input went off with nothing typed"
            );

            for c in "nhq".chars() {
                buffer.push(c);
            }
            assert_eq!(buffer.len(), 3, "{ending}");
            assert!(fake.held(), "{ending}");
            assert_eq!(
                fake.api.enables(),
                enables + 1,
                "{ending}: a key took secure input a second time"
            );

            let ended = match ending {
                "wipe" => {
                    buffer.wipe();
                    Some(buffer)
                }
                "expiry" => {
                    assert!(buffer.expire_if_due(SystemTime::now() + EXPIRY));
                    Some(buffer)
                }
                "hand-over" => {
                    assert_eq!(buffer.take_query(SystemTime::now()).as_str(), "nhq");
                    None
                }
                _ => {
                    drop(buffer);
                    None
                }
            };
            assert!(!fake.held(), "{ending}: secure input stayed on");
            assert_eq!(fake.api.disables(), disables + 1, "{ending}");
            drop(ended);
            assert_eq!(
                fake.api.disables(),
                disables + 1,
                "{ending}: the ended buffer let go of secure input twice"
            );
        }
    }

    /// The panel begins the buffer when it becomes key, which can be long
    /// before anything is typed, so the 30 seconds start with the first key.
    /// Until then not even a clock that went backwards expires it.
    #[gpui::test]
    fn the_thirty_seconds_start_with_the_first_key(cx: &mut TestAppContext) {
        install(cx, FakeApi::new());
        let mut buffer = cx.update(|cx| TypedAhead::begin(cx)).expect("begin");
        assert_eq!(buffer.started, None, "the clock started with no key typed");
        assert!(!buffer.expired(SystemTime::now() + Duration::from_secs(3600)));
        assert!(!buffer.expired(SystemTime::UNIX_EPOCH));

        let before = SystemTime::now();
        buffer.push('n');
        let first = buffer.started.expect("the first key started the clock");
        assert!(first >= before, "the clock started before the first key");
        buffer.push('h');
        buffer.backspace();
        buffer.backspace();
        buffer.push('q');
        assert_eq!(
            buffer.started,
            Some(first),
            "a later key started the clock again"
        );
        assert!(!buffer.expired(first + Duration::from_secs(29)));
        assert!(buffer.expired(first + EXPIRY));
    }

    #[gpui::test]
    fn typed_ahead_expires_after_thirty_seconds(cx: &mut TestAppContext) {
        install(cx, FakeApi::new());
        let buffer = typed(cx, "nhq");
        let start = buffer.started.expect("the first key started the clock");

        assert!(!buffer.expired(start));
        assert!(!buffer.expired(start + Duration::from_secs(29)));
        assert!(buffer.expired(start + Duration::from_secs(30)));
        assert!(buffer.expired(start + Duration::from_secs(3600)));
    }

    #[gpui::test]
    fn expire_if_due_wipes_the_buffer_and_releases_the_guard(cx: &mut TestAppContext) {
        let fake = install(cx, FakeApi::new());
        let mut buffer = typed(cx, "nhq");
        let start = buffer.started.expect("the first key started the clock");

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
        let earlier =
            buffer.started.expect("the first key started the clock") - Duration::from_secs(1);

        assert!(buffer.expired(earlier));
        assert!(buffer.expire_if_due(earlier));
        assert!(buffer.is_empty());
        assert!(!fake.held());
    }

    /// What a wipe leaves that a test can see. Whether `wipe` overwrote the
    /// bytes, rather than only shortening the string, can't be read back
    /// without `unsafe`; `typed_ahead_wipes_by_overwriting` in
    /// `tests/architecture.rs` holds `wipe` to `zeroize` instead.
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
            buffer.take_query(SystemTime::now()).as_str(),
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

        let query = buffer.take_query(SystemTime::now());
        assert_eq!(query.as_str(), "nhq 100");
        assert!(!fake.held(), "the buffer's guard outlived the hand-over");
        assert_eq!(fake.api.disables(), 1);
    }

    /// The Touch ID dialog takes key status from the panel. The panel lets go
    /// of secure input for that time and takes it again once it is key; the
    /// text and its clock stay, and no key goes in in between.
    #[gpui::test]
    fn secure_input_is_released_and_taken_again_around_the_dialog(cx: &mut TestAppContext) {
        let fake = install(cx, FakeApi::new());
        let mut buffer = typed(cx, "nhq");
        let start = buffer.started.expect("the first key started the clock");

        buffer.release_secure_input();
        assert!(
            !fake.held(),
            "secure input stayed on while the dialog had key status"
        );
        assert_eq!(fake.api.disables(), 1);
        assert_eq!(buffer.len(), 3, "letting go of secure input lost the text");
        assert_eq!(
            buffer.started,
            Some(start),
            "letting go of secure input restarted the clock"
        );
        buffer.push('x');
        assert_eq!(buffer.len(), 3, "a key went in without secure input");

        cx.update(|cx| buffer.rearm_secure_input(cx))
            .expect("secure input again");
        assert!(
            fake.held(),
            "secure input stayed off with the panel key again"
        );
        assert_eq!(fake.api.enables(), 2);
        cx.update(|cx| buffer.rearm_secure_input(cx))
            .expect("secure input still held");
        assert_eq!(
            fake.api.enables(),
            2,
            "a buffer that held secure input took it a second time"
        );
        buffer.push(' ');
        buffer.push('1');
        assert_eq!(buffer.started, Some(start), "a key restarted the clock");
        assert_eq!(
            buffer.take_query(start + Duration::from_secs(29)).as_str(),
            "nhq 1"
        );
        assert!(!fake.held());
        assert_eq!(fake.api.disables(), 2);
    }

    /// The 30 seconds run on while the dialog is up: the text is wiped at 30
    /// seconds whether or not the panel holds secure input. Once the panel is
    /// key again it gets a new, empty buffer under secure input.
    #[gpui::test]
    fn the_thirty_seconds_run_on_while_the_dialog_is_up(cx: &mut TestAppContext) {
        let fake = install(cx, FakeApi::new());
        let mut buffer = typed(cx, "nhq");
        let start = buffer.started.expect("the first key started the clock");
        buffer.release_secure_input();

        assert!(!buffer.expire_if_due(start + Duration::from_secs(29)));
        assert_eq!(buffer.len(), 3, "the text went before its 30 seconds");
        assert!(
            buffer.expire_if_due(start + EXPIRY),
            "the text outlived its 30 seconds while the dialog was up"
        );
        assert!(buffer.is_empty());
        assert_eq!(
            fake.api.disables(),
            1,
            "secure input was let go of a second time"
        );

        cx.update(|cx| buffer.rearm_secure_input(cx))
            .expect("a new buffer");
        assert!(fake.held(), "the panel is key again without secure input");
        assert_eq!(fake.api.enables(), 2);
        assert!(buffer.is_empty());
        assert_eq!(buffer.started, None, "the new buffer kept the old clock");
        buffer.push('n');
        assert_eq!(buffer.len(), 1);
    }

    /// The text becomes the search only if it has not expired; otherwise the
    /// search starts empty. The same while the panel has let go of secure
    /// input.
    #[gpui::test]
    fn an_expired_buffer_yields_no_query(cx: &mut TestAppContext) {
        install(cx, FakeApi::new());
        for released in [false, true] {
            let (mut kept, mut expired) = (typed(cx, "nhq"), typed(cx, "nhq"));
            if released {
                kept.release_secure_input();
                expired.release_secure_input();
            }
            let start = kept.started.expect("the first key started the clock");
            assert_eq!(
                kept.take_query(start + Duration::from_secs(29)).as_str(),
                "nhq",
                "released {released}"
            );
            let start = expired.started.expect("the first key started the clock");
            assert_eq!(
                expired.take_query(start + EXPIRY).as_str(),
                "",
                "released {released}: an expired buffer gave its text as the search"
            );
        }
    }

    /// If macOS refuses secure input when the panel is key again, the text is
    /// wiped and no more keys go in, as if `begin` had failed.
    #[gpui::test]
    fn a_refused_rearm_wipes_the_buffer(cx: &mut TestAppContext) {
        install(cx, FakeApi::new());
        let mut buffer = typed(cx, "nhq");
        buffer.release_secure_input();
        let refusing = install(cx, FakeApi::refusing(HoldError::Os(OsStatus(-50))));

        let rearmed = cx.update(|cx| buffer.rearm_secure_input(cx));
        assert_eq!(rearmed, Err(HoldError::Os(OsStatus(-50))));
        assert!(
            buffer.is_empty(),
            "the text stayed although macOS refused secure input"
        );
        buffer.push('x');
        assert!(buffer.is_empty(), "a key went in without secure input");
        assert!(!refusing.held());
    }

    #[gpui::test]
    fn typing_past_the_first_allocation_keeps_every_character(cx: &mut TestAppContext) {
        install(cx, FakeApi::new());
        let long: String = "nhq 100 ä ".repeat(20);
        let buffer = typed(cx, &long);
        assert_eq!(buffer.len(), long.chars().count());
        assert_eq!(buffer.take_query(SystemTime::now()).as_str(), long);
    }

    /// The edge of the allocation, for a character of each UTF-8 length.
    /// With exactly its bytes left it goes in place; with one byte fewer the
    /// text is copied into a bigger buffer first. A `push` that misjudged
    /// this and left the growing to `String::push` fails its own assertion.
    #[gpui::test]
    fn a_character_that_does_not_fit_grows_the_buffer_by_copying(cx: &mut TestAppContext) {
        install(cx, FakeApi::new());
        for c in ['n', 'ä', '€', '🦀'] {
            let width = c.len_utf8();
            for left in [width, width - 1] {
                let mut buffer = typed(cx, "");
                let capacity = buffer.text.capacity();
                let fill = "x".repeat(capacity - left);
                for x in fill.chars() {
                    buffer.push(x);
                }
                assert_eq!(buffer.text.capacity(), capacity);
                buffer.push(c);
                if left == width {
                    assert_eq!(
                        buffer.text.capacity(),
                        capacity,
                        "{c:?} with room for it grew the buffer"
                    );
                } else {
                    assert!(
                        buffer.text.capacity() > capacity,
                        "{c:?} without room for it did not grow the buffer"
                    );
                }
                assert_eq!(
                    buffer.take_query(SystemTime::now()).as_str(),
                    format!("{fill}{c}")
                );
            }
        }
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
