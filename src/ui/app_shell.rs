use crate::{
    app::{
        AppState, CopyValueKind, Overlay,
        actions::{
            APP_CONTEXT, CancelUnlock, CopyPassword, CopyUrl, CopyUsername, CreateVault,
            DeleteEntry, EditEntry, SaveVault, ToggleTheme,
            FocusSearch, LockVault, NewEntry, OpenConflictDemo, OpenConnect, OpenSyncSettings,
            OpenVault, SubmitPassword,
        },
    },
    keepass::KeePassRepository,
};
use gpui::{
    AppContext as _, ClickEvent, ClipboardItem, Context, Entity, FocusHandle, Focusable,
    InteractiveElement as _, ParentElement as _, PathPromptOptions, Render, ScrollStrategy,
    Styled as _, Subscription, Task, Window, div,
};
use std::time::Duration;
use gpui_component::{
    ActiveTheme as _, Root, VirtualListScrollHandle, WindowExt as _,
    input::{InputEvent, InputState},
    slider::{SliderState, SliderValue},
};
use std::path::PathBuf;

struct EditPrefill {
    id: String,
    title: String,
    username: String,
    url: String,
    notes: String,
    password: String,
    otp: String,
}

pub struct AppShell {
    state: Entity<AppState>,
    password_input: Entity<InputState>,
    keyfile_input: Entity<InputState>,
    search_input: Entity<InputState>,
    new_entry_title_input: Entity<InputState>,
    new_entry_username_input: Entity<InputState>,
    new_entry_password_input: Entity<InputState>,
    new_entry_url_input: Entity<InputState>,
    new_entry_notes_input: Entity<InputState>,
    new_entry_otp_input: Entity<InputState>,
    /// Length slider state for the AddEntry generator card. Range 4..64,
    /// default 18 (matches the prior hardcoded value the card used to display).
    /// We hold the entity so both the Slider widget and `generate_password`
    /// can read the same source of truth.
    gen_length_state: Entity<SliderState>,
    /// Toggle state for the four character classes shown next to the slider.
    /// Mutated by `toggle_gen_class` with a min-one-active guard so the user
    /// can't disable every class (which would silently fall back to lowercase
    /// inside the generator and surprise them).
    gen_classes: crate::keepass::password_gen::CharClasses,
    /// Persistent scroll position for the entry virtual list. Must outlive a single
    /// render — without it the list resets to the top on every re-render and
    /// mouse-wheel events appear to do nothing.
    entry_list_scroll: VirtualListScrollHandle,
    /// The shell's own focus handle. We `track_focus` it on the root div so the
    /// app always has a focused element in the dispatch path — without this,
    /// GPUI has nothing to walk and `cmd-f`-style key bindings never fire when
    /// the user hasn't clicked into a specific input yet.
    focus_handle: FocusHandle,
    /// Held debounce task for the search input. Replacing it cancels the prior task
    /// (GPUI cancels tasks on drop unless `.detach()`-ed), so only the latest
    /// keystroke gets to fire the actual filter rebuild.
    search_debounce: Option<Task<()>>,
    /// Inline confirmation state for "Delete forever". When set to an entry id,
    /// the detail-panel button group renders a red "Confirm delete forever"
    /// chip instead of the plain delete button. Click outside / different
    /// entry / Escape clears it.
    pending_perma_delete: Option<String>,
    /// 1 Hz tick task that drives the live TOTP countdown in the detail panel.
    /// Started after a vault opens and dropped (= cancelled) on lock. We only
    /// need this for the seconds-resolution countdown; the actual TOTP code
    /// changes every 30 s so a coarser tick would also work, but 1 Hz keeps
    /// the "valid for Xs" text moving smoothly.
    totp_tick: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl AppShell {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let password_input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Master password")
        });
        let keyfile_input = cx.new(|cx| InputState::new(window, cx).placeholder("Optional key file path"));
        let search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search vault…  ⌘F"));
        let new_entry_title_input = cx.new(|cx| InputState::new(window, cx).placeholder("Title"));
        let new_entry_username_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Username"));
        let new_entry_password_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Password"));
        let new_entry_url_input = cx.new(|cx| InputState::new(window, cx).placeholder("URL"));
        let new_entry_notes_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Notes (optional)"));
        let new_entry_otp_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("otpauth://… or base32 secret")
        });

        let gen_length_state = cx.new(|_| {
            SliderState::new()
                .min(4.0)
                .max(64.0)
                .step(1.0)
                .default_value(SliderValue::Single(18.0))
        });

        let focus_handle = cx.focus_handle();
        // Grab focus immediately so cmd-f / cmd-l / cmd-, fire on the very first
        // keystroke. Without this the window has no focused element until the
        // user clicks something, and key dispatch has no path to walk.
        window.focus(&focus_handle, cx);

        let _subscriptions = vec![
            cx.observe(&state, |shell: &mut AppShell, _, cx| {
                // Re-render whenever AppState notifies AND keep the TOTP tick
                // task in sync with whether a vault is currently open.
                shell.sync_totp_tick(cx);
                cx.notify();
            }),
            cx.subscribe_in(&password_input, window, Self::on_password_input_event),
            cx.subscribe_in(&keyfile_input, window, Self::on_keyfile_input_event),
            cx.subscribe_in(&search_input, window, Self::on_search_input_event),
            // Re-render on slider drag so the "Length: N" label and the
            // strength preview update live alongside the thumb.
            cx.observe(&gen_length_state, |_shell: &mut AppShell, _, cx| {
                cx.notify();
            }),
        ];

        Self {
            state,
            password_input,
            keyfile_input,
            search_input,
            new_entry_title_input,
            new_entry_username_input,
            new_entry_password_input,
            new_entry_url_input,
            new_entry_notes_input,
            new_entry_otp_input,
            gen_length_state,
            gen_classes: crate::keepass::password_gen::CharClasses::default(),
            entry_list_scroll: VirtualListScrollHandle::new(),
            focus_handle,
            search_debounce: None,
            pending_perma_delete: None,
            totp_tick: None,
            _subscriptions,
        }
    }

    pub fn pending_perma_delete(&self) -> Option<&str> {
        self.pending_perma_delete.as_deref()
    }

    /// Start the per-second TOTP refresh loop only when the currently-selected
    /// entry has an OTP field; cancel it otherwise. Called from the state
    /// observer so it follows both open/lock transitions and selection changes
    /// — moving away from a TOTP entry stops the tick, moving onto one starts
    /// it. Saves the wasted 1 Hz re-renders for the (common) case where the
    /// user isn't looking at a TOTP entry.
    fn sync_totp_tick(&mut self, cx: &mut Context<Self>) {
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
                // Drop the task — GPUI cancels it.
                self.totp_tick = None;
            }
            _ => {}
        }
    }

    pub fn arm_perma_delete(&mut self, entry_id: String, cx: &mut Context<Self>) {
        self.pending_perma_delete = Some(entry_id);
        cx.notify();
    }

    pub fn clear_perma_delete(&mut self, cx: &mut Context<Self>) {
        if self.pending_perma_delete.take().is_some() {
            cx.notify();
        }
    }

    pub fn state(&self) -> &Entity<AppState> {
        &self.state
    }

    pub fn entry_list_scroll(&self) -> &VirtualListScrollHandle {
        &self.entry_list_scroll
    }

    pub fn password_input(&self) -> &Entity<InputState> {
        &self.password_input
    }

    pub fn keyfile_input(&self) -> &Entity<InputState> {
        &self.keyfile_input
    }

    pub fn search_input(&self) -> &Entity<InputState> {
        &self.search_input
    }

    pub fn new_entry_title_input(&self) -> &Entity<InputState> {
        &self.new_entry_title_input
    }

    pub fn new_entry_username_input(&self) -> &Entity<InputState> {
        &self.new_entry_username_input
    }

    pub fn new_entry_password_input(&self) -> &Entity<InputState> {
        &self.new_entry_password_input
    }

    pub fn new_entry_url_input(&self) -> &Entity<InputState> {
        &self.new_entry_url_input
    }

    pub fn new_entry_notes_input(&self) -> &Entity<InputState> {
        &self.new_entry_notes_input
    }

    pub fn new_entry_otp_input(&self) -> &Entity<InputState> {
        &self.new_entry_otp_input
    }

    /// Snapshot the AddEntry form into an `EntryDraft`. Tags aren't editable
    /// in the modal yet; we leave them empty so create_entry doesn't accidentally
    /// add ghost tags.
    pub fn collect_entry_draft(&self, cx: &gpui::App) -> crate::keepass::EntryDraft {
        crate::keepass::EntryDraft {
            title: self.new_entry_title_input.read(cx).value().to_string(),
            username: self.new_entry_username_input.read(cx).value().to_string(),
            password: self.new_entry_password_input.read(cx).value().to_string(),
            url: self.new_entry_url_input.read(cx).value().to_string(),
            notes: self.new_entry_notes_input.read(cx).value().to_string(),
            tags: Vec::new(),
            otp: self.new_entry_otp_input.read(cx).value().to_string(),
        }
    }

    /// Clear every AddEntry input. Called after a successful save and on
    /// Cancel so the next "New entry" opens with a blank form.
    pub fn clear_entry_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for input in [
            &self.new_entry_title_input,
            &self.new_entry_username_input,
            &self.new_entry_password_input,
            &self.new_entry_url_input,
            &self.new_entry_notes_input,
            &self.new_entry_otp_input,
        ] {
            input.update(cx, |state, cx| state.set_value("", window, cx));
        }
    }

    /// Read the current generator length from the slider state. Centralised so
    /// `generate_password` and the AddEntry card render code stay in sync on
    /// rounding rules.
    pub fn gen_length(&self, cx: &gpui::App) -> usize {
        self.gen_length_state.read(cx).value().start().round() as usize
    }

    pub fn gen_length_state(&self) -> &Entity<SliderState> {
        &self.gen_length_state
    }

    pub fn gen_classes(&self) -> crate::keepass::password_gen::CharClasses {
        self.gen_classes
    }

    /// Toggle one of the four character classes by index (0=upper, 1=lower,
    /// 2=digits, 3=symbols). Refuses the toggle if it would leave zero classes
    /// enabled — without this the generator silently falls back to lowercase
    /// and the user's "all unchecked" state would produce passwords that
    /// disagree with the disabled chips.
    pub fn toggle_gen_class(&mut self, idx: usize, cx: &mut Context<Self>) {
        let mut next = self.gen_classes;
        let slot = match idx {
            0 => &mut next.upper,
            1 => &mut next.lower,
            2 => &mut next.digits,
            3 => &mut next.symbols,
            _ => return,
        };
        *slot = !*slot;
        let any_enabled = next.upper || next.lower || next.digits || next.symbols;
        if !any_enabled {
            return;
        }
        self.gen_classes = next;
        cx.notify();
    }

    /// Generate a fresh password using the current slider length and class
    /// toggles, then drop the result into the AddEntry password field.
    pub fn generate_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let length = self.gen_length(cx);
        let pw = crate::keepass::password_gen::generate(length, self.gen_classes);
        self.new_entry_password_input
            .update(cx, |state, cx| state.set_value(pw, window, cx));
    }

    fn on_action_open_vault(&mut self, _: &OpenVault, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt_for_vault_path(window, cx);
    }

    fn on_action_submit_password(
        &mut self,
        _: &SubmitPassword,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.submit_password(window, cx);
    }

    fn on_action_cancel_unlock(
        &mut self,
        _: &CancelUnlock,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Escape unwinds in priority order: armed perma-delete → open overlay
        // → unlock prompt. Each layer eats the keystroke so a single Escape
        // never collapses two layers at once.
        if self.pending_perma_delete.is_some() {
            self.clear_perma_delete(cx);
            return;
        }
        let closed = self
            .state
            .update(cx, |state, cx| state.close_overlay(cx));
        if closed {
            return;
        }
        self.cancel_unlock(window, cx);
    }

    fn on_action_lock_vault(
        &mut self,
        _: &LockVault,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.lock_vault(window, cx);
    }

    fn on_action_focus_search(
        &mut self,
        _: &FocusSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_search(window, cx);
    }

    fn on_action_copy_username(
        &mut self,
        _: &CopyUsername,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.copy_selected_value(CopyValueKind::Username, window, cx);
    }

    fn on_action_copy_url(&mut self, _: &CopyUrl, window: &mut Window, cx: &mut Context<Self>) {
        self.copy_selected_value(CopyValueKind::Url, window, cx);
    }

    fn on_action_copy_password(
        &mut self,
        _: &CopyPassword,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.copy_selected_value(CopyValueKind::Password, window, cx);
    }

    fn on_action_open_connect(
        &mut self,
        _: &OpenConnect,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state
            .update(cx, |state, cx| state.open_overlay(Overlay::Connect, cx));
    }

    fn on_action_open_sync_settings(
        &mut self,
        _: &OpenSyncSettings,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let is_open = matches!(
            self.state.read(cx).vault_status(),
            crate::app::VaultStatus::Open { .. }
        );
        if is_open {
            self.state
                .update(cx, |state, cx| state.open_overlay(Overlay::SyncSettings, cx));
        }
    }

    fn on_action_new_entry(
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
            // Reset the form before showing it — otherwise reopening the modal
            // after a previous Save/Cancel would carry the prior values.
            self.clear_entry_form(window, cx);
            self.state
                .update(cx, |state, cx| state.open_overlay(Overlay::AddEntry, cx));
        }
    }

    fn on_action_open_conflict_demo(
        &mut self,
        _: &OpenConflictDemo,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let is_open = matches!(
            self.state.read(cx).vault_status(),
            crate::app::VaultStatus::Open { .. }
        );
        if is_open {
            self.state
                .update(cx, |state, cx| state.open_overlay(Overlay::Conflict, cx));
        }
    }

    fn on_action_create_vault(
        &mut self,
        _: &CreateVault,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.push_notification("Create-vault flow is coming soon.", cx);
    }

    fn on_action_toggle_theme(
        &mut self,
        _: &ToggleTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        crate::ui::theme::toggle(window, cx);
    }

    fn on_action_save_vault(
        &mut self,
        _: &SaveVault,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state
            .update(cx, |state, cx| state.save_async(cx));
    }

    fn on_action_edit_entry(
        &mut self,
        _: &EditEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.begin_edit_selected_entry(window, cx);
    }

    fn on_action_delete_entry(
        &mut self,
        _: &DeleteEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.delete_selected_entry(window, cx);
    }

    /// Move the currently-selected entry to the Recycle Bin. No confirmation
    /// (it's recoverable from Trash). Toasts on success/failure.
    pub fn delete_selected_entry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected_entry_id(cx) else {
            window.push_notification("Select an entry to delete first.", cx);
            return;
        };
        let result = self
            .state
            .clone()
            .update(cx, |state, cx| state.delete_entry(&id, cx));
        match result {
            Ok(()) => window.push_notification("Moved to Trash.", cx),
            Err(e) => window.push_notification(format!("Could not delete entry: {e}"), cx),
        }
    }

    pub fn restore_selected_entry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.selected_entry_id(cx) else {
            return;
        };
        let result = self
            .state
            .clone()
            .update(cx, |state, cx| state.restore_entry(&id, cx));
        match result {
            Ok(()) => window.push_notification("Entry restored.", cx),
            Err(e) => window.push_notification(format!("Could not restore: {e}"), cx),
        }
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
        let result = self
            .state
            .clone()
            .update(cx, |state, cx| state.delete_entry_permanent(&entry_id, cx));
        self.pending_perma_delete = None;
        match result {
            Ok(()) => window.push_notification("Entry permanently deleted.", cx),
            Err(e) => {
                window.push_notification(format!("Could not delete: {e}"), cx);
            }
        }
        cx.notify();
    }

    fn selected_entry_id(&self, cx: &gpui::App) -> Option<String> {
        self.state
            .read(cx)
            .vault_browser()
            .and_then(|b| b.selected_entry_id)
    }

    /// Resolve the currently-selected entry, prefill the form with its values
    /// (incl. the live decrypted password from `VaultDocument`), and switch
    /// the overlay into Edit mode. Called from both the `cmd-e` action and
    /// the Edit button on the detail panel.
    pub fn begin_edit_selected_entry(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
            })
        };

        let Some(p) = prefill else {
            window.push_notification("Select an entry to edit first.", cx);
            return;
        };

        self.new_entry_title_input
            .update(cx, |s, cx| s.set_value(&p.title, window, cx));
        self.new_entry_username_input
            .update(cx, |s, cx| s.set_value(&p.username, window, cx));
        self.new_entry_password_input
            .update(cx, |s, cx| s.set_value(&p.password, window, cx));
        self.new_entry_url_input
            .update(cx, |s, cx| s.set_value(&p.url, window, cx));
        self.new_entry_notes_input
            .update(cx, |s, cx| s.set_value(&p.notes, window, cx));
        self.new_entry_otp_input
            .update(cx, |s, cx| s.set_value(&p.otp, window, cx));

        self.state.update(cx, |state, cx| {
            state.open_overlay(Overlay::EditEntry { entry_id: p.id }, cx);
        });
    }

    fn on_password_input_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::PressEnter { .. }) {
            self.submit_password(window, cx);
        }
    }

    fn on_keyfile_input_event(
        &mut self,
        keyfile_input: &Entity<InputState>,
        event: &InputEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Change) {
            let raw = keyfile_input.read(cx).value().to_string();
            let trimmed = raw.trim();
            let keyfile = if trimmed.is_empty() {
                None
            } else {
                Some(PathBuf::from(trimmed))
            };
            self.state
                .update(cx, |state, cx| state.set_unlock_keyfile(keyfile, cx));
        }
    }

    fn on_search_input_event(
        &mut self,
        search_input: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(event, InputEvent::Change) {
            return;
        }
        let query = search_input.read(cx).value().to_string();
        // Empty query — apply immediately so closing search feels instant.
        if query.is_empty() {
            self.search_debounce = None;
            self.state
                .update(cx, |state, cx| state.set_search_query(String::new(), cx));
            self.entry_list_scroll
                .scroll_to_item(0, ScrollStrategy::Top);
            return;
        }
        // 150ms is short enough to feel live as you type, long enough to skip the
        // intermediate filter rebuilds while you're still typing a word.
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;
            let _ = this.update(cx, |shell, cx| {
                shell
                    .state
                    .update(cx, |state, cx| state.set_search_query(query, cx));
                shell
                    .entry_list_scroll
                    .scroll_to_item(0, ScrollStrategy::Top);
            });
        });
        self.search_debounce = Some(task);
    }

    pub fn prompt_for_vault_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Select a KeePass database".into()),
        });

        let shell = cx.entity();
        cx.spawn_in(window, async move |_, window| {
            let path = paths.await.ok()?.ok()??.first()?.clone();

            window
                .update(|window, cx| {
                    shell.update(cx, |shell, cx| {
                        shell.select_vault_path(path, window, cx);
                    })
                })
                .ok()
        })
        .detach();
    }

    fn select_vault_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if !is_kdbx_path(&path) {
            self.state.update(cx, |state, cx| {
                state.fail_vault_selection(
                    Some(path),
                    "Selected file is not a .kdbx database.",
                    cx,
                );
            });
            return;
        }

        self.password_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        let suggested = KeePassRepository::suggested_keyfile(&path);
        self.keyfile_input.update(cx, |input, cx| {
            let display = suggested
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            input.set_value(&display, window, cx);
        });
        self.clear_search(window, cx);

        self.state
            .update(cx, |state, cx| state.request_password(path, cx));
    }

    pub fn submit_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.state.read(cx).pending_unlock_path() else {
            return;
        };
        let keyfile = self.state.read(cx).pending_unlock_keyfile();

        let password = self.password_input.read(cx).value().to_string();
        if password.is_empty() && keyfile.is_none() {
            self.state.update(cx, |state, cx| {
                state.set_unlock_error("Enter the master password or pick a key file.", cx)
            });
            self.password_input
                .update(cx, |input, cx| input.focus(window, cx));
            return;
        }

        self.password_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.state
            .update(cx, |state, cx| state.begin_open(path.clone(), cx));

        let state = self.state.downgrade();
        let path_for_task = path.clone();
        let keyfile_for_task = keyfile.clone();
        let open_task = cx.background_spawn(async move {
            let result = KeePassRepository::open(
                &path_for_task,
                &password,
                keyfile_for_task.as_deref(),
            )
            .map_err(|error| error.to_string());

            (path_for_task, result)
        });

        cx.spawn(async move |_, cx| {
            let (path, result) = open_task.await;
            let _ = state.update(cx, |state, cx| {
                state.finish_open_attempt(path, result, cx);
            });
        })
        .detach();
    }

    pub fn cancel_unlock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.read(cx).pending_unlock_path().is_none() {
            self.clear_search(window, cx);
            return;
        }

        self.password_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.lock_vault(window, cx);
    }

    pub fn lock_vault(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.password_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.search_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.state.update(cx, |state, cx| state.lock_vault(cx));
    }

    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.read(cx).vault_browser().is_some() {
            self.search_input
                .update(cx, |input, cx| input.focus(window, cx));
        }
    }

    fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.state.update(cx, |state, cx| state.clear_search(cx));
    }

    pub fn copy_selected_value(
        &mut self,
        kind: CopyValueKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(value) = self.state.read(cx).copy_selected_value(kind) {
            cx.write_to_clipboard(ClipboardItem::new_string(value));
            window.push_notification(format!("{} copied.", copy_value_label(kind)), cx);
        } else {
            window.push_notification(format!("No {} to copy.", copy_value_label(kind)), cx);
        }
    }

    pub fn click_open_vault(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt_for_vault_path(window, cx);
    }

    pub fn click_lock_vault(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.lock_vault(window, cx);
    }

    fn render_body(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let vault_status = self.state.read(cx).vault_status();
        let overlay = self.state.read(cx).overlay();

        match vault_status {
            crate::app::VaultStatus::Empty if matches!(overlay, Overlay::Connect) => {
                crate::ui::screens::connect::render(self, cx)
            }
            crate::app::VaultStatus::Empty => crate::ui::screens::welcome::render(self, cx),
            crate::app::VaultStatus::AwaitingPassword { .. } => {
                crate::ui::screens::unlock::render(self, cx)
            }
            crate::app::VaultStatus::Opening { .. } => {
                crate::ui::screens::vault::render(self, cx)
            }
            crate::app::VaultStatus::Open { .. } => match overlay {
                Overlay::SyncSettings => crate::ui::screens::sync_settings::render(self, cx),
                Overlay::Conflict => crate::ui::screens::conflict::render(self, cx),
                Overlay::AddEntry | Overlay::EditEntry { .. } => {
                    // The same modal renders both Add and Edit; the variant
                    // tells the inner save handler which AppState method to call.
                    crate::ui::screens::add_entry::render(self, cx)
                }
                _ => crate::ui::screens::vault::render(self, cx),
            },
            crate::app::VaultStatus::Error { .. } => crate::ui::screens::vault::render(self, cx),
        }
    }
}

impl Focusable for AppShell {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for AppShell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let body = AppShell::render_body(self, cx);

        div()
            .key_context(APP_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_action_open_vault))
            .on_action(cx.listener(Self::on_action_submit_password))
            .on_action(cx.listener(Self::on_action_cancel_unlock))
            .on_action(cx.listener(Self::on_action_lock_vault))
            .on_action(cx.listener(Self::on_action_focus_search))
            .on_action(cx.listener(Self::on_action_copy_username))
            .on_action(cx.listener(Self::on_action_copy_url))
            .on_action(cx.listener(Self::on_action_copy_password))
            .on_action(cx.listener(Self::on_action_open_connect))
            .on_action(cx.listener(Self::on_action_open_sync_settings))
            .on_action(cx.listener(Self::on_action_new_entry))
            .on_action(cx.listener(Self::on_action_open_conflict_demo))
            .on_action(cx.listener(Self::on_action_create_vault))
            .on_action(cx.listener(Self::on_action_toggle_theme))
            .on_action(cx.listener(Self::on_action_save_vault))
            .on_action(cx.listener(Self::on_action_edit_entry))
            .on_action(cx.listener(Self::on_action_delete_entry))
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(body)
    }
}

fn is_kdbx_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("kdbx"))
}

fn copy_value_label(kind: CopyValueKind) -> &'static str {
    match kind {
        CopyValueKind::Username => "Username",
        CopyValueKind::Url => "URL",
        CopyValueKind::Password => "Password",
    }
}

#[allow(dead_code)]
fn _root_marker(_: &Root) {}
