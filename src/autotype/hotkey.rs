//! Global hotkey registration and event plumbing.
//!
//! `global-hotkey` (Tauri team) wraps the macOS `RegisterEventHotKey`
//! Carbon API behind a safe interface. Constraints worth knowing:
//!
//! - The `GlobalHotKeyManager` MUST be created on a thread that has a
//!   running Core Foundation event loop. On macOS that's the main
//!   thread. GPUI runs its event loop on the main thread, and
//!   `app::services::hotkeys::reconcile`, the one place that makes the
//!   manager, runs there.
//! - The process has one manager, in one [`HotkeyRegistry`], and every
//!   global shortcut goes through it. Each manager installs a Carbon event
//!   handler of its own, so a manager per shortcut would mean one handler
//!   per shortcut for the same stream of events.
//! - The manager owns OS resources. The registry unregisters what it
//!   registered when it is dropped.
//! - Events arrive on a process-wide crossbeam channel. We don't `recv()`
//!   blocking (which would hang the executor); [`take_events`] drains it with
//!   `try_recv` on a periodic timer driven by the GPUI executor. ~30 Hz
//!   feels instant to the user and costs essentially nothing.

use std::str::FromStr;

use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, hotkey::HotKey};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum HotkeyError {
    /// User-typed combo isn't a valid hotkey string. We surface this in
    /// the Settings UI when the user types something `global-hotkey`
    /// can't parse - better than silently registering nothing and
    /// having the feature appear broken.
    #[error("invalid hotkey combo \"{combo}\": {source}")]
    Parse {
        combo: String,
        #[source]
        source: global_hotkey::hotkey::HotKeyParseError,
    },
    /// macOS refused the registration. Usually means the combo is
    /// already taken by another global hotkey (Spotlight, a window
    /// manager, a different password manager).
    #[error("OS rejected hotkey registration: {0}")]
    Register(String),
    /// The hotkey manager itself failed to initialise. Shouldn't
    /// happen in practice - the only documented cause is calling
    /// from a thread without a CF event loop, which we control for.
    #[error("could not create hotkey manager: {0}")]
    Init(String),
}

/// The default combo. ⌃⌥⌘V matches KeePassXC's macOS default and
/// avoids the well-known reserved combos (Spotlight ⌘Space, Mission
/// Control ⌃↑, Dock toggle ⌘⌥D).
pub const DEFAULT_HOTKEY: &str = "ctrl+alt+super+KeyV";

/// What the registry needs from the system: `global-hotkey`'s manager in the
/// app, and a fake that counts the calls in tests.
pub trait HotkeyBackend: Sized {
    /// Makes the manager. The registry calls this once, when it is made.
    fn create() -> Result<Self, HotkeyError>;
    /// Registers `hotkey` with the OS.
    fn register(&self, hotkey: HotKey) -> Result<(), HotkeyError>;
    /// Unregisters `hotkey`. Best effort: when the OS refuses (the process
    /// is tearing down and Carbon's table is gone), there is nothing left
    /// to do about it.
    fn unregister(&self, hotkey: HotKey);
}

impl HotkeyBackend for GlobalHotKeyManager {
    fn create() -> Result<Self, HotkeyError> {
        GlobalHotKeyManager::new().map_err(|e| HotkeyError::Init(e.to_string()))
    }

    fn register(&self, hotkey: HotKey) -> Result<(), HotkeyError> {
        GlobalHotKeyManager::register(self, hotkey)
            .map_err(|e| HotkeyError::Register(e.to_string()))
    }

    fn unregister(&self, hotkey: HotKey) {
        let _ = GlobalHotKeyManager::unregister(self, hotkey);
    }
}

/// The one manager and the shortcuts registered through it. Drop =
/// unregister them all.
///
/// Registration has two failure modes worth distinguishing for the UI: a
/// bad combo (user fixable in Settings) vs. an OS conflict (different
/// remediation: change the combo or quit the conflicting app). The error
/// enum splits them.
pub struct HotkeyRegistry<B: HotkeyBackend = GlobalHotKeyManager> {
    manager: B,
    registered: Vec<HotKey>,
}

impl<B: HotkeyBackend> HotkeyRegistry<B> {
    /// Makes the manager, with nothing registered yet. Must run on the main
    /// thread (see the module docs).
    pub fn new() -> Result<Self, HotkeyError> {
        Ok(Self {
            manager: B::create()?,
            registered: Vec::new(),
        })
    }

    /// Registers `combo` through the one manager and returns its id, the id
    /// its events carry. A combo this registry already holds is refused
    /// rather than registered twice.
    pub fn register(&mut self, combo: &str) -> Result<u32, HotkeyError> {
        let hotkey = parse_combo(combo)?;
        if self.registered.iter().any(|held| held.id() == hotkey.id()) {
            return Err(HotkeyError::Register(format!(
                "\"{combo}\" is already registered by FerrisPass"
            )));
        }
        self.manager.register(hotkey)?;
        self.registered.push(hotkey);
        Ok(hotkey.id())
    }

    /// Unregisters the shortcut with this id. An id that is not registered
    /// here changes nothing.
    pub fn unregister(&mut self, id: u32) {
        if let Some(at) = self.registered.iter().position(|held| held.id() == id) {
            let hotkey = self.registered.remove(at);
            self.manager.unregister(hotkey);
        }
    }
}

impl<B: HotkeyBackend> Drop for HotkeyRegistry<B> {
    fn drop(&mut self) {
        for hotkey in self.registered.drain(..) {
            self.manager.unregister(hotkey);
        }
    }
}

/// Validate (but don't register) a hotkey string. Used by the Settings
/// UI to surface a parse error without taking the OS-registration
/// hit on every keystroke the user types into the field.
pub fn parse_combo(combo: &str) -> Result<HotKey, HotkeyError> {
    HotKey::from_str(combo).map_err(|source| HotkeyError::Parse {
        combo: combo.to_string(),
        source,
    })
}

/// Every event the registered shortcuts sent since the last call, oldest
/// first, presses and releases alike. Never blocks: an empty channel gives an
/// empty list. Draining it all on every pass keeps a backlog from building
/// up; telling the events apart is `app::services::hotkeys::drain_pressed`'s
/// job.
pub fn take_events() -> Vec<GlobalHotKeyEvent> {
    std::iter::from_fn(|| GlobalHotKeyEvent::receiver().try_recv().ok()).collect()
}

/// A backend that stands in for the OS in tests, and an event channel the
/// tests can send on. `global-hotkey` keeps its own sender private, so tests
/// cannot put an event on the real channel.
///
/// Both are per thread: the GPUI test executor runs a test's tasks on the
/// test's own thread, so tests running side by side see only their own
/// calls and events.
#[cfg(test)]
pub(crate) mod fake {
    use std::cell::RefCell;

    use super::*;

    /// What the backends on this thread were asked to do.
    #[derive(Clone, Debug, Default, PartialEq, Eq)]
    pub(crate) struct Calls {
        /// Managers made.
        pub(crate) managers: usize,
        /// Ids registered, in order.
        pub(crate) registered: Vec<u32>,
        /// Ids unregistered, in order.
        pub(crate) unregistered: Vec<u32>,
    }

    thread_local! {
        static CALLS: RefCell<Calls> = RefCell::default();
        static REFUSED: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
        static EVENTS: RefCell<Vec<GlobalHotKeyEvent>> = const { RefCell::new(Vec::new()) };
    }

    /// Forgets the calls, refusals and events on this thread. With one test
    /// thread, tests run one after another on the same thread, so a test
    /// that counts calls starts here.
    pub(crate) fn reset() {
        CALLS.with(|calls| *calls.borrow_mut() = Calls::default());
        REFUSED.with(|refused| refused.borrow_mut().clear());
        EVENTS.with(|events| events.borrow_mut().clear());
    }

    /// The calls so far on this thread.
    pub(crate) fn calls() -> Calls {
        CALLS.with(|calls| calls.borrow().clone())
    }

    /// From here on, the OS refuses `combo` on this thread, the way it does
    /// when another app holds it.
    pub(crate) fn refuse(combo: &str) {
        let id = parse_combo(combo).expect("a combo that parses").id();
        REFUSED.with(|refused| refused.borrow_mut().push(id));
    }

    /// Puts `event` on this thread's channel.
    pub(crate) fn send(event: GlobalHotKeyEvent) {
        EVENTS.with(|events| events.borrow_mut().push(event));
    }

    /// [`super::take_events`] for this thread's channel.
    pub(crate) fn take_events() -> Vec<GlobalHotKeyEvent> {
        EVENTS.with(|events| std::mem::take(&mut *events.borrow_mut()))
    }

    pub(crate) struct FakeBackend;

    impl HotkeyBackend for FakeBackend {
        fn create() -> Result<Self, HotkeyError> {
            CALLS.with(|calls| calls.borrow_mut().managers += 1);
            Ok(Self)
        }

        fn register(&self, hotkey: HotKey) -> Result<(), HotkeyError> {
            if REFUSED.with(|refused| refused.borrow().contains(&hotkey.id())) {
                return Err(HotkeyError::Register("already taken".to_string()));
            }
            CALLS.with(|calls| calls.borrow_mut().registered.push(hotkey.id()));
            Ok(())
        }

        fn unregister(&self, hotkey: HotKey) {
            CALLS.with(|calls| calls.borrow_mut().unregistered.push(hotkey.id()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{self, FakeBackend};
    use super::*;

    #[test]
    fn parses_default_combo() {
        // Sanity: the constant we ship as a default must itself parse.
        // If we ever change `DEFAULT_HOTKEY` to something invalid this
        // test fails before users hit it at startup.
        assert!(parse_combo(DEFAULT_HOTKEY).is_ok());
    }

    #[test]
    fn rejects_invalid_combo() {
        assert!(parse_combo("not a real hotkey").is_err());
        // Empty string - would silently fail on register, surface
        // the error at parse time instead.
        assert!(parse_combo("").is_err());
    }

    #[test]
    fn parses_lowercase_and_alternate_modifier_names() {
        // global-hotkey accepts these spellings; we lean on the crate
        // for full coverage but pin a few representative combos so a
        // crate upgrade that breaks them shows up in CI.
        assert!(parse_combo("ctrl+alt+KeyA").is_ok());
        assert!(parse_combo("shift+super+KeyZ").is_ok());
    }

    /// Two combos through one registry are two registrations on one
    /// manager, with two ids, and dropping the registry unregisters both.
    #[test]
    fn a_registry_holds_several_shortcuts_on_one_manager() {
        fake::reset();
        let mut registry = HotkeyRegistry::<FakeBackend>::new().expect("registry");
        let first = registry.register("ctrl+alt+super+KeyP").expect("first");
        let second = registry.register(DEFAULT_HOTKEY).expect("second");
        assert_ne!(first, second);
        assert_eq!(fake::calls().managers, 1);
        assert_eq!(fake::calls().registered, vec![first, second]);

        // Unregistering an id that is not there changes nothing.
        registry.unregister(first ^ 1);
        assert!(fake::calls().unregistered.is_empty());

        drop(registry);
        assert_eq!(fake::calls().unregistered, vec![first, second]);
        assert_eq!(fake::calls().managers, 1);
    }

    #[test]
    fn a_combo_is_registered_once() {
        fake::reset();
        let mut registry = HotkeyRegistry::<FakeBackend>::new().expect("registry");
        registry.register(DEFAULT_HOTKEY).expect("first");
        assert!(matches!(
            registry.register(DEFAULT_HOTKEY),
            Err(HotkeyError::Register(_))
        ));
        assert_eq!(fake::calls().registered.len(), 1);
        assert!(matches!(
            registry.register("not a real hotkey"),
            Err(HotkeyError::Parse { .. })
        ));
    }

    #[test]
    fn a_refused_combo_is_not_held() {
        fake::reset();
        fake::refuse(DEFAULT_HOTKEY);
        let mut registry = HotkeyRegistry::<FakeBackend>::new().expect("registry");
        assert!(matches!(
            registry.register(DEFAULT_HOTKEY),
            Err(HotkeyError::Register(_))
        ));
        drop(registry);
        assert!(
            fake::calls().unregistered.is_empty(),
            "a combo the OS refused was unregistered as if it were held"
        );
    }
}
