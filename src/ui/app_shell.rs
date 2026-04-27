use crate::{
    app::{
        AppState, CopyValueKind, Overlay,
        actions::{
            APP_CONTEXT, CancelUnlock, CopyPassword, CopyUrl, CopyUsername, CreateVault,
            FocusSearch, LockVault, NewEntry, OpenConflictDemo, OpenConnect, OpenSyncSettings,
            OpenVault, SubmitPassword,
        },
    },
    keepass::KeePassRepository,
};
use gpui::{
    AppContext as _, ClickEvent, ClipboardItem, Context, Entity, InteractiveElement as _,
    ParentElement as _, PathPromptOptions, Render, Styled as _, Subscription, Window, div,
};
use gpui_component::{
    ActiveTheme as _, Root, VirtualListScrollHandle, WindowExt as _,
    input::{InputEvent, InputState},
};
use std::path::PathBuf;

pub struct AppShell {
    state: Entity<AppState>,
    password_input: Entity<InputState>,
    keyfile_input: Entity<InputState>,
    search_input: Entity<InputState>,
    new_entry_title_input: Entity<InputState>,
    new_entry_username_input: Entity<InputState>,
    new_entry_password_input: Entity<InputState>,
    new_entry_url_input: Entity<InputState>,
    /// Persistent scroll position for the entry virtual list. Must outlive a single
    /// render — without it the list resets to the top on every re-render and
    /// mouse-wheel events appear to do nothing.
    entry_list_scroll: VirtualListScrollHandle,
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

        let _subscriptions = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            cx.subscribe_in(&password_input, window, Self::on_password_input_event),
            cx.subscribe_in(&keyfile_input, window, Self::on_keyfile_input_event),
            cx.subscribe_in(&search_input, window, Self::on_search_input_event),
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
            entry_list_scroll: VirtualListScrollHandle::new(),
            _subscriptions,
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
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let is_open = matches!(
            self.state.read(cx).vault_status(),
            crate::app::VaultStatus::Open { .. }
        );
        if is_open {
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
        if matches!(event, InputEvent::Change) {
            let query = search_input.read(cx).value().to_string();
            self.state
                .update(cx, |state, cx| state.set_search_query(query, cx));
        }
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
                Overlay::AddEntry => crate::ui::screens::add_entry::render(self, cx),
                _ => crate::ui::screens::vault::render(self, cx),
            },
            crate::app::VaultStatus::Error { .. } => crate::ui::screens::vault::render(self, cx),
        }
    }
}

impl Render for AppShell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let body = AppShell::render_body(self, cx);

        div()
            .key_context(APP_CONTEXT)
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
