//! Input field events, the vault switcher, and picking a vault file.

use super::*;

impl AppShell {
    pub(super) fn on_password_input_event(
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

    pub(super) fn on_keyfile_input_event(
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

    pub(super) fn on_picker_query_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Change) {
            // Filter is purely client-side over the already-fetched list, so
            // no debounce - every keystroke updates instantly without an
            // API call.
            let q = input.read(cx).value().to_string();
            self.state
                .update(cx, |state, cx| state.set_picker_query(q, cx));
        }
    }

    pub(super) fn on_auto_type_sequence_input_event(
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

    /// Programmatic setter for the Auto-Type sequence - used by the
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

    /// Snapshot the current filter, intersect it with the recents list, and
    /// open the top match. Used by Enter on the filter input *and* the
    /// "Switch" button on each row (the row click takes a path directly so
    /// it bypasses this).
    pub(crate) fn activate_vault_switcher_top(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let query = self.vault_switcher_input.read(cx).value().to_string();
        let needle = query.trim().to_lowercase();
        let state = self.state.read(cx);
        let recents = state.recents().to_vec();
        let unlocked = state.unlocked_paths();
        let active = state.current_vault_path();

        // Prefer a parked-but-already-unlocked vault if the filter
        // matches one - Enter then performs an instant switch instead
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
        // pressing Enter is a no-op - just close the switcher. The file
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

    pub(super) fn on_search_input_event(
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
        // Empty query - apply immediately so closing search feels instant.
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
    /// as picking it through the file dialog - clears the unlock inputs
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
            self.clear_unlock_form(window, cx);
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
            input.set_masked(true, window, cx);
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
}
