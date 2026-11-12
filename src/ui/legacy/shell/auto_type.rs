//! The Auto-Type actions and the typing run behind them.

use super::*;

impl AppShell {
    /// Global-hotkey route. Reads the foreground window the user just
    /// left, finds the best-matching entry by URL hostname, and types
    /// the configured sequence into it. All preconditions surface as
    /// a toast notification - silent failure here would have the
    /// feature looking broken (the user pressed the hotkey, nothing
    /// happened, no clue why).
    pub(super) fn on_action_perform_auto_type(
        &mut self,
        _: &PerformAutoType,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cancellation = self.state.read(cx).auto_type_cancellation();
        self.perform_auto_type(None, cancellation, window, cx);
    }

    /// In-app ⌘⇧T route: types the *currently-selected* entry after a
    /// short countdown. The countdown is what lets the user press the
    /// shortcut from inside FerrisPass and still aim the keystrokes
    /// at a different window - the global hotkey is the better
    /// ergonomic, but this is the discoverable in-app entry point.
    pub(super) fn on_action_perform_auto_type_for_selected(
        &mut self,
        _: &PerformAutoTypeForSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entry_id) = self.selected_entry_id(cx) else {
            window.push_notification("Select an entry first.", cx);
            return;
        };
        // Quick preconditions check before we surface a countdown the
        // user would be waiting for in vain. Permission + vault-open
        // are the load-bearing ones; foreground is deferred to the
        // moment the typing happens (it'll have changed by then).
        if !autotype::permissions::is_trusted() {
            window.push_notification(
                "Auto-Type needs Accessibility access. Open Settings → Auto-Type to grant.",
                cx,
            );
            return;
        }
        if !matches!(
            self.state.read(cx).vault_status(),
            crate::app::VaultStatus::Open { .. }
        ) {
            window.push_notification("Unlock the vault first.", cx);
            return;
        }
        window.push_notification(
            format!(
                "Auto-Type starting in {AUTO_TYPE_COUNTDOWN_SECS} s - switch to the target window."
            ),
            cx,
        );
        let cancellation = self.state.read(cx).auto_type_cancellation();
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(AUTO_TYPE_COUNTDOWN_SECS))
                .await;
            if cancellation.is_cancelled() {
                return;
            }
            let _ = this.update_in(cx, |shell, window, cx| {
                shell.perform_auto_type(Some(entry_id.clone()), cancellation.clone(), window, cx);
            });
        })
        .detach();
    }

    /// The way in for the global Auto-Type shortcut, from the app-level loop
    /// in `services::hotkeys`. The run needs this window (its notifications
    /// land there), so it goes through the window handle rather than
    /// `App::dispatch_action`, which routes through `active_window()`: that
    /// is `None` while another app holds the focus, which is exactly when
    /// the shortcut is pressed. A window that is gone ends it quietly.
    pub(crate) fn dispatch_global_auto_type(shell: Entity<AppShell>, cx: &mut gpui::App) {
        let window_handle = shell.read(cx).window_handle;
        let _ = window_handle.update(cx, |_root, window, app| {
            shell.update(app, |shell, cx| {
                let cancellation = shell.state.read(cx).auto_type_cancellation();
                shell.perform_auto_type(None, cancellation, window, cx);
            });
        });
    }

    /// Shared implementation between the global-hotkey and in-app
    /// routes. `force_entry_id` = `Some(id)` skips the URL-matching
    /// step and types the given entry's credentials regardless of
    /// the foreground app.
    ///
    /// Two-phase flow against `autotype`'s orchestrator: `prepare`
    /// validates everything and produces an owned `TypePlan` on the
    /// foreground (cheap, must hold the AppState borrow to read the
    /// cleartext password); `execute` runs the blocking typer on a
    /// background task. The completion result is mapped back through
    /// the same `Outcome` enum the orchestrator uses everywhere else, so
    /// `TypingFailed` actually reaches the user instead of being silently
    /// swallowed by a fire-and-forget spawn.
    pub(super) fn perform_auto_type(
        &mut self,
        force_entry_id: Option<String>,
        cancellation: autotype::CancellationToken,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        #[cfg(test)]
        tests::note_run(window.window_handle());
        // Stale countdowns and queued hotkey work are deliberately silent.
        if cancellation.is_cancelled() {
            return;
        }
        let Some(foreground) = autotype::window::foreground() else {
            window.push_notification(
                "Could not read the foreground window. Check Accessibility permission.",
                cx,
            );
            return;
        };

        // Build the orchestrator input under a single AppState read.
        // The closure captures `&VaultDocument` for the password
        // resolver - that borrow is alive only while `prepare` runs,
        // which is synchronous.
        let template = self.settings.auto_type_sequence.clone();
        let prepared: Result<autotype::TypePlan, autotype::Outcome> = {
            let state = self.state.read(cx);
            match state.vault_status() {
                crate::app::VaultStatus::Open { document, .. } => {
                    let snapshot = document.snapshot_rc();
                    autotype::prepare(autotype::PerformInput {
                        cancellation,
                        foreground: foreground.clone(),
                        snapshot: &snapshot,
                        resolve_password: &|id: &str| document.password_for_entry(id),
                        sequence_template: &template,
                        force_entry_id,
                    })
                }
                _ => Err(autotype::Outcome::VaultLocked),
            }
        };

        let plan = match prepared {
            Ok(plan) => plan,
            Err(outcome) => {
                self.notify_auto_type_outcome(outcome, &foreground, window, cx);
                return;
            }
        };

        // Mark recently-used now - the typing succeeds asynchronously,
        // but from the user's perspective they've authenticated with
        // this entry the moment they pressed the hotkey. (Background
        // task failure still surfaces via the notification.)
        self.state
            .update(cx, |state, _| state.mark_entry_used(&plan.entry_id));

        // Spawn the (blocking) typer on a background task. The plan
        // - and the cleartext password it carries inside its TypeOps
        // - is moved into the task and dropped when the task ends.
        let completion_cancellation = plan.cancellation.clone();
        let task = cx.background_spawn(async move { autotype::execute(plan) });
        let foreground_for_callback = foreground;
        cx.spawn_in(window, async move |this, cx| {
            let outcome = task.await;
            let _ = this.update_in(cx, |shell, window, cx| {
                if completion_cancellation.is_cancelled() {
                    return;
                }
                shell.notify_auto_type_outcome(outcome, &foreground_for_callback, window, cx);
            });
        })
        .detach();
    }

    /// Translate an `autotype::Outcome` into a single user-facing
    /// toast. Centralised so the success and every failure path land
    /// through the same wording table, which makes copy-edits a
    /// one-place change and stops the UI from claiming success on a
    /// typer error.
    fn notify_auto_type_outcome(
        &mut self,
        outcome: autotype::Outcome,
        foreground: &autotype::ForegroundInfo,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use autotype::Outcome;
        match outcome {
            Outcome::Cancelled => {}
            Outcome::Typed { entry_title } => {
                window.push_notification(format!("Auto-typed {entry_title}."), cx);
            }
            Outcome::NotTrusted => {
                window.push_notification(
                    "Auto-Type needs Accessibility access. Open Settings → Auto-Type to grant.",
                    cx,
                );
            }
            Outcome::NoForeground => {
                window.push_notification(
                    "Could not read the foreground window. Check Accessibility permission.",
                    cx,
                );
            }
            Outcome::SelfForeground => {
                window.push_notification(
                    "Switch to your target window before triggering Auto-Type.",
                    cx,
                );
            }
            Outcome::VaultLocked => {
                window.push_notification("Unlock the vault first.", cx);
            }
            Outcome::NoMatch { window_title } => {
                // Prefer the window title; fall back to the app name so
                // the user at least sees what FerrisPass detected.
                // Browsers/apps that don't expose a title via the
                // Accessibility API would otherwise produce "No matching
                // entry for """, which looks like a bug.
                let title = if window_title.is_empty() {
                    foreground.window_title.clone()
                } else {
                    window_title
                };
                // Name the remedy, not just the failure: matching is
                // deliberately strict (exact app/hostname identity), and
                // the KeePass-standard way to widen it is a per-entry
                // Auto-Type window association.
                let hint = "Add an Auto-Type window association to the entry to match this window.";
                let message = if !title.is_empty() {
                    format!("No matching entry for \"{title}\". {hint}")
                } else if !foreground.app_name.is_empty() {
                    format!("No matching entry for {}. {hint}", foreground.app_name)
                } else {
                    format!("No matching entry for the foreground window. {hint}")
                };
                window.push_notification(message, cx);
            }
            Outcome::NoPassword => {
                window.push_notification("The matched entry has no password set.", cx);
            }
            Outcome::AutoTypeDisabled { entry_title } => {
                window.push_notification(
                    format!("Auto-Type is disabled for \"{entry_title}\" (KeePass entry setting)."),
                    cx,
                );
            }
            Outcome::BadSequence(error) => {
                // Cache for the Settings tab and surface inline so the
                // user knows which knob to turn.
                window.push_notification(format!("Auto-Type sequence invalid: {error}"), cx);
                self.auto_type_sequence_error = Some(error);
            }
            Outcome::TypingFailed(message) => {
                window.push_notification(format!("Auto-Type failed: {message}"), cx);
            }
            Outcome::FocusChanged { window_title } => {
                let message = if window_title.is_empty() {
                    "Auto-Type cancelled - the target window lost focus.".to_string()
                } else {
                    format!("Auto-Type cancelled - focus moved to \"{window_title}\".")
                };
                window.push_notification(message, cx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use global_hotkey::{GlobalHotKeyEvent, HotKeyState};
    use gpui::TestAppContext;

    use super::*;
    use crate::app::services::hotkeys;
    use crate::autotype::hotkey::fake;
    use crate::clipboard::{AccessBehavior, fake::FakePasteboard};

    thread_local! {
        static RUNS: RefCell<Vec<AnyWindowHandle>> = const { RefCell::new(Vec::new()) };
    }

    /// Called at the top of `perform_auto_type` in test builds, with the
    /// window it runs in.
    pub(super) fn note_run(window: AnyWindowHandle) {
        RUNS.with(|runs| runs.borrow_mut().push(window));
    }

    fn runs() -> Vec<AnyWindowHandle> {
        RUNS.with(|runs| runs.borrow().clone())
    }

    fn pressed(id: u32) -> GlobalHotKeyEvent {
        GlobalHotKeyEvent {
            id,
            state: HotKeyState::Pressed,
        }
    }

    /// The Auto-Type shortcut used to have a loop of its own on the main
    /// window. It now shares the app-level loop with the quick access
    /// shortcut, and a press still runs Auto-Type once, in the main window.
    /// A press of the quick access shortcut does not.
    #[gpui::test]
    fn the_auto_type_hotkey_still_reaches_the_shell(cx: &mut TestAppContext) {
        fake::reset();
        RUNS.with(|runs| runs.borrow_mut().clear());
        cx.update(|cx| {
            gpui_component::init(cx);
            crate::app::actions::init(cx);
            ClipboardService::install_for_test(
                cx,
                Box::new(FakePasteboard::new(AccessBehavior::Default)),
            );
        });
        let state = cx.new(|_| AppState::default());
        cx.update(|cx| crate::app::open_main_window_for_test(cx, state));
        cx.run_until_parked();
        let shell = cx
            .update(|cx| crate::app::with_shared_shell(cx, |_, cx| cx.entity()))
            .expect("the main window's shell");
        let main_window = shell.read_with(cx, |shell, _| shell.window_handle);
        shell.update(cx, |shell, cx| {
            shell.settings.quick_access_hotkey_enabled = true;
            shell.settings.auto_type_enabled = true;
            shell.sync_auto_type_listener(cx);
        });
        let registered = cx.update(|cx| hotkeys::registered(cx));
        let auto_type = registered.auto_type.expect("Auto-Type registered");
        let quick_access = registered.quick_access.expect("quick access registered");

        // Two presses: the first opens the panel, the second closes it.
        for _ in 0..2 {
            fake::send(pressed(quick_access));
            cx.executor().advance_clock(Duration::from_millis(100));
            cx.run_until_parked();
        }
        assert!(runs().is_empty(), "the quick access shortcut ran Auto-Type");
        assert!(!cx.update(|cx| crate::ui::windows::panel::is_open(cx)));

        fake::send(pressed(auto_type));
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        assert_eq!(
            runs(),
            vec![main_window],
            "one press, one Auto-Type run, in the main window"
        );
    }

    /// The quick access panel takes the keyboard while the app behind it
    /// stays in front. Auto-Type reads that app, so a run would find its
    /// entry and type the user name and the password into the panel's
    /// search field. While the panel is open the shortcut runs nothing; once
    /// it is closed, a press runs Auto-Type as before.
    #[gpui::test]
    fn the_auto_type_hotkey_types_nothing_while_the_panel_is_open(cx: &mut TestAppContext) {
        use crate::ui::windows::panel;

        // No address and no window pattern: no app in front matches it.
        let app = panel::tests::open_app(cx, &[panel::tests::entry("Zq panel fixture")]);
        RUNS.with(|runs| runs.borrow_mut().clear());
        let shell = cx
            .update(|cx| crate::app::with_shared_shell(cx, |_, cx| cx.entity()))
            .expect("the main window's shell");
        let main_window = shell.read_with(cx, |shell, _| shell.window_handle);
        shell.update(cx, |shell, cx| {
            shell.settings.quick_access_hotkey_enabled = true;
            shell.settings.auto_type_enabled = true;
            shell.sync_auto_type_listener(cx);
        });
        let auto_type = cx
            .update(|cx| hotkeys::registered(cx))
            .auto_type
            .expect("Auto-Type registered");
        let press = |cx: &mut TestAppContext| {
            fake::send(pressed(auto_type));
            cx.executor().advance_clock(Duration::from_millis(100));
            cx.run_until_parked();
        };

        let panel_window = app.open_panel(cx);
        panel_window
            .update(cx, |_, window, _| window.activate_window())
            .expect("the panel");
        cx.run_until_parked();
        assert!(cx.update(|cx| panel::is_open(cx)));
        press(cx);
        assert!(
            runs().is_empty(),
            "Auto-Type ran while the panel had the keyboard"
        );
        assert!(cx.update(|cx| panel::is_open(cx)), "the panel stays open");

        cx.update(panel::close);
        cx.run_until_parked();
        press(cx);
        assert_eq!(
            runs(),
            vec![main_window],
            "with the panel closed, a press runs Auto-Type in the main window"
        );
    }
}
