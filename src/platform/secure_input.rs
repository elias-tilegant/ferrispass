//! Secure event input for the master password field.
//!
//! While secure event input is on, macOS stops passing keystrokes to event
//! taps and other programs that watch the keyboard; only the app that turned
//! it on sees what is typed. The switch is Carbon's `EnableSecureEventInput`
//! and `DisableSecureEventInput`. No crate we depend on wraps them, so the
//! `carbon` module below declares the three functions itself, and calling a
//! foreign function is `unsafe`. That module, compiled on macOS only, is the
//! whole reason this file allows `unsafe_code`: one `extern` block and one
//! `unsafe` call per function. Everything else here is safe Rust and the same
//! on every platform.
//!
//! The system keeps a single count for the login session. Each Enable adds
//! one, each Disable takes one away, and secure input stays on while the count
//! is above zero, even with another app in front. An Enable without its
//! Disable leaves every other app without keyboard events it may rely on,
//! which is what Apple's TN2150 ("Using Secure Event Input Fairly") warns
//! about. So this module keeps four rules, each held by a test below:
//!
//! 1. At most one Enable is outstanding, however many guards are alive. The
//!    guards of the current generation are counted; Enable runs only when the
//!    count goes from 0 to 1 and no Enable is outstanding, Disable only when
//!    it goes from 1 to 0.
//! 2. Every Enable gets its Disable: when the last guard drops, in
//!    [`release_all`] (lock, quit), and in the panic hook. `release_all` sets
//!    the count to zero and starts a new generation, and a guard from an
//!    earlier generation does nothing when it drops, so it cannot switch off
//!    a hold taken after the release. A Disable that fails leaves its Enable
//!    outstanding: the next hold does not add another, and the next release
//!    tries the Disable again.
//! 3. Carbon is called on the main thread only. [`hold`] refuses on any other
//!    thread, `release_all` does nothing there, and the panic hook releases
//!    only when the panic is on the main thread. A panic on another thread
//!    leaves everything as it was: the guards live on the main thread, which
//!    still releases on blur, lock and quit, and macOS drops secure input when
//!    the process ends.
//! 4. A failed `hold` holds nothing and is never silent: the caller gets the
//!    error, and the OSStatus goes to the log (no secret is in it). Where
//!    there is no secure event input at all, `hold` returns
//!    [`HoldError::Unsupported`], so nothing claims a protection it does not
//!    give.
//!
//! The count and the generation sit together under one lock, and every step
//! reads and writes both while holding it, so a guard dropped during
//! `release_all` sees the state before or after, never half of each. A guard
//! is neither `Send` nor `Sync`: it is dropped on the thread that took it,
//! and `hold` made sure that was the main thread.
//!
//! Code that has a GPUI `App` goes through [`hold`], [`release_all`] and
//! [`held`]; tests put a fake in place of Carbon there (`fake::install`).

#![allow(unsafe_code)]

use std::fmt;
use std::marker::PhantomData;
use std::sync::{Mutex, MutexGuard, Once, PoisonError, TryLockError};

use gpui::App;

/// A Carbon status code. Zero is success.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OsStatus(pub i32);

impl fmt::Display for OsStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "OSStatus {}", self.0)
    }
}

/// Why [`hold`] did not turn secure event input on. In every case nothing is
/// held.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum HoldError {
    #[error("macOS refused secure event input ({0})")]
    Os(OsStatus),
    #[error("secure event input was asked for off the main thread")]
    OffMainThread,
    #[error("secure event input is not available on this platform")]
    Unsupported,
}

/// What this module needs from the system: Carbon on macOS, the no-op
/// elsewhere, and a fake in tests.
pub(crate) trait SecureInputApi: Sync {
    /// One Enable. [`HoldError::Os`] when the system refuses,
    /// [`HoldError::Unsupported`] where there is no secure event input.
    fn enable(&self) -> Result<(), HoldError>;
    /// One Disable, paired with an earlier successful `enable`.
    fn disable(&self) -> Result<(), OsStatus>;
    /// Whether secure event input is on for the login session, from any app.
    fn is_enabled(&self) -> bool;
    /// Whether the caller runs on the process's main thread.
    fn is_main_thread(&self) -> bool;
    /// Whether this platform has secure event input at all.
    fn is_supported(&self) -> bool;
}

/// The count of live guards, the generation they belong to, and whether an
/// Enable of ours is outstanding. Only ever read or written under the
/// switch's lock.
struct State {
    holders: usize,
    generation: u64,
    /// An Enable that no successful Disable has matched yet. The same as
    /// `holders > 0`, except after a Disable failed: then it stays set, so
    /// the next hold does not stack a second Enable and the next release
    /// tries the Disable again.
    enabled: bool,
}

/// Counts the holders for one `SecureInputApi` and turns it on and off.
/// The app has one, `SYSTEM`; tests make their own around a fake.
pub(crate) struct Switch {
    api: &'static dyn SecureInputApi,
    state: Mutex<State>,
    /// Every line this switch wrote to the log, so a test can count them.
    #[cfg(test)]
    notes: Mutex<Vec<String>>,
}

#[cfg(target_os = "macos")]
static SYSTEM: Switch = Switch::new(&carbon::Carbon);
#[cfg(not(target_os = "macos"))]
static SYSTEM: Switch = Switch::new(&noop::Noop);

impl Switch {
    const fn new(api: &'static dyn SecureInputApi) -> Self {
        Self {
            api,
            state: Mutex::new(State {
                holders: 0,
                generation: 0,
                enabled: false,
            }),
            #[cfg(test)]
            notes: Mutex::new(Vec::new()),
        }
    }

    /// Taken as it is even after a panic poisoned it. Nothing in this module
    /// panics while holding the lock, and refusing every later hold and
    /// release over a poisoned lock would be the worse outcome.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn hold(&'static self) -> Result<SecureInputGuard, HoldError> {
        if !self.api.is_main_thread() {
            self.note("secure event input was asked for off the main thread and refused");
            return Err(HoldError::OffMainThread);
        }
        let mut state = self.lock();
        if !state.enabled {
            if let Err(error) = self.api.enable() {
                drop(state);
                if let HoldError::Os(status) = error {
                    self.note(&format!(
                        "EnableSecureEventInput failed with {status}; the field is not protected"
                    ));
                }
                return Err(error);
            }
            state.enabled = true;
        }
        state.holders += 1;
        Ok(SecureInputGuard {
            switch: self,
            generation: state.generation,
            _not_send: PhantomData,
        })
    }

    /// What a guard does when it drops.
    fn release(&self, generation: u64) {
        let mut state = self.lock();
        if state.generation != generation {
            // `release_all` already took this guard's share.
            return;
        }
        // A guard of the current generation always finds itself counted.
        // If it ever did not, a release build stays at zero instead of
        // wrapping around and calls no Disable.
        let remaining = state.holders.checked_sub(1);
        state.holders = remaining.unwrap_or(0);
        let disabled = (remaining == Some(0))
            .then(|| self.disable(&mut state))
            .flatten();
        drop(state);
        self.note_disable(disabled);
        debug_assert!(
            remaining.is_some(),
            "a guard of the current generation found no holder to release"
        );
    }

    fn release_all(&self) {
        if !self.api.is_main_thread() {
            self.note(
                "release of secure event input was asked for off the main thread and ignored",
            );
            return;
        }
        let mut state = self.lock();
        let disabled = self.release_everything(&mut state);
        drop(state);
        self.note_disable(disabled);
    }

    /// The panic hook's part. On the main thread it releases like
    /// `release_all`; on any other thread it does nothing, see rule
    /// 3 at the top. It never waits for the lock: if this thread panicked
    /// while holding it, waiting would never end, and the process is going
    /// down anyway.
    fn release_after_panic(&self) {
        if !self.api.is_main_thread() {
            return;
        }
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(TryLockError::WouldBlock) => return,
        };
        let _ = self.release_everything(&mut state);
    }

    /// Sets the count to zero, starts a new generation and calls Disable if
    /// an Enable is outstanding. Returns that Disable's result.
    fn release_everything(&self, state: &mut State) -> Option<Result<(), OsStatus>> {
        state.generation = state.generation.wrapping_add(1);
        state.holders = 0;
        self.disable(state)
    }

    /// Calls Disable for the outstanding Enable, if there is one. Only a
    /// Disable that succeeded clears it; see `State::enabled`.
    fn disable(&self, state: &mut State) -> Option<Result<(), OsStatus>> {
        if !state.enabled {
            return None;
        }
        let result = self.api.disable();
        state.enabled = result.is_err();
        Some(result)
    }

    /// A guard is alive, or a Disable failed and its Enable is still
    /// outstanding.
    fn held(&self) -> bool {
        let state = self.lock();
        state.holders > 0 || state.enabled
    }

    fn system_enabled(&self) -> bool {
        self.api.is_main_thread() && self.api.is_enabled()
    }

    fn note_disable(&self, disabled: Option<Result<(), OsStatus>>) {
        if let Some(Err(status)) = disabled {
            self.note(&format!(
                "DisableSecureEventInput failed with {status}; the next release tries again"
            ));
        }
    }

    fn note(&self, line: &str) {
        eprintln!("ferrispass: {line}");
        #[cfg(test)]
        self.notes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(line.to_owned());
    }
}

/// Keeps secure event input on while it lives. Dropping the last guard turns
/// it off; [`release_all`] turns it off for every guard at once, and a guard
/// dropped after that does nothing.
///
/// Not `Send`: a guard is dropped on the main thread, where it was taken.
#[must_use = "secure event input goes off again as soon as the guard is dropped"]
pub struct SecureInputGuard {
    switch: &'static Switch,
    generation: u64,
    _not_send: PhantomData<*const ()>,
}

impl Drop for SecureInputGuard {
    fn drop(&mut self) {
        self.switch.release(self.generation);
    }
}

/// The guard has to stay on the thread that took it. This must not compile:
///
/// ```compile_fail,E0277
/// fn assert_send<T: Send>() {}
/// assert_send::<ferrispass::platform::secure_input::SecureInputGuard>();
/// ```
#[cfg(doctest)]
pub fn guard_is_not_send() {}

/// Turns secure event input on, or keeps it on, until the guard drops. Main
/// thread only. On failure nothing is held, and the reason is in the error
/// and, for a refusal by macOS, in the log.
pub fn hold(cx: &App) -> Result<SecureInputGuard, HoldError> {
    switch(cx).hold()
}

/// Turns secure event input off for every guard there is, at once. Guards
/// taken before stay harmless: dropping them changes nothing. Runs at lock
/// and quit; calling it again does nothing. Main thread only.
pub fn release_all(cx: &App) {
    switch(cx).release_all();
}

/// Whether this app holds secure event input right now.
pub fn held(cx: &App) -> bool {
    switch(cx).held()
}

/// Whether secure event input is on for the login session, held by this app
/// or by any other. Main thread only, like every Carbon call; on any other
/// thread this answers `false`.
pub fn system_enabled() -> bool {
    SYSTEM.system_enabled()
}

/// Releases at quit and on a panic. Called once at launch.
///
/// Quitting runs `release_all` from the app's quit handler, the second chance
/// after the quit action's own call. The panic hook runs before the hook that
/// was there, so secure event input is off before anything is printed or the
/// process unwinds. It is installed once per process. Where secure event
/// input does not exist, this writes one line to the log saying so.
pub fn install_hooks(cx: &mut App) {
    let switch = switch(cx);
    if !switch.api.is_supported() {
        switch.note("secure event input is not available on this platform");
    }
    cx.on_app_quit(|cx| {
        release_all(cx);
        async {}
    })
    .detach();

    static PANIC_HOOK: Once = Once::new();
    PANIC_HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            SYSTEM.release_after_panic();
            previous(info);
        }));
    });
}

fn switch(#[cfg_attr(not(test), allow(unused_variables))] cx: &App) -> &'static Switch {
    #[cfg(test)]
    if let Some(installed) = cx.try_global::<fake::Installed>() {
        return installed.0;
    }
    &SYSTEM
}

/// Carbon's secure event input. The current SDK no longer declares these
/// three functions in a header (older SDKs had them in HIToolbox's
/// `CarbonEvents.h`), but HIToolbox still exports them; they are listed in
/// the SDK's `Carbon.tbd`. Their C signatures are `OSStatus
/// EnableSecureEventInput(void)`, `OSStatus DisableSecureEventInput(void)`
/// and `Boolean IsSecureEventInputEnabled(void)`, where `OSStatus` is a
/// 32-bit signed integer and `Boolean` an unsigned char.
#[cfg(target_os = "macos")]
mod carbon {
    use super::{HoldError, OsStatus, SecureInputApi};

    #[link(name = "Carbon", kind = "framework")]
    unsafe extern "C" {
        fn EnableSecureEventInput() -> i32;
        fn DisableSecureEventInput() -> i32;
        fn IsSecureEventInputEnabled() -> u8;
    }

    pub(super) struct Carbon;

    impl SecureInputApi for Carbon {
        fn enable(&self) -> Result<(), HoldError> {
            // SAFETY: no arguments, no pointers, and the declaration above
            // matches the C signature. `Switch::hold` calls this on the main
            // thread only, and only when its count goes from 0 to 1.
            let status = unsafe { EnableSecureEventInput() };
            if status == 0 {
                Ok(())
            } else {
                Err(HoldError::Os(OsStatus(status)))
            }
        }

        fn disable(&self) -> Result<(), OsStatus> {
            // SAFETY: as for `EnableSecureEventInput`. `Switch` calls this on
            // the main thread, and only while an Enable of its own is
            // outstanding: when its count goes from 1 to 0, or at
            // `release_all`. After a failed Disable it calls it again at the
            // next release, never for an Enable that was not made.
            let status = unsafe { DisableSecureEventInput() };
            if status == 0 {
                Ok(())
            } else {
                Err(OsStatus(status))
            }
        }

        fn is_enabled(&self) -> bool {
            // SAFETY: a query without arguments that changes nothing; the
            // declaration matches the C signature. Main thread only, checked
            // by `Switch::system_enabled`.
            unsafe { IsSecureEventInputEnabled() != 0 }
        }

        fn is_main_thread(&self) -> bool {
            objc2::MainThreadMarker::new().is_some()
        }

        fn is_supported(&self) -> bool {
            true
        }
    }
}

/// Every platform other than macOS. There is no secure event input to turn
/// on, so a hold is refused instead of pretending to protect anything. Also
/// built for tests on macOS, so the tests can check it there.
#[cfg(any(test, not(target_os = "macos")))]
mod noop {
    use super::{HoldError, OsStatus, SecureInputApi};

    pub(super) struct Noop;

    impl SecureInputApi for Noop {
        fn enable(&self) -> Result<(), HoldError> {
            Err(HoldError::Unsupported)
        }

        fn disable(&self) -> Result<(), OsStatus> {
            Ok(())
        }

        fn is_enabled(&self) -> bool {
            false
        }

        /// Nothing is ever turned on here, so no thread rule applies.
        /// Answering `true` lets `hold` reach `enable` and report
        /// `Unsupported` rather than a thread problem.
        fn is_main_thread(&self) -> bool {
            true
        }

        fn is_supported(&self) -> bool {
            false
        }
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread::{self, ThreadId};

    use super::*;

    type DisableHook = Box<dyn FnOnce() + Send>;

    /// Stands in for Carbon: counts the calls, refuses when told to, and
    /// treats the thread that made it as the main thread.
    pub(crate) struct FakeApi {
        main: ThreadId,
        refusal: Option<HoldError>,
        enables: AtomicUsize,
        disables: AtomicUsize,
        most_outstanding: AtomicUsize,
        refuse_next_disable: Mutex<Option<OsStatus>>,
        during_disable: Mutex<Option<DisableHook>>,
    }

    impl FakeApi {
        pub(crate) fn new() -> Self {
            Self {
                main: thread::current().id(),
                refusal: None,
                enables: AtomicUsize::new(0),
                disables: AtomicUsize::new(0),
                most_outstanding: AtomicUsize::new(0),
                refuse_next_disable: Mutex::new(None),
                during_disable: Mutex::new(None),
            }
        }

        /// Every Enable fails with `error`.
        pub(crate) fn refusing(error: HoldError) -> Self {
            Self {
                refusal: Some(error),
                ..Self::new()
            }
        }

        pub(crate) fn enables(&self) -> usize {
            self.enables.load(Ordering::SeqCst)
        }

        /// Disables that succeeded.
        pub(crate) fn disables(&self) -> usize {
            self.disables.load(Ordering::SeqCst)
        }

        /// The most Enables that were ever outstanding at once.
        pub(crate) fn most_outstanding(&self) -> usize {
            self.most_outstanding.load(Ordering::SeqCst)
        }

        /// The next Disable fails with `status` and changes nothing.
        pub(crate) fn refuse_next_disable(&self, status: OsStatus) {
            *self
                .refuse_next_disable
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(status);
        }

        /// Runs `hook` inside the next Disable, while the switch still holds
        /// its lock.
        pub(crate) fn during_next_disable(&self, hook: impl FnOnce() + Send + 'static) {
            *self
                .during_disable
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(Box::new(hook));
        }
    }

    impl SecureInputApi for FakeApi {
        fn enable(&self) -> Result<(), HoldError> {
            if let Some(error) = self.refusal {
                return Err(error);
            }
            let enables = self.enables.fetch_add(1, Ordering::SeqCst) + 1;
            self.most_outstanding
                .fetch_max(enables - self.disables(), Ordering::SeqCst);
            Ok(())
        }

        fn disable(&self) -> Result<(), OsStatus> {
            let refused = self
                .refuse_next_disable
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            if let Some(status) = refused {
                return Err(status);
            }
            self.disables.fetch_add(1, Ordering::SeqCst);
            let hook = self
                .during_disable
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            if let Some(hook) = hook {
                hook();
            }
            Ok(())
        }

        fn is_enabled(&self) -> bool {
            self.enables() > self.disables()
        }

        fn is_main_thread(&self) -> bool {
            thread::current().id() == self.main
        }

        fn is_supported(&self) -> bool {
            self.refusal != Some(HoldError::Unsupported)
        }
    }

    /// A fake and a switch of its own. Both are leaked: a guard keeps a
    /// `'static` reference to its switch.
    #[derive(Clone, Copy)]
    pub(crate) struct Fake {
        pub(crate) api: &'static FakeApi,
        pub(crate) switch: &'static Switch,
    }

    impl Fake {
        pub(crate) fn new(api: FakeApi) -> Self {
            let api: &'static FakeApi = Box::leak(Box::new(api));
            let switch: &'static Switch = Box::leak(Box::new(Switch::new(api)));
            Self { api, switch }
        }

        pub(crate) fn hold(&self) -> Result<SecureInputGuard, HoldError> {
            self.switch.hold()
        }

        pub(crate) fn held(&self) -> bool {
            self.switch.held()
        }

        /// The lines this switch wrote to the log.
        pub(crate) fn notes(&self) -> Vec<String> {
            self.switch
                .notes
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }
    }

    pub(super) struct Installed(pub(super) &'static Switch);

    impl gpui::Global for Installed {}

    /// From here on, everything that reaches secure event input through `cx`
    /// uses this fake instead of Carbon.
    pub(crate) fn install(cx: &mut App, api: FakeApi) -> Fake {
        let fake = Fake::new(api);
        cx.set_global(Installed(fake.switch));
        fake
    }
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use super::fake::{Fake, FakeApi};
    use super::*;

    fn fake() -> Fake {
        Fake::new(FakeApi::new())
    }

    #[test]
    fn at_most_one_enable_outstanding() {
        let fake = fake();
        let guards: Vec<_> = (0..3).map(|_| fake.hold().expect("hold")).collect();
        assert_eq!(fake.api.enables(), 1, "three guards, one Enable");
        assert!(fake.api.is_enabled());
        drop(guards);
        assert_eq!(fake.api.disables(), 1);
        assert!(!fake.api.is_enabled());
    }

    #[test]
    fn overlapping_guards_release_only_after_the_last_drop() {
        let fake = fake();
        let first = fake.hold().expect("hold");
        let second = fake.hold().expect("hold");
        drop(first);
        assert!(fake.held(), "the second guard still holds");
        assert_eq!(
            fake.api.disables(),
            0,
            "dropping one of two guards switched it off"
        );
        assert!(fake.api.is_enabled());
        drop(second);
        assert!(!fake.held());
        assert_eq!(fake.api.disables(), 1);
    }

    #[test]
    fn drop_releases() {
        let fake = fake();
        let guard = fake.hold().expect("hold");
        assert!(fake.held());
        drop(guard);
        assert!(!fake.held(), "the dropped guard still counts");
        assert_eq!(fake.api.enables(), 1);
        assert_eq!(
            fake.api.disables(),
            1,
            "dropping the only guard left secure input on"
        );
    }

    #[test]
    fn release_all_is_idempotent() {
        let fake = fake();
        let guard = fake.hold().expect("hold");
        fake.switch.release_all();
        fake.switch.release_all();
        assert_eq!(fake.api.disables(), 1);
        drop(guard);
        fake.switch.release_all();
        assert_eq!(fake.api.disables(), 1, "a second release disabled again");
        assert_eq!(fake.api.enables(), 1);
    }

    #[test]
    fn release_all_with_live_guards_counts_once() {
        let fake = fake();
        let first = fake.hold().expect("hold");
        let second = fake.hold().expect("hold");
        fake.switch.release_all();
        assert!(!fake.held(), "release_all left guards counted");
        assert_eq!(fake.api.disables(), 1);
        drop(first);
        drop(second);
        assert_eq!(
            fake.api.disables(),
            1,
            "guards dropped after release_all disabled again"
        );
        assert!(!fake.api.is_enabled());
    }

    #[test]
    fn hold_after_release_all_enables_again() {
        let fake = fake();
        let old = fake.hold().expect("hold");
        fake.switch.release_all();
        let new = fake.hold().expect("hold");
        assert_eq!(
            fake.api.enables(),
            2,
            "the hold after release_all did not Enable"
        );
        assert!(fake.held());
        drop(new);
        drop(old);
        assert_eq!(fake.api.disables(), 2);
        assert!(!fake.api.is_enabled());
    }

    #[test]
    fn an_old_guard_dropped_after_release_all_leaves_the_new_hold_on() {
        let fake = fake();
        let a = fake.hold().expect("hold");
        fake.switch.release_all();
        let b = fake.hold().expect("hold");
        drop(a);
        assert!(fake.held(), "an old guard switched off the new hold");
        assert_eq!(fake.api.disables(), 1);
        assert!(fake.api.is_enabled());
        drop(b);
        assert!(!fake.held());
        assert_eq!(fake.api.disables(), 2);
    }

    /// A Disable that fails leaves our Enable counted by the system. The
    /// next hold must not add a second one on top, and the next release
    /// tries the Disable again.
    #[test]
    fn a_failed_disable_is_retried_and_never_stacks_an_enable() {
        let fake = fake();
        let guard = fake.hold().expect("hold");
        fake.api.refuse_next_disable(OsStatus(-50));
        drop(guard);
        assert!(fake.api.is_enabled(), "the fake keeps a refused Disable on");
        assert!(
            fake.notes()
                .iter()
                .any(|line| line.contains("DisableSecureEventInput failed with OSStatus -50")),
            "the failed Disable was not logged: {:?}",
            fake.notes()
        );
        let guard = fake.hold().expect("hold");
        assert_eq!(
            fake.api.enables(),
            1,
            "a second Enable on top of one that was never disabled"
        );
        drop(guard);
        assert!(
            !fake.api.is_enabled(),
            "the failed Disable was not tried again"
        );

        // The same through release_all.
        let guard = fake.hold().expect("hold");
        fake.api.refuse_next_disable(OsStatus(-50));
        fake.switch.release_all();
        drop(guard);
        let again = fake.hold().expect("hold");
        drop(again);
        assert_eq!(fake.api.most_outstanding(), 1);
        assert!(!fake.api.is_enabled());
    }

    #[test]
    fn panic_hook_releases_on_the_main_thread() {
        let fake = fake();
        let first = fake.hold().expect("hold");
        let second = fake.hold().expect("hold");
        fake.switch.release_after_panic();
        assert!(!fake.held(), "the panic hook left secure input held");
        assert_eq!(fake.api.disables(), 1);
        drop(first);
        drop(second);
        assert_eq!(fake.api.disables(), 1);
        let again = fake.hold().expect("hold after the panic hook");
        assert_eq!(fake.api.enables(), 2);
        drop(again);
    }

    #[test]
    fn panic_hook_off_the_main_thread_touches_nothing() {
        let fake = fake();
        let guard = fake.hold().expect("hold");
        thread::spawn(move || fake.switch.release_after_panic())
            .join()
            .expect("hook thread");
        assert_eq!(
            fake.api.disables(),
            0,
            "a panic off the main thread called Disable"
        );
        assert_eq!(fake.api.enables(), 1);
        assert!(fake.held());
        drop(guard);
        assert_eq!(fake.api.disables(), 1);
        let again = fake.hold().expect("hold after a background panic");
        assert_eq!(fake.api.enables(), 2);
        drop(again);
    }

    #[test]
    fn failed_enable_holds_nothing() {
        let fake = Fake::new(FakeApi::refusing(HoldError::Os(OsStatus(-50))));
        let result = fake.hold();
        assert!(
            matches!(result, Err(HoldError::Os(OsStatus(-50)))),
            "a refused Enable came back as {:?}",
            result.as_ref().err()
        );
        drop(result);
        assert!(!fake.held(), "a refused Enable still counts as held");
        assert_eq!(fake.api.disables(), 0);
        assert!(
            fake.notes()
                .iter()
                .any(|line| line.contains("OSStatus -50")),
            "the refusal was not logged: {:?}",
            fake.notes()
        );
        // Still refused the second time, not counted as already held.
        assert!(fake.hold().is_err());
        assert!(!fake.held());
    }

    #[test]
    fn hold_off_the_main_thread_is_refused() {
        let fake = fake();
        let refused = thread::spawn(move || fake.hold().err())
            .join()
            .expect("hold thread");
        assert_eq!(refused, Some(HoldError::OffMainThread));
        assert_eq!(fake.api.enables(), 0);
        assert!(!fake.held());
    }

    #[test]
    fn release_all_off_the_main_thread_does_nothing() {
        let fake = fake();
        let guard = fake.hold().expect("hold");
        thread::spawn(move || fake.switch.release_all())
            .join()
            .expect("release thread");
        assert_eq!(fake.api.disables(), 0);
        assert!(fake.held());
        drop(guard);
        assert_eq!(fake.api.disables(), 1);
    }

    #[test]
    fn drop_never_underflows() {
        let fake = fake();
        let guard = fake.hold().expect("hold");
        let generation = guard.generation;
        drop(guard);
        assert_eq!(fake.api.disables(), 1);
        // A second release for the same guard, which only a bug could cause.
        // Debug builds stop at the assertion; either way the count stays at
        // zero and nothing is disabled twice.
        let _ = catch_unwind(AssertUnwindSafe(|| fake.switch.release(generation)));
        assert!(!fake.held());
        assert_eq!(fake.api.disables(), 1, "an extra release disabled again");
        let again = fake.hold().expect("hold");
        assert_eq!(
            fake.api.enables(),
            2,
            "after an extra release the next hold did not Enable"
        );
        drop(again);
        assert_eq!(fake.api.disables(), 2);
    }

    /// A guard from before `release_all`, dropped on another thread while
    /// `release_all` is still inside Disable. The fake makes that possible;
    /// in the app a guard never leaves the main thread.
    #[test]
    fn release_all_and_an_old_drop_on_another_thread_do_not_interleave() {
        let fake = fake();
        let old = fake.hold().expect("hold");
        let generation = old.generation;
        // The other thread stands in for this guard's drop.
        std::mem::forget(old);

        let (done_tx, done_rx) = mpsc::channel();
        let worker = std::sync::Arc::new(Mutex::new(None));
        let worker_slot = worker.clone();
        fake.api.during_next_disable(move || {
            let handle = thread::spawn(move || {
                fake.switch.release(generation);
                let _ = done_tx.send(());
            });
            // Give the drop every chance to run in the middle of release_all.
            // With the lock it cannot, and the wait runs out.
            let _ = done_rx.recv_timeout(Duration::from_millis(200));
            *worker_slot.lock().unwrap() = Some(handle);
        });
        fake.switch.release_all();
        worker
            .lock()
            .unwrap()
            .take()
            .expect("Disable ran the hook")
            .join()
            .expect("drop thread");

        assert!(!fake.held());
        assert_eq!(fake.api.disables(), 1);
        let again = fake.hold().expect("hold");
        assert_eq!(
            fake.api.enables(),
            2,
            "the hold after release_all did not Enable"
        );
        drop(again);
        assert_eq!(fake.api.disables(), 2);
    }

    /// However the app quits, its quit handlers run, and the one from
    /// `install_hooks` releases there.
    #[gpui::test]
    fn quit_hook_releases(cx: &mut gpui::TestAppContext) {
        let fake = cx.update(|cx| fake::install(cx, FakeApi::new()));
        cx.update(install_hooks);
        let guard = cx.update(|cx| hold(cx)).expect("hold");
        assert!(cx.update(|cx| held(cx)));
        cx.quit();
        assert!(!fake.held(), "the quit hook left secure input held");
        assert_eq!(fake.api.disables(), 1);
        drop(guard);
        assert_eq!(fake.api.disables(), 1);
    }

    #[test]
    fn hold_is_unsupported_off_macos() {
        // The no-op is what every build other than macOS uses.
        let switch: &'static Switch = Box::leak(Box::new(Switch::new(&noop::Noop)));
        assert!(matches!(switch.hold(), Err(HoldError::Unsupported)));
        assert!(!switch.held());
        assert!(!switch.system_enabled());
        #[cfg(not(target_os = "macos"))]
        assert!(matches!(SYSTEM.hold(), Err(HoldError::Unsupported)));
    }

    /// The real Carbon calls. libtest runs every test on a worker thread, and
    /// there is no way to get onto the main thread from here, so this test
    /// lets its own thread pass the main-thread check; the app never calls
    /// Carbon anywhere else. If another app already holds secure event
    /// input, the system stays on after the drop, so the test compares with
    /// the state before.
    #[cfg(target_os = "macos")]
    #[test]
    fn carbon_calls_balance() {
        struct CarbonOnThisThread(thread::ThreadId);

        impl SecureInputApi for CarbonOnThisThread {
            fn enable(&self) -> Result<(), HoldError> {
                carbon::Carbon.enable()
            }
            fn disable(&self) -> Result<(), OsStatus> {
                carbon::Carbon.disable()
            }
            fn is_enabled(&self) -> bool {
                carbon::Carbon.is_enabled()
            }
            fn is_main_thread(&self) -> bool {
                thread::current().id() == self.0
            }
            fn is_supported(&self) -> bool {
                true
            }
        }

        let api: &'static CarbonOnThisThread =
            Box::leak(Box::new(CarbonOnThisThread(thread::current().id())));
        let switch: &'static Switch = Box::leak(Box::new(Switch::new(api)));
        let before = switch.system_enabled();
        let guard = switch.hold().expect("EnableSecureEventInput");
        assert!(switch.system_enabled(), "Enable did not switch it on");
        drop(guard);
        assert_eq!(
            switch.system_enabled(),
            before,
            "Disable did not switch it off again"
        );
    }
}
