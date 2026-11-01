//! Sync history, save and sync status, the connect flow, TOTP for the
//! shown entry, group expansion, and favicon downloads.

use super::*;

impl AppState {
    pub fn sync_history(&self) -> &[SyncHistoryEntry] {
        &self.sync_history
    }

    /// Remove one row from the active vault's session-scoped sync activity
    /// log. This never mutates the KeePass database or its entry history.
    pub fn remove_sync_history_entry(
        &mut self,
        entry: &SyncHistoryEntry,
        cx: &mut Context<Self>,
    ) -> bool {
        let removed = sync_history::remove_one(&mut self.sync_history, entry);
        if removed {
            cx.notify();
        }
        removed
    }

    /// Append already-computed history entries to whichever slot
    /// (active vault or parked session) currently owns `target`. Mirrors
    /// the routing done by `apply_sync_status` / `with_sync_binding_mut_for`
    /// so a sync that completes after the user switched away still logs
    /// against the vault it actually changed. Vaults that were locked
    /// between dispatch and callback are dropped silently - there's no
    /// active session to surface the history against.
    pub(super) fn append_sync_history_for(
        &mut self,
        target: &Path,
        entries: Vec<SyncHistoryEntry>,
    ) {
        if entries.is_empty() {
            return;
        }
        if self.is_active_vault(target) {
            sync_history::append_capped(&mut self.sync_history, entries);
        } else if let Some(parked) = self.parked.get_mut(target) {
            sync_history::append_capped(&mut parked.sync_history, entries);
        }
    }

    pub fn save_status(&self) -> &SaveStatus {
        &self.save_status
    }

    pub fn sync_status(&self) -> &SyncStatus {
        &self.sync_status
    }

    /// What the active vault's sync is doing, as a `Copy` summary. Callers
    /// that only need to branch use this instead of cloning `SyncStatus`,
    /// whose `Conflict` variant owns two decrypted databases.
    pub fn sync_activity(&self) -> SyncActivity {
        SyncActivity::of(&self.sync_status)
    }

    pub fn sync_binding(&self) -> Option<&SyncBinding> {
        self.sync.as_ref()
    }

    pub fn connect_flow(&self) -> Option<&ConnectFlow> {
        self.connect_flow.as_ref()
    }

    /// Reset the Connect overlay to its initial step. Called when the user
    /// opens Connect from Welcome.
    pub fn begin_connect_flow(&mut self, cx: &mut Context<Self>) {
        if matches!(self.vault, VaultStatus::LockedPendingSave) {
            return;
        }
        self.connect_operations.advance();
        // Normal (first-time) Connect: make sure no stale reconnect target
        // survives from an abandoned reconnect, or the token poll would
        // rebind instead of showing the file picker.
        self.reconnect_target = None;
        self.connect_flow = Some(ConnectFlow::PickProvider);
        cx.notify();
    }

    /// Drop any in-progress Connect flow state. Called by Cancel + on
    /// successful completion.
    pub fn clear_connect_flow(&mut self, cx: &mut Context<Self>) {
        self.connect_operations.advance();
        if self.connect_flow.is_some() {
            self.connect_flow = None;
            cx.notify();
        }
    }

    /// Replace the current connect flow step. Used by the Connect overlay's
    /// Back / provider-pick buttons.
    pub fn connect_flow_set(&mut self, flow: ConnectFlow, cx: &mut Context<Self>) {
        if matches!(self.vault, VaultStatus::LockedPendingSave) {
            return;
        }
        self.connect_operations.advance();
        self.connect_flow = Some(flow);
        cx.notify();
    }

    /// Compute the live TOTP code for the currently-selected entry, if any.
    /// Recomputed on every render (cheap, ~µs); the per-second AppShell tick
    /// triggers `cx.notify` which causes the detail panel to re-call this.
    /// The entry the detail panel is actually showing.
    ///
    /// `selected_entry_id` can point at an entry that is no longer in the
    /// visible list: `move_entry` deliberately leaves it dangling because
    /// `vault_browser` falls back to the first visible row. Anything that
    /// read the raw id instead disagreed with the panel around it, so after
    /// dragging the selected entry into another group the panel showed entry
    /// X while the TOTP box showed Y's code and clicking it copied Y's.
    pub(super) fn shown_entry_id(&self) -> Option<&str> {
        let VaultStatus::Open {
            selected_entry_id,
            visible_entries,
            ..
        } = &self.vault
        else {
            return None;
        };
        match selected_entry_id.as_deref() {
            Some(id) if visible_entries.iter().any(|entry| entry.id == id) => Some(id),
            _ => visible_entries.first().map(|entry| entry.id.as_str()),
        }
    }

    pub fn totp_for_selected_entry(&self) -> Option<OtpDisplay> {
        let id = self.shown_entry_id()?;
        let VaultStatus::Open { document, .. } = &self.vault else {
            return None;
        };
        document.totp_for_entry(id)
    }

    /// Flip the expanded/collapsed state of a sidebar group. Reads the
    /// current flag from the snapshot, writes the inverse via the
    /// document, and queues a *local-only* save so the change is
    /// durable across restarts without firing a cloud-sync push for
    /// every chevron click. The flag still rides out to the cloud
    /// piggybacking on the next real mutation's save_async. No-ops
    /// silently if the group has vanished mid-flight (rare race when
    /// the user clicks while a sync overwrites the tree).
    pub fn toggle_group_expanded(&mut self, group_id: &str, cx: &mut Context<Self>) {
        let VaultStatus::Open { document, .. } = &mut self.vault else {
            return;
        };
        let Some(group) = document.snapshot().find_group(group_id) else {
            return;
        };
        let new_value = !group.is_expanded;
        if document.set_group_expanded(group_id, new_value).is_ok() {
            cx.notify();
            self.save_async_local_only(cx);
        }
    }

    /// Walk the open vault, find every entry with a non-empty URL and no
    /// existing custom icon, and pull a favicon for each from DuckDuckGo.
    /// Successful fetches are written into the database as Custom Icons;
    /// once the loop is done we trigger a single `save_async` so the
    /// flushed bytes ride out via the normal save → sync path.
    ///
    /// Sequential by design: a typical vault has 30-200 entries with
    /// URLs, and DDG's icon service is fast - running these in parallel
    /// would mostly just shave a few seconds while burning more cache
    /// quota. Keeping it serial also gives us a clean progress label
    /// without coordinating shared mutable state across workers.
    ///
    /// Re-entrancy: if a run is already in flight, additional triggers
    /// are dropped (the UI also disables its button).
    pub fn start_favicon_download(&mut self, cx: &mut Context<Self>) {
        if self.favicon_status.is_running() {
            return;
        }
        let VaultStatus::Open { path, document, .. } = &self.vault else {
            return;
        };
        // A run over a few hundred entries is minutes of sequential HTTP. Bind
        // it to the vault that started it: without this the icons, and the
        // save and cloud push that follow, landed on whichever vault the user
        // had switched to meanwhile.
        let vault_path = path.clone();
        let Some(session_id) = self.active_vault_session_id else {
            return;
        };

        // Snapshot the (id, url) pairs up front so the spawned task
        // doesn't have to re-borrow the snapshot every iteration. We
        // skip entries that already have a custom icon - re-running
        // shouldn't blow away user-curated icons. Deleted entries are
        // skipped too; fetching an icon for something in the Trash would
        // dirty the vault for nothing.
        let targets: Vec<(String, String)> = document
            .snapshot()
            .live_entries()
            .into_iter()
            .filter(|entry| !entry.url.trim().is_empty())
            .filter(|entry| entry.favicon.image.is_none())
            .map(|entry| (entry.id.clone(), entry.url.clone()))
            .collect();

        let total = targets.len();
        if total == 0 {
            self.favicon_status = FaviconDownloadStatus::Finished {
                succeeded: 0,
                total: 0,
            };
            cx.notify();
            return;
        }

        self.favicon_status = FaviconDownloadStatus::Running {
            done: 0,
            total,
            succeeded: 0,
        };
        cx.notify();

        cx.spawn(async move |this, cx| {
            let mut succeeded = 0usize;
            for (idx, (entry_id, url)) in targets.into_iter().enumerate() {
                // Each fetch off the UI thread. The favicon path is a
                // synchronous wrapper around the shared client, so calling it
                // here would block the renderer for the whole round trip.
                let url_for_task = url.clone();
                let bytes_result = cx
                    .background_spawn(async move { crate::favicon::fetch_favicon(&url_for_task) })
                    .await;

                let still_ours = this
                    .update(cx, |state, cx| {
                        if !state.vault_session_is_current(&vault_path, session_id) {
                            return false;
                        }
                        if let Ok(bytes) = bytes_result
                            && let Some(document) =
                                state.document_mut_for_session(&vault_path, session_id)
                        {
                            // Errors here mean the entry vanished
                            // mid-run (e.g. user deleted it) - fine to
                            // silently skip.
                            if document.set_entry_custom_icon(&entry_id, bytes).is_ok() {
                                succeeded += 1;
                                // Mark it dirty now rather than at the end of
                                // the run: an auto-lock partway through used to
                                // take the clean path and throw the icons away.
                                state.mark_vault_session_unpersisted(session_id);
                            }
                        }
                        state.favicon_status = FaviconDownloadStatus::Running {
                            done: idx + 1,
                            total,
                            succeeded,
                        };
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !still_ours {
                    return;
                }
            }

            let _ = this.update(cx, |state, cx| {
                state.favicon_status = FaviconDownloadStatus::Finished { succeeded, total };
                cx.notify();
                // Persist whichever icons we managed to land, against the
                // session that fetched them.
                if succeeded > 0 {
                    state.request_save_for_session(vault_path.clone(), session_id, true, cx);
                }
            });
        })
        .detach();
    }
}
