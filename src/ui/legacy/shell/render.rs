//! Rendering: the body for the current state, the modal overlays, and the
//! clipboard pill.

use super::*;

impl AppShell {
    fn render_body(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let vault_status = self.state.read(cx).vault_status();
        let overlay = self.state.read(cx).overlay();

        // Settings is a global overlay - accessible regardless of
        // whether a vault is open (matches the Mac ⌘, convention of
        // Preferences always being reachable).
        if matches!(overlay, Overlay::Settings) {
            return crate::ui::screens::settings::render(self, cx);
        }
        if matches!(overlay, Overlay::Connect) {
            return crate::ui::screens::connect::render(self, cx);
        }
        match vault_status {
            crate::app::VaultStatus::Empty => crate::ui::screens::welcome::render(self, cx),
            crate::app::VaultStatus::AwaitingPassword { .. } => {
                crate::ui::screens::unlock::render(self, cx)
            }
            crate::app::VaultStatus::Opening { .. } => crate::ui::screens::vault::render(self, cx),
            crate::app::VaultStatus::Open { .. } => match overlay {
                Overlay::Conflict => crate::ui::screens::conflict::render(self, cx),
                Overlay::AddEntry | Overlay::EditEntry { .. } => {
                    // The same modal renders both Add and Edit; the variant
                    // tells the inner save handler which AppState method to call.
                    crate::ui::screens::add_entry::render(self, cx)
                }
                Overlay::AddGroup { .. } | Overlay::RenameGroup { .. } => {
                    crate::ui::screens::add_group::render(self, cx)
                }
                _ => crate::ui::screens::vault::render(self, cx),
            },
            crate::app::VaultStatus::LockedPendingSave => {
                crate::ui::screens::vault::render(self, cx)
            }
            crate::app::VaultStatus::Error { .. } => crate::ui::screens::vault::render(self, cx),
        }
    }
}

impl Focusable for AppShell {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl AppShell {
    fn render_modal_overlay(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let overlay = self.state.read(cx).overlay();
        if matches!(overlay, Overlay::VaultSwitcher) {
            return Some(self.vault_switcher.clone().into_any_element());
        }
        if matches!(overlay, Overlay::AddVault) {
            return Some(crate::ui::screens::add_vault::render(cx));
        }
        if matches!(overlay, Overlay::WhatsNew { .. }) {
            return Some(crate::ui::screens::whats_new::render(self, cx));
        }
        if matches!(overlay, Overlay::About) {
            return Some(crate::ui::screens::about::render(self, cx));
        }
        None
    }

    /// Tiny pill in the bottom-right corner that counts down the
    /// remaining seconds until the clipboard auto-clear fires. `None`
    /// when no clear is pending or the deadline has lapsed (the wipe
    /// task drops the deadline as part of its cleanup).
    fn render_clipboard_pill(&self) -> Option<impl gpui::IntoElement> {
        let deadline = self.clipboard_clear_deadline?;
        let remaining = deadline.checked_duration_since(Instant::now())?;
        let secs = remaining.as_secs();
        if secs == 0 {
            return None;
        }
        Some(
            div().absolute().bottom_4().right_4().child(
                gpui::div()
                    .h(px(28.))
                    .px(px(12.))
                    .rounded(px(14.))
                    .bg(crate::ui::palette::panel())
                    .border_1()
                    .border_color(crate::ui::palette::border_strong())
                    .text_xs()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(crate::ui::palette::text_muted())
                    .flex()
                    .items_center()
                    .child(format!("Clipboard clears in {secs} s")),
            ),
        )
    }
}

impl Render for AppShell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let body = AppShell::render_body(self, cx);
        let clipboard_pill = self.render_clipboard_pill();
        let modal_overlay = self.render_modal_overlay(cx);

        div()
            .key_context(APP_CONTEXT)
            .track_focus(&self.focus_handle)
            // Reset the idle clock on any user input. Setting an
            // `Instant` is cheap and we deliberately don't `cx.notify`
            // here - only the auto-lock checker reads the field, and
            // it does so on its own schedule.
            .on_mouse_move(cx.listener(|shell: &mut AppShell, _, _, _| {
                shell.last_activity = SystemTime::now();
            }))
            .on_key_down(cx.listener(|shell: &mut AppShell, _, _, _| {
                shell.last_activity = SystemTime::now();
            }))
            .on_action(cx.listener(Self::on_action_open_vault))
            .on_action(cx.listener(Self::on_action_open_vault_switcher))
            .on_action(cx.listener(Self::on_action_open_add_vault))
            .on_action(cx.listener(Self::on_action_submit_password))
            .on_action(cx.listener(Self::on_action_submit_biometric_unlock))
            .on_action(cx.listener(Self::on_action_toggle_biometric_enrollment))
            .on_action(cx.listener(Self::on_action_forget_biometric))
            .on_action(cx.listener(Self::on_action_cancel_unlock))
            .on_action(cx.listener(Self::on_action_lock_vault))
            .on_action(cx.listener(Self::on_action_focus_search))
            .on_action(cx.listener(Self::on_action_copy_username))
            .on_action(cx.listener(Self::on_action_copy_url))
            .on_action(cx.listener(Self::on_action_copy_password))
            .on_action(cx.listener(Self::on_action_open_connect))
            .on_action(cx.listener(Self::on_action_open_reconnect))
            .on_action(cx.listener(Self::on_action_add_sharepoint_vault))
            .on_action(cx.listener(Self::on_action_open_settings))
            .on_action(cx.listener(Self::on_action_open_sync_settings))
            .on_action(cx.listener(Self::on_action_install_update))
            .on_action(cx.listener(Self::on_action_open_whats_new))
            .on_action(cx.listener(Self::on_action_open_about))
            .on_action(cx.listener(Self::on_action_sync_now))
            .on_action(cx.listener(Self::on_action_download_favicons))
            .on_action(cx.listener(Self::on_action_new_entry))
            .on_action(cx.listener(Self::on_action_create_vault))
            .on_action(cx.listener(Self::on_action_toggle_theme))
            .on_action(cx.listener(Self::on_action_save_vault))
            .on_action(cx.listener(Self::on_action_edit_entry))
            .on_action(cx.listener(Self::on_action_delete_entry))
            .on_action(cx.listener(Self::on_action_new_group))
            .on_action(cx.listener(Self::on_action_new_subgroup))
            .on_action(cx.listener(Self::on_action_rename_group_op))
            .on_action(cx.listener(Self::on_action_delete_group))
            .on_action(cx.listener(Self::on_action_perform_auto_type))
            .on_action(cx.listener(Self::on_action_perform_auto_type_for_selected))
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(body)
            .children(modal_overlay)
            .children(clipboard_pill)
    }
}
