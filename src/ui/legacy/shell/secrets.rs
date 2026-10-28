//! Copying with a timed clipboard clear, launching an entry, revealing a
//! password, and the settings.

use super::*;

impl AppShell {
    pub fn copy_selected_value(
        &mut self,
        kind: CopyValueKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(value) = self.state.read(cx).copy_selected_value(kind) {
            // Password and username copies count as "actually used"
            // for the Recently-Used filter. URL copies don't - the
            // user might just be sharing a link, not authenticating.
            if matches!(kind, CopyValueKind::Password | CopyValueKind::Username) {
                self.state.update(cx, |state, _| state.mark_selected_used());
            }
            self.copy_with_auto_clear(value, copy_value_label(kind), window, cx);
        } else {
            window.push_notification(format!("No {} to copy.", copy_value_label(kind)), cx);
        }
    }

    /// Click handler for the "Additional fields" rows in the detail
    /// panel. Reads the cleartext value off the open vault, writes it
    /// to the clipboard, and schedules the standard auto-clear timer.
    /// `key` is matched against the entry's `custom_fields[].key`
    /// verbatim (case-sensitive).
    pub fn copy_custom_field(
        &mut self,
        entry_id: &str,
        key: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let value = self.state.read(cx).custom_field_value(entry_id, key);
        let Some(value) = value.filter(|v| !v.is_empty()) else {
            window.push_notification(format!("No value for {key}."), cx);
            return;
        };
        // Mark the entry as recently-used too - copying a custom
        // field counts as authenticating with the entry, same rule
        // as for password / username copies.
        self.state.update(cx, |state, _| state.mark_selected_used());
        self.copy_with_auto_clear(value, key, window, cx);
    }

    /// Launch the currently-selected entry in its native external app
    /// (SAP GUI today, more later). Pulls the entry snapshot + cleartext
    /// password out of state, hands them to the matching `Launcher`,
    /// parks the returned handle in `pending_launches`, and schedules a
    /// cleanup task that drops the head of the queue after the
    /// user-configured TTL - that drop unlinks the temp payload file.
    ///
    /// All failure paths surface a toast and leave no temp file on
    /// disk. The launcher itself is responsible for never leaving
    /// half-written payloads on a partial failure.
    pub fn launch_selected_entry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self
            .state
            .read(cx)
            .vault_browser()
            .and_then(|b| b.selected_entry)
        else {
            window.push_notification("Select an entry first.", cx);
            return;
        };
        let Some(launcher) = launch::primary_launcher_for(&entry) else {
            window.push_notification("No launcher available for this entry.", cx);
            return;
        };
        // Read the password - copy_selected_value returns the cleartext
        // *without* writing to the clipboard when called via &self. The
        // launcher needs it to compose the .sapc body; we don't want it
        // accidentally landing on the clipboard for this flow.
        let password = self
            .state
            .read(cx)
            .copy_selected_value(CopyValueKind::Password);

        let ctx = LaunchContext {
            entry: &entry,
            password: password.as_deref().map(String::as_str),
            custom_fields: &entry.custom_fields,
        };
        match launch::launch(launcher, ctx) {
            Ok(handle) => {
                window.push_notification(format!("Starting {}…", launcher.label()), cx);
                let launch_id = self.next_launch_id;
                self.next_launch_id = self.next_launch_id.wrapping_add(1);
                self.pending_launches.insert(launch_id, handle);
                self.schedule_launch_cleanup(launch_id, cx);
                // Treat a launch the same as a copy for the recently-
                // used filter - the user just authenticated with this
                // entry, even if no clipboard touch happened.
                self.state.update(cx, |state, _| state.mark_selected_used());
            }
            Err(LaunchError::NoPassword) => {
                window.push_notification("No password set on this entry.", cx);
            }
            Err(LaunchError::MissingField(key)) => {
                window.push_notification(format!("Missing field: {key}"), cx);
            }
            Err(LaunchError::UnsupportedTarget(target)) => {
                window.push_notification(format!("{target} launch is not supported here."), cx);
            }
            Err(LaunchError::UnsupportedCharacter { field }) => {
                // Naming the field is safe; showing the value is not.
                window.push_notification(
                    format!(
                        "The {field} value contains & = or a line break, which this connection \
                         file cannot carry. Change it in the entry and try again."
                    ),
                    cx,
                );
            }
            Err(LaunchError::Io(e)) => {
                // Show only the kind, never the file body. The path is
                // ours, but even leaking it is unnecessary for the user.
                window.push_notification(format!("Launch failed: {}", e.kind()), cx);
            }
        }
    }

    /// Park a one-shot timer that drops the oldest pending launch
    /// after the configured TTL. Drop = `TempLaunchFile::drop` runs =
    /// payload unlinked. Each launch gets its own timer so multiple
    /// rapid launches don't share a deadline.
    /// Unlink one launch payload after its time-to-live.
    ///
    /// Each timer carries the id of the payload it belongs to. The previous
    /// version assumed the timers would fire in the order they were created
    /// and always collected `pending_launches[0]`, which is wrong the moment
    /// the user lowers the cleanup TTL between two launches: the second,
    /// shorter timer fires first and unlinks the *first* launch's file while
    /// SAP GUI is still reading it. It also dropped a `Task` from inside that
    /// task's own closure.
    fn schedule_launch_cleanup(&mut self, launch_id: u64, cx: &mut Context<Self>) {
        let ttl =
            std::time::Duration::from_secs(self.settings.launch_cleanup_secs_clamped() as u64);
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(ttl).await;
            let _ = this.update(cx, |this, _| {
                // Dropping the handle unlinks the file.
                this.pending_launches.remove(&launch_id);
                // Not the task itself: it is the one running. Its slot is
                // reclaimed by the next launch that reuses the map.
                this.launch_cleanup_tasks.remove(&launch_id);
            });
        });
        self.launch_cleanup_tasks.insert(launch_id, task);
    }

    /// Single source of truth for "put this on the clipboard, tell the
    /// user, schedule a clear". Used by the saved-fields copy path
    /// (`copy_selected_value`) and the live-TOTP copy in the detail
    /// panel. Replaces any in-flight clear so the latest copy always
    /// wins - older timers would otherwise wipe the new value early.
    /// When `clipboard_clear_secs` is `None` (user picked "Never"), no
    /// timer is scheduled; the clipboard still gets wiped at lock time.
    pub fn copy_with_auto_clear(
        &mut self,
        value: Zeroizing<String>,
        label: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        #[cfg(target_os = "macos")]
        let write = match write_secret_text(&value) {
            Ok(change_count) => PendingClipboardWrite::Native(change_count),
            Err(_) => {
                // Fail closed: a native write error must never downgrade to a
                // normal pasteboard item that clipboard history or Universal
                // Clipboard can retain. Clear any previous write we owned.
                self.clear_owned_clipboard(cx);
                self.clipboard_clear_task = None;
                self.clipboard_clear_deadline = None;
                self.clipboard_pill_tick = None;
                window.push_notification(format!("Could not copy {label}."), cx);
                return;
            }
        };
        #[cfg(not(target_os = "macos"))]
        let write = {
            cx.write_to_clipboard(ClipboardItem::new_string(value));
            self.next_portable_clipboard_write = self.next_portable_clipboard_write.wrapping_add(1);
            PendingClipboardWrite::Portable(self.next_portable_clipboard_write)
        };

        let toast = match self.settings.clipboard_clear_secs {
            Some(secs) => format!("{label} copied. Clipboard clears in {secs} s."),
            None => format!("{label} copied."),
        };
        window.push_notification(toast, cx);
        self.schedule_clipboard_clear(write, cx);
    }

    fn schedule_clipboard_clear(&mut self, write: PendingClipboardWrite, cx: &mut Context<Self>) {
        self.pending_clipboard_write = Some(write);
        // Drop any prior timer by replacing the slot. GPUI tasks cancel
        // on drop (same pattern as `totp_tick`).
        let Some(secs) = self.settings.clipboard_clear_secs else {
            // "Never": don't spawn a timer or pill. The opaque write token
            // remains so lock-time cleanup can still clear our clipboard.
            self.clipboard_clear_task = None;
            self.clipboard_clear_deadline = None;
            self.clipboard_pill_tick = None;
            return;
        };
        // Track the deadline so the countdown pill can render
        // `clears in N s…`. Re-rendering at 1 Hz comes from
        // `clipboard_pill_tick` below.
        self.clipboard_clear_deadline = Some(Instant::now() + Duration::from_secs(secs));
        self.clipboard_pill_tick = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let keep_ticking = this
                    .update(cx, |shell, cx| {
                        if shell.clipboard_clear_deadline.is_none() {
                            // Wipe fired (or settings flipped to Never).
                            return false;
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !keep_ticking {
                    break;
                }
            }
        }));
        self.clipboard_clear_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(secs))
                .await;
            let _ = this.update(cx, |shell, cx| {
                // A cancelled timer can race a replacement copy. Its token
                // must match before it may touch either clipboard or state.
                if shell.pending_clipboard_write != Some(write) {
                    return;
                }
                shell.clear_owned_clipboard(cx);
                shell.clipboard_clear_task = None;
                shell.clipboard_clear_deadline = None;
                shell.clipboard_pill_tick = None;
                cx.notify();
            });
        }));
    }

    /// True iff `entry_id`'s password is currently revealed in the
    /// detail panel. Driven by `revealed_entry_id` - see the state
    /// observer for the auto-mask-on-switch logic.
    pub fn is_password_revealed(&self, entry_id: &str) -> bool {
        self.revealed_entry_id.as_deref() == Some(entry_id)
    }

    /// Toggle the masked password into / out of view for `entry_id`.
    /// Idempotent - clicking twice ends up where you started.
    pub fn toggle_password_reveal(&mut self, entry_id: String, cx: &mut Context<Self>) {
        if self.revealed_entry_id.as_deref() == Some(&entry_id) {
            self.revealed_entry_id = None;
        } else {
            self.revealed_entry_id = Some(entry_id);
        }
        cx.notify();
    }

    pub fn settings(&self) -> &AppSettings {
        &self.settings
    }

    /// Persist a new `AppSettings`. Background-saves to disk
    /// (fire-and-forget - failures are non-fatal, the in-memory
    /// settings still apply for this session) and re-runs
    /// `sync_auto_lock_task` so a freshly-disabled timer is cancelled
    /// or a freshly-enabled one starts immediately.
    pub fn update_settings(&mut self, new_settings: AppSettings, cx: &mut Context<Self>) {
        if self.settings == new_settings {
            return;
        }
        self.settings = new_settings.clone();
        crate::keepass::limits::set_confirmed_kdf_costs(&self.settings.confirmed_kdf_costs);
        // If the user just toggled auto-lock on/off, we need to start
        // or cancel the checker task immediately rather than waiting
        // for the next state notification.
        self.sync_auto_lock_task(cx);
        // Same for auto-sync: toggling the interval (including to/from
        // "Never") must start or cancel the timer now, not on the next
        // state notification.
        self.sync_auto_sync_task(cx);
        // Same rule for auto-type: toggling the feature on, changing
        // the combo, or editing the sequence template all need to
        // re-evaluate the hotkey registration and parse-error cache
        // synchronously so the Settings UI shows the right state on
        // the very next render.
        self.sync_auto_type_listener(cx);
        // A failed write used to be invisible, so a preference the UI said
        // was saved silently reverted on the next launch.
        let write = cx.background_spawn(async move { crate::app::settings::save(&new_settings) });
        cx.spawn(async move |this, cx| {
            if let Err(error) = write.await {
                let _ = this.update(cx, |_shell, cx| {
                    if let Some(handle) = cx.active_window() {
                        let _ = handle.update(cx, |_root, window, cx| {
                            Self::notify_error(
                                window,
                                format!("Could not save your settings: {error}"),
                                cx,
                            );
                        });
                    }
                });
            }
        })
        .detach();
        cx.notify();
    }

    pub fn click_open_vault(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt_for_vault_path(window, cx);
    }

    pub fn click_lock_vault(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Same gate as Cmd+L: locking clears the editor, and the button and
        // the shortcut are the same action.
        if self.entry_draft_blocks_close(cx) {
            return;
        }
        self.lock_vault(window, cx);
    }
}
