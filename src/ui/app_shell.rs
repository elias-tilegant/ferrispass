use crate::{
    app::{
        AppSettings, AppState, CopyValueKind, Overlay,
        actions::{
            APP_CONTEXT, CancelUnlock, CopyPassword, CopyUrl, CopyUsername, CreateVault,
            DeleteEntry, DeleteGroup, DownloadFavicons, EditEntry, FocusSearch, InstallUpdate,
            LaunchEntry, LockVault, NewEntry, NewGroup, NewSubgroup, OpenConflictDemo, OpenConnect,
            OpenSettings, OpenSyncSettings, OpenVault, OpenVaultSwitcher, OpenWhatsNew,
            PerformAutoType, PerformAutoTypeForSelected, RenameGroupOp, SaveVault, SubmitPassword,
            SyncNow, ToggleTheme,
        },
    },
    autotype,
    keepass::KeePassRepository,
    launch::{self, LaunchContext, LaunchError, LaunchHandle},
};

/// Which section of the unified Settings overlay is currently active.
/// Lives in AppShell because it's UI-local state — not worth persisting,
/// reset to General whenever the overlay closes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsTab {
    General,
    Sync,
    AutoType,
}
use gpui::{
    AnyWindowHandle, AppContext as _, ClickEvent, ClipboardItem, Context, Entity, FocusHandle,
    Focusable, InteractiveElement as _, ParentElement as _, PathPromptOptions, Render,
    ScrollStrategy, SharedString, Styled as _, Subscription, Task, Window, div, px,
};
use gpui_component::{
    ActiveTheme as _, Root, VirtualListScrollHandle, WindowExt as _,
    input::{InputEvent, InputState},
    slider::{SliderState, SliderValue},
};
use std::path::PathBuf;
use std::time::{Duration, Instant};

struct EditPrefill {
    id: String,
    title: String,
    username: String,
    url: String,
    notes: String,
    password: String,
    otp: String,
    custom_fields: Vec<crate::domain::CustomField>,
}

/// One row in the AddEntry / EditEntry modal's "Additional fields"
/// section. Each row owns its own pair of `InputState` entities so
/// the gpui input stack can track focus / cursor position
/// independently — sharing them across rows would have selection
/// jumping when the user reorders or deletes a row.
pub struct CustomFieldDraftInputs {
    /// Stable id used as the gpui element key. Monotonic counter via
    /// `AppShell::next_custom_field_id` — never reused, so deleting
    /// row 3 and adding a new one doesn't accidentally restore
    /// stale gpui state for the old row.
    pub id: usize,
    pub key_input: Entity<InputState>,
    pub value_input: Entity<InputState>,
    pub protected: bool,
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
    /// Single text input shared by the Add-group and Rename-group
    /// overlays. Prefilled with the current name for Rename; cleared on
    /// open for Add. Only one of those overlays is active at a time, so
    /// reusing one input keeps `AppShell` from accumulating identical
    /// fields.
    new_group_name_input: Entity<InputState>,
    /// Live filter for the Connect overlay's "Pick a file" step. We
    /// subscribe to `Change` events and forward the value to AppState
    /// (which keeps the filtered list in `connect_flow.Picking.query`).
    picker_query_input: Entity<InputState>,
    /// Filter input for the vault-switcher overlay (`Overlay::VaultSwitcher`).
    /// Cleared on every overlay-open so the picker always starts empty;
    /// Enter on the input opens whichever recent currently floats to the top.
    vault_switcher_input: Entity<InputState>,
    /// Free-form text input for the Auto-Type sequence template. Lives
    /// here (rather than being recreated per render) so the focus,
    /// cursor position, and undo history persist across Settings
    /// re-renders.
    auto_type_sequence_input: Entity<InputState>,
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
    /// `~/Library/Application Support/ferrispass/settings.json` at
    /// construction; mutated by the Settings overlay; persisted async
    /// on every change.
    settings: AppSettings,
    /// Selected tab inside the unified Settings overlay. Reset to
    /// General when the overlay opens via ⌘,, jumped to Sync when
    /// opened via ⌘⇧, or any "Sync settings"-style button.
    settings_tab: SettingsTab,
    /// Optional user-chosen target group for the AddEntry overlay. When
    /// `Some`, overrides the auto-derived "currently-selected group"
    /// fallback in `add_entry::render`. Cleared on overlay close so a
    /// fresh ⌘N starts from the user's current sidebar selection again.
    new_entry_target_group_id: Option<String>,
    /// Whether the inline group picker inside the AddEntry modal is
    /// currently expanded. Independent of `new_entry_target_group_id`
    /// so the user can flip it open without committing a change.
    new_entry_picker_open: bool,
    /// Dynamic "Additional fields" rows in the AddEntry / EditEntry
    /// modal. Reset by `clear_entry_form`, populated by
    /// `prefill_edit_form`. Saved into `EntryDraft.custom_fields`.
    new_entry_custom_fields: Vec<CustomFieldDraftInputs>,
    /// Monotonic source for `CustomFieldDraftInputs.id`. Wraps to 0
    /// on overflow, but at one increment per editor row that's never
    /// going to happen in practice.
    next_custom_field_id: usize,
    /// Live launches whose temp file is still on disk. Each handle owns
    /// its file (drop = unlink). Capped by paired entries in
    /// `launch_cleanup_tasks`, which drop the head of this Vec after
    /// the user-configured TTL fires.
    pending_launches: Vec<LaunchHandle>,
    /// One-shot timers, one per pending launch. Holding the `Task`
    /// keeps the timer alive; dropping it cancels (used when lock/quit
    /// purges everything early).
    launch_cleanup_tasks: Vec<Task<()>>,
    /// Active global-hotkey registration for auto-type. `Some` only
    /// when `settings.auto_type_enabled && settings.auto_type_hotkey`
    /// parses & registers cleanly; dropped (= unregistered) when the
    /// user toggles the feature off or changes the combo. The
    /// matching poll loop in `auto_type_poll_task` reads the registered
    /// id to filter events.
    auto_type_listener: Option<autotype::HotkeyListener>,
    /// Background task that polls the global-hotkey event channel at
    /// ~30 Hz and dispatches `PerformAutoType` when our combo fires.
    /// Dropped together with the listener — leaving it running with no
    /// listener would still be safe (the poll would find no matching
    /// events) but would burn cycles for no reason.
    auto_type_poll_task: Option<Task<()>>,
    /// Most recent parse-error from the user's auto-type sequence, or
    /// `None` when the template is valid. Cached so the Settings UI
    /// can surface it without re-parsing on every render. Cleared
    /// when `update_settings` accepts a new template.
    auto_type_sequence_error: Option<autotype::ParseError>,
    /// Most recent registration / parse error from the user's auto-
    /// type hotkey combo. Same caching rationale as
    /// `auto_type_sequence_error`. The Settings UI uses this to
    /// surface a clear "this combo is in use by another app" hint
    /// rather than letting the feature silently appear broken.
    auto_type_hotkey_error: Option<String>,
    /// Handle to the window that hosts this shell. Captured at
    /// construction so the global-hotkey poll task can dispatch
    /// `perform_auto_type` directly into this window's context — bypassing
    /// `App::dispatch_action`, which routes through `active_window()` and
    /// no-ops when FerrisPass isn't the OS-focused app (exactly the
    /// case a global hotkey is for).
    window_handle: AnyWindowHandle,
    _subscriptions: Vec<Subscription>,
}

/// How often the auto-lock checker wakes to compare `last_activity`
/// against the configured timeout. Coarser than the typical timeout
/// so the lock always fires within this window of crossing the
/// threshold.
const AUTO_LOCK_TICK_SECS: u64 = 5;

/// Polling interval for the global-hotkey event channel. ~30 Hz —
/// fast enough that the user can't perceive any lag between the
/// hotkey press and the auto-type firing, slow enough that the
/// idle-app cost is negligible (the loop is two channel `try_recv`
/// calls plus a sleep).
const AUTO_TYPE_POLL_INTERVAL: Duration = Duration::from_millis(33);

/// Countdown for the in-app ⌘⇧T (PerformAutoTypeForSelected) route.
/// Mirrors KeePass's classic "type-in-3-seconds" pattern: the user
/// presses the shortcut from inside FerrisPass, switches to the
/// target window before the countdown ends, and we type into
/// whatever has focus then. 3 s is the KeePass default and feels
/// right in practice.
const AUTO_TYPE_COUNTDOWN_SECS: u64 = 3;

impl AppShell {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let password_input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Master password")
        });
        let keyfile_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Optional key file path"));
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
        let new_entry_otp_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("otpauth://… or base32 secret"));
        let new_group_name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Group name"));
        let picker_query_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter by name or folder…"));
        let vault_switcher_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Switch vault — type to filter…"));
        // Seed the sequence input with whatever's currently persisted —
        // settings::load runs above in the field initializer below.
        // We re-read here so the input's display value matches the
        // canonical settings value from disk.
        let seq_seed = crate::app::settings::load().auto_type_sequence;
        let auto_type_sequence_input = cx.new(|cx| {
            let mut state =
                InputState::new(window, cx).placeholder("{USERNAME}{TAB}{PASSWORD}{ENTER}");
            state.set_value(&seq_seed, window, cx);
            state
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
                if shell.revealed_entry_id.is_some() && shell.revealed_entry_id != current_id {
                    shell.revealed_entry_id = None;
                }
                cx.notify();
            }),
            cx.subscribe_in(&password_input, window, Self::on_password_input_event),
            cx.subscribe_in(&keyfile_input, window, Self::on_keyfile_input_event),
            cx.subscribe_in(&search_input, window, Self::on_search_input_event),
            cx.subscribe_in(&picker_query_input, window, Self::on_picker_query_event),
            cx.subscribe_in(
                &vault_switcher_input,
                window,
                Self::on_vault_switcher_input_event,
            ),
            cx.subscribe_in(
                &auto_type_sequence_input,
                window,
                Self::on_auto_type_sequence_input_event,
            ),
            cx.subscribe_in(&new_group_name_input, window, Self::on_new_group_name_event),
            // Re-render on slider drag so the "Length: N" label and the
            // strength preview update live alongside the thumb.
            cx.observe(&gen_length_state, |_shell: &mut AppShell, _, cx| {
                cx.notify();
            }),
        ];

        // Sweep launch payloads orphaned by a previous crash. We only
        // touch files older than 60 s — short enough that a near-miss
        // (us starting the moment a previous instance dropped a
        // payload) doesn't kill it, long enough that anything from a
        // previous session that didn't shut down cleanly is gone.
        crate::launch::sweeper::sweep_stale(std::time::Duration::from_secs(60));

        let mut shell = Self {
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
            new_group_name_input,
            picker_query_input,
            vault_switcher_input,
            auto_type_sequence_input,
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
            new_entry_target_group_id: None,
            new_entry_picker_open: false,
            new_entry_custom_fields: Vec::new(),
            next_custom_field_id: 0,
            pending_launches: Vec::new(),
            launch_cleanup_tasks: Vec::new(),
            auto_type_listener: None,
            auto_type_poll_task: None,
            auto_type_sequence_error: None,
            auto_type_hotkey_error: None,
            window_handle: window.window_handle(),
            _subscriptions,
        };
        // Bring up the global-hotkey registration immediately if the
        // user has Auto-Type enabled. Done after Self is built (rather
        // than starting the task during the struct literal) so the
        // weak handle that `cx.spawn` captures resolves to the
        // already-constructed entity. Idempotent if Auto-Type is off
        // — `sync_auto_type_listener` is the only entry point and it
        // no-ops cleanly when both desired-state and current-state are
        // "no listener".
        shell.sync_auto_type_listener(cx);
        shell
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
        // "Any vault in memory" — keeps the timer ticking when the active
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
                            if shell.last_activity.elapsed() >= Duration::from_secs(threshold) {
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

    /// Reconcile the global-hotkey listener with the current settings.
    /// Called from `new` (initial wire-up) and `update_settings`
    /// (toggle / combo change). Idempotent on the no-change path.
    ///
    /// Failure modes are user-actionable, so we cache the error in
    /// `auto_type_hotkey_error` for the Settings UI to display instead
    /// of silently leaving the feature off — that's exactly the
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
                        // directly — `App::dispatch_action` would route
                        // through `active_window()`, which is `None`
                        // while another app holds OS focus, and would
                        // then fall back to `dispatch_global_action`
                        // (skipping our element-tree handler entirely).
                        // Going through the window handle gives us a
                        // real `&mut Window` regardless of focus.
                        let entered = window_handle.update(cx, |_root, window, app| {
                            this.update(app, |shell, cx| {
                                shell.perform_auto_type(None, window, cx);
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

    /// Trigger the system prompt that opens the Privacy → Accessibility
    /// pane. Called from the "Grant access" button in Settings. The
    /// return value isn't actionable here — even on grant, the macOS
    /// trust bit only refreshes for new processes, so the user must
    /// restart FerrisPass after granting.
    pub fn auto_type_request_trust(&self) {
        let _ = autotype::permissions::request_trust();
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
        // Drop any pending launch payloads — each handle's Drop unlinks
        // its file. Cancel the cleanup timers since we just did the
        // cleanup ourselves. Then purge the launch tempdir for good
        // measure: covers anything raced in between by another instance
        // or anything our own Drop missed.
        self.pending_launches.clear();
        self.launch_cleanup_tasks.clear();
        crate::launch::sweeper::purge_all();
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

    pub fn new_group_name_input(&self) -> &Entity<InputState> {
        &self.new_group_name_input
    }

    pub fn picker_query_input(&self) -> &Entity<InputState> {
        &self.picker_query_input
    }

    pub fn vault_switcher_input(&self) -> &Entity<InputState> {
        &self.vault_switcher_input
    }

    pub fn auto_type_sequence_input(&self) -> &Entity<InputState> {
        &self.auto_type_sequence_input
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
        let custom_fields = self
            .new_entry_custom_fields
            .iter()
            .map(|row| crate::domain::CustomField {
                key: row.key_input.read(cx).value().to_string(),
                value: row.value_input.read(cx).value().to_string(),
                protected: row.protected,
            })
            // Empty-key rows are treated as "row the user added but
            // never filled" — silently dropped on save rather than
            // polluting the database with `""`-keyed attributes.
            .filter(|f| !f.key.trim().is_empty())
            .collect();
        crate::keepass::EntryDraft {
            title: self.new_entry_title_input.read(cx).value().to_string(),
            username: self.new_entry_username_input.read(cx).value().to_string(),
            password: self.new_entry_password_input.read(cx).value().to_string(),
            url: self.new_entry_url_input.read(cx).value().to_string(),
            notes: self.new_entry_notes_input.read(cx).value().to_string(),
            tags: Vec::new(),
            otp: self.new_entry_otp_input.read(cx).value().to_string(),
            custom_fields,
        }
    }

    /// Read-only access to the live editor rows, used by the modal's
    /// render code to lay out the inputs.
    pub fn new_entry_custom_fields(&self) -> &[CustomFieldDraftInputs] {
        &self.new_entry_custom_fields
    }

    /// Append a fresh empty row at the bottom of the editor. Returns
    /// the new row's id so the click handler can focus it (future).
    pub fn add_custom_field_row(&mut self, window: &mut Window, cx: &mut Context<Self>) -> usize {
        let id = self.next_custom_field_id;
        self.next_custom_field_id = self.next_custom_field_id.wrapping_add(1);
        let key_input = cx.new(|cx| InputState::new(window, cx).placeholder("Key"));
        let value_input = cx.new(|cx| InputState::new(window, cx).placeholder("Value"));
        self.new_entry_custom_fields.push(CustomFieldDraftInputs {
            id,
            key_input,
            value_input,
            protected: false,
        });
        cx.notify();
        id
    }

    /// Quick-add: drop in the canonical SAP-connection rows in one
    /// click. Keys are pre-filled (`SAP_HOST`, `SAP_INSTANCE`, …) and
    /// each value input gets a placeholder hint so the user knows
    /// what to type. Skips any row whose key already exists in the
    /// editor — clicking the button twice is idempotent rather than
    /// producing duplicate rows.
    pub fn add_sap_connection_template(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for (key, value_placeholder) in crate::launch::sap::QUICK_ADD_KEYS {
            // Idempotency: don't stack multiple SAP_HOST rows on
            // repeat clicks. Match against any existing row regardless
            // of whether the user has typed into it yet.
            let already_present = self
                .new_entry_custom_fields
                .iter()
                .any(|row| row.key_input.read(cx).value().to_string() == *key);
            if already_present {
                continue;
            }
            let id = self.next_custom_field_id;
            self.next_custom_field_id = self.next_custom_field_id.wrapping_add(1);
            let placeholder: SharedString = (*value_placeholder).into();
            let key_input = cx.new(|cx| InputState::new(window, cx).placeholder("Key"));
            let value_input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
            // Pre-fill the key so the row is functional immediately;
            // the value stays empty for the user to fill.
            key_input.update(cx, |s, cx| s.set_value(*key, window, cx));
            self.new_entry_custom_fields.push(CustomFieldDraftInputs {
                id,
                key_input,
                value_input,
                protected: false,
            });
        }
        cx.notify();
    }

    /// Remove a row by its stable id (the trash button on each row).
    pub fn remove_custom_field_row(&mut self, id: usize, cx: &mut Context<Self>) {
        self.new_entry_custom_fields.retain(|row| row.id != id);
        cx.notify();
    }

    /// Flip the `protected` flag on a row. Used by the lock/unlock
    /// toggle button in each editor row.
    pub fn toggle_custom_field_protected(&mut self, id: usize, cx: &mut Context<Self>) {
        for row in &mut self.new_entry_custom_fields {
            if row.id == id {
                row.protected = !row.protected;
                cx.notify();
                break;
            }
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
        // Drop any explicit target-group pick so the next open re-derives
        // from the current sidebar selection, and snap the picker shut.
        self.new_entry_target_group_id = None;
        self.new_entry_picker_open = false;
        // Drop all custom-field rows. Their `Entity<InputState>`s are
        // released on Vec drop; gpui will GC them once no view still
        // references them. Don't reset `next_custom_field_id` — keeping
        // it monotonic across opens guarantees stable element keys
        // even when the user rapidly opens/closes the modal.
        self.new_entry_custom_fields.clear();
    }

    /// User's explicit target-group pick for the AddEntry modal, if any.
    /// `None` means "follow the sidebar selection" (the fallback handled
    /// in `add_entry::render`).
    pub fn new_entry_target_group_id(&self) -> Option<&str> {
        self.new_entry_target_group_id.as_deref()
    }

    pub fn set_new_entry_target_group(
        &mut self,
        group_id: impl Into<String>,
        cx: &mut Context<Self>,
    ) {
        self.new_entry_target_group_id = Some(group_id.into());
        self.new_entry_picker_open = false;
        cx.notify();
    }

    pub fn new_entry_picker_open(&self) -> bool {
        self.new_entry_picker_open
    }

    pub fn toggle_new_entry_picker(&mut self, cx: &mut Context<Self>) {
        self.new_entry_picker_open = !self.new_entry_picker_open;
        cx.notify();
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

    fn on_action_open_vault_switcher(
        &mut self,
        _: &OpenVaultSwitcher,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    fn on_action_submit_password(
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
        let closed = self.state.update(cx, |state, cx| state.close_overlay(cx));
        if closed {
            return;
        }
        self.cancel_unlock(window, cx);
    }

    fn on_action_lock_vault(&mut self, _: &LockVault, window: &mut Window, cx: &mut Context<Self>) {
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

    fn on_action_launch_entry(
        &mut self,
        _: &LaunchEntry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.launch_selected_entry(window, cx);
    }

    /// Global-hotkey route. Reads the foreground window the user just
    /// left, finds the best-matching entry by URL hostname, and types
    /// the configured sequence into it. All preconditions surface as
    /// a toast notification — silent failure here would have the
    /// feature looking broken (the user pressed the hotkey, nothing
    /// happened, no clue why).
    fn on_action_perform_auto_type(
        &mut self,
        _: &PerformAutoType,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.perform_auto_type(None, window, cx);
    }

    /// In-app ⌘⇧T route: types the *currently-selected* entry after a
    /// short countdown. The countdown is what lets the user press the
    /// shortcut from inside FerrisPass and still aim the keystrokes
    /// at a different window — the global hotkey is the better
    /// ergonomic, but this is the discoverable in-app entry point.
    fn on_action_perform_auto_type_for_selected(
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
                "Auto-Type starting in {AUTO_TYPE_COUNTDOWN_SECS} s — switch to the target window."
            ),
            cx,
        );
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(AUTO_TYPE_COUNTDOWN_SECS))
                .await;
            let _ = this.update_in(cx, |shell, window, cx| {
                shell.perform_auto_type(Some(entry_id.clone()), window, cx);
            });
        })
        .detach();
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
    /// the same `Outcome` enum the orchestrator uses everywhere else
    /// — so `TypingFailed` actually reaches the user instead of being
    /// silently swallowed by a fire-and-forget spawn.
    fn perform_auto_type(
        &mut self,
        force_entry_id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(foreground) = autotype::window::foreground() else {
            window.push_notification(
                "Could not read the foreground window. Check Accessibility permission.",
                cx,
            );
            return;
        };

        // Build the orchestrator input under a single AppState read.
        // The closure captures `&VaultDocument` for the password
        // resolver — that borrow is alive only while `prepare` runs,
        // which is synchronous.
        let template = self.settings.auto_type_sequence.clone();
        let prepared: Result<autotype::TypePlan, autotype::Outcome> = {
            let state = self.state.read(cx);
            match state.vault_status() {
                crate::app::VaultStatus::Open { document, .. } => {
                    let snapshot = document.snapshot_rc();
                    autotype::prepare(autotype::PerformInput {
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

        // Mark recently-used now — the typing succeeds asynchronously,
        // but from the user's perspective they've authenticated with
        // this entry the moment they pressed the hotkey. (Background
        // task failure still surfaces via the notification.)
        self.state
            .update(cx, |state, _| state.mark_entry_used(&plan.entry_id));

        // Spawn the (blocking) typer on a background task. The plan
        // — and the cleartext password it carries inside its TypeOps
        // — is moved into the task and dropped when the task ends.
        let task = cx.background_spawn(async move { autotype::execute(plan) });
        let foreground_for_callback = foreground;
        cx.spawn_in(window, async move |this, cx| {
            let outcome = task.await;
            let _ = this.update_in(cx, |shell, window, cx| {
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
                let title = if window_title.is_empty() {
                    foreground.window_title.clone()
                } else {
                    window_title
                };
                window.push_notification(format!("No matching entry for \"{title}\"."), cx);
            }
            Outcome::NoPassword => {
                window.push_notification("The matched entry has no password set.", cx);
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
        }
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

    fn on_action_install_update(
        &mut self,
        _: &InstallUpdate,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.update(cx, |state, cx| state.install_update(cx));
    }

    fn on_action_open_whats_new(
        &mut self,
        _: &OpenWhatsNew,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.update(cx, |state, cx| state.open_whats_new(cx));
    }

    fn on_action_sync_now(&mut self, _: &SyncNow, window: &mut Window, cx: &mut Context<Self>) {
        use crate::app::SyncStatus;

        let status = self.state.read(cx).sync_status().clone();
        match status {
            // Nothing to sync against — fall back to the Sync settings tab
            // so the user can connect or re-authenticate.
            SyncStatus::Disconnected | SyncStatus::Reconnect => {
                window.dispatch_action(Box::new(OpenSyncSettings), cx);
            }
            // Already in flight or awaiting user input — do nothing.
            SyncStatus::Syncing
            | SyncStatus::Connecting
            | SyncStatus::Restoring
            | SyncStatus::Conflict(_) => {}
            SyncStatus::Idle | SyncStatus::Synced { .. } | SyncStatus::Failed(_) => {
                self.state.update(cx, |state, cx| state.sync_now(cx));
            }
        }
    }

    fn on_action_download_favicons(
        &mut self,
        _: &DownloadFavicons,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state
            .update(cx, |state, cx| state.start_favicon_download(cx));
    }

    fn on_action_new_entry(&mut self, _: &NewEntry, window: &mut Window, cx: &mut Context<Self>) {
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
        self.state.update(cx, |state, cx| state.save_async(cx));
    }

    fn on_action_edit_entry(&mut self, _: &EditEntry, window: &mut Window, cx: &mut Context<Self>) {
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

    fn on_action_new_group(&mut self, _: &NewGroup, window: &mut Window, cx: &mut Context<Self>) {
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

    fn on_action_new_subgroup(
        &mut self,
        action: &NewSubgroup,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.begin_add_group(action.parent_group_id.clone(), window, cx);
    }

    fn on_action_rename_group_op(
        &mut self,
        action: &RenameGroupOp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
    }

    fn on_action_delete_group(
        &mut self,
        action: &DeleteGroup,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let group_id = action.group_id.clone();
        let result = self
            .state
            .clone()
            .update(cx, |state, cx| state.delete_group(&group_id, cx));
        match result {
            Ok(()) => window.push_notification("Group moved to Trash.", cx),
            Err(crate::keepass::MutationError::CannotDeleteRoot) => {
                window.push_notification("The root group cannot be deleted.", cx)
            }
            Err(crate::keepass::MutationError::CannotDeleteRecycleBin) => {
                window.push_notification("The Recycle Bin cannot be deleted.", cx)
            }
            Err(e) => window.push_notification(format!("Could not delete group: {e}"), cx),
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

    fn on_new_group_name_event(
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

        // Rebuild the custom-fields editor rows from the entry. Each
        // row gets a fresh Entity<InputState> with the current value
        // pre-filled — necessary because `add_custom_field_row` only
        // creates blank rows.
        self.new_entry_custom_fields.clear();
        for cf in p.custom_fields {
            let id = self.next_custom_field_id;
            self.next_custom_field_id = self.next_custom_field_id.wrapping_add(1);
            let key_input = cx.new(|cx| InputState::new(window, cx).placeholder("Key"));
            let value_input = cx.new(|cx| InputState::new(window, cx).placeholder("Value"));
            key_input.update(cx, |s, cx| s.set_value(&cf.key, window, cx));
            value_input.update(cx, |s, cx| s.set_value(&cf.value, window, cx));
            self.new_entry_custom_fields.push(CustomFieldDraftInputs {
                id,
                key_input,
                value_input,
                protected: cf.protected,
            });
        }

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

    fn on_auto_type_sequence_input_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(event, InputEvent::Change) {
            return;
        }
        // Mirror the typed value back into the persisted settings. The
        // sync_auto_type_listener call inside `update_settings`
        // re-parses + re-registers the hotkey, which also refreshes the
        // `auto_type_sequence_error` cache for the inline error label.
        let new_value = self.auto_type_sequence_input.read(cx).value().to_string();
        if new_value == self.settings.auto_type_sequence {
            return;
        }
        let new_settings = AppSettings {
            auto_type_sequence: new_value,
            ..self.settings.clone()
        };
        self.update_settings(new_settings, cx);
    }

    /// Programmatic setter for the Auto-Type sequence — used by the
    /// preset chips in Settings. Updates both the input widget and the
    /// persisted settings so the chip click feels instantaneous.
    pub fn set_auto_type_sequence(
        &mut self,
        template: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.auto_type_sequence_input.read(cx).value().as_ref() == template {
            return;
        }
        self.auto_type_sequence_input.update(cx, |state, cx| {
            state.set_value(template, window, cx);
        });
        let new_settings = AppSettings {
            auto_type_sequence: template.to_string(),
            ..self.settings.clone()
        };
        self.update_settings(new_settings, cx);
    }

    fn on_vault_switcher_input_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            // Re-render the filtered list. Filtering itself is computed at
            // render time from the current input value; here we just nudge.
            InputEvent::Change => cx.notify(),
            // Enter activates the current top match. If the filter eliminated
            // every recent, fall through to the file dialog so the user
            // doesn't get stuck on an empty list.
            InputEvent::PressEnter { .. } => self.activate_vault_switcher_top(window, cx),
            _ => {}
        }
    }

    /// Snapshot the current filter, intersect it with the recents list, and
    /// open the top match. Used by Enter on the filter input *and* the
    /// "Switch" button on each row (the row click takes a path directly so
    /// it bypasses this).
    fn activate_vault_switcher_top(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.vault_switcher_input.read(cx).value().to_string();
        let needle = query.trim().to_lowercase();
        let state = self.state.read(cx);
        let recents = state.recents().to_vec();
        let unlocked = state.unlocked_paths();
        let active = state.current_vault_path();

        // Prefer a parked-but-already-unlocked vault if the filter
        // matches one — Enter then performs an instant switch instead
        // of a cold password prompt.
        let unlocked_top = unlocked
            .iter()
            .filter(|p| active.as_deref() != Some(p.as_path()))
            .find(|p| matches_path_needle(p, &needle))
            .cloned();
        let recent_top = recents
            .iter()
            .filter(|entry| !unlocked.iter().any(|p| p == &entry.path))
            .find(|entry| matches_path_needle(&entry.path, &needle))
            .map(|entry| entry.path.clone());

        if let Some(path) = unlocked_top.or(recent_top) {
            self.state.update(cx, |state, cx| state.close_overlay(cx));
            self.open_recent(path, window, cx);
            return;
        }

        // No switchable target. If the only thing the filter found was
        // the currently-active vault (e.g. the user typed its name),
        // pressing Enter is a no-op — just close the switcher. The file
        // dialog only opens when nothing in the visible list matched at
        // all.
        let active_matches = active
            .as_deref()
            .is_some_and(|p| matches_path_needle(p, &needle));
        self.state.update(cx, |state, cx| state.close_overlay(cx));
        if !active_matches {
            self.prompt_for_vault_path(window, cx);
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
    ///
    /// Fast path: if `path` is already unlocked (active or parked from a
    /// previous switch in this session), swap the active vault without
    /// re-prompting for the master password.
    pub fn open_recent(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let switched = self
            .state
            .update(cx, |state, cx| state.switch_to_unlocked(&path, cx));
        if switched {
            // Same-vault no-op or successful instant switch: clear any
            // residual unlock inputs so a later cold-open starts clean.
            self.password_input
                .update(cx, |input, cx| input.set_value("", window, cx));
            return;
        }
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
            let result =
                KeePassRepository::open(&path_for_task, &password, keyfile_for_task.as_deref())
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
            // Password and username copies count as "actually used"
            // for the Recently-Used filter. URL copies don't — the
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
        // Mark the entry as recently-used too — copying a custom
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
    /// user-configured TTL — that drop unlinks the temp payload file.
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
        // Read the password — copy_selected_value returns the cleartext
        // *without* writing to the clipboard when called via &self. The
        // launcher needs it to compose the .sapc body; we don't want it
        // accidentally landing on the clipboard for this flow.
        let password = self
            .state
            .read(cx)
            .copy_selected_value(CopyValueKind::Password);

        let ctx = LaunchContext {
            entry: &entry,
            password: password.as_deref(),
            custom_fields: &entry.custom_fields,
        };
        match launcher.launch(ctx) {
            Ok(handle) => {
                window.push_notification(format!("Starting {}…", launcher.label()), cx);
                self.pending_launches.push(handle);
                self.schedule_launch_cleanup(cx);
                // Treat a launch the same as a copy for the recently-
                // used filter — the user just authenticated with this
                // entry, even if no clipboard touch happened.
                self.state.update(cx, |state, _| state.mark_selected_used());
            }
            Err(LaunchError::NoPassword) => {
                window.push_notification("No password set on this entry.", cx);
            }
            Err(LaunchError::MissingField(key)) => {
                window.push_notification(format!("Missing field: {key}"), cx);
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
    fn schedule_launch_cleanup(&mut self, cx: &mut Context<Self>) {
        let ttl =
            std::time::Duration::from_secs(self.settings.launch_cleanup_secs_clamped() as u64);
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(ttl).await;
            let _ = this.update(cx, |this, _| {
                if !this.pending_launches.is_empty() {
                    // FIFO — the oldest pending launch is the one that
                    // matches our timer. Drop drops the TempLaunchFile,
                    // which unlinks the file.
                    this.pending_launches.remove(0);
                }
                if !this.launch_cleanup_tasks.is_empty() {
                    // The Task we drop here is *this* timer (the one
                    // that just woke us up). Letting it drop cancels
                    // its slot — `remove(0)` returns the Task by
                    // value, which is then dropped immediately.
                    let _ = this.launch_cleanup_tasks.remove(0);
                }
            });
        });
        self.launch_cleanup_tasks.push(task);
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
    /// (fire-and-forget — failures are non-fatal, the in-memory
    /// settings still apply for this session) and re-runs
    /// `sync_auto_lock_task` so a freshly-disabled timer is cancelled
    /// or a freshly-enabled one starts immediately.
    pub fn update_settings(&mut self, new_settings: AppSettings, cx: &mut Context<Self>) {
        if self.settings == new_settings {
            return;
        }
        self.settings = new_settings.clone();
        // If the user just toggled auto-lock on/off, we need to start
        // or cancel the checker task immediately rather than waiting
        // for the next state notification.
        self.sync_auto_lock_task(cx);
        // Same rule for auto-type: toggling the feature on, changing
        // the combo, or editing the sequence template all need to
        // re-evaluate the hotkey registration and parse-error cache
        // synchronously so the Settings UI shows the right state on
        // the very next render.
        self.sync_auto_type_listener(cx);
        cx.background_spawn(async move {
            let _ = crate::app::settings::save(&new_settings);
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
        self.lock_vault(window, cx);
    }

    fn render_body(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let vault_status = self.state.read(cx).vault_status();
        let overlay = self.state.read(cx).overlay();

        // Settings is a global overlay — accessible regardless of
        // whether a vault is open (matches the Mac ⌘, convention of
        // Preferences always being reachable).
        if matches!(overlay, Overlay::WhatsNew { .. }) {
            return crate::ui::screens::whats_new::render(self, cx);
        }
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
            return Some(crate::ui::screens::vault_switcher::render(self, cx));
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let body = AppShell::render_body(self, cx);
        // Without this layer the `Root::notification` `NotificationList`
        // never gets painted — `window.push_notification(...)` queues
        // toasts into a list that nothing renders, so the user sees
        // nothing despite the call site looking correct. Mirrors the
        // gpui-component story-shell pattern.
        let notification_layer = Root::render_notification_layer(window, cx);
        let clipboard_pill = self.render_clipboard_pill();
        let modal_overlay = self.render_modal_overlay(cx);

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
            .on_action(cx.listener(Self::on_action_open_vault_switcher))
            .on_action(cx.listener(Self::on_action_submit_password))
            .on_action(cx.listener(Self::on_action_cancel_unlock))
            .on_action(cx.listener(Self::on_action_lock_vault))
            .on_action(cx.listener(Self::on_action_focus_search))
            .on_action(cx.listener(Self::on_action_copy_username))
            .on_action(cx.listener(Self::on_action_copy_url))
            .on_action(cx.listener(Self::on_action_copy_password))
            .on_action(cx.listener(Self::on_action_launch_entry))
            .on_action(cx.listener(Self::on_action_open_connect))
            .on_action(cx.listener(Self::on_action_open_settings))
            .on_action(cx.listener(Self::on_action_open_sync_settings))
            .on_action(cx.listener(Self::on_action_install_update))
            .on_action(cx.listener(Self::on_action_open_whats_new))
            .on_action(cx.listener(Self::on_action_sync_now))
            .on_action(cx.listener(Self::on_action_download_favicons))
            .on_action(cx.listener(Self::on_action_new_entry))
            .on_action(cx.listener(Self::on_action_open_conflict_demo))
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
            .children(notification_layer)
    }
}

fn matches_path_needle(path: &std::path::Path, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    let file_name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    let parent = path
        .parent()
        .map(|p| p.display().to_string().to_lowercase())
        .unwrap_or_default();
    file_name.contains(needle) || parent.contains(needle)
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
