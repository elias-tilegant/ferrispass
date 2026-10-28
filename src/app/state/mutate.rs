//! Creating, editing, moving and deleting entries and groups.

use super::*;

impl AppState {
    /// Create an entry inside the given group, refresh the snapshot-derived
    /// caches, focus the new entry, and trigger a background save. Returns the
    /// new entry's id on success.
    pub fn create_entry(
        &mut self,
        group_id: &str,
        draft: EntryDraft,
        cx: &mut Context<Self>,
    ) -> Result<String, MutationError> {
        let new_id = {
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
                return Err(MutationError::EntryNotFound);
            };

            let new_id = document.create_entry(group_id, &draft)?;

            // Snap the user to the entry's group so they can see what they
            // just created - otherwise creating from inside "Favorites" or a
            // tag filter would silently land the entry off-screen.
            *selection = LibrarySelection::Group(group_id.to_string());
            search_query.clear();

            let entries = entries_for_selection(document.snapshot(), selection, "", last_used);
            *selected_entry_id = Some(new_id.clone());
            *visible_entries = Rc::new(entries);
            *selected_strength = document.strength_for_entry(&new_id);

            new_id
        };
        cx.notify();
        self.save_async(cx);
        Ok(new_id)
    }

    pub fn update_entry(
        &mut self,
        entry_id: &str,
        draft: EntryDraft,
        cx: &mut Context<Self>,
    ) -> Result<(), MutationError> {
        {
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
                return Err(MutationError::EntryNotFound);
            };

            document.update_entry(entry_id, &draft)?;

            *visible_entries = Rc::new(entries_for_selection(
                document.snapshot(),
                selection,
                search_query,
                last_used,
            ));
            // Re-score; the password may have changed.
            if selected_entry_id.as_deref() == Some(entry_id) {
                *selected_strength = document.strength_for_entry(entry_id);
            }
        }
        cx.notify();
        self.save_async(cx);
        Ok(())
    }

    /// Move an entry to the recycle bin (creating one if necessary). Selection
    /// jumps to the next visible entry so the detail pane stays populated.
    pub fn delete_entry(
        &mut self,
        entry_id: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), MutationError> {
        self.run_entry_mutation(cx, |doc| doc.delete_entry(entry_id), entry_id)
    }

    /// Permanent (unrecoverable) delete. Use only after a confirmation step in
    /// the UI - `save_async` flushes the result to disk and the entry is gone.
    pub fn delete_entry_permanent(
        &mut self,
        entry_id: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), MutationError> {
        self.run_entry_mutation(cx, |doc| doc.delete_entry_permanent(entry_id), entry_id)
    }

    /// Restore an entry from the recycle bin to the vault root.
    pub fn restore_entry(
        &mut self,
        entry_id: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), MutationError> {
        self.run_entry_mutation(cx, |doc| doc.restore_entry(entry_id), entry_id)
    }

    /// Move an entry into a different group. Used by drag-and-drop.
    /// Mirrors the update/delete pattern: refresh the visible-entries
    /// cache against the current selection (which may now exclude the
    /// moved entry, e.g. when viewing only one group), then schedule a
    /// background save. Selection-tracking is intentionally lazy -
    /// `vault_browser()` falls back to the first visible entry if the
    /// previously-selected one disappears from view, so we don't need
    /// to repoint `selected_entry_id` here.
    pub fn move_entry(
        &mut self,
        entry_id: &str,
        target_group_id: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), MutationError> {
        {
            let VaultStatus::Open {
                document,
                selection,
                search_query,
                visible_entries,
                last_used,
                ..
            } = &mut self.vault
            else {
                return Err(MutationError::EntryNotFound);
            };

            document.move_entry(entry_id, target_group_id)?;

            *visible_entries = Rc::new(entries_for_selection(
                document.snapshot(),
                selection,
                search_query,
                last_used,
            ));
        }
        cx.notify();
        self.save_async(cx);
        Ok(())
    }

    /// Create a new group under `parent_id` and select it so the user
    /// lands on the freshly-created (empty) group. This is a real content
    /// mutation, so it uses the full `save_async` path and the change syncs
    /// to the cloud, unlike the cosmetic `toggle_group_expanded`.
    pub fn create_group(
        &mut self,
        parent_id: &str,
        name: &str,
        cx: &mut Context<Self>,
    ) -> Result<String, MutationError> {
        let new_id = {
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
                return Err(MutationError::GroupNotFound);
            };

            let new_id = document.create_group(parent_id, name)?;
            *selection = LibrarySelection::Group(new_id.clone());
            search_query.clear();
            *selected_entry_id = None;
            *selected_strength = None;
            let entries = entries_for_selection(document.snapshot(), selection, "", last_used);
            *visible_entries = Rc::new(entries);
            new_id
        };
        cx.notify();
        self.save_async(cx);
        Ok(new_id)
    }

    /// Rename an existing group. Refreshes `visible_entries` because
    /// `EntryRow::group_path` carries the group names and would
    /// otherwise render stale text in the entry list.
    pub fn rename_group(
        &mut self,
        group_id: &str,
        name: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), MutationError> {
        {
            let VaultStatus::Open {
                document,
                selection,
                search_query,
                visible_entries,
                last_used,
                ..
            } = &mut self.vault
            else {
                return Err(MutationError::GroupNotFound);
            };

            document.rename_group(group_id, name)?;
            *visible_entries = Rc::new(entries_for_selection(
                document.snapshot(),
                selection,
                search_query,
                last_used,
            ));
        }
        cx.notify();
        self.save_async(cx);
        Ok(())
    }

    /// Soft-delete a group: hand off to `VaultDocument::delete_group`,
    /// then if the deleted group was the active selection, snap back to
    /// the root so the entry list isn't pointing at a now-orphaned id.
    /// Move a group and everything under it into the Recycle Bin.
    pub fn delete_group(
        &mut self,
        group_id: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), MutationError> {
        self.run_group_mutation(cx, |document| document.delete_group(group_id))
    }

    /// Lift a deleted group back out of the Recycle Bin, subtree included.
    /// Reachable from the "Deleted groups" section of the Trash view, which
    /// is the only place a trashed group is visible.
    pub fn restore_group(
        &mut self,
        group_id: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), MutationError> {
        self.run_group_mutation(cx, |document| document.restore_group(group_id))
    }

    /// Shared bookkeeping for the group operations that relocate a subtree.
    /// The search query is dropped because the tree it ran against just
    /// changed shape, and a selection pointing at a group that moved into the
    /// bin falls back to the root instead of rendering an empty list with no
    /// explanation.
    fn run_group_mutation<F>(
        &mut self,
        cx: &mut Context<Self>,
        mutate: F,
    ) -> Result<(), MutationError>
    where
        F: FnOnce(&mut VaultDocument) -> Result<(), MutationError>,
    {
        {
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
                return Err(MutationError::GroupNotFound);
            };

            mutate(document)?;

            let snapshot = document.snapshot();
            let selection_is_stale = match &*selection {
                LibrarySelection::Group(id) => {
                    snapshot.find_group(id).is_none_or(|g| g.in_recycle_bin)
                }
                _ => false,
            };
            if selection_is_stale {
                *selection = LibrarySelection::Group(snapshot.root.id.clone());
            }
            search_query.clear();
            let entries = entries_for_selection(snapshot, selection, "", last_used);
            *selected_entry_id = entries.first().map(|e| e.id.clone());
            *selected_strength = selected_entry_id
                .as_deref()
                .and_then(|id| document.strength_for_entry(id));
            *visible_entries = Rc::new(entries);
        }
        cx.notify();
        self.save_async(cx);
        Ok(())
    }

    /// Toggle the favourite-marker on an entry. Mutates the underlying
    /// `Favorite` tag through `VaultDocument`, refreshes the visible
    /// snapshot caches so the star icon updates immediately, and
    /// schedules a background save (which chains into sync if connected).
    pub fn toggle_starred(
        &mut self,
        entry_id: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), MutationError> {
        {
            let VaultStatus::Open {
                document,
                selection,
                search_query,
                visible_entries,
                last_used,
                ..
            } = &mut self.vault
            else {
                return Err(MutationError::EntryNotFound);
            };

            document.toggle_starred(entry_id)?;

            *visible_entries = Rc::new(entries_for_selection(
                document.snapshot(),
                selection,
                search_query,
                last_used,
            ));
        }
        cx.notify();
        self.save_async(cx);
        Ok(())
    }

    /// Shared post-mutation bookkeeping for delete / restore / permanent-delete:
    /// run the mutation, refresh the visible entry list, repoint the selection
    /// if the affected entry was selected, then schedule the autosave.
    fn run_entry_mutation<F>(
        &mut self,
        cx: &mut Context<Self>,
        mutate: F,
        entry_id: &str,
    ) -> Result<(), MutationError>
    where
        F: FnOnce(&mut VaultDocument) -> Result<(), MutationError>,
    {
        {
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
                return Err(MutationError::EntryNotFound);
            };

            mutate(document)?;

            let entries =
                entries_for_selection(document.snapshot(), selection, search_query, last_used);
            if selected_entry_id.as_deref() == Some(entry_id) {
                *selected_entry_id = entries.first().map(|e| e.id.clone());
                *selected_strength = selected_entry_id
                    .as_deref()
                    .and_then(|id| document.strength_for_entry(id));
            }
            *visible_entries = Rc::new(entries);
        }
        cx.notify();
        self.save_async(cx);
        Ok(())
    }
}
