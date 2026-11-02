//! The idle clock behind the auto-lock. There is one for the whole app, not
//! one per window.
//!
//! Activity is input the user gives FerrisPass, and each kind has a name in
//! [`ActivitySource`]: input in a FerrisPass window, keys and clicks in the
//! quick access panel, a global shortcut, a click on the menu bar item. The
//! rules:
//!
//! - Only those sources move the clock. Every call of [`touch`] names its
//!   source, and `activity_is_touched_only_where_listed` in
//!   `tests/architecture.rs` holds the list of files allowed to call it, with
//!   the source each one names. A new caller is a new line in that list.
//! - Background work never moves it: sync, the TOTP tick, favicon downloads,
//!   the update check, launch cleanup, and the auto-lock check itself, which
//!   only reads it.
//! - Nothing outside the process reaches it. No function here turns an event
//!   from another app into a [`touch`], and a global shortcut counts when it
//!   is pressed, not while it is registered. The pointer passing over a
//!   FerrisPass window while another app is in front is not input either.
//! - The clock runs on wall-clock time ([`SystemTime`]), not [`Instant`],
//!   because `Instant` stops while the Mac sleeps and hours asleep would
//!   count as no idle time at all. A clock that jumps backwards makes the
//!   lock due at once rather than postponing it.
//! - The auto-lock task sets the baseline when it starts ([`reset`]), so a
//!   vault opened after a long pause is not locked on the first check.
//!
//! [`Instant`]: std::time::Instant

use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime};

/// Where input that counts as activity comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActivitySource {
    /// A key press, a click or a scroll in a FerrisPass window, or the
    /// pointer moving over one while it is in front.
    WindowInput,
    /// A key press or a click in the quick access panel.
    Panel,
    /// A global shortcut, at the moment it is pressed.
    Hotkey,
    /// A click on the menu bar item.
    StatusItem,
}

/// Note input from `source` now.
///
/// The clock does not tell the sources apart. The argument is there so that
/// every caller says where its input comes from, which is what the list of
/// call sites in `tests/architecture.rs` is checked against.
pub fn touch(_source: ActivitySource) {
    with_last_input(|last| last.set(SystemTime::now()));
}

/// Start counting idle time from `now`, whatever came before. The auto-lock
/// task calls this when it starts.
pub fn reset(now: SystemTime) {
    with_last_input(|last| last.set(now));
}

/// How long the app has been idle at `now`. A `now` before the last input,
/// which only a clock that went backwards produces, reads as
/// [`Duration::MAX`], so the lock is due instead of postponed.
pub fn idle_for(now: SystemTime) -> Duration {
    let last = with_last_input(LastInput::get);
    now.duration_since(last).unwrap_or(Duration::MAX)
}

/// Whether the auto-lock is due after `idle`. `threshold` is the setting;
/// `None` means the auto-lock is off.
pub fn lock_is_due(idle: Duration, threshold: Option<Duration>) -> bool {
    threshold.is_some_and(|threshold| idle >= threshold)
}

/// The time of the last input. It starts at the epoch, so a clock that was
/// never set reads as idle for decades and the lock is due, not held off.
struct LastInput(Mutex<SystemTime>);

impl LastInput {
    const fn new() -> Self {
        Self(Mutex::new(SystemTime::UNIX_EPOCH))
    }

    fn get(&self) -> SystemTime {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn set(&self, at: SystemTime) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = at;
    }
}

#[cfg(not(test))]
fn with_last_input<R>(f: impl FnOnce(&LastInput) -> R) -> R {
    static LAST_INPUT: LastInput = LastInput::new();
    f(&LAST_INPUT)
}

/// Unit tests run side by side in one process, each on a thread of its own,
/// and the GPUI test executor runs a test's tasks on that thread. With one
/// clock per thread, a test that types into its window cannot reset the clock
/// another test is watching. Integration tests and the app use the static.
#[cfg(test)]
fn with_last_input<R>(f: impl FnOnce(&LastInput) -> R) -> R {
    thread_local! {
        static LAST_INPUT: LastInput = const { LastInput::new() };
    }
    LAST_INPUT.with(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOUR_MINUTES: Duration = Duration::from_secs(240);

    /// Every source. `only_listed_sources_exist` fails to compile until a new
    /// variant is added here.
    const ALL_SOURCES: [ActivitySource; 4] = [
        ActivitySource::WindowInput,
        ActivitySource::Panel,
        ActivitySource::Hotkey,
        ActivitySource::StatusItem,
    ];

    fn minutes_ago(minutes: u64) -> SystemTime {
        SystemTime::now() - Duration::from_secs(minutes * 60)
    }

    #[test]
    fn lock_is_due_at_threshold() {
        assert!(!lock_is_due(
            FOUR_MINUTES - Duration::from_secs(1),
            Some(FOUR_MINUTES)
        ));
        assert!(lock_is_due(FOUR_MINUTES, Some(FOUR_MINUTES)));
        assert!(lock_is_due(
            FOUR_MINUTES + Duration::from_secs(1),
            Some(FOUR_MINUTES)
        ));
    }

    #[test]
    fn lock_is_due_when_clock_goes_backwards() {
        let now = SystemTime::now();
        reset(now);
        let earlier = now - Duration::from_secs(60);
        let idle = idle_for(earlier);
        assert_eq!(idle, Duration::MAX, "a clock that went back reads as idle");
        assert!(lock_is_due(idle, Some(FOUR_MINUTES)));
        assert!(lock_is_due(idle, Some(Duration::from_secs(u64::MAX))));
    }

    #[test]
    fn lock_never_due_without_threshold() {
        assert!(!lock_is_due(Duration::ZERO, None));
        assert!(!lock_is_due(FOUR_MINUTES, None));
        assert!(!lock_is_due(Duration::MAX, None));
    }

    #[test]
    fn touch_resets_idle() {
        for source in ALL_SOURCES {
            reset(minutes_ago(10));
            assert!(idle_for(SystemTime::now()) >= Duration::from_secs(600));
            touch(source);
            let idle = idle_for(SystemTime::now());
            assert!(idle < Duration::from_secs(60), "{source:?} left {idle:?}");
        }
    }

    #[test]
    fn only_listed_sources_exist() {
        // No `_` arm: a new source does not compile here until it is added to
        // `ALL_SOURCES`, and given its call sites in tests/architecture.rs.
        let mut seen = [false; ALL_SOURCES.len()];
        for source in ALL_SOURCES {
            let slot = match source {
                ActivitySource::WindowInput => 0,
                ActivitySource::Panel => 1,
                ActivitySource::Hotkey => 2,
                ActivitySource::StatusItem => 3,
            };
            assert!(!seen[slot], "{source:?} is listed twice");
            seen[slot] = true;
        }
        assert!(seen.iter().all(|&listed| listed));
    }

    #[test]
    fn reading_the_clock_does_not_reset_it() {
        let last = minutes_ago(10);
        reset(last);
        let now = SystemTime::now();
        let idle = now.duration_since(last).unwrap();
        assert_eq!(idle_for(now), idle);
        assert!(lock_is_due(idle_for(now), Some(FOUR_MINUTES)));
        assert_eq!(idle_for(now), idle, "reading the clock moved it");
    }

    #[test]
    fn reset_moves_the_baseline_both_ways() {
        let now = SystemTime::now();
        reset(now);
        assert_eq!(idle_for(now), Duration::ZERO);
        reset(now - FOUR_MINUTES);
        assert_eq!(idle_for(now), FOUR_MINUTES);
    }

    #[test]
    fn a_clock_never_set_reads_as_idle() {
        assert!(lock_is_due(idle_for(SystemTime::now()), Some(FOUR_MINUTES)));
    }
}
