//! Cancelling an unlock, locking by hand or automatically, and the search
//! field.

use super::*;

impl AppShell {
    pub fn cancel_unlock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.read(cx).pending_unlock_path().is_none() {
            self.clear_search(window, cx);
            return;
        }

        self.clear_unlock_form(window, cx);
        // Esc on the unlock screen: if the user had a vault open before
        // and was just about to unlock another, snap back to the original
        // instead of hard-locking everything.
        let rehydrated = self
            .state
            .update(cx, |state, cx| state.rehydrate_most_recent_park(cx));
        if !rehydrated {
            self.lock_vault(window, cx);
        }
    }

    pub fn lock_vault(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // AppState rotates the Auto-Type context before it revokes decrypted
        // ownership. Do this first so form and clipboard cleanup cannot leave
        // an old background typer authorized in parallel.
        self.state.update(cx, |state, cx| {
            let _ = state.lock_vault(cx);
        });
        self.clear_unlock_form(window, cx);
        self.clear_entry_form(window, cx);
        self.pending_perma_delete = None;
        self.pending_group_delete = None;
        self.disconnect_armed = false;
        self.search_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        // If Settings was sitting on the Sync tab when the user locked,
        // bounce them back to General - the sidebar will mark Sync
        // disabled while locked, but the active-tab state should
        // already match what's allowed.
        if matches!(self.settings_tab, SettingsTab::Sync) {
            self.settings_tab = SettingsTab::General;
        }
        // Same secret-wipe as auto-lock: drop reveal, cancel pending
        // clipboard timer, flush clipboard if it still holds our copy.
        self.wipe_session_secrets(cx);
    }

    /// Lock because the idle timer or the screen lock said so, not the user.
    ///
    /// An unsaved draft cannot veto this one: refusing to lock would turn the
    /// security feature off for anyone who left the editor open. The draft is
    /// discarded either way, so the difference this makes is that the user is
    /// told, rather than finding an empty editor and guessing.
    pub fn lock_vault_automatically(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let had_unsaved_draft = self.entry_form_is_dirty(cx);
        self.lock_vault(window, cx);
        if had_unsaved_draft {
            Self::notify_error(
                window,
                "The vault locked while an entry was open. Its unsaved changes were discarded.",
                cx,
            );
        }
    }

    /// Moves the focus into the vault search. Since GPUI Kit 0.7 a text field
    /// passes Cmd+F on to the shell, so this also runs from inside a field; that
    /// is intended, Cmd+F always searches. While an overlay is open the search
    /// field is hidden or behind a modal, so the focus stays where it is.
    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        if state.vault_browser().is_some() && !state.overlay().is_active() {
            self.search_input
                .update(cx, |input, cx| input.focus(window, cx));
        }
    }

    pub(super) fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.state.update(cx, |state, cx| state.clear_search(cx));
    }
}
