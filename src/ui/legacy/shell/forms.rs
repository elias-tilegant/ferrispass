//! Forms and their inputs: the unlock form, the input accessors, the file
//! and iCloud prompts, the entry form with its custom fields, and the
//! password generator.

use super::*;

impl AppShell {
    pub(super) fn clear_unlock_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.password_input.update(cx, |input, cx| {
            input.set_masked(true, window, cx);
            input.set_value("", window, cx);
        });
        self.keyfile_input
            .update(cx, |input, cx| input.set_value("", window, cx));
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
            state.update(cx, |state, cx| {
                state.pick_kdbx_file(hit, path, cx);
            });
        })
        .detach();
    }

    /// First half of the iCloud "open existing" flow. The provider validates
    /// that the selected path is actually ubiquitous before publishing any
    /// local state.
    pub fn prompt_for_icloud_vault(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Select a KeePass vault in iCloud Drive".into()),
        });
        let shell = cx.entity();
        cx.spawn_in(window, async move |_, window| {
            let remote = paths.await.ok()?.ok()??.first()?.clone();
            window
                .update(|window, cx| {
                    shell.update(cx, |shell, cx| {
                        shell.prompt_for_icloud_local_copy(remote, window, cx)
                    })
                })
                .ok()
        })
        .detach();
    }

    fn prompt_for_icloud_local_copy(
        &mut self,
        remote: PathBuf,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let suggested_name = remote.file_name().map_or_else(
            || "vault.kdbx".to_string(),
            |name| name.to_string_lossy().into_owned(),
        );
        let initial_dir = std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join("Documents"))
            .unwrap_or_else(|| PathBuf::from("."));
        let picker = cx.prompt_for_new_path(&initial_dir, Some(&suggested_name));
        let state = self.state.clone();
        cx.spawn(async move |_, cx| {
            let Ok(Ok(Some(local))) = picker.await else {
                return;
            };
            state.update(cx, |state, cx| {
                state.connect_icloud_existing(remote, local, cx);
            });
        })
        .detach();
    }

    pub fn prompt_for_icloud_publish(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let suggested_name = self
            .state
            .read(cx)
            .active_vault_file_name()
            .unwrap_or_else(|| "vault.kdbx".into());
        let initial_dir = std::env::var_os("HOME")
            .map(|home| {
                let cloud =
                    PathBuf::from(&home).join("Library/Mobile Documents/com~apple~CloudDocs");
                if cloud.is_dir() {
                    cloud
                } else {
                    PathBuf::from(home).join("Documents")
                }
            })
            .unwrap_or_else(|| PathBuf::from("."));
        let picker = cx.prompt_for_new_path(&initial_dir, Some(&suggested_name));
        let state = self.state.clone();
        cx.spawn(async move |_, cx| {
            let Ok(Ok(Some(remote))) = picker.await else {
                return;
            };
            state.update(cx, |state, cx| {
                state.publish_active_to_icloud(remote, cx);
            });
        })
        .detach();
    }

    /// Open a URL in the user's default browser. Used by the device-code
    /// step's "Open in browser" button - opens https://microsoft.com/devicelogin
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
            // never filled" - silently dropped on save rather than
            // polluting the database with `""`-keyed attributes.
            .filter(|f| !f.key.trim().is_empty())
            .collect();
        crate::keepass::EntryDraft {
            title: self.new_entry_title_input.read(cx).value().to_string(),
            username: self.new_entry_username_input.read(cx).value().to_string(),
            password: Zeroizing::new(self.new_entry_password_input.read(cx).value().to_string()),
            url: self.new_entry_url_input.read(cx).value().to_string(),
            notes: self.new_entry_notes_input.read(cx).value().to_string(),
            tags: Vec::new(),
            otp: Zeroizing::new(self.new_entry_otp_input.read(cx).value().to_string()),
            custom_fields,
        }
    }

    /// Save the entry editor, from the Save button or from Enter.
    ///
    /// It used to live inline in the render closure, which is why Enter did
    /// nothing here while the identical Add Group modal submitted on Enter,
    /// and why a missing title arrived as a toast rather than as a message
    /// next to the field it is about.
    pub(crate) fn submit_entry_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editing_id = match self.state.read(cx).overlay() {
            Overlay::EditEntry { entry_id } => Some(entry_id.clone()),
            _ => None,
        };
        let draft = self.collect_entry_draft(cx);
        if draft.title.trim().is_empty() {
            self.entry_form_error = Some("A title is required.".into());
            cx.notify();
            return;
        }
        let target_group_id = self.new_entry_target_group_id.clone().or_else(|| {
            self.state
                .read(cx)
                .vault_browser()
                .map(|browser| match browser.selection {
                    crate::app::LibrarySelection::Group(id) => id,
                    _ => browser.snapshot.root.id.clone(),
                })
        });

        let state = self.state.clone();
        let result = match editing_id.as_ref() {
            Some(entry_id) => state.update(cx, |state, cx| state.update_entry(entry_id, draft, cx)),
            None => {
                let Some(group_id) = target_group_id else {
                    self.entry_form_error = Some("Pick a group to save this entry in.".into());
                    cx.notify();
                    return;
                };
                state
                    .update(cx, |state, cx| state.create_entry(&group_id, draft, cx))
                    .map(|_id| ())
            }
        };
        match result {
            Ok(()) => {
                self.clear_entry_form(window, cx);
                self.state.clone().update(cx, |state, cx| {
                    let _ = state.close_overlay(cx);
                });
                window.push_notification(
                    if editing_id.is_some() {
                        "Changes saved."
                    } else {
                        "Entry saved."
                    },
                    cx,
                );
            }
            Err(error) => {
                self.entry_form_error = Some(errors::mutation_message(&error));
                cx.notify();
            }
        }
    }

    pub fn entry_form_error(&self) -> Option<&str> {
        self.entry_form_error.as_deref()
    }

    pub(super) fn on_entry_form_input_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Change) {
            // A validation message is about what was in the field a moment
            // ago, and so is an armed discard: the user was warned about the
            // draft as it stood, kept typing, and the next closing action
            // would otherwise have thrown away edits nobody warned them
            // about. Typing re-arms the question.
            let stale = self.entry_form_error.take().is_some() || self.entry_discard_armed;
            self.entry_discard_armed = false;
            if stale {
                cx.notify();
            }
        }
        if matches!(event, InputEvent::PressEnter { .. }) {
            self.submit_entry_form(window, cx);
        }
    }

    /// Fingerprint the editor's current contents. Hashing the same draft the
    /// save path builds means a field the save ignores cannot count as an
    /// unsaved change.
    fn entry_form_fingerprint(&self, cx: &gpui::App) -> [u8; 32] {
        use sha2::{Digest as _, Sha256};
        let draft = self.collect_entry_draft(cx);
        let mut hasher = Sha256::new();
        for part in [
            &draft.title,
            &draft.username,
            &draft.password,
            &draft.url,
            &draft.notes,
            &draft.otp,
        ] {
            hasher.update(part.as_bytes());
            hasher.update([0]);
        }
        for field in &draft.custom_fields {
            hasher.update(field.key.as_bytes());
            hasher.update([0]);
            hasher.update(field.value.as_bytes());
            hasher.update([u8::from(field.protected)]);
        }
        hasher.finalize().into()
    }

    /// Remember what the editor looked like when it opened. Call after any
    /// prefill, or the prefill itself reads as the user's typing.
    pub(crate) fn mark_entry_form_pristine(&mut self, cx: &gpui::App) {
        self.entry_form_baseline = Some(self.entry_form_fingerprint(cx));
        self.entry_discard_armed = false;
    }

    pub fn entry_form_is_dirty(&self, cx: &gpui::App) -> bool {
        self.entry_form_baseline
            .is_some_and(|baseline| baseline != self.entry_form_fingerprint(cx))
    }

    pub fn entry_discard_armed(&self) -> bool {
        self.entry_discard_armed
    }

    /// The one gate in front of everything that would throw away a
    /// half-written entry: Cancel, Escape, Cmd+O, Cmd+L, Cmd+comma, Cmd+W and
    /// Cmd+Q all clear the editor, and none of them is covered by the
    /// unpersisted-save veto because the draft never reached the vault.
    ///
    /// Returns `true` when the caller must stand down. The first attempt arms
    /// the discard banner and consumes the gesture; repeating it goes through,
    /// which is the same two-step the Escape key has always used.
    pub(crate) fn entry_draft_blocks_close(&mut self, cx: &mut Context<Self>) -> bool {
        if self.entry_form_is_dirty(cx) && !self.entry_discard_armed {
            self.entry_discard_armed = true;
            cx.notify();
            return true;
        }
        false
    }

    /// Close the entry editor, asking first if there is anything to lose.
    /// Shared by the Cancel button and the Escape key so both behave the same.
    pub(crate) fn request_close_entry_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.entry_draft_blocks_close(cx) {
            return;
        }
        self.clear_entry_form(window, cx);
        self.state.clone().update(cx, |state, cx| {
            let _ = state.close_overlay(cx);
        });
    }

    /// Read-only access to the live editor rows, used by the modal's
    /// render code to lay out the inputs.
    pub fn new_entry_custom_fields(&self) -> &[CustomFieldDraftInputs] {
        &self.new_entry_custom_fields
    }

    /// Build one custom-field row and wire its two inputs to the same
    /// handler the fixed fields use, so Enter saves and typing counts as a
    /// change here too. Every construction site goes through this: rows built
    /// by hand were invisible to both.
    pub(super) fn custom_field_row(
        &mut self,
        key_input: Entity<InputState>,
        value_input: Entity<InputState>,
        protected: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> CustomFieldDraftInputs {
        let id = self.next_custom_field_id;
        self.next_custom_field_id = self.next_custom_field_id.wrapping_add(1);
        let subscriptions = [
            cx.subscribe_in(&key_input, window, Self::on_entry_form_input_event),
            cx.subscribe_in(&value_input, window, Self::on_entry_form_input_event),
        ];
        CustomFieldDraftInputs {
            id,
            key_input,
            value_input,
            protected,
            _subscriptions: subscriptions,
        }
    }

    /// Append a fresh empty row at the bottom of the editor. Returns
    /// the new row's id so the click handler can focus it (future).
    pub fn add_custom_field_row(&mut self, window: &mut Window, cx: &mut Context<Self>) -> usize {
        let key_input = cx.new(|cx| InputState::new(window, cx).placeholder("Key"));
        let value_input = cx.new(|cx| InputState::new(window, cx).placeholder("Value"));
        let row = self.custom_field_row(key_input, value_input, false, window, cx);
        let id = row.id;
        self.new_entry_custom_fields.push(row);
        cx.notify();
        id
    }

    /// Quick-add: drop in the canonical SAP-connection rows in one
    /// click. Keys are pre-filled (`SAP_HOST`, `SAP_INSTANCE`, …) and
    /// each value input gets a placeholder hint so the user knows
    /// what to type. Skips any row whose key already exists in the
    /// editor - clicking the button twice is idempotent rather than
    /// producing duplicate rows.
    pub fn add_sap_connection_template(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for (key, value_placeholder) in crate::launch::sap::QUICK_ADD_KEYS {
            // Idempotency: don't stack multiple SAP_HOST rows on
            // repeat clicks. Match against any existing row regardless
            // of whether the user has typed into it yet.
            let already_present = self
                .new_entry_custom_fields
                .iter()
                .any(|row| row.key_input.read(cx).value().as_ref() == *key);
            if already_present {
                continue;
            }
            let placeholder: SharedString = (*value_placeholder).into();
            let key_input = cx.new(|cx| InputState::new(window, cx).placeholder("Key"));
            let value_input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
            // Pre-fill the key so the row is functional immediately;
            // the value stays empty for the user to fill.
            key_input.update(cx, |s, cx| s.set_value(*key, window, cx));
            let row = self.custom_field_row(key_input, value_input, false, window, cx);
            self.new_entry_custom_fields.push(row);
        }
        cx.notify();
    }

    /// Remove a row by its stable id (the trash button on each row).
    pub fn remove_custom_field_row(
        &mut self,
        id: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(row) = self.new_entry_custom_fields.iter().find(|row| row.id == id) {
            row.key_input
                .update(cx, |state, cx| state.set_value("", window, cx));
            row.value_input
                .update(cx, |state, cx| state.set_value("", window, cx));
        }
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
        self.entry_form_baseline = None;
        self.entry_discard_armed = false;
        self.entry_form_error = None;
        for input in [
            &self.new_entry_title_input,
            &self.new_entry_username_input,
            &self.new_entry_url_input,
            &self.new_entry_notes_input,
        ] {
            input.update(cx, |state, cx| state.set_value("", window, cx));
        }
        for input in [&self.new_entry_password_input, &self.new_entry_otp_input] {
            input.update(cx, |state, cx| {
                state.set_masked(true, window, cx);
                state.set_value("", window, cx);
            });
        }
        // Clear row entities before dropping our handles. GPUI may retain an
        // entity until the end of the frame, so dropping the Vec alone is not
        // an immediate in-memory wipe of protected custom values.
        for row in &self.new_entry_custom_fields {
            row.key_input
                .update(cx, |state, cx| state.set_value("", window, cx));
            row.value_input.update(cx, |state, cx| {
                if row.protected {
                    state.set_masked(true, window, cx);
                }
                state.set_value("", window, cx);
            });
        }
        // Drop any explicit target-group pick so the next open re-derives
        // from the current sidebar selection, and snap the picker shut.
        self.new_entry_target_group_id = None;
        self.new_entry_picker_open = false;
        // Drop all custom-field rows. Their `Entity<InputState>`s are
        // released on Vec drop; gpui will GC them once no view still
        // references them. Don't reset `next_custom_field_id` - keeping
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
    /// enabled - without this the generator silently falls back to lowercase
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
        self.new_entry_password_input.update(cx, |state, cx| {
            state.set_masked(true, window, cx);
            state.set_value(pw, window, cx);
        });
    }
}
