//! The panel's root view: the search field over the results, and the footer
//! that says what Return does. While no vault is open it is the locked panel
//! of `locked.rs`.
//!
//! Once something is typed, or a row is picked, the selected entry shows on
//! the right: its title and vault, the user name, the password as a secret
//! field and the one-time code as a ring, each with a button that copies it
//! through the clipboard service and leaves the panel open. The panel never
//! reveals the password and never unmasks the code; whether VoiceOver runs
//! comes from `a11y_prefs` alone, read when the panel opens and whenever it
//! becomes key, and the two components put a secret into the accessibility
//! tree only while it is revealed and VoiceOver runs, so in the panel never.
//! The password and the code are read by the selected entry's id while the
//! panel draws, never through the main window's selection, and kept nowhere.
//!
//! A search without a match names the vault and offers the main window, to
//! unlock other vaults or to add an entry. What failed stands in one carmine
//! line above the search, an alert, until the next key.
//!
//! The keyboard stays in the search field. Cmd+C, which the GPUI Kit input
//! binds to its own Copy, and Cmd+1 to Cmd+9 and Cmd+W, which an app-wide
//! binding would take, are bound in the context `Panel > Input`; so are ↑
//! and ↓, which reach the panel either way, because the Kit's single-line
//! input has no handler for them. The others go through `Panel`
//! (`app::actions::bind_panel_keys` says why). The field's own Copy and Cut,
//! which the menus send, are caught at the root before the field sees them,
//! so the field never writes to the pasteboard. Every key and click in the
//! window counts as activity for the auto-lock, in `PanelView::new` and
//! `input_activity`, and nowhere else.

use std::rc::Rc;
use std::time::Duration;

use gpui::{
    Action, AnyElement, App, AppContext as _, Context, Div, Entity, FocusHandle, Focusable as _,
    Hsla, InteractiveElement as _, IntoElement, ParentElement as _, Render, Role, SharedString,
    Stateful, StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, div, px,
};
use gpui_component::input::{Copy as FieldCopy, Cut as FieldCut, InputEvent, InputState};
use zeroize::Zeroizing;

use crate::app::actions::{
    ClosePanel, NewEntry, PANEL_CONTEXT, PanelCopyCode, PanelCopyPassword, PanelCopyUsername,
    PanelMoveDown, PanelMoveUp, PanelNewEntry, PanelPrimary, PanelSelect, PanelShowInApp,
};
use crate::app::kinds::EntryKind;
use crate::app::registry::CommandId;
use crate::app::services::activity::{self, ActivitySource};
use crate::app::services::clipboard::ClipboardService;
use crate::app::unlock::{locked_panel, touch_id};
use crate::app::{AppState, LibrarySelection};
use crate::keepass::OtpDisplay;
use crate::launch::{self, LaunchError};
use crate::tr;
use crate::ui::ds::ActiveTokens as _;
use crate::ui::ds::components::{
    AccessibilityId, Button, Chip, CopyState, IconButton, OtpRing, OtpSize, PanelFooter,
    ResultList, Row, RowDensity, SearchGroup, SecretField, Shortcut, TextField, Tile, VaultBadge,
};
use crate::ui::ds::icons::FpIcon;
use crate::ui::ds::motion::{self, Input};
use crate::ui::ds::tokens::dimension;
use crate::ui::ds::typography::{TextStyle, TextStyled as _, u};
use crate::ui::vm::panel::{
    FooterModel, FooterNote, Hit, PanelError, PanelVm, PreviewModel, web_url,
};

use super::locked::Locked;

/// From this many rows on, the list uses the one-line rows.
const COMPACT_FROM: usize = 7;

/// The width of the preview next to the list, in points.
const PREVIEW_WIDTH: f32 = 280.0;

/// The width of the labels in the preview, in points.
const PREVIEW_LABEL_WIDTH: f32 = 64.0;

/// How often the one-time code ring is drawn again while it shows.
const OTP_TICK: Duration = Duration::from_millis(1000 / motion::TIMER_OTP_HZ as u64);

/// The keys of the New button, bound to `PanelNewEntry` in the panel.
const NEW_ENTRY_KEYS: &str = "cmd-n";

/// The panel's root view.
pub struct PanelView {
    pub(super) vm: Entity<PanelVm>,
    pub(super) state: Entity<AppState>,
    pub(super) search: Entity<InputState>,
    /// The locked panel's field. It has the keyboard while no vault is open
    /// and the search field is not shown.
    pub(super) focus: FocusHandle,
    /// What the locked panel holds; empty once a vault is open.
    pub(super) locked: Locked,
    /// VoiceOver runs, as `a11y_prefs` said when the panel opened or last
    /// became key. The preview's secret field and ring get it, and nothing
    /// else sets it.
    assistive: bool,
    /// The entry and the value the last copy took, and what the copy button
    /// shows for it now.
    copied: Option<(String, Value, CopyState)>,
    /// Moves `copied` from the flash to the tick and back to idle.
    _copy_flash: Option<Task<()>>,
    /// Draws the one-time code ring again, while one shows.
    _otp_tick: Option<Task<()>>,
    /// The locked panel's clock, while it is locked.
    _tick: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

/// What the panel copies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Value {
    Password,
    User,
    Code,
}

/// Whether VoiceOver runs, as macOS says. The one source of the preview's
/// `assistive` flag; tests put their own answer in its place.
fn voiceover_running() -> bool {
    #[cfg(test)]
    {
        super::tests::apps::voiceover_running()
    }
    #[cfg(not(test))]
    {
        crate::ui::ds::a11y_prefs::read().voiceover
    }
}

impl PanelView {
    pub(super) fn new(
        vm: Entity<PanelVm>,
        state: Entity<AppState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let query = vm.read(cx).query().to_string();
        let search = cx.new(|cx| {
            let mut input =
                InputState::new(window, cx).placeholder(tr!("quick.search.placeholder"));
            input.set_value(query, window, cx);
            input
        });
        let focus = cx.focus_handle();
        let this_window = window.window_handle();
        let keyed_vm = vm.clone();
        let subscriptions = vec![
            cx.subscribe_in(&search, window, Self::on_search_event),
            cx.observe_in(&vm, window, |view, _, window, cx| {
                view.keep_focus(window, cx);
                view.sync_otp_tick(cx);
                cx.notify();
            }),
            cx.observe_in(&state, window, |view, _, window, cx| {
                view.locked_state_changed(window, cx);
            }),
            // Taking the keyboard begins the locked panel's buffer. A click
            // into another app takes it away: the panel closes, as it does
            // on Escape, unless the Touch ID dialog took it (`locked.rs`).
            // Each time it is key, VoiceOver is asked again.
            cx.observe_window_activation(window, |view, window, cx| {
                let active = window.is_window_active();
                if active {
                    view.assistive = voiceover_running();
                }
                view.on_activation(active, window, cx);
            }),
            // Every key press in this window, typed text and shortcuts alike,
            // before key bindings and the text field see it. It also takes
            // the line about what failed away.
            cx.intercept_keystrokes(move |_, window, cx| {
                if window.window_handle() == this_window {
                    activity::touch(ActivitySource::Panel);
                    keyed_vm.update(cx, |vm, cx| {
                        if vm.clear_error() {
                            cx.notify();
                        }
                    });
                }
            }),
        ];
        let mut view = Self {
            vm,
            state,
            search,
            focus,
            locked: Locked::default(),
            assistive: voiceover_running(),
            copied: None,
            _copy_flash: None,
            _otp_tick: None,
            _tick: None,
            _subscriptions: subscriptions,
        };
        if view.is_locked(cx) {
            view.read_kind(cx);
            view._tick = Some(Self::start_tick(window, cx));
        }
        view.keep_focus(window, cx);
        view.sync_otp_tick(cx);
        view
    }

    /// Draws the panel again ten times a second while the preview shows a
    /// one-time code, and not at all otherwise.
    fn sync_otp_tick(&mut self, cx: &mut Context<Self>) {
        let shows_code = self
            .vm
            .read(cx)
            .preview()
            .is_some_and(|preview| preview.has_code);
        match (shows_code, self._otp_tick.is_some()) {
            (true, false) => {
                self._otp_tick = Some(cx.spawn(async move |view, cx| {
                    loop {
                        cx.background_executor().timer(OTP_TICK).await;
                        let drawn = view.update(cx, |_, cx| {
                            #[cfg(test)]
                            super::tests::apps::note_otp_tick();
                            cx.notify();
                        });
                        if drawn.is_err() {
                            break;
                        }
                    }
                }));
            }
            (false, true) => self._otp_tick = None,
            _ => {}
        }
    }

    /// The keyboard goes to the search field, or to the locked panel's field
    /// while no vault is open.
    fn keep_focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        if self.vm.read(cx).vault_name().is_some() {
            if !self.search.read(cx).focus_handle(cx).is_focused(window) {
                self.search.update(cx, |input, cx| input.focus(window, cx));
            }
        } else if !self.focus.is_focused(window) {
            window.focus(&self.focus, cx);
        }
    }

    fn on_search_event(
        &mut self,
        search: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !matches!(event, InputEvent::Change) {
            return;
        }
        let query = search.read(cx).value().to_string();
        let state = self.state.clone();
        self.vm.update(cx, |vm, cx| {
            vm.set_query(query, state.read(cx));
            cx.notify();
        });
    }

    fn update_vm(&self, cx: &mut Context<Self>, update: impl FnOnce(&mut PanelVm)) {
        self.vm.update(cx, |vm, cx| {
            update(vm);
            cx.notify();
        });
    }

    /// Says in the line above the search what failed, until the next key.
    fn fail(&self, error: PanelError, cx: &mut Context<Self>) {
        self.update_vm(cx, |vm| vm.fail(error));
    }

    /// Escape clears a search that holds text; Cmd+W and Escape on an empty
    /// search close the panel, and nothing else.
    fn on_close(&mut self, action: &ClosePanel, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_locked(cx) {
            if action.clear_search_first && self.escape_locked(window, cx) {
                return;
            }
            self.close(window, cx);
            return;
        }
        if action.clear_search_first && !self.vm.read(cx).query().is_empty() {
            self.search
                .update(cx, |input, cx| input.set_value("", window, cx));
            let state = self.state.clone();
            self.vm.update(cx, |vm, cx| {
                vm.set_query("", state.read(cx));
                cx.notify();
            });
            return;
        }
        self.close(window, cx);
    }

    /// Closes the panel from inside its window. What was typed while locked
    /// is wiped first.
    pub(super) fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.locked.wipe();
        super::close_from(window, cx);
    }

    fn on_move_up(&mut self, _: &PanelMoveUp, _: &mut Window, cx: &mut Context<Self>) {
        self.update_vm(cx, |vm| vm.move_selection(-1));
    }

    fn on_move_down(&mut self, _: &PanelMoveDown, _: &mut Window, cx: &mut Context<Self>) {
        self.update_vm(cx, |vm| vm.move_selection(1));
    }

    fn on_select(&mut self, action: &PanelSelect, _: &mut Window, cx: &mut Context<Self>) {
        let number = action.number;
        self.update_vm(cx, |vm| vm.select_number(number));
    }

    fn on_copy_password(&mut self, _: &PanelCopyPassword, _: &mut Window, cx: &mut Context<Self>) {
        self.copy(Value::Password, Input::Keyboard, cx);
    }

    fn on_copy_username(&mut self, _: &PanelCopyUsername, _: &mut Window, cx: &mut Context<Self>) {
        self.copy(Value::User, Input::Keyboard, cx);
    }

    fn on_copy_code(&mut self, _: &PanelCopyCode, _: &mut Window, cx: &mut Context<Self>) {
        self.copy(Value::Code, Input::Keyboard, cx);
    }

    fn on_show_in_app(&mut self, _: &PanelShowInApp, window: &mut Window, cx: &mut Context<Self>) {
        self.show_in_app(None, window, cx);
    }

    /// Cmd+N and the New button: the main window, with a new entry.
    fn on_new_entry(&mut self, _: &PanelNewEntry, window: &mut Window, cx: &mut Context<Self>) {
        self.show_in_app(Some(Box::new(NewEntry)), window, cx);
    }

    /// The search field's own Copy, from the Edit menu or the field's edit
    /// menu, copies the password as Cmd+C does. Caught on the way down, so
    /// the field never writes the search to the pasteboard itself.
    fn on_field_copy(&mut self, _: &FieldCopy, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.copy(Value::Password, Input::Keyboard, cx);
    }

    /// The search field's own Cut does nothing in the panel: it would write
    /// the search to the pasteboard past the clipboard service.
    fn on_field_cut(&mut self, _: &FieldCut, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
    }

    /// Return: the main action of the selected entry, when Return may run
    /// one, and in the locked panel Touch ID. A copy keeps the panel open;
    /// signing in and opening a website hand over to another app and close
    /// it.
    fn on_primary(&mut self, _: &PanelPrimary, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_locked(cx) {
            self.on_return(window, cx);
            return;
        }
        // Return acts on what was on screen: not before the results are
        // drawn, not within 300 ms of that, and not while a vault is still
        // being unlocked, unless the one exact match may take it at once.
        let now = cx.background_executor().now();
        let under_way = touch_id::unlock_under_way(self.state.read(cx));
        let gate = self.vm.read(cx).return_gate(under_way);
        if !locked_panel::return_accepted(&gate, now) {
            return;
        }
        let vm = self.vm.read(cx);
        let (Some(command), Some(hit)) = (vm.footer().primary, vm.selected()) else {
            return;
        };
        let entry_id = hit.entry_id.clone();
        if command == CommandId::ENTRY_COPY_PASSWORD {
            self.copy(Value::Password, Input::Keyboard, cx);
        } else if command == CommandId::ENTRY_SAP_LOGIN {
            self.sign_in(&entry_id, window, cx);
        } else if command == CommandId::ENTRY_OPEN_WEBSITE {
            self.open_website(&entry_id, window, cx);
        }
    }

    /// Copies a value of the selected entry, for its keys.
    fn copy(&mut self, what: Value, input: Input, cx: &mut Context<Self>) {
        let Some(entry_id) = self.vm.read(cx).selected().map(|hit| hit.entry_id.clone()) else {
            return;
        };
        self.copy_entry(&entry_id, what, input, cx);
    }

    /// Copies a value of the entry with `entry_id` through the clipboard
    /// service, which clears it after the time the settings give, and says
    /// so in the footer; a password that cannot be copied is said in the
    /// line above the search. The panel stays open. A password or a user
    /// name copied counts as using the entry. The preview's buttons copy
    /// through here with the id of the entry they show.
    fn copy_entry(&mut self, entry_id: &str, what: Value, input: Input, cx: &mut Context<Self>) {
        let value = {
            let state = self.state.read(cx);
            match what {
                Value::Password => state.password_for_entry(entry_id),
                Value::User => entry_of(state, entry_id)
                    .map(|entry| entry.username)
                    .filter(|user| !user.trim().is_empty())
                    .map(Zeroizing::new),
                Value::Code => state.totp_for_entry(entry_id).map(|otp| digits_of(&otp)),
            }
        };
        let Some(value) = value else {
            if matches!(what, Value::Password) {
                self.fail(PanelError::CopyFailed, cx);
            }
            return;
        };
        let clear_after =
            crate::app::with_shared_shell(cx, |shell, _| shell.settings().clipboard_clear_secs)
                .unwrap_or_else(|| crate::app::AppSettings::default().clipboard_clear_secs)
                .map(std::time::Duration::from_secs);
        let written = ClipboardService::handle(cx).update(cx, |service, cx| {
            service.write_secret(value, clear_after, cx)
        });
        if written.is_err() {
            self.fail(PanelError::CopyFailed, cx);
            return;
        }
        let note = match what {
            Value::Password => FooterNote::CopiedPassword,
            Value::User => FooterNote::CopiedUser,
            Value::Code => FooterNote::CopiedCode,
        };
        if matches!(what, Value::Password | Value::User) {
            self.state
                .update(cx, |state, _| state.mark_entry_used(entry_id));
        }
        self.update_vm(cx, |vm| vm.note(note));
        self.flash_copy(entry_id, what, input, cx);
    }

    /// The copy shows on its button: the value flashes, then the button
    /// shows a tick for a moment. After a key there is no flash.
    fn flash_copy(&mut self, entry_id: &str, what: Value, input: Input, cx: &mut Context<Self>) {
        let flash = motion::duration(motion::Token::Micro, input, cx);
        let first = if flash.is_zero() {
            CopyState::Check
        } else {
            CopyState::Flash
        };
        self.copied = Some((entry_id.to_string(), what, first));
        self._copy_flash = Some(cx.spawn(async move |view, cx| {
            if !flash.is_zero() {
                cx.background_executor().timer(flash).await;
                let _ = view.update(cx, |view, cx| {
                    if let Some((_, _, state)) = &mut view.copied {
                        *state = CopyState::Check;
                    }
                    cx.notify();
                });
            }
            cx.background_executor()
                .timer(motion::HOLD_COPY_CHECK)
                .await;
            let _ = view.update(cx, |view, cx| {
                view.copied = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// What the copy button of `what` shows for the entry `entry_id`.
    fn copied_state(&self, entry_id: &str, what: Value) -> CopyState {
        match &self.copied {
            Some((copied, value, state)) if copied == entry_id && *value == what => *state,
            _ => CopyState::Idle,
        }
    }

    /// Signs in to the selected SAP system through SAP GUI, with the entry
    /// the panel selected, then closes the panel. When SAP GUI cannot be
    /// started for it, the panel stays open and the line above the search
    /// says so, without the entry's title or any of its values; a password
    /// SAP GUI cannot take gets a sentence of its own, which does not name
    /// the character. Nothing counts as used then.
    fn sign_in(&mut self, entry_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = entry_of(self.state.read(cx), entry_id) else {
            self.fail(PanelError::SignInFailed, cx);
            return;
        };
        let Some(target) = launch::primary_launcher_for(&entry) else {
            self.fail(PanelError::SignInFailed, cx);
            return;
        };
        let password = self.state.read(cx).password_for_entry(entry_id);
        let context = launch::LaunchContext {
            entry: &entry,
            password: password.as_deref().map(String::as_str),
            custom_fields: &entry.custom_fields,
        };
        match super::sign_in(target, context, cx) {
            Ok(()) => {
                self.state
                    .update(cx, |state, _| state.mark_entry_used(entry_id));
                self.close(window, cx);
            }
            Err(LaunchError::UnsupportedCharacter { field: "password" }) => {
                self.fail(PanelError::PasswordRefused, cx);
            }
            Err(_) => self.fail(PanelError::SignInFailed, cx),
        }
    }

    /// Opens the selected web login's website, if its address is http or
    /// https, then closes the panel. Any other address is refused, and the
    /// line above the search says that the website did not open.
    fn open_website(&mut self, entry_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let url = entry_of(self.state.read(cx), entry_id).and_then(|entry| web_url(&entry.url));
        let Some(url) = url else {
            self.fail(PanelError::WebsiteFailed, cx);
            return;
        };
        cx.open_url(&url);
        self.state
            .update(cx, |state, _| state.mark_entry_used(entry_id));
        self.close(window, cx);
    }

    /// ⌘O: closes the panel and shows the selected entry in the main window,
    /// or just the main window when no vault is open. With `then`, the main
    /// window gets that action instead of the entry: a new entry. The one way of the panel that brings FerrisPass to the
    /// front, because the user asked. Over an app that was hidden, the app
    /// is not hidden again, and its windows come back.
    pub(super) fn show_in_app(
        &mut self,
        then: Option<Box<dyn Action>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let entry_id = then
            .is_none()
            .then(|| self.vm.read(cx).selected().map(|hit| hit.entry_id.clone()))
            .flatten();
        self.locked.wipe();
        super::close_for_main_window(window, cx);
        if let Some(entry_id) = entry_id {
            self.state.update(cx, |state, cx| {
                let listed = state
                    .vault_browser()
                    .is_some_and(|browser| browser.entries.iter().any(|e| e.id == entry_id));
                if !listed {
                    state.select_library(LibrarySelection::AllItems, cx);
                }
                state.select_entry(entry_id, cx);
            });
        }
        crate::app::show_main_window(cx);
        if let Some(action) = then
            && let Some(main) = crate::app::actions::main_window(cx)
        {
            let _ = main.update(cx, |_, window, cx| window.dispatch_action(action, cx));
        }
    }
}

/// The digits of a one-time code, without the space the display puts in.
fn digits_of(otp: &OtpDisplay) -> Zeroizing<String> {
    Zeroizing::new(otp.code.chars().filter(|c| !c.is_whitespace()).collect())
}

/// The entry with this id in the open vault, as the snapshot holds it.
fn entry_of(state: &AppState, entry_id: &str) -> Option<crate::domain::VaultEntry> {
    state
        .vault_browser()?
        .snapshot
        .find_entry(entry_id)
        .cloned()
}

/// Every click and scroll in the panel counts as activity. The listeners go
/// on the window, painted by an empty canvas, so nothing drawn above them
/// can keep a click from counting.
fn input_activity() -> impl IntoElement {
    gpui::canvas(
        |_, _, _| {},
        |_, _, window, _| {
            window.on_mouse_event(|_: &gpui::MouseDownEvent, phase, _, _| {
                if phase == gpui::DispatchPhase::Capture {
                    activity::touch(ActivitySource::Panel);
                }
            });
            window.on_mouse_event(|_: &gpui::ScrollWheelEvent, phase, _, _| {
                if phase == gpui::DispatchPhase::Capture {
                    activity::touch(ActivitySource::Panel);
                }
            });
        },
    )
    .absolute()
    .size_full()
}

/// The glyph on an entry's tile.
fn tile_icon(kind: &EntryKind) -> FpIcon {
    match kind {
        EntryKind::Sap(_) => FpIcon::TypeSap,
        EntryKind::Remote(_) => FpIcon::TypeRemote,
        EntryKind::Web(_) => FpIcon::TypeWeb,
    }
}

/// The footer for `model`. Its menu button does nothing yet.
pub(super) fn footer(model: &FooterModel) -> PanelFooter {
    let mut footer = PanelFooter::new("panel-footer", |_, _, _| {}).tone(model.tone.clone());
    if let Some((keys, words)) = &model.main {
        footer = footer.primary(Shortcut::from_keystroke(keys, words.clone()));
    }
    for command in &model.more {
        footer = footer.more(Shortcut::for_command(*command));
    }
    if let Some(note) = &model.note {
        footer = footer.more(Shortcut::from_keystroke("", note.clone()));
    }
    footer
}

/// The row for `hit` at `index` of `count`. The selected row carries the
/// footer's sentence as its description. Its id is
/// `fp.panel.row.<index>`.
pub(super) fn row(
    hit: &Hit,
    index: usize,
    count: usize,
    selected: bool,
    description: &SharedString,
) -> Row {
    let density = if count >= COMPACT_FROM {
        RowDensity::Panel32
    } else {
        RowDensity::Panel44
    };
    let mut row = Row::new(
        ("panel-row", index),
        index,
        count,
        hit.parts.clone(),
        Tile::new(tile_icon(&hit.kind)),
        hit.badge.clone(),
    )
    .density(density)
    .selected(selected)
    .accessibility_id(AccessibilityId::new("panel", "row", index));
    if let Some((tone, letters)) = &hit.chip {
        row = row.chip(Chip::new(*tone, letters.clone()));
    }
    if let Ok(number) = u8::try_from(index + 1)
        && number <= 9
    {
        row = row.shortcut_index(number);
    }
    if selected && !description.is_empty() {
        row = row.description(description.clone());
    }
    row
}

/// The search field over the results, under one node that carries the
/// footer's sentence while the list is empty and no row can.
pub(super) fn search_group(
    vm: &PanelVm,
    search: &Entity<InputState>,
    vault: &SharedString,
) -> SearchGroup {
    let description = footer(&vm.footer()).description_text();
    let hits: Vec<Hit> = vm.results().to_vec();
    let count = hits.len();
    let selected = vm.selection().index();
    let row_description = description.clone();
    let list = ResultList::new(
        "panel-results",
        tr!("a11y.results"),
        count,
        move |index, _, _| {
            row(
                &hits[index],
                index,
                count,
                selected == Some(index),
                &row_description,
            )
        },
    )
    .selected(selected)
    .report_drawn(vm.drawn());
    let field = TextField::new(
        "panel-search",
        search,
        tr!("quick.header.searching_vault", vault = vault),
    )
    .accessibility_id(AccessibilityId::new("panel", "search", 0));
    let group = SearchGroup::new("panel-search-group", tr!("a11y.search"), field, list);
    if count == 0 && !description.is_empty() {
        group.description(description)
    } else {
        group
    }
}

/// What a copy button does: copies the value it is for.
pub(super) type CopyHandler = Rc<dyn Fn(Value, &mut Window, &mut App)>;

/// The secret values of the previewed entry, read by its id for one frame.
pub(super) struct PreviewValues {
    pub(super) password: Option<Zeroizing<String>>,
    pub(super) otp: Option<OtpDisplay>,
}

/// Every element of the preview, before they are put in place: the title
/// with the vault's badge, the user name with its copy button, the password
/// as a secret field with its own copy button, and the one-time code as a
/// ring with a copy button.
pub(super) struct PreviewParts {
    pub(super) title: SharedString,
    pub(super) badge: VaultBadge,
    pub(super) user: Option<(SharedString, IconButton)>,
    pub(super) password: Option<SecretField>,
    pub(super) code: Option<(OtpRing, IconButton)>,
}

/// The glyph of a copy button: a tick right after a copy.
fn copy_glyph(copied: CopyState) -> FpIcon {
    match copied {
        CopyState::Idle => FpIcon::Copy,
        CopyState::Flash | CopyState::Check => FpIcon::Check,
    }
}

/// Builds the preview of `model` with `values`. `assistive` says whether
/// VoiceOver runs; the secret field and the ring get it as it is. The
/// secret field is never revealed and the ring stays masked, so neither
/// puts its secret into the accessibility tree. `copied` says what each
/// copy button shows; `on_copy` copies.
pub(super) fn preview(
    model: &PreviewModel,
    values: PreviewValues,
    assistive: bool,
    copied: impl Fn(Value) -> CopyState,
    on_copy: CopyHandler,
) -> PreviewParts {
    let masked = true;
    #[cfg(test)]
    note_preview_flags(PreviewFlags {
        assistive,
        revealed: false,
        masked,
    });
    let copy = |what: Value| {
        let on_copy = on_copy.clone();
        move |_: &gpui::ClickEvent, window: &mut Window, cx: &mut App| on_copy(what, window, cx)
    };
    let user = model.user.clone().map(|user| {
        let button = IconButton::new(
            "panel-preview-copy-user",
            copy_glyph(copied(Value::User)),
            tr!("action.copy_username.label"),
        )
        .on_click(copy(Value::User));
        (user, button)
    });
    let password = model.has_password.then(|| {
        let field = SecretField::new("panel-preview-password", tr!("quick.preview.password"))
            .assistive(assistive)
            .copied(copied(Value::Password))
            .on_copy(copy(Value::Password));
        match values.password {
            Some(password) => field.value(password),
            None => field,
        }
    });
    let code = values.otp.filter(|_| model.has_code).map(|otp| {
        let ring = OtpRing::new(
            "panel-preview-code",
            Duration::from_secs(u64::from(otp.remaining_secs)),
            Duration::from_secs(u64::from(otp.period_secs)),
        )
        .code(digits_of(&otp))
        .masked(masked)
        .assistive(assistive)
        .size(OtpSize::Detail18);
        let button = IconButton::new(
            "panel-preview-copy-code",
            copy_glyph(copied(Value::Code)),
            tr!("action.copy_code.label"),
        )
        .on_click(copy(Value::Code));
        (ring, button)
    });
    PreviewParts {
        title: model.title.clone(),
        badge: model.badge.clone(),
        user,
        password,
        code,
    }
}

/// Puts the preview's parts in place, next to the list.
fn assemble_preview(parts: PreviewParts, primary: Hsla, secondary: Hsla, rule: Hsla) -> Div {
    let PreviewParts {
        title,
        badge,
        user,
        password,
        code,
    } = parts;
    let label = |text: SharedString| {
        div()
            .w(u(PREVIEW_LABEL_WIDTH))
            .flex_none()
            .text_style(TextStyle::Callout)
            .text_color(secondary)
            .child(text)
    };
    let line = || {
        div()
            .flex()
            .items_center()
            .gap(u(dimension::SPACE_8))
            .min_h(u(dimension::HEIGHT_CONTROL))
    };
    let heading = div()
        .flex()
        .items_center()
        .gap(u(dimension::SPACE_6))
        .child(
            div()
                .min_w(px(0.))
                .text_style(TextStyle::Headline)
                .text_color(primary)
                .text_ellipsis()
                .child(title),
        )
        .child(badge);
    div()
        .w(u(PREVIEW_WIDTH))
        .flex_none()
        .flex()
        .flex_col()
        .gap(u(dimension::SPACE_8))
        .pl(u(dimension::SPACE_12))
        .border_l_1()
        .border_color(rule)
        .child(heading)
        .children(user.map(|(user, button)| {
            line()
                .child(label(tr!("quick.preview.user")))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_style(TextStyle::Body)
                        .text_color(primary)
                        .child(user),
                )
                .child(
                    div()
                        .debug_selector(|| "fp-preview-copy-user".into())
                        .child(button),
                )
        }))
        .children(password.map(|field| {
            line()
                .child(label(tr!("quick.preview.password")))
                .child(field)
        }))
        .children(code.map(|(ring, button)| {
            line()
                .child(label(tr!("glossary.one_time_code")))
                .child(ring)
                .child(
                    div()
                        .debug_selector(|| "fp-preview-copy-code".into())
                        .child(button),
                )
        }))
}

/// What a search without a match shows: the sentence that names the vault,
/// the button that unlocks other vaults in the main window, only while
/// another vault is unlocked besides the open one, and the button for a new
/// entry with its keys.
pub(super) struct NoMatchParts {
    pub(super) text: SharedString,
    pub(super) unlock: Option<Button>,
    pub(super) new_entry: Button,
}

pub(super) fn no_match(vm: &PanelVm, vault: &SharedString) -> NoMatchParts {
    NoMatchParts {
        text: tr!("quick.no_match.text", vault = vault),
        unlock: (vm.parked_vaults() > 0)
            .then(|| Button::secondary("panel-no-match-unlock", tr!("quick.no_match.unlock"))),
        new_entry: Button::secondary("panel-no-match-new", tr!("quick.no_match.new_entry"))
            .shortcut(NEW_ENTRY_KEYS),
    }
}

/// The line above the search that says what failed: an alert, in carmine.
/// Its text is one of the panel's fixed sentences and names no entry.
pub(super) fn error_line(error: PanelError) -> Stateful<Div> {
    let text = error.text();
    div()
        .id("panel-error")
        .debug_selector(|| "fp-panel-error".into())
        .role(Role::Alert)
        .aria_label(text.clone())
        .text_style(TextStyle::Callout)
        .child(text)
}

impl PanelView {
    /// The open panel: `group`, the search over the list, then a search
    /// without a match, and the preview next to them.
    fn render_open(
        &mut self,
        group: SearchGroup,
        vault: &SharedString,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.ds().colors;
        let (missing, shown) = {
            let vm = self.vm.read(cx);
            (vm.no_match().then(|| no_match(vm, vault)), vm.preview())
        };
        let missing = missing.map(|parts| {
            let NoMatchParts {
                text,
                unlock,
                new_entry,
            } = parts;
            let unlock = unlock.map(|button| {
                button.on_click(cx.listener(|view, _, window, cx| {
                    view.show_in_app(None, window, cx);
                }))
            });
            let new_entry = new_entry.on_click(cx.listener(|view, _, window, cx| {
                view.show_in_app(Some(Box::new(NewEntry)), window, cx);
            }));
            div()
                .flex()
                .flex_col()
                .gap(u(dimension::SPACE_8))
                .child(
                    div()
                        .text_style(TextStyle::Body)
                        .text_color(colors.text_secondary.hsla)
                        .child(text),
                )
                .child(
                    div()
                        .flex()
                        .gap(u(dimension::SPACE_8))
                        .children(unlock)
                        .child(new_entry),
                )
        });
        let list = div()
            .flex_1()
            .min_w(px(0.))
            .flex()
            .flex_col()
            .gap(u(dimension::SPACE_8))
            .child(group)
            .children(missing);
        let Some(model) = shown else {
            return list.into_any_element();
        };
        let values = {
            let state = self.state.read(cx);
            PreviewValues {
                password: model
                    .has_password
                    .then(|| state.password_for_entry(&model.entry_id))
                    .flatten(),
                otp: model
                    .has_code
                    .then(|| state.totp_for_entry(&model.entry_id))
                    .flatten(),
            }
        };
        let view = cx.entity().downgrade();
        let entry_id = model.entry_id.clone();
        let on_copy: CopyHandler = Rc::new(move |what, _, cx| {
            let _ = view.update(cx, |view, cx| {
                view.copy_entry(&entry_id, what, Input::Pointer, cx);
            });
        });
        let parts = preview(
            &model,
            values,
            self.assistive,
            |what| self.copied_state(&model.entry_id, what),
            on_copy,
        );
        div()
            .flex()
            .size_full()
            .gap(u(dimension::SPACE_12))
            .child(list)
            .child(assemble_preview(
                parts,
                colors.text_primary.hsla,
                colors.text_secondary.hsla,
                colors.border_separator.hsla,
            ))
            .into_any_element()
    }
}

impl Render for PanelView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.ds().colors;
        let open = self.vm.read(cx).vault_name().cloned();
        let (body, footer): (AnyElement, PanelFooter) = match open {
            Some(vault) => {
                let (group, footer) = {
                    let vm = self.vm.read(cx);
                    (search_group(vm, &self.search, &vault), footer(&vm.footer()))
                };
                (self.render_open(group, &vault, cx), footer)
            }
            None => self.render_locked(cx),
        };
        let error = self.vm.read(cx).error();
        div()
            .id("panel")
            .role(Role::Dialog)
            .aria_label(tr!("a11y.quick_access"))
            .key_context(PANEL_CONTEXT)
            .capture_action(cx.listener(Self::on_field_copy))
            .capture_action(cx.listener(Self::on_field_cut))
            .on_action(cx.listener(Self::on_close))
            .on_action(cx.listener(Self::on_move_up))
            .on_action(cx.listener(Self::on_move_down))
            .on_action(cx.listener(Self::on_select))
            .on_action(cx.listener(Self::on_primary))
            .on_action(cx.listener(Self::on_copy_password))
            .on_action(cx.listener(Self::on_copy_username))
            .on_action(cx.listener(Self::on_copy_code))
            .on_action(cx.listener(Self::on_show_in_app))
            .on_action(cx.listener(Self::on_new_entry))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .gap(u(dimension::SPACE_8))
            .p(u(dimension::SPACE_12))
            .rounded(px(dimension::RADIUS_LG))
            .bg(colors.bg_elevated.hsla)
            .child(input_activity())
            .children(
                error.map(|error| error_line(error).text_color(colors.status_danger_text.hsla)),
            )
            .child(div().flex_1().min_h(px(0.)).flex().flex_col().child(body))
            .child(footer)
    }
}

/// What the copy button of the last copy shows, for the tests.
#[cfg(test)]
impl PanelView {
    pub(super) fn copied_for_test(&self) -> Option<CopyState> {
        self.copied.as_ref().map(|(_, _, state)| *state)
    }
}

/// The flags the preview last handed the secret field and the ring, for
/// the tests.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PreviewFlags {
    pub(super) assistive: bool,
    pub(super) revealed: bool,
    pub(super) masked: bool,
}

#[cfg(test)]
thread_local! {
    static PREVIEW_FLAGS: std::cell::Cell<Option<PreviewFlags>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn note_preview_flags(flags: PreviewFlags) {
    PREVIEW_FLAGS.set(Some(flags));
}

/// The flags of the preview built last on this thread, and forgets them.
#[cfg(test)]
pub(super) fn preview_flags() -> Option<PreviewFlags> {
    PREVIEW_FLAGS.take()
}

/// The row of each result, as the list draws it, for the tests.
#[cfg(test)]
pub(super) fn rows_of(vm: &PanelVm) -> Vec<Row> {
    let description = footer(&vm.footer()).description_text();
    let count = vm.results().len();
    vm.results()
        .iter()
        .enumerate()
        .map(|(index, hit)| {
            row(
                hit,
                index,
                count,
                vm.selection().index() == Some(index),
                &description,
            )
        })
        .collect()
}
