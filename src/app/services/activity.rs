//! The idle clock behind the auto-lock. There is one for the whole app, not
//! one per window.
//!
//! Activity is input the user gives FerrisPass, and each kind has a name in
//! [`ActivitySource`]: input in a FerrisPass window, keys and clicks in the
//! quick access panel, a global shortcut, a click on the menu bar item. The
//! rules:
//!
//! - Only those sources move the clock, and the auto-lock task when it
//!   starts (the last rule). Every call of [`touch`] names its source, and
//!   `activity_is_touched_only_where_listed` in `tests/architecture.rs`
//!   holds the files allowed to call it, with the source each one names, and
//!   each call of [`reset`], with the function it is in. A new caller is a
//!   new line in one of those lists.
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

use crate::platform::system_idle::IdleError;

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
/// task calls this when it starts. Nothing else in the app may: a reset holds
/// off the lock just as input does.
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

/// Whether the auto-lock is due when the user may also have chosen to lock
/// after the Mac has been idle. `app_idle` is this clock, `mac_idle` what
/// `platform::system_idle::idle_for` reported, and the thresholds are the two
/// settings, `None` meaning off.
///
/// - Without a Mac threshold nothing changes: this is [`lock_is_due`] with the
///   app's clock and the app's threshold.
/// - With a Mac threshold, only the Mac's idle time counts, against that
///   threshold. The app's clock and threshold play no part.
/// - With a Mac threshold and no reading from macOS (an error of any kind),
///   the app's clock counts, against the Mac threshold. An error never
///   postpones the lock, so with the Mac threshold set there is no reading on
///   which the lock is never due.
///
/// A clock that went backwards reads as [`Duration::MAX`] in `app_idle` and
/// makes the lock due wherever the app's clock counts.
pub fn lock_is_due_with(
    app_idle: Duration,
    mac_idle: Result<Duration, IdleError>,
    app_threshold: Option<Duration>,
    mac_threshold: Option<Duration>,
) -> bool {
    let Some(mac_threshold) = mac_threshold else {
        return lock_is_due(app_idle, app_threshold);
    };
    mac_idle.unwrap_or(app_idle) >= mac_threshold
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
/// another test is watching. Integration tests and the app use the static;
/// `tests/idle_clock.rs` checks it.
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

    /// Every source, for the tests that go through them all. A new variant
    /// stops `only_listed_sources_exist` from compiling until it gets an arm
    /// in its `match`, but that arm does not add it here. Add it here too, or
    /// `touch_resets_idle` never tries it; nothing checks that second step.
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
        // No `_` arm, so this test does not compile with a source the `match`
        // leaves out. A new source needs its arm here, a place in
        // `ALL_SOURCES` (the arm alone does not put it there) and its call
        // sites in tests/architecture.rs.
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

    const FIFTEEN_MINUTES: Duration = Duration::from_secs(15 * 60);

    fn minutes(minutes: u64) -> Duration {
        Duration::from_secs(minutes * 60)
    }

    #[test]
    fn mac_idle_wins_when_the_alternative_is_set() {
        // Ten minutes away from FerrisPass would lock with the app's clock,
        // but the Mac was in use two minutes ago.
        assert!(lock_is_due(minutes(10), Some(FOUR_MINUTES)));
        assert!(!lock_is_due_with(
            minutes(10),
            Ok(minutes(2)),
            Some(FOUR_MINUTES),
            Some(FIFTEEN_MINUTES)
        ));
        assert!(!lock_is_due_with(
            Duration::MAX,
            Ok(Duration::ZERO),
            Some(FOUR_MINUTES),
            Some(FIFTEEN_MINUTES)
        ));
        // The Mac idle for fifteen minutes locks, whatever the app's clock
        // says.
        for app_idle in [Duration::ZERO, minutes(2), minutes(10)] {
            assert!(lock_is_due_with(
                app_idle,
                Ok(FIFTEEN_MINUTES),
                Some(FOUR_MINUTES),
                Some(FIFTEEN_MINUTES)
            ));
            assert!(!lock_is_due_with(
                app_idle,
                Ok(FIFTEEN_MINUTES - Duration::from_secs(1)),
                Some(FOUR_MINUTES),
                Some(FIFTEEN_MINUTES)
            ));
        }
        // The app's threshold may be off; the Mac's still applies.
        assert!(lock_is_due_with(
            Duration::ZERO,
            Ok(FIFTEEN_MINUTES),
            None,
            Some(FIFTEEN_MINUTES)
        ));
    }

    #[test]
    fn an_unreadable_mac_idle_falls_back_to_the_app_clock_with_the_mac_threshold() {
        for error in [IdleError::Unsupported, IdleError::Invalid] {
            for app_threshold in [Some(FOUR_MINUTES), None] {
                assert!(
                    !lock_is_due_with(
                        minutes(14),
                        Err(error),
                        app_threshold,
                        Some(FIFTEEN_MINUTES)
                    ),
                    "{error:?} with {app_threshold:?}: locked before the Mac threshold"
                );
                assert!(
                    lock_is_due_with(
                        FIFTEEN_MINUTES,
                        Err(error),
                        app_threshold,
                        Some(FIFTEEN_MINUTES)
                    ),
                    "{error:?} with {app_threshold:?}: an error postponed the lock"
                );
            }
        }
    }

    #[test]
    fn without_the_alternative_the_app_clock_rules() {
        let idles = [
            Duration::ZERO,
            minutes(2),
            FOUR_MINUTES - Duration::from_secs(1),
            FOUR_MINUTES,
            minutes(10),
            Duration::MAX,
        ];
        let readings = [
            Ok(Duration::ZERO),
            Ok(Duration::MAX),
            Err(IdleError::Unsupported),
            Err(IdleError::Invalid),
        ];
        for app_idle in idles {
            for app_threshold in [Some(FOUR_MINUTES), Some(FIFTEEN_MINUTES), None] {
                for mac_idle in readings {
                    assert_eq!(
                        lock_is_due_with(app_idle, mac_idle, app_threshold, None),
                        lock_is_due(app_idle, app_threshold),
                        "{app_idle:?} against {app_threshold:?}, Mac {mac_idle:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_backwards_clock_is_due_at_once() {
        let now = SystemTime::now();
        reset(now);
        let backwards = idle_for(now - Duration::from_secs(60));
        assert_eq!(backwards, Duration::MAX);
        assert!(lock_is_due_with(
            backwards,
            Ok(Duration::ZERO),
            Some(FOUR_MINUTES),
            None
        ));
        for error in [IdleError::Unsupported, IdleError::Invalid] {
            assert!(lock_is_due_with(
                backwards,
                Err(error),
                Some(FOUR_MINUTES),
                Some(FIFTEEN_MINUTES)
            ));
            assert!(lock_is_due_with(
                backwards,
                Err(error),
                None,
                Some(Duration::from_secs(u64::MAX))
            ));
        }
    }
}
