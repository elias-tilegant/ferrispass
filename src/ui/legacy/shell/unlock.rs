//! Unlocking with a password or Touch ID, and confirming the KDF cost.

use super::*;

impl AppShell {
    pub fn submit_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.state.read(cx).pending_unlock_path() else {
            return;
        };
        let keyfile = self.state.read(cx).pending_unlock_keyfile();

        // `Zeroizing` wipes this copy on drop. `VaultDocument` keeps only
        // the derived key, so once the unlock consumes this buffer the
        // password is gone unless Touch ID enrolment stored it.
        let password = Zeroizing::new(self.password_input.read(cx).value().to_string());
        if password.is_empty() && keyfile.is_none() {
            self.state.update(cx, |state, cx| {
                state.set_unlock_error("Enter the master password or pick a key file.", cx)
            });
            self.password_input
                .update(cx, |input, cx| input.focus(window, cx));
            return;
        }

        // Snapshot the "Enable Touch ID" checkbox state *before* we
        // clear the password input - by the time the open task
        // completes the input has been wiped, and the password we
        // need to enrol with is gone. Move it into the spawn closure
        // alongside the path.
        let enroll_after_unlock = self.state.read(cx).pending_biometric_enrollment();

        self.confirm_pending_kdf_cost(cx);
        self.clear_unlock_form(window, cx);
        let attempt = self.state.update(cx, |state, cx| {
            state.begin_open(path.clone(), cx);
            state.open_attempt()
        });

        let state = self.state.downgrade();
        let path_for_task = path.clone();
        let keyfile_for_task = keyfile.clone();
        let password_for_task = password.clone();
        let keyfile_for_enroll = keyfile.clone();
        let open_task = cx.background_spawn(async move {
            let result = KeePassRepository::open(
                &path_for_task,
                &password_for_task,
                keyfile_for_task.as_deref(),
            );

            (path_for_task, result)
        });

        cx.spawn(async move |this, cx| {
            let (path, result) = open_task.await;
            // Mapped here, not rendered raw: the unlock screen used to show
            // the upstream crate's own wording, so a mistyped password read
            // like a corrupt file.
            let kdf_cost = result
                .as_ref()
                .err()
                .and_then(crate::keepass::limits::confirmation_required);
            let result = result.map_err(|error| errors::open_message(&error));
            // Gate enrolment on whether the vault *actually* opened for
            // this path - not merely on a successful KDF. The user may
            // have cancelled or switched vaults between the KDF
            // finishing and this update landing, in which case
            // `finish_open_attempt` no-ops and we must not write this
            // vault's master password to the keychain. The attempt number
            // covers A -> B -> A, which the path alone cannot tell apart.
            let did_open = state
                .update(cx, |state, cx| {
                    if !state.open_attempt_is_current(&path, attempt) {
                        return false;
                    }
                    if let Some(cost) = kdf_cost {
                        state.note_kdf_check(path.clone(), Some(cost), cx);
                    }
                    state.finish_open_attempt(path.clone(), result, cx)
                })
                .unwrap_or(false);
            if did_open && enroll_after_unlock {
                // Vault is `Open` now, so `set_unlock_error` would be a
                // silent no-op; surface enrolment failure as an in-app
                // notification instead.
                let enroll_result = state.update(cx, |state, cx| {
                    state.complete_biometric_enrollment(path, keyfile_for_enroll, &password, cx)
                });
                if let Ok(Err(err)) = enroll_result {
                    let _ = this.update_in(cx, |_shell, window, cx| {
                        window.push_notification(
                            format!(
                                "Touch ID enrolment failed: {err}. Vault is open; \
                                 try enabling Touch ID again on the next unlock."
                            ),
                            cx,
                        );
                    });
                }
            }
            // `password` (Zeroizing) is wiped here as the closure drops.
        })
        .detach();
    }

    /// Off-thread Touch ID unlock. Mirrors `submit_password` but:
    /// (1) gets the password from the OS Keychain rather than the
    /// text input, and (2) maps a `BiometricError` back to a state
    /// transition the Unlock screen can render
    /// (`BiometricAttempt::Error`).
    ///
    /// Note on the cleartext password: it is held in a `Zeroizing` buffer
    /// inside the background task and fed straight into
    /// `KeePassRepository::open`. It does not round-trip through `AppState`
    /// as a bare value, and the opened `VaultDocument` keeps only the derived
    /// key, so nothing retains the password for the life of the session.
    pub fn submit_biometric_unlock(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        // A click (or the shortcut) is the user's answer to a cost notice,
        // like "Allow and unlock" is.
        self.confirm_pending_kdf_cost(cx);
        self.start_biometric_unlock(false, cx);
    }

    /// The Touch ID prompt the unlock screen raises on its own. Never an
    /// answer to a cost notice: if one is up, or the header check has not
    /// come back yet, it stays quiet and leaves the decision to the user.
    fn start_automatic_biometric_unlock(&mut self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        if !state.kdf_check_settled_for_pending() || state.kdf_confirmation_for_pending().is_some()
        {
            return;
        }
        self.start_biometric_unlock(true, cx);
    }

    /// `automatic`: raised by the unlock screen itself rather than by a
    /// click. Only such an attempt is re-run when macOS withdraws it; a
    /// withdrawn click is left for the user to click again.
    fn start_biometric_unlock(&mut self, automatic: bool, cx: &mut Context<Self>) {
        let Some(launch) = self
            .state
            .update(cx, |state, cx| state.begin_biometric_unlock(cx))
        else {
            return;
        };

        // begin_open clears VaultStatus to Opening{} only on the
        // *successful* path; for a Touch ID retrieve we don't want
        // to flip away from AwaitingPassword until the OS prompt
        // actually resolves. So we postpone begin_open until after
        // the retrieve returns.
        let state = self.state.downgrade();
        let id = launch.id.clone();
        let path = launch.path.clone();
        let keyfile = launch.keyfile.clone();
        let store = launch.store;
        let generation = launch.generation;
        let prompt = format!("Unlock {}", file_name_for_prompt(&path));
        // Read the per-app preference once, here on the foreground.
        // The background task gets a plain bool - no AppSettings
        // reference travels into the spawn closure.
        let options = crate::biometric::RetrieveOptions {
            allow_device_passcode: self.settings.biometric_allow_passcode_fallback,
        };

        let retrieve_task = cx.background_spawn(async move {
            let outcome = store.retrieve(&id, &prompt, options);
            match outcome {
                Ok(password) => {
                    let result = KeePassRepository::open(&path, &password, keyfile.as_deref());
                    // `password` (Zeroizing) drops here, wiping the
                    // transient buffer.
                    BiometricUnlockOutcome::Open {
                        path,
                        result,
                        generation,
                    }
                }
                Err(err) => BiometricUnlockOutcome::Failed {
                    path,
                    err,
                    generation,
                },
            }
        });

        cx.spawn(async move |this, cx| {
            let outcome = retrieve_task.await;
            // Decided before the state update below, which clears the
            // attempt and with it the generation this compares against.
            let interrupted = automatic
                && match &outcome {
                    BiometricUnlockOutcome::Failed {
                        err: crate::biometric::BiometricError::Interrupted,
                        generation,
                        ..
                    } => state
                        .read_with(cx, |state, _| {
                            state.biometric_unlock_is_current(*generation)
                        })
                        .unwrap_or(false),
                    _ => false,
                };
            let _ = state.update(cx, |state, cx| match outcome {
                BiometricUnlockOutcome::Open {
                    path,
                    result,
                    generation,
                } => {
                    // A Touch ID prompt the user satisfied *after*
                    // cancelling / re-arming (even for the same vault
                    // path) carries a stale generation; ignore it so
                    // it can't drive the current screen.
                    if !state.biometric_unlock_is_current(generation) {
                        state.clear_biometric_attempt_public();
                        return;
                    }
                    let result = result.map_err(|error| {
                        Self::note_kdf_confirmation(state, &path, &error, cx);
                        errors::open_message(&error)
                    });
                    state.begin_open(path.clone(), cx);
                    let _ = state.finish_open_attempt(path, result, cx);
                }
                BiometricUnlockOutcome::Failed {
                    path,
                    err,
                    generation,
                } => {
                    state.fail_biometric_unlock(path, generation, err, cx);
                }
            });
            if interrupted {
                let _ = this.update_in(cx, |shell, window, cx| {
                    shell.note_automatic_biometric_interruption(window, cx);
                });
            }
        })
        .detach();
    }

    /// macOS withdrew the automatic prompt. With the window in the
    /// background, the next activation re-runs it. With the window already
    /// back in front, that activation has come and gone before this report,
    /// so re-run it now, once.
    fn note_automatic_biometric_interruption(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !window.is_window_active() {
            self.automatic_biometric_interrupted = true;
            return;
        }
        if std::mem::replace(&mut self.automatic_biometric_rearmed, true) {
            return;
        }
        self.automatic_biometric_attempted_for = None;
        self.schedule_automatic_biometric_unlock(window, cx);
    }

    pub(super) fn schedule_automatic_biometric_unlock(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (path, overlay_active, awaiting_password) = {
            let state = self.state.read(cx);
            // Hold the automatic Touch ID prompt until the header check is
            // in, and skip it while a cost is waiting for confirmation: the
            // user has to read that first, and a fingerprint prompt on top
            // of it would both hide it and decide it for them.
            let kdf_ready = state.kdf_check_settled_for_pending()
                && state.kdf_confirmation_for_pending().is_none();
            (
                state
                    .automatic_biometric_unlock_path(
                        self.settings.biometric_allow_passcode_fallback,
                    )
                    .filter(|_| kdf_ready),
                state.overlay().is_active(),
                state.pending_unlock_path().is_some(),
            )
        };

        match path {
            // Only with the window in front. From the background macOS
            // either refuses the prompt or shows it over whatever the user
            // is doing; both used to spend the one automatic attempt, so
            // after an auto-lock Touch ID never came up on its own.
            Some(path)
                if !overlay_active
                    && window.is_window_active()
                    && self.automatic_biometric_attempted_for.as_ref() != Some(&path) =>
            {
                self.automatic_biometric_attempted_for = Some(path);
                // Never mutate AppState while rendering. Defer the launch to
                // the next foreground turn, where begin_biometric_unlock
                // revalidates every eligibility condition before prompting.
                // Not the SubmitBiometricUnlock action: that one is the
                // user's click and counts as allowing a cost notice.
                let shell = cx.entity();
                window.defer(cx, move |_window, cx| {
                    shell.update(cx, |shell, cx| shell.start_automatic_biometric_unlock(cx));
                });
            }
            None if !awaiting_password => {
                self.automatic_biometric_attempted_for = None;
                self.automatic_biometric_interrupted = false;
                self.automatic_biometric_rearmed = false;
            }
            _ => {}
        }
    }

    /// Read the pending vault's header off the foreground thread and record
    /// whether its Argon2 cost needs the user's confirmation, once per visit
    /// to the unlock screen. The file may be an iCloud placeholder that
    /// downloads on first read, so this never runs on the UI thread.
    pub(super) fn sync_kdf_check(&mut self, cx: &mut Context<Self>) {
        let pending = self.state.read(cx).pending_unlock_path();
        let Some(path) = pending else {
            self.kdf_check_started_for = None;
            return;
        };
        if self.kdf_check_started_for.as_ref() == Some(&path) {
            return;
        }
        self.kdf_check_started_for = Some(path.clone());
        let generation = self.state.update(cx, |state, cx| state.begin_kdf_check(cx));
        let state = self.state.downgrade();
        let check_path = path.clone();
        let check =
            cx.background_spawn(
                async move { KeePassRepository::pending_kdf_confirmation(&check_path) },
            );
        cx.spawn(async move |_, cx| {
            let cost = check.await;
            let _ = state.update(cx, |state, cx| {
                state.finish_kdf_check(generation, path, cost, cx)
            });
        })
        .detach();
    }

    /// Starting an unlock while the screen shows a cost notice is the
    /// confirmation: the notice says so, and the primary button reads
    /// "Allow and unlock". Remembered as the exact values, so the next
    /// unlock of this vault asks nothing.
    fn confirm_pending_kdf_cost(&mut self, cx: &mut Context<Self>) {
        let (path, cost) = {
            let state = self.state.read(cx);
            match (
                state.pending_unlock_path(),
                state.kdf_confirmation_for_pending(),
            ) {
                (Some(path), Some(cost)) => (path, cost),
                _ => return,
            }
        };
        self.remember_kdf_cost(cost, cx);
        // Answered: a wrong password after this must not bring the notice
        // (and the blocked automatic Touch ID) back for a cost already allowed.
        self.state
            .update(cx, |state, cx| state.note_kdf_check(path, None, cx));
    }

    /// Allow `cost` for the rest of this process at once, and in
    /// settings.json for later launches and the CLI.
    fn remember_kdf_cost(&mut self, cost: KdfCost, cx: &mut Context<Self>) {
        crate::keepass::limits::confirm_kdf_cost(cost);
        if !self.settings.confirmed_kdf_costs.contains(&cost) {
            let mut settings = self.settings.clone();
            settings.confirmed_kdf_costs.push(cost);
            self.update_settings(settings, cx);
        }
    }

    /// "Allow and sync" on the Settings → Sync notice: remember the remote's
    /// cost and run the sync again, interactively, so a real conflict behind
    /// it can open the Conflict screen.
    pub(crate) fn confirm_remote_kdf_cost(&mut self, cx: &mut Context<Self>) {
        let Some((target, cost)) = self
            .state
            .update(cx, |state, cx| state.take_remote_kdf_confirmation(cx))
        else {
            return;
        };
        self.remember_kdf_cost(cost, cx);
        self.state
            .update(cx, |state, cx| state.sync_now_for_path(&target, cx));
    }

    /// "Not now": drop the notice. The sync stays failed, and the next
    /// attempt asks again.
    pub(crate) fn dismiss_remote_kdf_cost(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| {
            state.take_remote_kdf_confirmation(cx);
        });
    }

    /// A failed open that turns out to be a cost question (the file changed
    /// after the pre-check, or the pre-check had not landed yet) puts the
    /// notice on the unlock screen instead of an error the user cannot act on.
    fn note_kdf_confirmation(
        state: &mut AppState,
        path: &Path,
        error: &keepass::db::DatabaseOpenError,
        cx: &mut Context<AppState>,
    ) {
        if let Some(cost) = crate::keepass::limits::confirmation_required(error) {
            state.note_kdf_check(path.to_path_buf(), Some(cost), cx);
        }
    }
}
