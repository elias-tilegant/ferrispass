//! Action handlers, the group form, entry and group commands, and error
//! notices.

use super::*;

impl AppShell {
    pub(super) fn on_action_open_vault(
        &mut self,
        _: &OpenVault,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.entry_draft_blocks_close(cx) {
            return;
        }
        self.prompt_for_vault_path(window, cx);
    }

    pub(super) fn on_action_open_vault_switcher(
        &mut self,
        _: &OpenVaultSwitcher,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Opening the switcher replaces the entry editor, so the draft has
        // to be settled first.
        if self.entry_draft_blocks_close(cx) {
            return;
        }
        // Toggle: ⌘O while the switcher is already open closes it. Same
        // ergonomics as ⌘, on Settings.
        if matches!(self.state.read(cx).overlay(), Overlay::VaultSwitcher) {
            self.state.update(cx, |state, cx| state.close_overlay(cx));
            return;
        }
        // Always start with an empty filter so previous queries don't carry
        // over between invocations.
        self.vault_switcher_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        self.state.update(cx, |state, cx| {
            state.open_overlay(Overlay::VaultSwitcher, cx)
        });
    }

    pub(super) fn on_action_open_add_vault(
        &mut self,
        _: &OpenAddVault,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Every overlay replaces the entry editor, so a draft in it has to
        // be settled first.
        if self.entry_draft_blocks_close(cx) {
            return;
        }
        self.state
            .update(cx, |state, cx| state.open_overlay(Overlay::AddVault, cx));
    }

    pub(super) fn on_action_submit_password(
        &mut self,
        _: &SubmitPassword,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The Enter keybinding is global, but pressing Enter inside an
        // overlay (vault switcher, conflict picker, …) shouldn't also
        // submit the underlying unlock prompt. The overlay's own Input
        // handles its `PressEnter` event; let it own the keystroke.
        if self.state.read(cx).overlay().is_active() {
            return;
        }
        self.submit_password(window, cx);
    }

    pub(super) fn on_action_submit_biometric_unlock(
        &mut self,
        _: &crate::app::actions::SubmitBiometricUnlock,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Same overlay gate as `submit_password`: don't fire while
        // an overlay's own Input owns the keystroke.
        if self.state.read(cx).overlay().is_active() {
            return;
        }
        self.submit_biometric_unlock(window, cx);
    }

    pub(super) fn on_action_toggle_biometric_enrollment(
        &mut self,
        _: &crate::app::actions::ToggleBiometricEnrollment,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let next = !self.state.read(cx).pending_biometric_enrollment();
        self.state.update(cx, |state, cx| {
            state.set_pending_biometric_enrollment(next, cx)
        });
    }

    pub(super) fn on_action_forget_biometric(
        &mut self,
        _: &crate::app::actions::ForgetBiometric,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self.state.read(cx).pending_unlock_path() else {
            return;
        };
        let removed = self
            .state
            .update(cx, |state, cx| state.forget_biometric(&path, cx));
        if !removed {
            // Keychain delete couldn't be confirmed - the registry
            // pointer is kept on purpose so the password isn't
            // orphaned. Tell the user rather than silently pretending
            // it worked.
            window.push_notification(
                "Couldn't remove the Touch ID entry from the keychain. \
                 The vault still has Touch ID enabled - try again.",
                cx,
            );
        }
    }

    pub(super) fn on_action_cancel_unlock(
        &mut self,
        _: &CancelUnlock,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Escape unwinds in priority order: armed perma-delete → armed group
        // delete → open overlay → unlock prompt. Each layer eats the keystroke
        // so a single Escape never collapses two layers at once.
        if self.pending_perma_delete.is_some() {
            self.clear_perma_delete(cx);
            return;
        }
        if self.pending_group_delete.is_some() {
            self.cancel_group_delete(cx);
            return;
        }
        if self.disconnect_armed {
            self.cancel_disconnect(cx);
            return;
        }
        // A half-written entry, and its freshly generated password, used to
        // vanish on one Escape with no warning. The first Escape arms the
        // discard, the second one performs it.
        if self.entry_draft_blocks_close(cx) {
            return;
        }
        let closed = self.state.update(cx, |state, cx| state.close_overlay(cx));
        if closed {
            return;
        }
        self.cancel_unlock(window, cx);
    }

    pub(super) fn on_action_lock_vault(
        &mut self,
        _: &LockVault,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.entry_draft_blocks_close(cx) {
            return;
        }
        self.lock_vault(window, cx);
    }

    pub(super) fn on_action_focus_search(
        &mut self,
        _: &FocusSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_search(window, cx);
    }

    pub(super) fn on_action_copy_username(
        &mut self,
        _: &CopyUsername,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.copy_selected_value(CopyValueKind::Username, window, cx);
    }

    pub(super) fn on_action_copy_url(
        &mut self,
        _: &CopyUrl,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.copy_selected_value(CopyValueKind::Url, window, cx);
    }

    pub(super) fn on_action_copy_password(
        &mut self,
        _: &CopyPassword,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.copy_selected_value(CopyValueKind::Password, window, cx);
    }

    pub(super) fn on_action_open_connect(
        &mut self,
        _: &OpenConnect,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Every overlay replaces the entry editor, so a draft in it has to
        // be settled first.
        if self.entry_draft_blocks_close(cx) {
            return;
        }
        // The input entity outlives an individual Connect overlay. Clear its
        // displayed value as well as resetting ConnectFlow::Picking::query,
        // otherwise reopening the flow shows the previous session's filter.
        self.picker_query_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.state.update(cx, |state, cx| {
            state.open_overlay(Overlay::Connect, cx);
            state.begin_connect_flow(cx);
        });
    }

    pub(super) fn on_action_open_reconnect(
        &mut self,
        _: &OpenReconnect,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Every overlay replaces the entry editor, so a draft in it has to
        // be settled first.
        if self.entry_draft_blocks_close(cx) {
            return;
        }
        // Token-only re-auth of the active synced vault - NOT the full
        // picker-driven connect flow (which would download a duplicate
        // local copy). `start_sharepoint_reconnect` reuses the existing
        // on-disk SyncConfig and opens straight onto the device-code step.
        self.state
            .update(cx, |state, cx| state.start_sharepoint_reconnect(cx));
    }

    pub(super) fn on_action_add_sharepoint_vault(
        &mut self,
        _: &AddSharePointVault,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Every overlay replaces the entry editor, so a draft in it has to
        // be settled first.
        if self.entry_draft_blocks_close(cx) {
            return;
        }
        self.picker_query_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.state.update(cx, |state, cx| {
            state.open_overlay(Overlay::Connect, cx);
            state.begin_connect_flow(cx);
            state.start_sharepoint_connect(cx);
        });
    }

    pub(super) fn on_action_open_settings(
        &mut self,
        _: &OpenSettings,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.entry_draft_blocks_close(cx) {
            return;
        }
        // Universally available - no vault-open gate.
        self.settings_tab = SettingsTab::General;
        self.state
            .update(cx, |state, cx| state.open_overlay(Overlay::Settings, cx));
    }

    pub(super) fn on_action_open_sync_settings(
        &mut self,
        _: &OpenSyncSettings,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.entry_draft_blocks_close(cx) {
            return;
        }
        // Same Settings overlay as ⌘, but jumps directly to the Sync
        // tab. Reachable from the vault-header sync chip and ⌘⇧, so
        // users can land where they were going without a tab click.
        //
        // Gate: Sync makes no sense without a decrypted vault - fall
        // back to General so the keybinding still opens *something*
        // useful from the locked perspective rather than dropping the
        // user onto a tab they can't act on.
        let vault_open = matches!(
            self.state.read(cx).vault_status(),
            crate::app::VaultStatus::Open { .. }
        );
        self.settings_tab = if vault_open {
            SettingsTab::Sync
        } else {
            SettingsTab::General
        };
        self.state
            .update(cx, |state, cx| state.open_overlay(Overlay::Settings, cx));
    }

    pub(super) fn on_action_install_update(
        &mut self,
        _: &InstallUpdate,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.update(cx, |state, cx| state.install_update(cx));
    }

    pub(super) fn on_action_open_whats_new(
        &mut self,
        _: &OpenWhatsNew,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Every overlay replaces the entry editor, so a draft in it has to
        // be settled first.
        if self.entry_draft_blocks_close(cx) {
            return;
        }
        self.state.update(cx, |state, cx| state.open_whats_new(cx));
    }

    pub(super) fn on_action_open_about(
        &mut self,
        _: &OpenAbout,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Every overlay replaces the entry editor, so a draft in it has to
        // be settled first.
        if self.entry_draft_blocks_close(cx) {
            return;
        }
        self.state.update(cx, |state, cx| state.open_about(cx));
    }

    pub(super) fn on_action_sync_now(
        &mut self,
        _: &SyncNow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::app::SyncActivity;

        // The `Copy` summary, not a clone of the status: the branch below
        // needs four cases, while cloning `SyncStatus` deep-copies two
        // decrypted databases whenever a conflict is open.
        match self.state.read(cx).sync_activity() {
            // Nothing to sync against - fall back to the Sync settings tab
            // so the user can connect or re-authenticate.
            SyncActivity::Inactive => {
                window.dispatch_action(Box::new(OpenSyncSettings), cx);
            }
            // Already in flight or awaiting user input - do nothing.
            SyncActivity::Busy => {}
            SyncActivity::Resting | SyncActivity::Failed => {
                self.state.update(cx, |state, cx| state.sync_now(cx));
            }
        }
    }

    pub(super) fn on_action_download_favicons(
        &mut self,
        _: &DownloadFavicons,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state
            .update(cx, |state, cx| state.start_favicon_download(cx));
    }

    pub(super) fn on_action_new_entry(
        &mut self,
        _: &NewEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let is_open = matches!(
            self.state.read(cx).vault_status(),
            crate::app::VaultStatus::Open { .. }
        );
        if is_open {
            // Cmd+N over an open editor resets it, so a draft in there has to
            // be settled first.
            if self.entry_draft_blocks_close(cx) {
                return;
            }
            // Reset the form before showing it - otherwise reopening the modal
            // after a previous Save/Cancel would carry the prior values.
            self.clear_entry_form(window, cx);
            self.mark_entry_form_pristine(cx);
            self.state
                .update(cx, |state, cx| state.open_overlay(Overlay::AddEntry, cx));
            // Otherwise the caret stays where it was, and after Cmd+F that is
            // the search box behind the dimmed modal: the user types into an
            // apparently empty form and re-filters the list underneath.
            self.new_entry_title_input
                .update(cx, |input, cx| input.focus(window, cx));
        }
    }

    pub(super) fn on_action_create_vault(
        &mut self,
        _: &CreateVault,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.push_notification("Create-vault flow is coming soon.", cx);
    }

    pub(super) fn on_action_toggle_theme(
        &mut self,
        _: &ToggleTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Rotates System, Light, Dark and remembers the choice. It used to
        // flip the live theme and forget it on quit.
        let next = self.settings.theme.next();
        crate::ui::theme::apply_choice(next, Some(window), cx);
        let mut settings = self.settings.clone();
        settings.theme = next;
        self.update_settings(settings, cx);
        window.push_notification(format!("Theme: {}", next.label()), cx);
    }

    pub(super) fn on_action_save_vault(
        &mut self,
        _: &SaveVault,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.update(cx, |state, cx| state.save_async(cx));
    }

    pub(super) fn on_action_edit_entry(
        &mut self,
        _: &EditEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Opening a different entry overwrites whatever is in the editor.
        if self.entry_draft_blocks_close(cx) {
            return;
        }
        self.begin_edit_selected_entry(window, cx);
    }

    pub(super) fn on_action_delete_entry(
        &mut self,
        _: &DeleteEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_selected_entry(window, cx);
    }

    pub(super) fn on_action_new_group(
        &mut self,
        _: &NewGroup,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Every overlay replaces the entry editor, so a draft in it has to
        // be settled first.
        if self.entry_draft_blocks_close(cx) {
            return;
        }
        let root_id = self
            .state
            .read(cx)
            .vault_browser()
            .map(|b| b.snapshot.root.id.clone());
        let Some(root_id) = root_id else {
            return;
        };
        self.begin_add_group(root_id, window, cx);
    }

    pub(super) fn on_action_new_subgroup(
        &mut self,
        action: &NewSubgroup,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Every overlay replaces the entry editor, so a draft in it has to
        // be settled first.
        if self.entry_draft_blocks_close(cx) {
            return;
        }
        self.begin_add_group(action.parent_group_id.clone(), window, cx);
    }

    pub(super) fn on_action_rename_group_op(
        &mut self,
        action: &RenameGroupOp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Every overlay replaces the entry editor, so a draft in it has to
        // be settled first.
        if self.entry_draft_blocks_close(cx) {
            return;
        }
        let current_name = self
            .state
            .read(cx)
            .vault_browser()
            .and_then(|b| {
                b.snapshot
                    .find_group(&action.group_id)
                    .map(|g| g.name.clone())
            })
            .unwrap_or_default();
        self.new_group_name_input
            .update(cx, |s, cx| s.set_value(&current_name, window, cx));
        let group_id = action.group_id.clone();
        self.state.clone().update(cx, |state, cx| {
            state.open_overlay(Overlay::RenameGroup { group_id }, cx)
        });
        self.new_group_name_input
            .update(cx, |input, cx| input.focus(window, cx));
    }

    pub(super) fn on_action_delete_group(
        &mut self,
        action: &DeleteGroup,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending_group_delete = Some(action.group_id.clone());
        cx.notify();
    }

    /// Second half of the armed group delete. The id comes from the banner so
    /// a stale arm can never delete a group the user is no longer looking at.
    pub(crate) fn confirm_group_delete(
        &mut self,
        group_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending_group_delete = None;
        self.run_mutation("Group moved to Trash.", window, cx, |state, cx| {
            state.delete_group(group_id, cx)
        });
    }

    pub(crate) fn cancel_group_delete(&mut self, cx: &mut Context<Self>) {
        if self.pending_group_delete.take().is_some() {
            cx.notify();
        }
    }

    fn begin_add_group(
        &mut self,
        parent_group_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_group_name_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.state.clone().update(cx, |state, cx| {
            state.open_overlay(Overlay::AddGroup { parent_group_id }, cx)
        });
        self.new_group_name_input
            .update(cx, |input, cx| input.focus(window, cx));
    }

    /// Snapshot the group-form input and dispatch to the right
    /// `AppState` mutation based on the active overlay variant. Toasts
    /// on success/failure and closes the overlay on success.
    pub fn submit_group_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.new_group_name_input.read(cx).value().to_string();
        let trimmed = name.trim().to_string();
        if trimmed.is_empty() {
            window.push_notification("Group name is required.", cx);
            return;
        }
        let overlay = self.state.read(cx).overlay().clone();
        let state = self.state.clone();
        match overlay {
            Overlay::AddGroup { parent_group_id } => {
                let result = state.update(cx, |state, cx| {
                    state.create_group(&parent_group_id, &trimmed, cx)
                });
                match result {
                    Ok(_) => {
                        self.state
                            .clone()
                            .update(cx, |state, cx| state.close_overlay(cx));
                        window.push_notification("Group created.", cx);
                    }
                    Err(e) => {
                        window.push_notification(format!("Could not create group: {e}"), cx);
                    }
                }
            }
            Overlay::RenameGroup { group_id } => {
                let result =
                    state.update(cx, |state, cx| state.rename_group(&group_id, &trimmed, cx));
                match result {
                    Ok(()) => {
                        self.state
                            .clone()
                            .update(cx, |state, cx| state.close_overlay(cx));
                        window.push_notification("Group renamed.", cx);
                    }
                    Err(e) => {
                        window.push_notification(format!("Could not rename group: {e}"), cx);
                    }
                }
            }
            _ => {}
        }
    }

    pub(super) fn on_new_group_name_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::PressEnter { .. }) {
            self.submit_group_form(window, cx);
        }
    }

    /// Move the currently-selected entry to the Recycle Bin. No confirmation
    /// (it's recoverable from Trash). Toasts on success/failure.
    /// An error the user has to read. Every `push_notification` in the app
    /// used the default builder, which auto-hides after a few seconds, so
    /// "Could not save entry" could disappear before the user looked up.
    /// Successes still auto-hide; a failure waits to be dismissed.
    pub(crate) fn notify_error(
        window: &mut Window,
        message: impl Into<SharedString>,
        cx: &mut gpui::App,
    ) {
        #[cfg(test)]
        let message = characterization_tests::record_error_notice(message.into());
        window.push_notification(
            gpui_component::notification::Notification::error(message).autohide(false),
            cx,
        );
    }

    /// Run a vault mutation and tell the user what happened, win or lose.
    /// Every mutation goes through here: the drag-and-drop and star paths
    /// used to discard their `Result`, so a failed drop looked exactly like
    /// a successful one.
    pub(crate) fn run_mutation<F>(
        &mut self,
        success: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
        mutate: F,
    ) where
        F: FnOnce(&mut AppState, &mut Context<AppState>) -> Result<(), MutationError>,
    {
        match self.state.clone().update(cx, mutate) {
            Ok(()) => window.push_notification(success.into(), cx),
            Err(error) => Self::notify_error(window, errors::mutation_message(&error), cx),
        }
    }

    pub fn delete_selected_entry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected_entry_id(cx) else {
            window.push_notification("Select an entry to delete first.", cx);
            return;
        };
        self.run_mutation("Moved to Trash.", window, cx, |state, cx| {
            state.delete_entry(&id, cx)
        });
    }

    pub fn restore_selected_entry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected_entry_id(cx) else {
            return;
        };
        self.run_mutation("Entry restored.", window, cx, |state, cx| {
            state.restore_entry(&id, cx)
        });
    }

    /// Flip the favourite marker. The star icon is its own success feedback,
    /// so this reports only failures.
    pub(crate) fn toggle_starred(
        &mut self,
        entry_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = self
            .state
            .clone()
            .update(cx, |state, cx| state.toggle_starred(entry_id, cx))
        {
            Self::notify_error(window, errors::mutation_message(&error), cx);
        }
    }

    /// Lift a deleted group back out of the Trash. Dispatched from the
    /// "Deleted groups" rows, which are the only place a trashed group shows.
    pub(crate) fn restore_group(
        &mut self,
        group_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_mutation("Group restored.", window, cx, |state, cx| {
            state.restore_group(group_id, cx)
        });
    }

    /// Permanently delete an entry (id passed in so the call site can match
    /// against the pending-confirm state precisely). Clears the pending flag
    /// on completion regardless of outcome.
    pub fn confirm_perma_delete(
        &mut self,
        entry_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending_perma_delete = None;
        self.run_mutation("Entry permanently deleted.", window, cx, |state, cx| {
            state.delete_entry_permanent(&entry_id, cx)
        });
        cx.notify();
    }

    pub(super) fn selected_entry_id(&self, cx: &gpui::App) -> Option<String> {
        self.state
            .read(cx)
            .vault_browser()
            .and_then(|b| b.selected_entry_id)
    }

    /// Resolve the currently-selected entry, prefill the form with its values
    /// (incl. the live decrypted password from `VaultDocument`), and switch
    /// the overlay into Edit mode. Called from both the `cmd-e` action and
    /// the Edit button on the detail panel.
    pub fn begin_edit_selected_entry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Pull the snapshot view of the selected entry + its actual password
        // out of state in one borrow, then drop the borrow before mutating
        // the inputs / overlay.
        let prefill = {
            let state = self.state.read(cx);
            let browser = state.vault_browser();
            let entry = browser.as_ref().and_then(|b| b.selected_entry.clone());
            let (password, otp) = match (&entry, state.vault_status()) {
                (Some(e), crate::app::VaultStatus::Open { document, .. }) => (
                    document.password_for_entry(&e.id),
                    document.otp_url_for_entry(&e.id),
                ),
                _ => (None, None),
            };
            entry.map(|e| EditPrefill {
                id: e.id.clone(),
                title: e.title.clone(),
                username: e.username.clone(),
                url: e.url.clone(),
                notes: e.notes.clone(),
                password: password.unwrap_or_default(),
                otp: otp.unwrap_or_default(),
                custom_fields: e.custom_fields.clone(),
            })
        };

        let Some(p) = prefill else {
            window.push_notification("Select an entry to edit first.", cx);
            return;
        };

        // Wipe any Add/Edit state from the previous overlay before installing
        // this entry's values. This also restores the secret inputs to masked.
        self.clear_entry_form(window, cx);
        self.new_entry_title_input
            .update(cx, |s, cx| s.set_value(&p.title, window, cx));
        self.new_entry_username_input
            .update(cx, |s, cx| s.set_value(&p.username, window, cx));
        self.new_entry_password_input.update(cx, |s, cx| {
            s.set_masked(true, window, cx);
            s.set_value(p.password.as_str(), window, cx);
        });
        self.new_entry_url_input
            .update(cx, |s, cx| s.set_value(&p.url, window, cx));
        self.new_entry_notes_input
            .update(cx, |s, cx| s.set_value(&p.notes, window, cx));
        self.new_entry_otp_input.update(cx, |s, cx| {
            s.set_masked(true, window, cx);
            s.set_value(p.otp.as_str(), window, cx);
        });

        // Rebuild the custom-fields editor rows from the entry. Each
        // row gets a fresh Entity<InputState> with the current value
        // pre-filled - necessary because `add_custom_field_row` only
        // creates blank rows.
        self.new_entry_custom_fields.clear();
        for cf in p.custom_fields {
            let key_input = cx.new(|cx| InputState::new(window, cx).placeholder("Key"));
            let value_input = cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(cf.protected)
                    .placeholder("Value")
            });
            key_input.update(cx, |s, cx| s.set_value(&cf.key, window, cx));
            value_input.update(cx, |s, cx| s.set_value(&cf.value, window, cx));
            let row = self.custom_field_row(key_input, value_input, cf.protected, window, cx);
            self.new_entry_custom_fields.push(row);
        }

        // After the prefill, not before: the values just written are the
        // starting point, not the user's edits.
        self.mark_entry_form_pristine(cx);
        self.state.update(cx, |state, cx| {
            state.open_overlay(Overlay::EditEntry { entry_id: p.id }, cx);
        });
        self.new_entry_title_input
            .update(cx, |input, cx| input.focus(window, cx));
    }
}
