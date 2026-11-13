//! One listener for the global shortcuts.
//!
//! FerrisPass has two: ⌃⌥⌘P opens the quick access panel, and the Auto-Type
//! shortcut (⌃⌥⌘V unless the user picked another) types into the app in
//! front. Both are registered through one [`HotkeyRegistry`], so the process
//! has one `GlobalHotKeyManager`, and one poll loop reads the channel both
//! report on. The loop runs at app level, not on a window: the panel opens
//! without one, and hiding the main window with Cmd+W does not stop it.
//!
//! - Each pass takes every event that arrived and hands each press of a
//!   registered shortcut on, in the order they came ([`drain_pressed`]).
//!   Releases and ids that are not ours are dropped. A pass hands on
//!   everything it took, so one shortcut pressed often cannot hold back
//!   the other.
//! - A press of the quick access shortcut counts as activity
//!   (`activity::touch(Hotkey)`) when it is handed on. Being registered
//!   does not count, and nothing else that happens in another app reaches
//!   the idle clock from here.
//! - While the quick access panel is open, a press of the Auto-Type
//!   shortcut runs nothing. The panel has the keyboard while the app behind
//!   it stays in front, so Auto-Type would find that app's entry and type
//!   the user name and the password into the panel's search field.
//! - [`reconcile`] registers and unregisters to match the settings. The
//!   manager is made the first time a shortcut is wanted and kept for the
//!   life of the process; switching both shortcuts off unregisters them and
//!   stops the loop, and switching one back on registers it on the same
//!   manager. What a save changes is let go of before anything is
//!   registered, so a combo one shortcut gives up goes to the other in the
//!   same save.

use std::time::Duration;

use global_hotkey::{GlobalHotKeyEvent, HotKeyState};
use gpui::{App, Global, Task};

use crate::app::AppSettings;
use crate::app::services::activity::{self, ActivitySource};
use crate::autotype::hotkey::{self, HotkeyRegistry};
use crate::ui::AppShell;

/// The quick access shortcut, ⌃⌥⌘P. It is not configurable; the settings
/// only switch it on and off.
pub const QUICK_ACCESS_HOTKEY: &str = "ctrl+alt+super+KeyP";

/// How often the loop drains the channel. ~30 Hz: fast enough that nobody
/// notices a delay between the press and what it opens, slow enough that
/// an idle app pays nothing for it.
const POLL_INTERVAL: Duration = Duration::from_millis(33);

#[cfg(not(test))]
type Registry = HotkeyRegistry;
/// Unit tests never register a shortcut with the OS.
#[cfg(test)]
type Registry = HotkeyRegistry<hotkey::fake::FakeBackend>;

#[cfg(test)]
use hotkey::fake::take_events;
#[cfg(not(test))]
use hotkey::take_events;

/// The global shortcuts FerrisPass registers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortcut {
    QuickAccess,
    AutoType,
}

/// The ids the registered shortcuts' events carry. `None` for a shortcut
/// that is not registered, because it is switched off or its registration
/// failed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Registered {
    pub quick_access: Option<u32>,
    pub auto_type: Option<u32>,
}

impl Registered {
    fn shortcut(&self, id: u32) -> Option<Shortcut> {
        if self.quick_access == Some(id) {
            Some(Shortcut::QuickAccess)
        } else if self.auto_type == Some(id) {
            Some(Shortcut::AutoType)
        } else {
            None
        }
    }

    fn slot(&mut self, shortcut: Shortcut) -> &mut Option<u32> {
        match shortcut {
            Shortcut::QuickAccess => &mut self.quick_access,
            Shortcut::AutoType => &mut self.auto_type,
        }
    }
}

/// The shortcuts pressed in `events`, in order: one entry per press of a
/// registered shortcut. Releases and events with an id that is not in `ids`
/// are dropped. Pure, so the order and the filtering can be tested without
/// the OS.
pub fn drain_pressed(
    events: impl Iterator<Item = GlobalHotKeyEvent>,
    ids: &Registered,
) -> Vec<Shortcut> {
    events
        .filter(|event| event.state == HotKeyState::Pressed)
        .filter_map(|event| ids.shortcut(event.id))
        .collect()
}

/// The one registry, what is registered on it, and the loop that reads the
/// channel.
#[derive(Default)]
struct HotkeyLoop {
    /// `None` until a shortcut is first wanted, and while making the
    /// manager fails.
    registry: Option<Registry>,
    registered: Registered,
    errors: Errors,
    /// Runs while at least one shortcut is registered. Drop = stop.
    task: Option<Task<()>>,
}

impl Global for HotkeyLoop {}

/// Why a shortcut the settings ask for is not registered.
#[derive(Default)]
struct Errors {
    quick_access: Option<String>,
    auto_type: Option<String>,
}

impl Errors {
    fn slot(&mut self, shortcut: Shortcut) -> &mut Option<String> {
        match shortcut {
            Shortcut::QuickAccess => &mut self.quick_access,
            Shortcut::AutoType => &mut self.auto_type,
        }
    }
}

impl HotkeyLoop {
    /// Registers `combo` for `shortcut`, or unregisters it for `None`. A
    /// shortcut that is already registered with the same combo stays as it
    /// is, so saving unrelated settings does not touch the OS.
    fn apply(&mut self, shortcut: Shortcut, combo: Option<&str>) {
        let wanted = match combo.map(hotkey::parse_combo) {
            None => None,
            Some(Ok(hotkey)) => Some(hotkey.id()),
            Some(Err(error)) => {
                self.unregister(shortcut);
                *self.errors.slot(shortcut) = Some(error.to_string());
                return;
            }
        };
        if *self.registered.slot(shortcut) == wanted {
            *self.errors.slot(shortcut) = None;
            return;
        }
        self.unregister(shortcut);
        *self.errors.slot(shortcut) = None;
        let Some(combo) = combo else {
            return;
        };
        if self.registry.is_none() {
            match Registry::new() {
                Ok(registry) => self.registry = Some(registry),
                Err(error) => {
                    *self.errors.slot(shortcut) = Some(error.to_string());
                    return;
                }
            }
        }
        let Some(registry) = &mut self.registry else {
            return;
        };
        match registry.register(combo) {
            Ok(id) => *self.registered.slot(shortcut) = Some(id),
            Err(error) => *self.errors.slot(shortcut) = Some(error.to_string()),
        }
    }

    /// Unregisters `shortcut` when the settings no longer ask for the combo
    /// it holds, so the other shortcut can take that combo in the same save.
    fn release_if_changed(&mut self, shortcut: Shortcut, combo: Option<&str>) {
        let wanted = combo
            .and_then(|combo| hotkey::parse_combo(combo).ok())
            .map(|hotkey| hotkey.id());
        if *self.registered.slot(shortcut) != wanted {
            self.unregister(shortcut);
        }
    }

    fn unregister(&mut self, shortcut: Shortcut) {
        if let Some(id) = self.registered.slot(shortcut).take()
            && let Some(registry) = &mut self.registry
        {
            registry.unregister(id);
        }
    }
}

/// Registers and unregisters the shortcuts to match `settings`: the quick
/// access shortcut with `quick_access_hotkey_enabled`, the Auto-Type one with
/// `auto_type_enabled` and `auto_type_hotkey`. Starts the loop when the first
/// shortcut is registered and stops it when none is left. Main thread only,
/// like every call into the manager.
pub fn reconcile(settings: &AppSettings, cx: &mut App) {
    let quick_access = settings
        .quick_access_hotkey_enabled
        .then_some(QUICK_ACCESS_HOTKEY);
    let auto_type = settings
        .auto_type_enabled
        .then_some(settings.auto_type_hotkey.as_str());

    let hotkeys = cx.default_global::<HotkeyLoop>();
    hotkeys.release_if_changed(Shortcut::QuickAccess, quick_access);
    hotkeys.release_if_changed(Shortcut::AutoType, auto_type);
    hotkeys.apply(Shortcut::QuickAccess, quick_access);
    hotkeys.apply(Shortcut::AutoType, auto_type);
    let wanted = hotkeys.registered != Registered::default();
    if !wanted {
        hotkeys.task = None;
        return;
    }
    if hotkeys.task.is_none() {
        let task = cx.spawn(async move |cx| {
            loop {
                cx.background_executor().timer(POLL_INTERVAL).await;
                let events = take_events();
                if !events.is_empty() {
                    cx.update(|cx| handle(events, cx));
                }
            }
        });
        cx.global_mut::<HotkeyLoop>().task = Some(task);
    }
}

/// The ids of the shortcuts registered now.
pub fn registered(cx: &App) -> Registered {
    cx.try_global::<HotkeyLoop>()
        .map(|hotkeys| hotkeys.registered)
        .unwrap_or_default()
}

/// Why `shortcut` is not registered although the settings ask for it, for
/// the settings screen: a combo that does not parse, one macOS refused, or a
/// manager that could not be made. `None` when it is registered or switched
/// off.
pub fn error(shortcut: Shortcut, cx: &App) -> Option<String> {
    let hotkeys = cx.try_global::<HotkeyLoop>()?;
    match shortcut {
        Shortcut::QuickAccess => hotkeys.errors.quick_access.clone(),
        Shortcut::AutoType => hotkeys.errors.auto_type.clone(),
    }
}

/// Hands each press in `events` on: the quick access shortcut opens or
/// closes the panel and counts as activity, the Auto-Type shortcut goes to
/// the main window, the way it did when the loop ran on the window, unless
/// the panel is open.
fn handle(events: Vec<GlobalHotKeyEvent>, cx: &mut App) {
    let ids = registered(cx);
    for shortcut in drain_pressed(events.into_iter(), &ids) {
        match shortcut {
            Shortcut::QuickAccess => {
                activity::touch(ActivitySource::Hotkey);
                crate::ui::windows::panel::toggle(cx);
            }
            Shortcut::AutoType => {
                // The typing would go to the panel, which has the keyboard.
                if crate::ui::windows::panel::is_open(cx) {
                    continue;
                }
                if let Some(shell) = crate::app::with_shared_shell(cx, |_, cx| cx.entity()) {
                    AppShell::dispatch_global_auto_type(shell, cx);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use global_hotkey::{GlobalHotKeyEvent, HotKeyState};
    use gpui::TestAppContext;

    use super::*;
    use crate::autotype::hotkey::{DEFAULT_HOTKEY, fake, parse_combo};

    const QUICK: u32 = 7;
    const AUTO: u32 = 11;

    fn pressed(id: u32) -> GlobalHotKeyEvent {
        GlobalHotKeyEvent {
            id,
            state: HotKeyState::Pressed,
        }
    }

    fn released(id: u32) -> GlobalHotKeyEvent {
        GlobalHotKeyEvent {
            id,
            state: HotKeyState::Released,
        }
    }

    fn both() -> Registered {
        Registered {
            quick_access: Some(QUICK),
            auto_type: Some(AUTO),
        }
    }

    fn id_of(combo: &str) -> u32 {
        parse_combo(combo).expect("combo").id()
    }

    fn settings(quick_access: bool, auto_type: bool) -> AppSettings {
        AppSettings {
            quick_access_hotkey_enabled: quick_access,
            auto_type_enabled: auto_type,
            auto_type_hotkey: DEFAULT_HOTKEY.to_string(),
            ..AppSettings::default()
        }
    }

    fn poll(cx: &mut TestAppContext) {
        cx.executor().advance_clock(POLL_INTERVAL);
        cx.run_until_parked();
    }

    #[test]
    fn drain_pressed_hands_both_shortcuts_on() {
        let shortcuts = drain_pressed([pressed(AUTO), pressed(QUICK)].into_iter(), &both());
        assert_eq!(shortcuts, vec![Shortcut::AutoType, Shortcut::QuickAccess]);

        let shortcuts = drain_pressed(
            [pressed(QUICK), pressed(AUTO), pressed(QUICK)].into_iter(),
            &both(),
        );
        assert_eq!(
            shortcuts,
            vec![
                Shortcut::QuickAccess,
                Shortcut::AutoType,
                Shortcut::QuickAccess
            ]
        );
    }

    #[test]
    fn drain_pressed_drops_released_and_foreign_events() {
        let shortcuts = drain_pressed(
            [
                released(QUICK),
                pressed(99),
                released(AUTO),
                released(99),
                pressed(AUTO),
            ]
            .into_iter(),
            &both(),
        );
        assert_eq!(shortcuts, vec![Shortcut::AutoType]);
        assert!(drain_pressed([pressed(QUICK)].into_iter(), &Registered::default()).is_empty());
    }

    #[test]
    fn drain_pressed_with_one_registration_ignores_the_other_id() {
        let quick_only = Registered {
            quick_access: Some(QUICK),
            auto_type: None,
        };
        assert_eq!(
            drain_pressed([pressed(AUTO), pressed(QUICK)].into_iter(), &quick_only),
            vec![Shortcut::QuickAccess]
        );
        let auto_only = Registered {
            quick_access: None,
            auto_type: Some(AUTO),
        };
        assert_eq!(
            drain_pressed([pressed(AUTO), pressed(QUICK)].into_iter(), &auto_only),
            vec![Shortcut::AutoType]
        );
    }

    #[gpui::test]
    fn one_manager_carries_both_registrations(cx: &mut TestAppContext) {
        fake::reset();
        let quick = id_of(QUICK_ACCESS_HOTKEY);
        let auto = id_of(DEFAULT_HOTKEY);

        cx.update(|cx| reconcile(&settings(true, true), cx));
        let calls = fake::calls();
        assert_eq!(calls.managers, 1, "one manager for both shortcuts");
        assert_eq!(calls.registered, vec![quick, auto]);
        assert_ne!(quick, auto);

        cx.update(|cx| reconcile(&settings(false, false), cx));
        let calls = fake::calls();
        assert_eq!(calls.unregistered, vec![quick, auto]);
        assert_eq!(calls.managers, 1, "switching off made or dropped a manager");

        cx.update(|cx| reconcile(&settings(true, true), cx));
        let calls = fake::calls();
        assert_eq!(calls.registered, vec![quick, auto, quick, auto]);
        assert_eq!(calls.managers, 1, "switching back on made a second manager");

        // Saving again with nothing changed leaves the OS alone.
        cx.update(|cx| reconcile(&settings(true, true), cx));
        assert_eq!(fake::calls(), calls);
    }

    #[gpui::test]
    fn reconcile_registers_only_what_the_settings_ask_for(cx: &mut TestAppContext) {
        fake::reset();
        let quick = id_of(QUICK_ACCESS_HOTKEY);
        let auto = id_of(DEFAULT_HOTKEY);
        let cases = [
            ((true, true), (Some(quick), Some(auto))),
            ((false, true), (None, Some(auto))),
            ((true, false), (Some(quick), None)),
            ((false, false), (None, None)),
            ((true, true), (Some(quick), Some(auto))),
        ];
        for ((quick_access, auto_type), (want_quick, want_auto)) in cases {
            cx.update(|cx| reconcile(&settings(quick_access, auto_type), cx));
            let registered = cx.update(|cx| registered(cx));
            assert_eq!(
                registered,
                Registered {
                    quick_access: want_quick,
                    auto_type: want_auto
                },
                "quick access {quick_access}, Auto-Type {auto_type}"
            );
            let running = cx.update(|cx| cx.global::<HotkeyLoop>().task.is_some());
            assert_eq!(
                running,
                quick_access || auto_type,
                "the loop runs exactly while a shortcut is registered"
            );
        }
        assert_eq!(fake::calls().managers, 1);

        // A new Auto-Type combo replaces the old registration.
        let mut changed = settings(true, true);
        changed.auto_type_hotkey = "alt+shift+KeyV".to_string();
        cx.update(|cx| reconcile(&changed, cx));
        assert_eq!(
            cx.update(|cx| registered(cx)).auto_type,
            Some(id_of("alt+shift+KeyV"))
        );
        assert_eq!(fake::calls().unregistered.last(), Some(&auto));
    }

    #[gpui::test]
    fn a_shortcut_that_cannot_be_registered_reports_why_and_leaves_the_other(
        cx: &mut TestAppContext,
    ) {
        fake::reset();
        fake::refuse(DEFAULT_HOTKEY);
        cx.update(|cx| reconcile(&settings(true, true), cx));
        let registered = cx.update(|cx| registered(cx));
        assert_eq!(registered.quick_access, Some(id_of(QUICK_ACCESS_HOTKEY)));
        assert_eq!(registered.auto_type, None);
        assert!(cx.update(|cx| error(Shortcut::AutoType, cx)).is_some());
        assert_eq!(cx.update(|cx| error(Shortcut::QuickAccess, cx)), None);

        // A combo that does not parse is reported too, and switching the
        // shortcut off clears the error.
        let mut broken = settings(true, true);
        broken.auto_type_hotkey = "not a real hotkey".to_string();
        cx.update(|cx| reconcile(&broken, cx));
        assert!(cx.update(|cx| error(Shortcut::AutoType, cx)).is_some());
        cx.update(|cx| reconcile(&settings(true, false), cx));
        assert_eq!(cx.update(|cx| error(Shortcut::AutoType, cx)), None);
    }

    /// A combo that one shortcut lets go of in a save goes to the other in
    /// the same save. Auto-Type held ⌃⌥⌘P, so quick access was refused; one
    /// save that moves Auto-Type to another combo registers both and leaves
    /// no error.
    #[gpui::test]
    fn a_combo_auto_type_lets_go_of_goes_to_quick_access_in_the_same_save(cx: &mut TestAppContext) {
        fake::reset();
        let quick = id_of(QUICK_ACCESS_HOTKEY);
        let mut clash = settings(false, true);
        clash.auto_type_hotkey = QUICK_ACCESS_HOTKEY.to_string();
        cx.update(|cx| reconcile(&clash, cx));
        assert_eq!(cx.update(|cx| registered(cx)).auto_type, Some(quick));

        clash.quick_access_hotkey_enabled = true;
        cx.update(|cx| reconcile(&clash, cx));
        assert_eq!(cx.update(|cx| registered(cx)).quick_access, None);
        assert!(
            cx.update(|cx| error(Shortcut::QuickAccess, cx)).is_some(),
            "the test needs quick access refused"
        );

        let mut moved = clash;
        moved.auto_type_hotkey = "alt+shift+KeyV".to_string();
        cx.update(|cx| reconcile(&moved, cx));
        assert_eq!(
            cx.update(|cx| registered(cx)),
            Registered {
                quick_access: Some(quick),
                auto_type: Some(id_of("alt+shift+KeyV")),
            },
            "one save registers both"
        );
        assert_eq!(cx.update(|cx| error(Shortcut::QuickAccess, cx)), None);
        assert_eq!(cx.update(|cx| error(Shortcut::AutoType, cx)), None);
    }

    /// A press of ⌃⌥⌘P counts as activity and opens the panel; the next
    /// closes it. Being registered, a release or another id counts for
    /// nothing.
    #[gpui::test]
    fn the_quick_access_hotkey_counts_as_activity(cx: &mut TestAppContext) {
        use crate::ui::windows::panel;

        let _app = panel::tests::open_app(cx, &[panel::tests::entry("Salesforce")]);
        cx.update(|cx| reconcile(&settings(true, true), cx));
        let quick = id_of(QUICK_ACCESS_HOTKEY);
        let ten_minutes = Duration::from_secs(600);

        activity::reset(SystemTime::now() - ten_minutes);
        poll(cx);
        fake::send(released(quick));
        fake::send(pressed(quick ^ 1));
        poll(cx);
        assert!(activity::idle_for(SystemTime::now()) >= ten_minutes);
        assert!(!cx.update(|cx| panel::is_open(cx)));

        fake::send(pressed(quick));
        poll(cx);
        let idle = activity::idle_for(SystemTime::now());
        assert!(
            idle < Duration::from_secs(1),
            "the quick access shortcut left the clock idle for {idle:?}"
        );
        assert!(
            cx.update(|cx| panel::is_open(cx)),
            "the press opened the panel"
        );

        fake::send(pressed(quick));
        poll(cx);
        assert!(
            !cx.update(|cx| panel::is_open(cx)),
            "the next press closed it"
        );
    }
}
