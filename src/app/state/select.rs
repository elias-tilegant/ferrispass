//! Selection, search, copying values, and the model the vault browser
//! renders.

use super::*;

impl AppState {
    pub fn select_group(&mut self, group_id: impl Into<String>, cx: &mut Context<Self>) {
        let group_id = group_id.into();

        let VaultStatus::Open {
            document,
            selection,
            selected_entry_id,
            search_query,
            visible_entries,
            selected_strength,
            last_used,
            ..
        } = &mut self.vault
        else {
            return;
        };

        let snapshot = document.snapshot();
        if snapshot.find_group(&group_id).is_none() {
            return;
        }

        *selection = LibrarySelection::Group(group_id);
        search_query.clear();
        let entries = entries_for_selection(snapshot, selection, "", last_used);
        *selected_entry_id = entries.first().map(|entry| entry.id.clone());
        *selected_strength = selected_entry_id
            .as_deref()
            .and_then(|id| document.strength_for_entry(id));
        *visible_entries = Rc::new(entries);
        cx.notify();
    }

    pub fn select_library(&mut self, sel: LibrarySelection, cx: &mut Context<Self>) {
        let VaultStatus::Open {
            document,
            selection,
            selected_entry_id,
            search_query,
            visible_entries,
            selected_strength,
            last_used,
            ..
        } = &mut self.vault
        else {
            return;
        };
        if *selection == sel {
            return;
        }
        *selection = sel;
        search_query.clear();
        let entries = entries_for_selection(document.snapshot(), selection, "", last_used);
        *selected_entry_id = entries.first().map(|entry| entry.id.clone());
        *selected_strength = selected_entry_id
            .as_deref()
            .and_then(|id| document.strength_for_entry(id));
        *visible_entries = Rc::new(entries);
        cx.notify();
    }

    pub fn select_entry(&mut self, entry_id: impl Into<String>, cx: &mut Context<Self>) {
        let entry_id = entry_id.into();

        let VaultStatus::Open {
            document,
            selected_entry_id,
            selected_strength,
            ..
        } = &mut self.vault
        else {
            return;
        };

        if document.snapshot().find_entry(&entry_id).is_some() {
            *selected_strength = document.strength_for_entry(&entry_id);
            *selected_entry_id = Some(entry_id);
            cx.notify();
        }
    }

    pub fn set_search_query(&mut self, query: impl Into<String>, cx: &mut Context<Self>) {
        let query = query.into();

        let VaultStatus::Open {
            document,
            selection,
            selected_entry_id,
            search_query,
            visible_entries,
            selected_strength,
            last_used,
            ..
        } = &mut self.vault
        else {
            return;
        };

        if *search_query == query {
            return;
        }

        *search_query = query;
        let entries =
            entries_for_selection(document.snapshot(), selection, search_query, last_used);
        let selected_entry_is_visible = selected_entry_id
            .as_deref()
            .is_some_and(|id| entries.iter().any(|entry| entry.id == id));

        if !selected_entry_is_visible {
            *selected_entry_id = entries.first().map(|entry| entry.id.clone());
            *selected_strength = selected_entry_id
                .as_deref()
                .and_then(|id| document.strength_for_entry(id));
        }

        *visible_entries = Rc::new(entries);
        cx.notify();
    }

    pub fn clear_search(&mut self, cx: &mut Context<Self>) {
        let VaultStatus::Open {
            document,
            selection,
            selected_entry_id,
            search_query,
            visible_entries,
            selected_strength,
            last_used,
            ..
        } = &mut self.vault
        else {
            return;
        };

        if search_query.is_empty() {
            return;
        }

        search_query.clear();
        let entries = entries_for_selection(document.snapshot(), selection, "", last_used);
        *selected_entry_id = entries.first().map(|entry| entry.id.clone());
        *selected_strength = selected_entry_id
            .as_deref()
            .and_then(|id| document.strength_for_entry(id));
        *visible_entries = Rc::new(entries);
        cx.notify();
    }

    /// Stamp the currently-selected entry as "just used" in the
    /// in-memory access log. Called from the AppShell after a
    /// successful password / username copy. No-op when no vault is
    /// open or no entry is selected. Doesn't notify - the
    /// `RecentlyUsed` list is rebuilt on the next selection change,
    /// which matches KeePassXC (the list is a snapshot, not live).
    pub fn mark_selected_used(&mut self) {
        let Some(id) = self.shown_entry_id().map(str::to_string) else {
            return;
        };
        if let VaultStatus::Open { last_used, .. } = &mut self.vault {
            last_used.insert(id, Local::now());
        }
    }

    /// Same as `mark_selected_used` but for an explicit entry id -
    /// used by the auto-type path, where the credential we just typed
    /// is the foreground-matched entry, not necessarily the one the
    /// user has selected in the sidebar.
    pub fn mark_entry_used(&mut self, entry_id: &str) {
        if let VaultStatus::Open { last_used, .. } = &mut self.vault {
            last_used.insert(entry_id.to_string(), Local::now());
        }
    }

    pub fn copy_selected_value(&self, kind: CopyValueKind) -> Option<Zeroizing<String>> {
        let model = self.vault_browser()?;
        let entry = model.selected_entry?;

        match kind {
            CopyValueKind::Username => non_empty_copy(entry.username).map(Zeroizing::new),
            CopyValueKind::Url => non_empty_copy(entry.url).map(Zeroizing::new),
            CopyValueKind::Password => {
                let VaultStatus::Open { document, .. } = &self.vault else {
                    return None;
                };

                document.password_for_entry(&entry.id)
            }
        }
    }

    /// Read a single custom-field value off any entry by id. Drives
    /// the detail-panel "Additional fields" copy buttons and the
    /// launcher path's per-key lookups. Returns `None` when no vault
    /// is open, the entry doesn't exist, or the field is unset.
    pub fn custom_field_value(&self, entry_id: &str, key: &str) -> Option<Zeroizing<String>> {
        let VaultStatus::Open { document, .. } = &self.vault else {
            return None;
        };
        document.custom_field_value(entry_id, key)
    }

    pub fn vault_browser(&self) -> Option<VaultBrowserModel> {
        let VaultStatus::Open {
            document,
            selection,
            selected_entry_id,
            search_query,
            visible_entries,
            selected_strength,
            ..
        } = &self.vault
        else {
            return None;
        };

        let snapshot = document.snapshot_rc();
        let showing_search_results = !search_query.trim().is_empty();

        // Same rule as `shown_entry_id`, which the TOTP and recently-used
        // paths read; this one needs the whole entry rather than the id.
        let selected_entry = selected_entry_id
            .as_deref()
            .and_then(|id| visible_entries.iter().find(|entry| entry.id == id))
            .cloned()
            .or_else(|| visible_entries.first().cloned());

        let selection_label = selection_label_for(selection, &snapshot);

        Some(VaultBrowserModel {
            snapshot,
            selection: selection.clone(),
            selection_label,
            selected_entry_id: selected_entry.as_ref().map(|entry| entry.id.clone()),
            entries: Rc::clone(visible_entries),
            selected_entry,
            selected_strength: *selected_strength,
            search_query: search_query.clone(),
            showing_search_results,
        })
    }
}
