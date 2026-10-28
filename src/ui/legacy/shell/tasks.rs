//! Background tasks that follow the state: the TOTP tick, auto-lock, the
//! session lock, auto-sync and the Auto-Type hotkey, and wiping session
//! secrets.

use super::*;

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
    /// checker. Runs only while a vault is open AND `auto_lock_secs`
    /// is configured; cancelled (= dropped) otherwise. The task wakes
    /// every `AUTO_LOCK_TICK_SECS` and triggers a lock once
    /// `last_activity` has been silent for the configured threshold.
    /// Reads the threshold inside the loop so settings changes apply
    /// at the next tick without restarting the task.
    pub(super) fn sync_auto_lock_task(&mut self, cx: &mut Context<Self>) {
        // "Any vault in memory" - keeps the timer ticking when the active
        // slot is on the unlock screen but parked sessions are still
        // decrypted. Global auto-lock semantics: a single idle timeout
        // sweeps active + parked together via `AppState::lock_vault`.
        let any_unlocked = self.state.read(cx).has_any_unlocked();
        let auto_lock_enabled = self.settings.auto_lock_secs.is_some();
        let should_run = any_unlocked && auto_lock_enabled;

        match (should_run, self.auto_lock_task.is_some()) {
            (true, false) => {
                // Reset the activity baseline at vault-open so a stale
                // timestamp from earlier in the session doesn't trip an
                // immediate lock.
                self.last_activity = SystemTime::now();
                let window_handle = self.window_handle;
                self.auto_lock_task = Some(cx.spawn(async move |this, cx| {
                    loop {
                        cx.background_executor()
                            .timer(Duration::from_secs(AUTO_LOCK_TICK_SECS))
                            .await;
                        let decision = this.update(cx, |shell, _| {
                            // Re-read on each tick so toggling the
                            // setting takes effect within ~5 s.
                            let Some(threshold) = shell.settings.auto_lock_secs else {
                                return None; // settings disabled - exit.
                            };
                            // A backwards clock jump must not postpone a
                            // lock, so an unreadable elapsed time counts as
                            // "long enough".
                            let idle = SystemTime::now()
                                .duration_since(shell.last_activity)
                                .unwrap_or(Duration::MAX);
                            if idle >= Duration::from_secs(threshold) {
                                Some(true)
                            } else {
                                Some(false)
                            }
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
        let events = self._session_lock_monitor.events();
        let lock_latch = self._session_lock_monitor.lock_latch();
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

    /// Reconcile the global-hotkey listener with the current settings.
    /// Called from `new` (initial wire-up) and `update_settings`
    /// (toggle / combo change). Idempotent on the no-change path.
    ///
    /// Failure modes are user-actionable, so we cache the error in
    /// `auto_type_hotkey_error` for the Settings UI to display instead
    /// of silently leaving the feature off - that's exactly the
    /// "looks broken" state we want to avoid.
    pub fn sync_auto_type_listener(&mut self, cx: &mut Context<Self>) {
        // Validate the sequence regardless of `enabled` so the Settings
        // UI can show a parse error even when the user is editing a
        // disabled-but-being-set-up feature. Cheap: pure string work.
        self.auto_type_sequence_error =
            autotype::sequence::parse(&self.settings.auto_type_sequence).err();

        let want_listener = self.settings.auto_type_enabled;
        let current_combo = self
            .auto_type_listener
            .as_ref()
            .map(|l| l.id())
            .unwrap_or(0);
        let parsed = autotype::hotkey::parse_combo(&self.settings.auto_type_hotkey);
        let target_id = parsed.as_ref().map(|h| h.id()).unwrap_or(0);

        // No-op shortcut: feature on, combo unchanged, listener already
        // registered. Saves the OS round-trip on every settings save.
        if want_listener && self.auto_type_listener.is_some() && current_combo == target_id {
            self.auto_type_hotkey_error = None;
            return;
        }

        // Tear down any existing listener+poll task. Drop order matters
        // only insofar as the poll task should stop reading the channel
        // before the registration is removed; in practice both are safe
        // to drop concurrently because the channel is process-global.
        self.auto_type_listener = None;
        self.auto_type_poll_task = None;

        if !want_listener {
            self.auto_type_hotkey_error = None;
            return;
        }

        match autotype::HotkeyListener::register(&self.settings.auto_type_hotkey) {
            Ok(listener) => {
                let expected_id = listener.id();
                let window_handle = self.window_handle;
                self.auto_type_listener = Some(listener);
                self.auto_type_hotkey_error = None;
                self.auto_type_poll_task = Some(cx.spawn(async move |this, cx| {
                    loop {
                        cx.background_executor()
                            .timer(AUTO_TYPE_POLL_INTERVAL)
                            .await;
                        // Liveness check + non-blocking drain on the
                        // entity context (cheap; no window borrow needed).
                        let fired = this.update(cx, |_shell, _cx| {
                            autotype::hotkey::poll_pressed(expected_id)
                        });
                        let fired = match fired {
                            Ok(f) => f,
                            Err(_) => break, // shell dropped
                        };
                        if !fired {
                            continue;
                        }
                        // Hotkey fired. We dispatch into the window
                        // directly - `App::dispatch_action` would route
                        // through `active_window()`, which is `None`
                        // while another app holds OS focus, and would
                        // then fall back to `dispatch_global_action`
                        // (skipping our element-tree handler entirely).
                        // Going through the window handle gives us a
                        // real `&mut Window` regardless of focus.
                        let entered = window_handle.update(cx, |_root, window, app| {
                            this.update(app, |shell, cx| {
                                let cancellation = shell.state.read(cx).auto_type_cancellation();
                                shell.perform_auto_type(None, cancellation, window, cx);
                            })
                            .ok();
                        });
                        if entered.is_err() {
                            break; // window closed
                        }
                    }
                }));
            }
            Err(e) => {
                self.auto_type_hotkey_error = Some(e.to_string());
            }
        }
    }

    /// Read-only accessors for the Settings UI. Cached errors are
    /// recomputed on every `update_settings` so the UI can render the
    /// up-to-date status without re-parsing on every frame.
    pub fn auto_type_hotkey_error(&self) -> Option<&str> {
        self.auto_type_hotkey_error.as_deref()
    }

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

    /// Drop reveal state, cancel timers, and clear the clipboard write owned
    /// by this shell without retaining or comparing the copied secret.
    pub(super) fn wipe_session_secrets(&mut self, cx: &mut Context<Self>) {
        self.revealed_entry_id = None;
        self.clear_owned_clipboard(cx);
        self.clipboard_clear_task = None;
        self.clipboard_clear_deadline = None;
        self.clipboard_pill_tick = None;
        // Drop any pending launch payloads - each handle's Drop unlinks
        // its file. Cancel the cleanup timers since we just did the
        // cleanup ourselves. Then purge the launch tempdir for good
        // measure: covers anything raced in between by another instance
        // or anything our own Drop missed.
        self.pending_launches.clear();
        self.launch_cleanup_tasks.clear();
        crate::launch::sweeper::purge_all();
    }

    pub(super) fn clear_owned_clipboard(&mut self, _cx: &mut Context<Self>) {
        let Some(write) = self.pending_clipboard_write.take() else {
            return;
        };
        match write {
            #[cfg(target_os = "macos")]
            PendingClipboardWrite::Native(change_count) => {
                let _ = clear_if_unchanged(change_count);
            }
            #[cfg(not(target_os = "macos"))]
            PendingClipboardWrite::Portable(_) => {
                // GPUI has no portable clipboard generation. Clearing is
                // unconditional here, but only while our opaque write token
                // is still current; no secret is retained for comparison.
                _cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
            }
        }
    }
}
