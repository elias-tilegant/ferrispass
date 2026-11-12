//! The panel's root view: the search field over the results, and the footer
//! that says what Return does. While no vault is open it is the locked panel
//! of `locked.rs`.
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

use gpui::{
    AnyElement, AppContext as _, Context, Entity, FocusHandle, Focusable as _,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Task, Window, div, px,
};
use gpui_component::IconName;
use gpui_component::input::{Copy as FieldCopy, Cut as FieldCut, InputEvent, InputState};
use zeroize::Zeroizing;

use crate::app::actions::{
    ClosePanel, PANEL_CONTEXT, PanelCopyCode, PanelCopyPassword, PanelCopyUsername, PanelMoveDown,
    PanelMoveUp, PanelPrimary, PanelSelect, PanelShowInApp,
};
use crate::app::kinds::EntryKind;
use crate::app::registry::CommandId;
use crate::app::services::activity::{self, ActivitySource};
use crate::app::services::clipboard::ClipboardService;
use crate::app::{AppState, LibrarySelection};
use crate::launch;
use crate::tr;
use crate::ui::ds::ActiveTokens as _;
use crate::ui::ds::components::{
    AccessibilityId, Chip, PanelFooter, ResultList, Row, RowDensity, SearchGroup, Shortcut,
    TextField, Tile,
};
use crate::ui::ds::tokens::dimension;
use crate::ui::ds::typography::u;
use crate::ui::vm::panel::{FooterModel, FooterNote, Hit, PanelVm, web_url};

use super::locked::Locked;

/// From this many rows on, the list uses the one-line rows.
const COMPACT_FROM: usize = 7;

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
    /// The locked panel's clock, while it is locked.
    _tick: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

/// What the panel copies.
#[derive(Clone, Copy)]
enum Value {
    Password,
    User,
    Code,
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
        let subscriptions = vec![
            cx.subscribe_in(&search, window, Self::on_search_event),
            cx.observe_in(&vm, window, |view, _, window, cx| {
                view.keep_focus(window, cx);
                cx.notify();
            }),
            cx.observe_in(&state, window, |view, _, window, cx| {
                view.locked_state_changed(window, cx);
            }),
            // Taking the keyboard begins the locked panel's buffer. A click
            // into another app takes it away: the panel closes, as it does
            // on Escape, unless the Touch ID dialog took it (`locked.rs`).
            cx.observe_window_activation(window, |view, window, cx| {
                view.on_activation(window.is_window_active(), window, cx);
            }),
            // Every key press in this window, typed text and shortcuts alike,
            // before key bindings and the text field see it.
            cx.intercept_keystrokes(move |_, window, _| {
                if window.window_handle() == this_window {
                    activity::touch(ActivitySource::Panel);
                }
            }),
        ];
        let mut view = Self {
            vm,
            state,
            search,
            focus,
            locked: Locked::default(),
            _tick: None,
            _subscriptions: subscriptions,
        };
        if view.is_locked(cx) {
            view.read_kind(cx);
            view._tick = Some(Self::start_tick(window, cx));
        }
        view.keep_focus(window, cx);
        view
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
        self.copy(Value::Password, cx);
    }

    fn on_copy_username(&mut self, _: &PanelCopyUsername, _: &mut Window, cx: &mut Context<Self>) {
        self.copy(Value::User, cx);
    }

    fn on_copy_code(&mut self, _: &PanelCopyCode, _: &mut Window, cx: &mut Context<Self>) {
        self.copy(Value::Code, cx);
    }

    fn on_show_in_app(&mut self, _: &PanelShowInApp, window: &mut Window, cx: &mut Context<Self>) {
        self.show_in_app(window, cx);
    }

    /// The search field's own Copy, from the Edit menu or the field's edit
    /// menu, copies the password as Cmd+C does. Caught on the way down, so
    /// the field never writes the search to the pasteboard itself.
    fn on_field_copy(&mut self, _: &FieldCopy, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.copy(Value::Password, cx);
    }

    /// The search field's own Cut does nothing in the panel: it would write
    /// the search to the pasteboard past the clipboard service.
    fn on_field_cut(&mut self, _: &FieldCut, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
    }

    /// Return: the main action of the selected entry, when Return may run
    /// one. A copy keeps the panel open; signing in and opening a website
    /// hand over to another app and close it.
    fn on_primary(&mut self, _: &PanelPrimary, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_locked(cx) {
            self.on_return(window, cx);
            return;
        }
        let vm = self.vm.read(cx);
        let (Some(command), Some(hit)) = (vm.footer().primary, vm.selected()) else {
            return;
        };
        let entry_id = hit.entry_id.clone();
        if command == CommandId::ENTRY_COPY_PASSWORD {
            self.copy(Value::Password, cx);
        } else if command == CommandId::ENTRY_SAP_LOGIN {
            self.sign_in(&entry_id, window, cx);
        } else if command == CommandId::ENTRY_OPEN_WEBSITE {
            self.open_website(&entry_id, window, cx);
        }
    }

    /// Copies a value of the selected entry through the clipboard service,
    /// which clears it after the time the settings give, and says so in the
    /// footer. The panel stays open. A password or a user name copied counts
    /// as using the entry.
    fn copy(&mut self, what: Value, cx: &mut Context<Self>) {
        let Some(entry_id) = self.vm.read(cx).selected().map(|hit| hit.entry_id.clone()) else {
            return;
        };
        let value = {
            let state = self.state.read(cx);
            match what {
                Value::Password => state.password_for_entry(&entry_id),
                Value::User => entry_of(state, &entry_id)
                    .map(|entry| entry.username)
                    .filter(|user| !user.trim().is_empty())
                    .map(Zeroizing::new),
                Value::Code => state.totp_for_entry(&entry_id).map(|otp| {
                    Zeroizing::new(otp.code.chars().filter(|c| !c.is_whitespace()).collect())
                }),
            }
        };
        let Some(value) = value else {
            if matches!(what, Value::Password) {
                self.update_vm(cx, |vm| vm.note(FooterNote::CopyFailed));
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
        let note = match (written, what) {
            (Ok(()), Value::Password) => FooterNote::CopiedPassword,
            (Ok(()), Value::User) => FooterNote::CopiedUser,
            (Ok(()), Value::Code) => FooterNote::CopiedCode,
            (Err(_), _) => FooterNote::CopyFailed,
        };
        if matches!(note, FooterNote::CopiedPassword | FooterNote::CopiedUser) {
            self.state
                .update(cx, |state, _| state.mark_entry_used(&entry_id));
        }
        self.update_vm(cx, |vm| vm.note(note));
    }

    /// Signs in to the selected SAP system through SAP GUI, with the entry
    /// the panel selected, then closes the panel.
    fn sign_in(&mut self, entry_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = entry_of(self.state.read(cx), entry_id) else {
            return;
        };
        let Some(target) = launch::primary_launcher_for(&entry) else {
            return;
        };
        let password = self.state.read(cx).password_for_entry(entry_id);
        let context = launch::LaunchContext {
            entry: &entry,
            password: password.as_deref().map(String::as_str),
            custom_fields: &entry.custom_fields,
        };
        if super::sign_in(target, context, cx).is_ok() {
            self.state
                .update(cx, |state, _| state.mark_entry_used(entry_id));
            self.close(window, cx);
        }
    }

    /// Opens the selected web login's website, if its address is http or
    /// https, then closes the panel. Any other address is refused, and the
    /// footer says that the website did not open.
    fn open_website(&mut self, entry_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let url = entry_of(self.state.read(cx), entry_id).and_then(|entry| web_url(&entry.url));
        let Some(url) = url else {
            self.update_vm(cx, |vm| vm.note(FooterNote::WebsiteFailed));
            return;
        };
        cx.open_url(&url);
        self.state
            .update(cx, |state, _| state.mark_entry_used(entry_id));
        self.close(window, cx);
    }

    /// ⌘O: closes the panel and shows the selected entry in the main window,
    /// or just the main window when no vault is open. The one action of the
    /// panel that brings FerrisPass to the front, because the user asked.
    pub(super) fn show_in_app(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let entry_id = self.vm.read(cx).selected().map(|hit| hit.entry_id.clone());
        self.close(window, cx);
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
    }
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
fn tile_icon(kind: &EntryKind) -> IconName {
    match kind {
        EntryKind::Sap(_) => IconName::Building2,
        EntryKind::Remote(_) => IconName::Network,
        EntryKind::Web(_) => IconName::Globe,
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

impl Render for PanelView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let background = cx.ds().colors.bg_elevated.hsla;
        let open = self.vm.read(cx).vault_name().cloned();
        let (body, footer): (AnyElement, PanelFooter) = match open {
            Some(vault) => {
                let vm = self.vm.read(cx);
                (
                    search_group(vm, &self.search, &vault).into_any_element(),
                    footer(&vm.footer()),
                )
            }
            None => self.render_locked(cx),
        };
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
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .gap(u(dimension::SPACE_8))
            .p(u(dimension::SPACE_12))
            .rounded(px(dimension::RADIUS_LG))
            .bg(background)
            .child(input_activity())
            .child(div().flex_1().min_h(px(0.)).flex().flex_col().child(body))
            .child(footer)
    }
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
