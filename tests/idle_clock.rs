//! The idle clock as the app builds it.
//!
//! Unit tests give each test thread a clock of its own (see
//! `src/app/services/activity.rs`), so the clock the app uses, one `static`
//! for every thread, is compiled only for code outside the crate's own tests.
//! This file is that code. The auto-lock task reads the clock on the main
//! thread, and a global shortcut will move it from the thread that listens
//! for it, so a touch or a reset on one thread has to be seen on the others.
//!
//! One test, not several: tests in this file would share the `static` while
//! libtest runs them side by side.

use std::thread;
use std::time::{Duration, SystemTime};

use ferrispass::app::services::activity::{self, ActivitySource};

const FOUR_MINUTES: Duration = Duration::from_secs(240);
const TEN_MINUTES: Duration = Duration::from_secs(600);

#[test]
fn the_app_has_one_idle_clock_for_every_thread() {
    let idle = activity::idle_for(SystemTime::now());
    assert!(
        activity::lock_is_due(idle, Some(FOUR_MINUTES)),
        "a clock never set held off the lock, idle for {idle:?}"
    );

    let now = SystemTime::now();
    activity::reset(now - TEN_MINUTES);
    assert_eq!(
        activity::idle_for(now),
        TEN_MINUTES,
        "idle_for did not read the time reset set"
    );

    thread::spawn(|| activity::touch(ActivitySource::Hotkey))
        .join()
        .expect("touching thread");
    let idle = activity::idle_for(SystemTime::now());
    assert!(
        idle < Duration::from_secs(60),
        "a touch on another thread left {idle:?}"
    );

    thread::spawn(move || activity::reset(now - TEN_MINUTES))
        .join()
        .expect("resetting thread");
    assert_eq!(
        activity::idle_for(now),
        TEN_MINUTES,
        "a reset on another thread was not seen here"
    );
}
