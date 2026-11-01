//! The root view of the main window. The type, its fields and its
//! constructor are here; the methods are split by concern into the
//! submodules.

#[cfg(target_os = "macos")]
use crate::clipboard::{ClipboardChangeCount, clear_if_unchanged, write_secret_text};
use crate::{
    app::{
        AppSettings, AppState, CopyValueKind, Overlay,
        actions::{
            APP_CONTEXT, AddSharePointVault, CancelUnlock, CopyPassword, CopyUrl, CopyUsername,
            CreateVault, DeleteEntry, DeleteGroup, DownloadFavicons, EditEntry, FocusSearch,
            InstallUpdate, LockVault, NewEntry, NewGroup, NewSubgroup, OpenAbout, OpenAddVault,
            OpenConnect, OpenReconnect, OpenSettings, OpenSyncSettings, OpenVault,
            OpenVaultSwitcher, OpenWhatsNew, PerformAutoType, PerformAutoTypeForSelected,
            RenameGroupOp, SaveVault, SubmitPassword, SyncNow, ToggleTheme,
        },
    },
    autotype,
    keepass::{KeePassRepository, MutationError, limits::KdfCost},
    launch::{self, LaunchContext, LaunchError, LaunchHandle},
    ui::errors,
};

/// Which section of the unified Settings overlay is currently active.
/// Lives in AppShell because it's UI-local state - not worth persisting,
/// reset to General whenever the overlay closes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsTab {
    General,
    Sync,
    AutoType,
}
#[cfg(not(target_os = "macos"))]
use gpui::ClipboardItem;
use gpui::{
    AnyWindowHandle, AppContext as _, ClickEvent, Context, Entity, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement as _, ParentElement as _, PathPromptOptions, Render,
    ScrollStrategy, SharedString, Styled as _, Subscription, Task, Window, div, px,
};
use gpui_component::{
    ActiveTheme as _, VirtualListScrollHandle, WindowExt as _,
    input::{InputEvent, InputState},
    slider::{SliderState, SliderValue},
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};
use zeroize::Zeroizing;

mod auto_type;
mod forms;
mod handlers;
mod inputs;
mod lock;
mod render;
mod secrets;
mod tasks;
mod unlock;

struct EditPrefill {
    id: String,
    title: String,
    username: String,
    url: String,
    notes: String,
    password: Zeroizing<String>,
    otp: Zeroizing<String>,
    custom_fields: Vec<crate::domain::CustomField>,
}

/// One row in the AddEntry / EditEntry modal's "Additional fields"
/// section. Each row owns its own pair of `InputState` entities so
/// the gpui input stack can track focus / cursor position
/// independently - sharing them across rows would have selection
/// jumping when the user reorders or deletes a row.
pub struct CustomFieldDraftInputs {
    /// Stable id used as the gpui element key. Monotonic counter via
    /// `AppShell::next_custom_field_id` - never reused, so deleting
    /// row 3 and adding a new one doesn't accidentally restore
    /// stale gpui state for the old row.
    pub id: usize,
    pub key_input: Entity<InputState>,
    pub value_input: Entity<InputState>,
    pub protected: bool,
    /// Enter-to-save and dirty tracking for this row's two inputs. Held here
    /// rather than in the shell's subscription list so it dies with the row:
    /// rows come and go for every entry the user opens.
    _subscriptions: [Subscription; 2],
}

/// Opaque identity of the clipboard write owned by the current clear timer.
/// It deliberately contains neither the copied value nor a digest of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingClipboardWrite {
    #[cfg(target_os = "macos")]
    Native(ClipboardChangeCount),
    #[cfg(not(target_os = "macos"))]
    Portable(u64),
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
    /// Isolated overlay view for the vault switcher. Its input changes only
    /// re-render this child view, not the whole vault browser behind it.
    vault_switcher: Entity<crate::ui::screens::vault_switcher::VaultSwitcher>,
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
    /// render - without it the list resets to the top on every re-render and
    /// mouse-wheel events appear to do nothing.
    entry_list_scroll: VirtualListScrollHandle,
    /// The shell's own focus handle. We `track_focus` it on the root div so the
    /// app always has a focused element in the dispatch path - without this,
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
    /// Group the user asked to delete, waiting for confirmation. Deleting a
    /// group takes its whole subtree with it, which used to happen on one
    /// unconfirmed click from a context menu.
    pending_group_delete: Option<String>,
    /// Set by the first click on "Disconnect", cleared by the second.
    disconnect_armed: bool,
    /// 1 Hz tick task that drives the live TOTP countdown in the detail panel.
    /// Started after a vault opens and dropped (= cancelled) on lock. We only
    /// need this for the seconds-resolution countdown; the actual TOTP code
    /// changes every 30 s so a coarser tick would also work, but 1 Hz keeps
    /// the "valid for Xs" text moving smoothly.
    totp_tick: Option<Task<()>>,
    /// Background timer that wipes the clipboard `CLIPBOARD_CLEAR_SECS` after
    /// a copy. Holding the `Task` here means a new copy replaces (= cancels)
    /// the previous timer - only the latest copy's clear fires.
    clipboard_clear_task: Option<Task<()>>,
    /// Opaque identity of our latest clipboard write. On macOS this is the
    /// NSPasteboard change count, which lets the timer compare-and-clear
    /// without retaining the copied secret in application state.
    pending_clipboard_write: Option<PendingClipboardWrite>,
    /// Monotonic identity for portable clipboard writes. Only compiled on
    /// platforms where GPUI cannot expose an OS clipboard generation.
    #[cfg(not(target_os = "macos"))]
    next_portable_clipboard_write: u64,
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
    /// Last vault context observed by the window-bound state observer. A
    /// path/status transition wipes editor and clipboard secrets before the
    /// newly-selected vault is rendered.
    secret_context_path: Option<PathBuf>,
    secret_context_was_open: bool,
    /// Tracks leaving Add/Edit through any route, including global Escape or
    /// another overlay replacing the editor without calling its Cancel button.
    entry_editor_was_open: bool,
    /// Fingerprint of the entry editor's contents when it opened. Compared on
    /// close to tell an untouched form from one the user typed into, so
    /// Escape, a lock, or switching overlays cannot discard a half-written
    /// entry (and its freshly generated password) without asking.
    ///
    /// A hash, not a copy of the draft: the draft carries a cleartext
    /// password and this outlives every keystroke that produced it.
    entry_form_baseline: Option<[u8; 32]>,
    /// Set by the first Escape on a dirty editor. The second one discards.
    entry_discard_armed: bool,
    /// Validation or save failure for the entry editor, shown next to the
    /// form rather than as a toast that floats away from the field it means.
    entry_form_error: Option<String>,
    /// Wall-clock timestamp of the last user input event (mouse-move,
    /// click, key-down). Updated cheaply on every event without
    /// triggering a re-render - only the auto-lock checker reads it.
    /// `SystemTime`, not `Instant`: `Instant` does not advance while macOS
    /// sleeps, so a machine asleep for hours accrued no idle time and the
    /// auto-lock threshold was measured in awake-seconds only. `AppState`
    /// documents the same hazard for `last_unlock_at`.
    last_activity: SystemTime,
    /// Periodic checker that locks the vault after the configured
    /// auto-lock timeout. Only running while a vault is open AND a
    /// non-`None` timeout is configured (managed alongside `totp_tick`
    /// via the state observer). Drop = cancel.
    auto_lock_task: Option<Task<()>>,
    /// Retains the native macOS notification registrations for the lifetime
    /// of this window and the GPUI task that consumes their coalesced events.
    #[cfg(target_os = "macos")]
    _session_lock_monitor: crate::session_lock::SessionLockMonitor,
    #[cfg(target_os = "macos")]
    _session_lock_task: Option<Task<()>>,
    /// Periodic background auto-sync + token keep-alive. Runs only while a
    /// synced vault is in memory AND `settings.auto_sync_secs` is set
    /// (managed alongside `auto_lock_task` via the state observer +
    /// `update_settings`). Each tick pulls remote changes and refreshes
    /// the OAuth token, keeping the refresh token's inactivity window
    /// alive. Drop = cancel.
    auto_sync_task: Option<Task<()>>,
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
    pending_launches: HashMap<u64, LaunchHandle>,
    /// Ids handed out to `pending_launches`, so a timer collects the payload
    /// it was scheduled for.
    next_launch_id: u64,
    /// One-shot timers, one per pending launch. Holding the `Task`
    /// keeps the timer alive; dropping it cancels (used when lock/quit
    /// purges everything early).
    launch_cleanup_tasks: HashMap<u64, Task<()>>,
    /// Active global-hotkey registration for auto-type. `Some` only
    /// when `settings.auto_type_enabled && settings.auto_type_hotkey`
    /// parses & registers cleanly; dropped (= unregistered) when the
    /// user toggles the feature off or changes the combo. The
    /// matching poll loop in `auto_type_poll_task` reads the registered
    /// id to filter events.
    auto_type_listener: Option<autotype::HotkeyListener>,
    /// Background task that polls the global-hotkey event channel at
    /// ~30 Hz and dispatches `PerformAutoType` when our combo fires.
    /// Dropped together with the listener - leaving it running with no
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
    /// Vault for which this visit to the unlock screen already launched an
    /// automatic Touch ID prompt. Cleared after leaving AwaitingPassword so
    /// a later lock can auto-prompt again without looping after cancellation.
    automatic_biometric_attempted_for: Option<PathBuf>,
    /// macOS withdrew the last Touch ID prompt because the window lost
    /// focus. The next activation clears `automatic_biometric_attempted_for`
    /// so the automatic prompt runs again, once, when the user is back.
    automatic_biometric_interrupted: bool,
    /// An interruption that arrived with the window already back in front
    /// re-runs the automatic prompt at once, but only once per return to
    /// the window, so a prompt macOS keeps withdrawing cannot loop.
    automatic_biometric_rearmed: bool,
    /// Vault whose header pre-check (`pending_kdf_confirmation`) has been
    /// started for the current stay on the unlock screen. Cleared when the
    /// unlock screen goes away, so coming back to it checks again.
    kdf_check_started_for: Option<PathBuf>,
    /// Handle to the window that hosts this shell. Captured at
    /// construction so the global-hotkey poll task can dispatch
    /// `perform_auto_type` directly into this window's context - bypassing
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

/// Polling interval for the global-hotkey event channel. ~30 Hz -
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
        let new_entry_password_input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Password")
        });
        let new_entry_url_input = cx.new(|cx| InputState::new(window, cx).placeholder("URL"));
        let new_entry_notes_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Notes (optional)"));
        let new_entry_otp_input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("otpauth://… or base32 secret")
        });
        let new_group_name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Group name"));
        let picker_query_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Filter by name or folder…"));
        let vault_switcher_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Switch vault - type to filter…"));
        // Seed the sequence input with whatever's currently persisted -
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

        let shell = cx.weak_entity();
        let vault_switcher = cx.new(
            |cx: &mut Context<crate::ui::screens::vault_switcher::VaultSwitcher>| {
                crate::ui::screens::vault_switcher::VaultSwitcher::new(
                    shell,
                    state.clone(),
                    vault_switcher_input.clone(),
                    window,
                    cx,
                )
            },
        );

        let (secret_context_path, secret_context_was_open, entry_editor_was_open) = {
            let state = state.read(cx);
            (
                state.current_vault_path(),
                matches!(state.vault_status(), crate::app::VaultStatus::Open { .. }),
                matches!(
                    state.overlay(),
                    Overlay::AddEntry | Overlay::EditEntry { .. }
                ),
            )
        };

        // "System" has to mean "follow the system", not "read it once at
        // startup": macOS switches appearance at sunset for a lot of people.
        window
            .observe_window_appearance(|window, cx| {
                let choice = crate::app::settings::load().theme;
                if choice == crate::app::settings::ThemeChoice::System {
                    crate::ui::theme::apply_choice(choice, Some(window), cx);
                }
            })
            .detach();

        let _subscriptions = vec![
            cx.observe_in(&state, window, |shell: &mut AppShell, state, window, cx| {
                // Re-render whenever AppState notifies AND keep the
                // background tasks (TOTP tick + auto-lock checker) in
                // sync with whether a vault is currently open.
                shell.sync_totp_tick(cx);
                shell.sync_auto_lock_task(cx);
                // Start/stop the background auto-sync + keep-alive timer in
                // step with whether a synced vault is currently in memory.
                shell.sync_auto_sync_task(cx);
                // Auto-mask: if the user moves to a different entry (or
                // locks the vault), drop any reveal we held. Compared by
                // id so re-renders against the same entry don't trigger.
                let (current_id, current_path, current_is_open, editor_is_open) = {
                    let state = state.read(cx);
                    (
                        state
                            .vault_browser()
                            .and_then(|b| b.selected_entry)
                            .map(|e| e.id),
                        state.current_vault_path(),
                        matches!(state.vault_status(), crate::app::VaultStatus::Open { .. }),
                        matches!(
                            state.overlay(),
                            Overlay::AddEntry | Overlay::EditEntry { .. }
                        ),
                    )
                };
                // Reacting to a state change, not to a paint: this writes
                // `automatic_biometric_attempted_for`, and doing that from
                // `render` mutated shell state during layout.
                shell.sync_kdf_check(cx);
                shell.schedule_automatic_biometric_unlock(window, cx);
                // Failures raised where no window was in hand park a message
                // on the state; show it once, here.
                if let Some(message) = state.update(cx, |state, _| state.take_pending_notice()) {
                    Self::notify_error(window, message, cx);
                }
                let secret_context_changed = shell.secret_context_path != current_path
                    || shell.secret_context_was_open != current_is_open;
                let editor_closed = shell.entry_editor_was_open && !editor_is_open;
                shell.secret_context_path = current_path;
                shell.secret_context_was_open = current_is_open;
                shell.entry_editor_was_open = editor_is_open;

                if secret_context_changed {
                    shell.wipe_session_secrets(cx);
                    shell.clear_unlock_form(window, cx);
                    shell.clear_entry_form(window, cx);
                } else if editor_closed {
                    shell.clear_entry_form(window, cx);
                }
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
                &auto_type_sequence_input,
                window,
                Self::on_auto_type_sequence_input_event,
            ),
            cx.subscribe_in(&new_group_name_input, window, Self::on_new_group_name_event),
            // Enter submits the entry editor, the way it already did in the
            // identically-shaped Add Group modal. Notes stays out: there
            // Enter is a line break, and Cmd+Enter is not worth a second
            // binding for a field people rarely finish on.
            cx.subscribe_in(
                &new_entry_title_input,
                window,
                Self::on_entry_form_input_event,
            ),
            cx.subscribe_in(
                &new_entry_username_input,
                window,
                Self::on_entry_form_input_event,
            ),
            cx.subscribe_in(
                &new_entry_password_input,
                window,
                Self::on_entry_form_input_event,
            ),
            cx.subscribe_in(
                &new_entry_url_input,
                window,
                Self::on_entry_form_input_event,
            ),
            cx.subscribe_in(
                &new_entry_otp_input,
                window,
                Self::on_entry_form_input_event,
            ),
            // Re-render on slider drag so the "Length: N" label and the
            // strength preview update live alongside the thumb.
            cx.observe(&gen_length_state, |_shell: &mut AppShell, _, cx| {
                cx.notify();
            }),
            // The automatic Touch ID prompt only runs with the window in
            // front (see `schedule_automatic_biometric_unlock`), so coming
            // back to it is when a held-back or interrupted prompt runs.
            cx.observe_window_activation(window, |shell: &mut AppShell, window, cx| {
                if !window.is_window_active() {
                    return;
                }
                shell.automatic_biometric_rearmed = false;
                if std::mem::take(&mut shell.automatic_biometric_interrupted) {
                    shell.automatic_biometric_attempted_for = None;
                }
                shell.schedule_automatic_biometric_unlock(window, cx);
            }),
        ];

        // Sweep launch payloads orphaned by a previous crash. We only
        // touch files older than 60 s - short enough that a near-miss
        // (us starting the moment a previous instance dropped a
        // payload) doesn't kill it, long enough that anything from a
        // previous session that didn't shut down cleanly is gone.
        crate::launch::sweeper::sweep_stale(std::time::Duration::from_secs(60));
        // …and once more after the grace window has fully elapsed. A
        // crash followed by an immediate relaunch (the typical user
        // reaction) leaves the orphaned payload younger than 60 s, so
        // the startup sweep above spares it - and without this second
        // pass nothing would ever delete the cleartext file for the
        // rest of the session. At +120 s (2× the maximum launch TTL)
        // every survivor from a previous run is unambiguously stale,
        // while this session's own pending launches are still protected
        // by the 60 s age check. Unlocks re-sweep too (see
        // `AppState::finish_open_attempt`); this timer covers sessions
        // that resume without an unlock.
        let resweep_delay = cx
            .background_executor()
            .timer(std::time::Duration::from_secs(120));
        cx.background_spawn(async move {
            resweep_delay.await;
            crate::launch::sweeper::sweep_stale(std::time::Duration::from_secs(60));
        })
        .detach();

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
            vault_switcher,
            auto_type_sequence_input,
            gen_length_state,
            gen_classes: crate::keepass::password_gen::CharClasses::default(),
            entry_list_scroll: VirtualListScrollHandle::new(),
            focus_handle,
            search_debounce: None,
            pending_perma_delete: None,
            pending_group_delete: None,
            disconnect_armed: false,
            totp_tick: None,
            clipboard_clear_task: None,
            pending_clipboard_write: None,
            #[cfg(not(target_os = "macos"))]
            next_portable_clipboard_write: 0,
            clipboard_clear_deadline: None,
            clipboard_pill_tick: None,
            revealed_entry_id: None,
            secret_context_path,
            secret_context_was_open,
            entry_editor_was_open,
            entry_form_baseline: None,
            entry_discard_armed: false,
            entry_form_error: None,
            last_activity: SystemTime::now(),
            auto_lock_task: None,
            #[cfg(target_os = "macos")]
            _session_lock_monitor: crate::session_lock::SessionLockMonitor::new(),
            #[cfg(target_os = "macos")]
            _session_lock_task: None,
            auto_sync_task: None,
            settings: crate::app::settings::load(),
            settings_tab: SettingsTab::General,
            new_entry_target_group_id: None,
            new_entry_picker_open: false,
            new_entry_custom_fields: Vec::new(),
            next_custom_field_id: 0,
            pending_launches: HashMap::new(),
            next_launch_id: 0,
            launch_cleanup_tasks: HashMap::new(),
            auto_type_listener: None,
            auto_type_poll_task: None,
            auto_type_sequence_error: None,
            auto_type_hotkey_error: None,
            automatic_biometric_attempted_for: None,
            automatic_biometric_interrupted: false,
            automatic_biometric_rearmed: false,
            kdf_check_started_for: None,
            window_handle: window.window_handle(),
            _subscriptions,
        };
        // Bring up the global-hotkey registration immediately if the
        // user has Auto-Type enabled. Done after Self is built (rather
        // than starting the task during the struct literal) so the
        // weak handle that `cx.spawn` captures resolves to the
        // already-constructed entity. Idempotent if Auto-Type is off
        // - `sync_auto_type_listener` is the only entry point and it
        // no-ops cleanly when both desired-state and current-state are
        // "no listener".
        shell.sync_auto_type_listener(cx);
        #[cfg(target_os = "macos")]
        shell.start_session_lock_task(cx);
        // A launch that resumes the last vault is already on the unlock
        // screen before this shell exists, and the state observer only runs
        // on the next change, which may never come. Without this pass the
        // automatic Touch ID prompt fired at launch only by luck.
        cx.defer_in(window, |shell, window, cx| {
            shell.sync_kdf_check(cx);
            shell.schedule_automatic_biometric_unlock(window, cx);
        });
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

    pub fn pending_group_delete(&self) -> Option<&str> {
        self.pending_group_delete.as_deref()
    }

    /// Armed state for "Disconnect". Deleting the OAuth grant and the on-disk
    /// binding used to take one click, less friction than deleting a single
    /// password, and it also cleared the sync activity log.
    pub fn disconnect_armed(&self) -> bool {
        self.disconnect_armed
    }

    pub(crate) fn arm_disconnect(&mut self, cx: &mut Context<Self>) {
        self.disconnect_armed = true;
        cx.notify();
    }

    pub(crate) fn cancel_disconnect(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.disconnect_armed) {
            cx.notify();
        }
    }

    pub(crate) fn confirm_disconnect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.disconnect_armed = false;
        self.state.clone().update(cx, |state, cx| {
            state.disconnect_sync(cx);
            let _ = state.close_overlay(cx);
        });
        // The removal itself runs in the background; a failure there arrives
        // through the pending-notice channel the observer drains.
        window.push_notification("Cloud sync disconnected.", cx);
    }
}

/// Either-shaped result the biometric-unlock background task hands
/// back to the foreground update. `Open` carries the KDF outcome so
/// the success path runs the same `finish_open_attempt` codepath as a
/// password-typed unlock; `Failed` carries the OS-side
/// `BiometricError` so the Unlock screen can render the right error
/// message and (when applicable) clear an invalidated enrolment.
///
/// The `Open` variant is large (it owns a `VaultDocument`) while
/// `Failed` is small; we don't box it because the value is moved
/// exactly once per unlock and the surrounding unlock path already
/// passes `VaultDocument` by value.
#[allow(clippy::large_enum_variant)]
enum BiometricUnlockOutcome {
    Open {
        path: PathBuf,
        result: Result<crate::keepass::VaultDocument, keepass::db::DatabaseOpenError>,
        /// Generation of the launch that produced this outcome, so a
        /// stale resolution can be discarded.
        generation: u64,
    },
    Failed {
        path: PathBuf,
        err: crate::biometric::BiometricError,
        generation: u64,
    },
}

/// UI-friendly file name for the Touch ID prompt header. Strips the
/// `.kdbx` suffix so the prompt reads "Unlock my-vault" rather than
/// "Unlock my-vault.kdbx". Used only for human-facing strings - never
/// as a keychain account or any other security-relevant identifier.
fn file_name_for_prompt(path: &std::path::Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "vault".to_string())
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

#[cfg(test)]
mod characterization_tests;
