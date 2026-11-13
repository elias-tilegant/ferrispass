//! Background tasks that follow the state: the TOTP tick, auto-lock, the
//! session lock, auto-sync and the Auto-Type hotkey, and wiping session
//! secrets.

use super::*;
use crate::app::services::activity;
use crate::platform::system_idle;

impl AppShell {
    /// Start the per-second TOTP refresh loop only when the currently-selected
    /// entry has an OTP field; cancel it otherwise. Called from the state
    /// observer so it follows both open/lock transitions and selection
    /// changes: moving away from a TOTP entry stops the tick, moving onto one
    /// starts it. Saves the wasted 1 Hz re-renders for the (common) case where the
    /// user isn't looking at a TOTP entry.
    pub(super) fn sync_totp_tick(&mut self, cx: &mut Context<Self>) {
        let needs_tick = self
            .state
            .read(cx)
            .vault_browser()
            .and_then(|b| b.selected_entry)
            .map(|e| e.has_otp)
            .unwrap_or(false);

        match (needs_tick, self.totp_tick.is_some()) {
            (true, false) => {
                let state = self.state.downgrade();
                self.totp_tick = Some(cx.spawn(async move |_, cx| {
                    loop {
                        cx.background_executor()
                            .timer(std::time::Duration::from_secs(1))
                            .await;
                        // Notify the AppState entity so anything observing it
                        // (notably the detail panel via the AppShell observer
                        // above) re-renders and recomputes the live TOTP code.
                        // If the entity is gone we exit cleanly.
                        if state.update(cx, |_, cx| cx.notify()).is_err() {
                            break;
                        }
                    }
                }));
            }
            (false, true) => {
                // Drop the task - GPUI cancels it.
                self.totp_tick = None;
            }
            _ => {}
        }
    }

    /// Mirror of `sync_totp_tick` for the idle-timeout auto-lock
    /// checker. Runs only while a vault is open AND an auto-lock is
    /// configured, `auto_lock_secs`, `mac_idle_lock_secs` or both;
    /// cancelled (= dropped) otherwise. The task wakes every
    /// `AUTO_LOCK_TICK_SECS` and triggers a lock once
    /// `activity::lock_is_due_with` says so: by the app-wide idle clock
    /// (`services::activity`) against `auto_lock_secs`, or, with
    /// `mac_idle_lock_secs` set, by the Mac's idle time against that,
    /// falling back to the app-wide clock when macOS cannot report it. It
    /// only reads the clocks; input moves them. Reads both settings inside
    /// the loop so settings changes apply at the next tick without
    /// restarting the task.
    pub(super) fn sync_auto_lock_task(&mut self, cx: &mut Context<Self>) {
        // "Any vault in memory" - keeps the timer ticking when the active
        // slot is on the unlock screen but parked sessions are still
        // decrypted. Global auto-lock semantics: a single idle timeout
        // sweeps active + parked together via `AppState::lock_vault`.
        let any_unlocked = self.state.read(cx).has_any_unlocked();
        let auto_lock_enabled =
            self.settings.auto_lock_secs.is_some() || self.settings.mac_idle_lock_secs.is_some();
        let should_run = any_unlocked && auto_lock_enabled;

        match (should_run, self.auto_lock_task.is_some()) {
            (true, false) => {
                // Reset the activity baseline at vault-open so a stale
                // timestamp from earlier in the session doesn't trip an
                // immediate lock.
                activity::reset(SystemTime::now());
                let window_handle = self.window_handle;
                self.auto_lock_task = Some(cx.spawn(async move |this, cx| {
                    loop {
                        cx.background_executor()
                            .timer(Duration::from_secs(AUTO_LOCK_TICK_SECS))
                            .await;
                        let decision = this.update(cx, |shell, cx| {
                            // Re-read on each tick so changing either
                            // setting takes effect within ~5 s.
                            let app_threshold =
                                shell.settings.auto_lock_secs.map(Duration::from_secs);
                            let mac_threshold =
                                shell.settings.mac_idle_lock_secs.map(Duration::from_secs);
                            if app_threshold.is_none() && mac_threshold.is_none() {
                                return None; // settings disabled - exit.
                            }
                            // A backwards clock jump must not postpone a
                            // lock; `idle_for` reads it as idle long enough.
                            let app_idle = activity::idle_for(SystemTime::now());
                            // The Mac's idle time is only asked for while
                            // it counts; `lock_is_due_with` ignores it
                            // without a Mac threshold.
                            let mac_idle = match mac_threshold {
                                Some(_) => system_idle::idle_for(cx),
                                None => Err(system_idle::IdleError::Unsupported),
                            };
                            Some(activity::lock_is_due_with(
                                app_idle,
                                mac_idle,
                                app_threshold,
                                mac_threshold,
                            ))
                        });
                        match decision {
                            Ok(Some(false)) => continue,
                            Ok(Some(true)) => {
                                let _ = window_handle.update(cx, |_root, window, app| {
                                    this.update(app, |shell, cx| {
                                        shell.lock_vault_automatically(window, cx)
                                    })
                                    .ok();
                                });
                                break;
                            }
                            Ok(None) | Err(_) => break,
                        }
                    }
                }));
            }
            (false, true) => {
                self.auto_lock_task = None;
            }
            _ => {}
        }
    }

    #[cfg(target_os = "macos")]
    pub(super) fn start_session_lock_task(&mut self, cx: &mut Context<Self>) {
        let (events, lock_latch) = session_lock_events(&self._session_lock_monitor, cx);
        let window_handle = self.window_handle;
        self._session_lock_task = Some(cx.spawn(async move |this, cx| {
            while events.recv().await.is_ok() {
                // Sleep/lock transitions commonly emit several events in a
                // burst. Drain the queue, then consult the latch - it is
                // the authoritative Lock record and survives even events
                // dropped by a full channel.
                while events.try_recv().is_ok() {}
                let strongest = if lock_latch.swap(false, std::sync::atomic::Ordering::AcqRel) {
                    crate::session_lock::SessionLockEvent::Lock
                } else {
                    crate::session_lock::SessionLockEvent::PostWake
                };

                let updated = window_handle.update(cx, |_root, window, app| {
                    this.update(app, |shell, cx| {
                        // Post-wake notifications (DidWake, ScreensDidWake,
                        // SessionDidBecomeActive, …) are trailing fail-safes
                        // delivered seconds after the user is back. If they
                        // already unlocked the vault again, such an event
                        // must not wipe that unlock mid-typing. Genuine Lock
                        // events never get this grace.
                        const POST_UNLOCK_GRACE: std::time::Duration =
                            std::time::Duration::from_secs(5);
                        if strongest == crate::session_lock::SessionLockEvent::PostWake
                            && shell.state.read(cx).unlocked_within(POST_UNLOCK_GRACE)
                        {
                            return;
                        }
                        shell.lock_vault_automatically(window, cx);
                    })
                    .is_ok()
                });
                if !matches!(updated, Ok(true)) {
                    break;
                }
            }
        }));
    }

    /// Sibling of `sync_auto_lock_task` for background auto-sync + token
    /// keep-alive. Runs only while a synced vault is in memory AND
    /// `auto_sync_secs` is set; cancelled (= dropped) otherwise. The task
    /// sleeps for the configured interval, then runs a pull/keep-alive
    /// pass over every synced vault. The interval is re-read each loop so
    /// a settings change takes effect on the next cycle, and a `None`
    /// (= "Never") read exits the loop so toggling the feature off stops
    /// the timer promptly.
    pub(super) fn sync_auto_sync_task(&mut self, cx: &mut Context<Self>) {
        let has_binding = self.state.read(cx).has_any_sync_binding();
        let enabled = self.settings.auto_sync_secs_clamped().is_some();
        let should_run = has_binding && enabled;

        match (should_run, self.auto_sync_task.is_some()) {
            (true, false) => {
                self.auto_sync_task = Some(cx.spawn(async move |this, cx| {
                    loop {
                        // Read the interval before sleeping so a freshly
                        // saved setting governs the very next wait.
                        let interval = match this
                            .update(cx, |shell, _| shell.settings.auto_sync_secs_clamped())
                        {
                            Ok(Some(secs)) => Duration::from_secs(secs),
                            // Setting turned off, or the entity went away -
                            // stop ticking.
                            _ => break,
                        };
                        cx.background_executor().timer(interval).await;
                        let kept_going = this.update(cx, |shell, cx| {
                            // Bail if the feature was switched off while we
                            // slept; the next observer pass will also clear
                            // the task handle, but exiting here is prompt.
                            if shell.settings.auto_sync_secs_clamped().is_none() {
                                return false;
                            }
                            shell.state.update(cx, |state, cx| state.auto_sync_all(cx));
                            true
                        });
                        match kept_going {
                            Ok(true) => continue,
                            Ok(false) | Err(_) => break,
                        }
                    }
                }));
            }
            (false, true) => {
                self.auto_sync_task = None;
            }
            _ => {}
        }
    }

    /// Reconcile the global shortcuts with the current settings. Called from
    /// `new` (initial wire-up) and `update_settings` (toggle / combo change).
    /// Idempotent on the no-change path.
    ///
    /// The registrations and the one loop that reads them live in
    /// `services::hotkeys`, at app level, so the quick access shortcut works
    /// without this window and Auto-Type keeps working after Cmd+W hides it.
    /// Their errors are cached there for the Settings UI to display instead
    /// of silently leaving the feature off - that's exactly the "looks
    /// broken" state we want to avoid.
    pub fn sync_auto_type_listener(&mut self, cx: &mut Context<Self>) {
        // Validate the sequence regardless of `enabled` so the Settings
        // UI can show a parse error even when the user is editing a
        // disabled-but-being-set-up feature. Cheap: pure string work.
        self.auto_type_sequence_error =
            autotype::sequence::parse(&self.settings.auto_type_sequence).err();
        crate::app::services::hotkeys::reconcile(&self.settings, cx);
    }

    /// Read-only accessors for the Settings UI. Cached errors are
    /// recomputed on every `update_settings` so the UI can render the
    /// up-to-date status without re-parsing on every frame.
    pub fn auto_type_sequence_error(&self) -> Option<&autotype::ParseError> {
        self.auto_type_sequence_error.as_ref()
    }

    /// `true` if the OS currently trusts FerrisPass to use the
    /// Accessibility APIs. Probed live (not cached) because the user
    /// can grant or revoke it at any time via System Settings.
    pub fn auto_type_is_trusted(&self) -> bool {
        autotype::permissions::is_trusted()
    }

    /// Trigger the system prompt that opens the Privacy → Accessibility pane.
    ///
    /// macOS only re-reads the trust bit for new processes, so the Settings
    /// label keeps saying "Not granted" however the user answers. Without the
    /// note that follows, the button looks broken.
    pub fn auto_type_request_trust(&self, window: &mut Window, cx: &mut gpui::App) {
        let _ = autotype::permissions::request_trust();
        window.push_notification(
            "Approve FerrisPass in System Settings, then restart the app for Auto-Type to see it.",
            cx,
        );
    }

    /// Drop reveal state, clear our clipboard copy if it is still the newest
    /// thing on the clipboard, and delete pending launch payloads. Nothing
    /// here retains or compares the copied secret.
    pub(super) fn wipe_session_secrets(&mut self, cx: &mut Context<Self>) {
        self.revealed_entry_id = None;
        crate::app::services::clipboard::clear_owned(cx);
        // Drop any pending launch payloads - each handle's Drop unlinks
        // its file. Cancel the cleanup timers since we just did the
        // cleanup ourselves. Then purge the launch tempdir for good
        // measure: covers anything raced in between by another instance
        // or anything our own Drop missed.
        self.pending_launches.clear();
        self.launch_cleanup_tasks.clear();
        crate::launch::sweeper::purge_all();
    }
}

/// The session lock monitor's events and its latch, where the session lock
/// task reads them. A test that installed a `fake_session_lock` on the
/// `App` gets that one's instead, so it can send a lock without locking
/// the Mac.
#[cfg(target_os = "macos")]
fn session_lock_events(
    monitor: &crate::session_lock::SessionLockMonitor,
    #[cfg_attr(not(test), allow(unused_variables))] cx: &gpui::App,
) -> (
    async_channel::Receiver<crate::session_lock::SessionLockEvent>,
    std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    #[cfg(test)]
    if let Some(fake) = cx.try_global::<fake_session_lock::Installed>() {
        return (fake.events.clone(), fake.lock_latch.clone());
    }
    (monitor.events(), monitor.lock_latch())
}

/// Session lock events for tests: what a test sends here reaches the shell's
/// session lock task the way the monitor's events do.
#[cfg(all(test, target_os = "macos"))]
pub(super) mod fake_session_lock {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use async_channel::{Receiver, Sender};

    use crate::session_lock::SessionLockEvent;

    pub(super) struct Installed {
        pub(super) events: Receiver<SessionLockEvent>,
        pub(super) lock_latch: Arc<AtomicBool>,
    }

    impl gpui::Global for Installed {}

    pub(in crate::ui::legacy::shell) struct FakeSessionLock {
        sender: Sender<SessionLockEvent>,
        lock_latch: Arc<AtomicBool>,
    }

    impl FakeSessionLock {
        /// From here on, a shell built on `cx` takes its session lock events
        /// from the returned fake instead of from macOS.
        pub(in crate::ui::legacy::shell) fn install(cx: &mut gpui::App) -> Self {
            let (sender, events) = async_channel::bounded(8);
            let lock_latch = Arc::new(AtomicBool::new(false));
            cx.set_global(Installed {
                events,
                lock_latch: lock_latch.clone(),
            });
            Self { sender, lock_latch }
        }

        /// What the monitor does when the screen locks, the screen saver
        /// starts or the Mac goes to sleep: set the latch, then wake the task.
        pub(in crate::ui::legacy::shell) fn lock(&self) {
            self.lock_latch.store(true, Ordering::Release);
            let _ = self.sender.try_send(SessionLockEvent::Lock);
        }
    }
}
