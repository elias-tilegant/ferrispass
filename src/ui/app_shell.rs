use crate::{
    app::{
        AppSettings, AppState, CopyValueKind, Overlay,
        actions::{
            APP_CONTEXT, CancelUnlock, CopyPassword, CopyUrl, CopyUsername, CreateVault,
            DeleteEntry, EditEntry, SaveVault, ToggleTheme,
            FocusSearch, LockVault, NewEntry, OpenConflictDemo, OpenConnect, OpenSettings,
            OpenSyncSettings, OpenVault, SubmitPassword,
        },
    },
    keepass::KeePassRepository,
};

/// Which section of the unified Settings overlay is currently active.
/// Lives in AppShell because it's UI-local state — not worth persisting,
/// reset to General whenever the overlay closes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsTab {
    General,
    Sync,
}
use gpui::{
    AppContext as _, ClickEvent, ClipboardItem, Context, Entity, FocusHandle, Focusable,
    InteractiveElement as _, ParentElement as _, PathPromptOptions, Render, ScrollStrategy,
    Styled as _, Subscription, Task, Window, div, px,
};
use std::time::{Duration, Instant};
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
    /// Live filter for the Connect overlay's "Pick a file" step. We
    /// subscribe to `Change` events and forward the value to AppState
    /// (which keeps the filtered list in `connect_flow.Picking.query`).
    picker_query_input: Entity<InputState>,
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
    /// Background timer that wipes the clipboard `CLIPBOARD_CLEAR_SECS` after
    /// a copy. Holding the `Task` here means a new copy replaces (= cancels)
    /// the previous timer — only the latest copy's clear fires.
    clipboard_clear_task: Option<Task<()>>,
    /// Last value we wrote to the clipboard. Compared against the live
    /// clipboard contents before clearing — protects user-pasted content
    /// (e.g. an unrelated string they copied from another app in the
    /// meantime) from being clobbered by our timer.
    last_clipboard_value: Option<String>,
    /// Wall-clock deadline at which the pending clipboard-clear fires.
    /// Drives the countdown pill in the bottom-right corner. `None`
    /// when no clear is scheduled (no active copy, or settings have
    /// `clipboard_clear_secs = None`).
    clipboard_clear_deadline: Option<Instant>,
    /// 1 Hz tick that re-renders the countdown pill while a clear is
    /// pending. Dropped when the deadline lapses or the user copies
    /// something else.
    clipboard_pill_tick: Option<Task<()>>,
    /// Id of the entry whose password is currently revealed in the
    /// detail panel. `None` = masked. Compared against the live selection
    /// in the state observer so switching entries (or locking) auto-masks.
    revealed_entry_id: Option<String>,
    /// Wall-clock timestamp of the last user input event (mouse-move,
    /// click, key-down). Updated cheaply on every event without
    /// triggering a re-render — only the auto-lock checker reads it.
    last_activity: Instant,
    /// Periodic checker that locks the vault after the configured
    /// auto-lock timeout. Only running while a vault is open AND a
    /// non-`None` timeout is configured (managed alongside `totp_tick`
    /// via the state observer). Drop = cancel.
    auto_lock_task: Option<Task<()>>,
    /// User-tunable timeouts (auto-lock, clipboard-clear). Loaded from
    /// `~/Library/Application Support/stc-keepass/settings.json` at
    /// construction; mutated by the Settings overlay; persisted async
    /// on every change.
    settings: AppSettings,
    /// Selected tab inside the unified Settings overlay. Reset to
    /// General when the overlay opens via ⌘,, jumped to Sync when
    /// opened via ⌘⇧, or any "Sync settings"-style button.
    settings_tab: SettingsTab,
    _subscriptions: Vec<Subscription>,
}

/// How often the auto-lock checker wakes to compare `last_activity`
/// against the configured timeout. Coarser than the typical timeout
/// so the lock always fires within this window of crossing the
/// threshold.
const AUTO_LOCK_TICK_SECS: u64 = 5;

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
        let picker_query_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Filter by name or folder…")
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
            cx.observe(&state, |shell: &mut AppShell, state, cx| {
                // Re-render whenever AppState notifies AND keep the
                // background tasks (TOTP tick + auto-lock checker) in
                // sync with whether a vault is currently open.
                shell.sync_totp_tick(cx);
                shell.sync_auto_lock_task(cx);
                // Auto-mask: if the user moves to a different entry (or
                // locks the vault), drop any reveal we held. Compared by
                // id so re-renders against the same entry don't trigger.
                let current_id = state
                    .read(cx)
                    .vault_browser()
                    .and_then(|b| b.selected_entry)
                    .map(|e| e.id);
                if shell.revealed_entry_id.is_some()
                    && shell.revealed_entry_id != current_id
                {
                    shell.revealed_entry_id = None;
                }
                cx.notify();
            }),
            cx.subscribe_in(&password_input, window, Self::on_password_input_event),
            cx.subscribe_in(&keyfile_input, window, Self::on_keyfile_input_event),
            cx.subscribe_in(&search_input, window, Self::on_search_input_event),
            cx.subscribe_in(&picker_query_input, window, Self::on_picker_query_event),
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
            picker_query_input,
            gen_length_state,
            gen_classes: crate::keepass::password_gen::CharClasses::default(),
            entry_list_scroll: VirtualListScrollHandle::new(),
            focus_handle,
            search_debounce: None,
            pending_perma_delete: None,
            totp_tick: None,
            clipboard_clear_task: None,
            last_clipboard_value: None,
            clipboard_clear_deadline: None,
            clipboard_pill_tick: None,
            revealed_entry_id: None,
            last_activity: Instant::now(),
            auto_lock_task: None,
            settings: crate::app::settings::load(),
            settings_tab: SettingsTab::General,
            _subscriptions,
        }
    }

    pub fn settings_tab(&self) -> SettingsTab {
        self.settings_tab
    }

    pub fn set_settings_tab(&mut self, tab: SettingsTab, cx: &mut Context<Self>) {
        if self.settings_tab != tab {
            self.settings_tab = tab;
            cx.notify();
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

    /// Mirror of `sync_totp_tick` for the idle-timeout auto-lock
    /// checker. Runs only while a vault is open AND `auto_lock_secs`
    /// is configured; cancelled (= dropped) otherwise. The task wakes
    /// every `AUTO_LOCK_TICK_SECS` and triggers a lock once
    /// `last_activity` has been silent for the configured threshold.
    /// Reads the threshold inside the loop so settings changes apply
    /// at the next tick without restarting the task.
    fn sync_auto_lock_task(&mut self, cx: &mut Context<Self>) {
        let vault_open = matches!(
            self.state.read(cx).vault_status(),
            crate::app::VaultStatus::Open { .. }
        );
        let auto_lock_enabled = self.settings.auto_lock_secs.is_some();
        let should_run = vault_open && auto_lock_enabled;

        match (should_run, self.auto_lock_task.is_some()) {
            (true, false) => {
                // Reset the activity baseline at vault-open so a stale
                // timestamp from earlier in the session doesn't trip an
                // immediate lock.
                self.last_activity = Instant::now();
                self.auto_lock_task = Some(cx.spawn(async move |this, cx| {
                    loop {
                        cx.background_executor()
                            .timer(Duration::from_secs(AUTO_LOCK_TICK_SECS))
                            .await;
                        let triggered = this.update(cx, |shell, cx| {
                            // Re-read on each tick so toggling the
                            // setting takes effect within ~5 s.
                            let Some(threshold) = shell.settings.auto_lock_secs else {
                                return true; // settings disabled — exit.
                            };
                            if shell.last_activity.elapsed()
                                >= Duration::from_secs(threshold)
                            {
                                shell.auto_lock_now(cx);
                                true
                            } else {
                                false
                            }
                        });
                        match triggered {
                            Ok(true) | Err(_) => break,
                            Ok(false) => continue,
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

    /// Lock the vault from a non-foreground context (idle timeout).
    /// Wipes shell-side secrets and delegates to AppState. Skips
    /// clearing the password / search input fields because we don't
    /// have a `Window` here — they get reset by `select_vault_path`
    /// whenever the user picks a vault to re-open.
    fn auto_lock_now(&mut self, cx: &mut Context<Self>) {
        self.wipe_session_secrets(cx);
        self.state.update(cx, |state, cx| state.lock_vault(cx));
    }

    /// Drop reveal flag, cancel any pending clipboard-clear timer, and
    /// flush the OS clipboard *now* if it still holds the value we
    /// last wrote. Shared between auto-lock and manual lock so
    /// dropping the timer doesn't leave a copied password sitting on
    /// the clipboard until the next copy.
    fn wipe_session_secrets(&mut self, cx: &mut Context<Self>) {
        self.revealed_entry_id = None;
        if self.last_clipboard_value.is_some() {
            let still_ours = cx
                .read_from_clipboard()
                .and_then(|item| item.text())
                .as_deref()
                == self.last_clipboard_value.as_deref();
            if still_ours {
                cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
            }
        }
        self.last_clipboard_value = None;
        self.clipboard_clear_task = None;
        self.clipboard_clear_deadline = None;
        self.clipboard_pill_tick = None;
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

    pub fn picker_query_input(&self) -> &Entity<InputState> {
        &self.picker_query_input
    }

    /// Open a save-file picker prefilled with the picked file's name, then
    /// trigger the download once the user confirms. If they cancel, leave
    /// the connect flow in the Picking step so they can pick something else.
    pub fn start_pick_kdbx_file(
        &mut self,
        hit: crate::sync::graph::DriveItemHit,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let suggested_name = hit.name.clone();
        let initial_dir = std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join("Documents"))
            .unwrap_or_else(|| PathBuf::from("."));
        let picker = cx.prompt_for_new_path(&initial_dir, Some(&suggested_name));

        let state = self.state.clone();
        cx.spawn(async move |_, cx| {
            let Ok(Ok(Some(path))) = picker.await else {
                return;
            };
            let _ = state.update(cx, |state, cx| {
                state.pick_kdbx_file(hit, path, cx);
            });
        })
        .detach();
    }

    /// Open a URL in the user's default browser. Used by the device-code
    /// step's "Open in browser" button — opens https://microsoft.com/devicelogin
    /// (or whatever the server gave us) so the user doesn't have to copy/paste.
    pub fn open_browser(url: &str) {
        // macOS only for now (matches the rest of the MVP scope). On other
        // platforms this is a no-op; the user can copy the URL manually.
        #[cfg(target_os = "macos")]
        {
            let _ = std::process::Command::new("open").arg(url).spawn();
        }
        #[cfg(not(target_os = "macos"))]
        let _ = url;
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
        self.state.update(cx, |state, cx| {
            state.open_overlay(Overlay::Connect, cx);
            state.begin_connect_flow(cx);
        });
    }

    fn on_action_open_settings(
        &mut self,
        _: &OpenSettings,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Universally available — no vault-open gate.
        self.settings_tab = SettingsTab::General;
        self.state
            .update(cx, |state, cx| state.open_overlay(Overlay::Settings, cx));
    }

    fn on_action_open_sync_settings(
        &mut self,
        _: &OpenSyncSettings,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Same Settings overlay as ⌘, but jumps directly to the Sync
        // tab. Reachable from the vault-header sync chip and ⌘⇧, so
        // users can land where they were going without a tab click.
        self.settings_tab = SettingsTab::Sync;
        self.state
            .update(cx, |state, cx| state.open_overlay(Overlay::Settings, cx));
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

    fn on_picker_query_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Change) {
            // Filter is purely client-side over the already-fetched list, so
            // no debounce — every keystroke updates instantly without an
            // API call.
            let q = input.read(cx).value().to_string();
            self.state
                .update(cx, |state, cx| state.set_picker_query(q, cx));
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

    /// Open a vault from the Welcome screen's Recents list. Same effect
    /// as picking it through the file dialog — clears the unlock inputs
    /// and lands the user on the password prompt for `path`. The path
    /// has already been validated as a .kdbx by virtue of having been
    /// successfully opened before, so we go straight through
    /// `select_vault_path`.
    pub fn open_recent(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.select_vault_path(path, window, cx);
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
        // Same secret-wipe as auto-lock: drop reveal, cancel pending
        // clipboard timer, flush clipboard if it still holds our copy.
        self.wipe_session_secrets(cx);
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
            self.copy_with_auto_clear(value, copy_value_label(kind), window, cx);
        } else {
            window.push_notification(format!("No {} to copy.", copy_value_label(kind)), cx);
        }
    }

    /// Single source of truth for "put this on the clipboard, tell the
    /// user, schedule a clear". Used by the saved-fields copy path
    /// (`copy_selected_value`) and the live-TOTP copy in the detail
    /// panel. Replaces any in-flight clear so the latest copy always
    /// wins — older timers would otherwise wipe the new value early.
    /// When `clipboard_clear_secs` is `None` (user picked "Never"), no
    /// timer is scheduled; the clipboard still gets wiped at lock time.
    pub fn copy_with_auto_clear(
        &mut self,
        value: String,
        label: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.write_to_clipboard(ClipboardItem::new_string(value.clone()));
        let toast = match self.settings.clipboard_clear_secs {
            Some(secs) => format!("{label} copied. Clipboard clears in {secs} s."),
            None => format!("{label} copied."),
        };
        window.push_notification(toast, cx);
        self.schedule_clipboard_clear(value, cx);
    }

    fn schedule_clipboard_clear(&mut self, value: String, cx: &mut Context<Self>) {
        self.last_clipboard_value = Some(value);
        // Drop any prior timer by replacing the slot. GPUI tasks cancel
        // on drop (same pattern as `totp_tick`).
        let Some(secs) = self.settings.clipboard_clear_secs else {
            // "Never": don't spawn a timer or pill. We still record
            // the value so the lock-time wipe can compare-then-clear.
            self.clipboard_clear_task = None;
            self.clipboard_clear_deadline = None;
            self.clipboard_pill_tick = None;
            return;
        };
        // Track the deadline so the countdown pill can render
        // `clears in N s…`. Re-rendering at 1 Hz comes from
        // `clipboard_pill_tick` below.
        self.clipboard_clear_deadline =
            Some(Instant::now() + Duration::from_secs(secs));
        self.clipboard_pill_tick = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(1))
                    .await;
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
                // Only clear when the clipboard still holds *our* value.
                // If the user copied something else in between, leave
                // their content alone.
                let still_ours = cx
                    .read_from_clipboard()
                    .and_then(|item| item.text())
                    .as_deref()
                    == shell.last_clipboard_value.as_deref();
                if still_ours {
                    cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
                }
                shell.last_clipboard_value = None;
                shell.clipboard_clear_task = None;
                shell.clipboard_clear_deadline = None;
                shell.clipboard_pill_tick = None;
                cx.notify();
            });
        }));
    }

    /// True iff `entry_id`'s password is currently revealed in the
    /// detail panel. Driven by `revealed_entry_id` — see the state
    /// observer for the auto-mask-on-switch logic.
    pub fn is_password_revealed(&self, entry_id: &str) -> bool {
        self.revealed_entry_id.as_deref() == Some(entry_id)
    }

    /// Toggle the masked password into / out of view for `entry_id`.
    /// Idempotent — clicking twice ends up where you started.
    pub fn toggle_password_reveal(
        &mut self,
        entry_id: String,
        cx: &mut Context<Self>,
    ) {
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
    /// (fire-and-forget — failures are non-fatal, the in-memory
    /// settings still apply for this session) and re-runs
    /// `sync_auto_lock_task` so a freshly-disabled timer is cancelled
    /// or a freshly-enabled one starts immediately.
    pub fn update_settings(
        &mut self,
        new_settings: AppSettings,
        cx: &mut Context<Self>,
    ) {
        if self.settings == new_settings {
            return;
        }
        self.settings = new_settings.clone();
        // If the user just toggled auto-lock on/off, we need to start
        // or cancel the checker task immediately rather than waiting
        // for the next state notification.
        self.sync_auto_lock_task(cx);
        cx.background_spawn(async move {
            let _ = crate::app::settings::save(&new_settings);
        })
        .detach();
        cx.notify();
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

        // Settings is a global overlay — accessible regardless of
        // whether a vault is open (matches the Mac ⌘, convention of
        // Preferences always being reachable).
        if matches!(overlay, Overlay::Settings) {
            return crate::ui::screens::settings::render(self, cx);
        }

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

impl AppShell {
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
            div()
                .absolute()
                .bottom_4()
                .right_4()
                .child(
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let body = AppShell::render_body(self, cx);
        // Without this layer the `Root::notification` `NotificationList`
        // never gets painted — `window.push_notification(...)` queues
        // toasts into a list that nothing renders, so the user sees
        // nothing despite the call site looking correct. Mirrors the
        // gpui-component story-shell pattern.
        let notification_layer = Root::render_notification_layer(window, cx);
        let clipboard_pill = self.render_clipboard_pill();

        div()
            .key_context(APP_CONTEXT)
            .track_focus(&self.focus_handle)
            // Reset the idle clock on any user input. Setting an
            // `Instant` is cheap and we deliberately don't `cx.notify`
            // here — only the auto-lock checker reads the field, and
            // it does so on its own schedule.
            .on_mouse_move(cx.listener(|shell: &mut AppShell, _, _, _| {
                shell.last_activity = Instant::now();
            }))
            .on_key_down(cx.listener(|shell: &mut AppShell, _, _, _| {
                shell.last_activity = Instant::now();
            }))
            .on_action(cx.listener(Self::on_action_open_vault))
            .on_action(cx.listener(Self::on_action_submit_password))
            .on_action(cx.listener(Self::on_action_cancel_unlock))
            .on_action(cx.listener(Self::on_action_lock_vault))
            .on_action(cx.listener(Self::on_action_focus_search))
            .on_action(cx.listener(Self::on_action_copy_username))
            .on_action(cx.listener(Self::on_action_copy_url))
            .on_action(cx.listener(Self::on_action_copy_password))
            .on_action(cx.listener(Self::on_action_open_connect))
            .on_action(cx.listener(Self::on_action_open_settings))
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
            .children(clipboard_pill)
            .children(notification_layer)
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
