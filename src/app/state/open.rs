//! Opening and unlocking a vault: the unlock form, the open attempt,
//! recents, the KDF check, and biometric unlock.
//!
//! The biometric methods are grouped so the read path (`biometric_*`-getters)
//! and the write path (`begin/finish/complete`) are visible together.
//! Production wiring (Touch ID prompt + background tasks) lives in
//! `AppShell`; this module only ever sees the synchronous state transitions
//! and the already-resolved password buffer.

use super::*;

impl AppState {
    pub fn set_unlock_keyfile(&mut self, keyfile: Option<PathBuf>, cx: &mut Context<Self>) {
        if let VaultStatus::AwaitingPassword {
            keyfile: existing,
            error,
            ..
        } = &mut self.vault
        {
            *existing = keyfile;
            *error = None;
            cx.notify();
        }
    }

    pub fn pending_unlock_keyfile(&self) -> Option<PathBuf> {
        match &self.vault {
            VaultStatus::AwaitingPassword { keyfile, .. } => keyfile.clone(),
            _ => None,
        }
    }

    pub fn set_unlock_error(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        if let VaultStatus::AwaitingPassword { error, .. } = &mut self.vault {
            *error = Some(message.into());
            cx.notify();
        }
    }

    pub fn begin_open(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if matches!(self.vault, VaultStatus::LockedPendingSave) {
            return;
        }
        if self.switch_to_unlocked(&path, cx) {
            return;
        }
        self.rotate_auto_type_context();
        // If we're transitioning from a currently-open vault directly into
        // unlocking another one (Welcome-recent → submit_password while a
        // different vault is already Open), park the active one first so it
        // doesn't get overwritten silently.
        self.park_active();
        self.vault = VaultStatus::Opening { path };
        self.open_attempt += 1;
        cx.notify();
    }

    /// Number of the newest open attempt, taken right after `begin_open`.
    pub fn open_attempt(&self) -> u64 {
        self.open_attempt
    }

    /// Whether `attempt` is still the open in progress for `path`.
    pub fn open_attempt_is_current(&self, path: &Path, attempt: u64) -> bool {
        attempt == self.open_attempt
            && matches!(&self.vault, VaultStatus::Opening { path: current } if current == path)
    }

    /// Returns `true` iff this call actually transitioned the active
    /// slot to `Open` for `path`. Callers that fire post-unlock
    /// side-effects which themselves persist secrets (e.g. Touch ID
    /// enrolment) **must** gate on this - a `true` KDF result is not
    /// enough, because the user may have cancelled or switched vaults
    /// between the KDF finishing and this update landing, in which
    /// case we no-op and the vault must stay closed.
    #[must_use]
    pub fn finish_open_attempt(
        &mut self,
        path: PathBuf,
        result: Result<VaultDocument, String>,
        cx: &mut Context<Self>,
    ) -> bool {
        if !matches!(&self.vault, VaultStatus::Opening { path: active } if active == &path) {
            return false;
        }
        self.rotate_auto_type_context();

        // Track whether the unlock succeeded so we can fire post-open
        // side-effects (recents push, sync rebind) below - they need a
        // `&mut self` borrow that conflicts with the match arm.
        let mut opened_path: Option<PathBuf> = None;
        let mut opened_session_id: Option<VaultSessionId> = None;
        self.vault = match result {
            Ok(document) => {
                let snapshot = document.snapshot();
                let selection = LibrarySelection::Group(snapshot.root.id.clone());
                let selected_entry_id = snapshot.root.entries.first().map(|entry| entry.id.clone());
                let visible_entries = Rc::new(entries_for_selection(
                    snapshot,
                    &selection,
                    "",
                    &HashMap::new(),
                ));
                let selected_strength = selected_entry_id
                    .as_deref()
                    .and_then(|id| document.strength_for_entry(id));

                opened_path = Some(path.clone());
                opened_session_id = Some(VaultSessionId::next());
                self.last_unlock_at = Some(std::time::SystemTime::now());
                VaultStatus::Open {
                    path,
                    document: Box::new(document),
                    selection,
                    selected_entry_id,
                    search_query: String::new(),
                    visible_entries,
                    selected_strength,
                    last_used: HashMap::new(),
                }
            }
            Err(message) => VaultStatus::AwaitingPassword {
                path: path.clone(),
                keyfile: crate::keepass::KeePassRepository::suggested_keyfile(&path),
                error: Some(message),
            },
        };
        self.active_vault_session_id = opened_session_id;
        // Either branch ends the in-flight Touch ID attempt for this
        // path: on success the unlock screen is gone anyway, on
        // failure we replace any stale BiometricAttempt::Error with
        // the fresh password-auth error.
        self.clear_biometric_attempt();
        cx.notify();

        let did_open = opened_path.is_some();
        if let Some(opened) = opened_path {
            // Opportunistic hygiene: clear launch payloads orphaned by a
            // crash that the startup sweep spared (it only removes files
            // older than 60 s, so a crash + quick relaunch slips through).
            // `sweep_stale` - not `purge_all` - so this session's own
            // pending launches survive; anything past 60 s is a guaranteed
            // orphan because live payload TTLs cap at 60 s. Cheap: one
            // read_dir over a near-empty private tempdir.
            crate::launch::sweeper::sweep_stale(std::time::Duration::from_secs(60));
            // Remember the vault for next launch's auto-resume + the
            // Welcome screen's Recents list.
            self.push_recent(opened.clone(), cx);
            if self.install_pending_sync_for(&opened) {
                cx.notify();
                return did_open;
            }
            // If a sync config exists for this path, rebuild the
            // SyncBinding using the keychain refresh token. No-op for
            // local-only vaults.
            self.try_restore_sync_binding(opened, cx);
        }
        did_open
    }

    /// Prepend `path` to the in-memory recents list (dedup + truncate),
    /// then schedule an atomic write to disk in the background. Failures
    /// are intentionally swallowed - the next successful open will retry,
    /// and we don't want a transient disk error to surface as a UI toast.
    pub(super) fn push_recent(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        recents::push_front_in(&mut self.recents, path, recents::MAX_RECENTS);
        let snapshot = recents::RecentVaults {
            entries: self.recents.clone(),
        };
        cx.background_spawn(async move {
            let _ = recents::save(&snapshot);
        })
        .detach();
    }

    pub fn pending_unlock_path(&self) -> Option<PathBuf> {
        match &self.vault {
            VaultStatus::AwaitingPassword { path, .. } => Some(path.clone()),
            _ => None,
        }
    }

    /// Path of the vault the user is currently looking at, regardless of
    /// state (Open / Opening / AwaitingPassword). `None` on the Welcome
    /// screen. Used by the vault switcher to mark the active row.
    pub fn current_vault_path(&self) -> Option<PathBuf> {
        match &self.vault {
            VaultStatus::Open { path, .. }
            | VaultStatus::Opening { path }
            | VaultStatus::AwaitingPassword { path, .. } => Some(path.clone()),
            VaultStatus::Empty | VaultStatus::LockedPendingSave => None,
            VaultStatus::Error { path, .. } => path.clone(),
        }
    }

    pub fn unlock_prompt(&self) -> Option<UnlockPrompt> {
        match &self.vault {
            VaultStatus::AwaitingPassword {
                path,
                keyfile,
                error,
            } => Some(UnlockPrompt {
                path: path.clone(),
                file_name: file_name(path),
                display_path: path.display().to_string(),
                keyfile: keyfile.clone(),
                error: error.clone(),
                kdf_confirmation: self.kdf_confirmation_for_pending(),
            }),
            _ => None,
        }
    }

    /// The Argon2 cost the pending vault still needs confirmed, if the
    /// header pre-check found one for exactly this path.
    pub fn kdf_confirmation_for_pending(&self) -> Option<KdfCost> {
        let pending = self.pending_unlock_path()?;
        match &self.kdf_check {
            Some((path, cost)) if *path == pending => *cost,
            _ => None,
        }
    }

    /// Whether the header pre-check has finished for the pending vault.
    /// The automatic Touch ID attempt waits for it: prompting for a
    /// fingerprint and then stopping at a cost question is a wasted prompt.
    pub fn kdf_check_settled_for_pending(&self) -> bool {
        match (&self.kdf_check, self.pending_unlock_path()) {
            (Some((path, _)), Some(pending)) => *path == pending,
            _ => false,
        }
    }

    /// The remote cost waiting for confirmation on the active vault's sync.
    pub fn remote_kdf_confirmation_for_active(&self) -> Option<KdfCost> {
        let active = self.current_vault_path()?;
        self.remote_kdf_confirmations.get(&active).copied()
    }

    /// Hand the pending remote cost to the caller and forget it, for either
    /// answer: confirmed, it is persisted and the sync re-runs; declined, the
    /// sync stays failed until the next attempt asks again.
    pub fn take_remote_kdf_confirmation(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Option<(PathBuf, KdfCost)> {
        let active = self.current_vault_path()?;
        let cost = self.remote_kdf_confirmations.remove(&active)?;
        cx.notify();
        Some((active, cost))
    }

    /// Start a header pre-check. The returned generation goes back into
    /// `finish_kdf_check`; any later start or direct write supersedes it.
    pub fn begin_kdf_check(&mut self, cx: &mut Context<Self>) -> u64 {
        // A result from an earlier visit to the same vault is not an answer
        // for this one: until the fresh read lands, nothing counts as settled.
        if self.kdf_check.take().is_some() {
            cx.notify();
        }
        self.kdf_check_generation += 1;
        self.kdf_check_generation
    }

    /// Record a pre-check result, unless something newer already has.
    pub fn finish_kdf_check(
        &mut self,
        generation: u64,
        path: PathBuf,
        cost: Option<KdfCost>,
        cx: &mut Context<Self>,
    ) {
        if generation == self.kdf_check_generation {
            self.set_kdf_check(path, cost, cx);
        }
    }

    /// Record an answer that is more current than any pre-check in flight:
    /// a failed open that turned out to be a cost question, or a cost the
    /// user just allowed (`None`: nothing left to ask).
    ///
    /// Only while `path` is still the vault being unlocked: an open for A
    /// that fails after the user moved on to B must not cancel B's check.
    pub fn note_kdf_check(&mut self, path: PathBuf, cost: Option<KdfCost>, cx: &mut Context<Self>) {
        if !self.is_unlocking(&path) {
            return;
        }
        self.kdf_check_generation += 1;
        self.set_kdf_check(path, cost, cx);
    }

    pub(super) fn is_unlocking(&self, path: &Path) -> bool {
        matches!(
            &self.vault,
            VaultStatus::Opening { path: current }
                | VaultStatus::AwaitingPassword { path: current, .. }
                if current == path
        )
    }

    fn set_kdf_check(&mut self, path: PathBuf, cost: Option<KdfCost>, cx: &mut Context<Self>) {
        let next = Some((path, cost));
        if self.kdf_check != next {
            self.kdf_check = next;
            cx.notify();
        }
    }

    pub fn biometric_store(&self) -> Arc<dyn BiometricStore> {
        Arc::clone(&self.biometric)
    }

    pub fn biometric_registry(&self) -> &BiometricRegistry {
        &self.biometric_registry
    }

    pub fn biometric_attempt(&self) -> &BiometricAttempt {
        &self.biometric_attempt
    }

    pub fn pending_biometric_enrollment(&self) -> bool {
        self.pending_biometric_enrollment
    }

    /// Toggle for the "Enable Touch ID" checkbox on the Unlock screen.
    /// Pure state flip - the actual enrolment happens in
    /// `complete_biometric_enrollment` after the password unlock succeeds.
    pub fn set_pending_biometric_enrollment(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.pending_biometric_enrollment == on {
            return;
        }
        self.pending_biometric_enrollment = on;
        cx.notify();
    }

    /// Enrolment info for the *currently pending* vault, if any. Drives
    /// whether the Unlock screen shows the Touch ID button or the
    /// "Enable Touch ID" checkbox.
    pub fn biometric_for_pending(&self) -> Option<&BiometricEnrollment> {
        match &self.vault {
            VaultStatus::AwaitingPassword { path, .. } => self.biometric_registry.get(path),
            _ => None,
        }
    }

    /// Vault eligible for an automatic biometric attempt right now. Mirrors
    /// the Unlock screen's manual-button policy: an enrolled vault may prompt
    /// when Touch ID is directly reachable, or when the user enabled device
    /// authentication fallback. The latter keeps automatic unlock useful in
    /// clamshell mode, where macOS reports the built-in sensor unavailable but
    /// can still present its Touch ID / account-password authentication sheet.
    pub fn automatic_biometric_unlock_path(&self, allow_device_passcode: bool) -> Option<PathBuf> {
        let path = match &self.vault {
            VaultStatus::AwaitingPassword { path, .. } => path,
            _ => return None,
        };
        (matches!(self.biometric_attempt, BiometricAttempt::Idle)
            && self.biometric_registry.get(path).is_some()
            && (self.biometric.is_available() || allow_device_passcode))
            .then(|| path.clone())
    }

    pub fn begin_biometric_unlock(&mut self, cx: &mut Context<Self>) -> Option<BiometricLaunch> {
        // Only valid while sitting on the Unlock screen for a vault
        // that has an enrolment. Anything else is the UI dispatching
        // an action it shouldn't have offered - bail silently.
        let (path, keyfile) = match &self.vault {
            VaultStatus::AwaitingPassword { path, keyfile, .. } => (path.clone(), keyfile.clone()),
            _ => return None,
        };
        let enrollment = self.biometric_registry.get(&path)?.clone();
        // Prefer the keyfile the user had selected at enrolment time
        // over whatever's currently in the Unlock form - the saved
        // path is the one that successfully decrypted the vault back
        // then. Falls back to the current pending value if absent
        // (matches the back-compat path for older enrolments).
        let resolved_keyfile = enrollment.keyfile.clone().or(keyfile);
        // Stamp a fresh generation; any earlier in-flight attempt is
        // now stale and its late resolution will be ignored.
        self.biometric_generation = self.biometric_generation.wrapping_add(1);
        let generation = self.biometric_generation;
        self.biometric_attempt = BiometricAttempt::InFlight {
            path: path.clone(),
            generation,
        };
        // Clear any prior unlock error so the screen doesn't show two
        // contradictory messages at once.
        if let VaultStatus::AwaitingPassword { error, .. } = &mut self.vault {
            *error = None;
        }
        cx.notify();
        Some(BiometricLaunch {
            id: enrollment.id,
            path,
            keyfile: resolved_keyfile,
            store: Arc::clone(&self.biometric),
            generation,
        })
    }

    /// `true` iff `generation` matches the attempt currently in flight.
    /// The AppShell success path gates `begin_open` on this so a stale
    /// Touch ID prompt that resolves after the user cancelled and
    /// re-armed for the same vault can't drive the newer screen.
    pub fn biometric_unlock_is_current(&self, generation: u64) -> bool {
        matches!(
            &self.biometric_attempt,
            BiometricAttempt::InFlight { generation: g, .. } if *g == generation
        )
    }

    /// Called from the AppShell background task when the Touch ID
    /// retrieval errors out. Success path skips this - it feeds the
    /// password directly into `begin_open` + `finish_open_attempt`
    /// without going back through state. `generation` is the launch's
    /// stamp; a mismatch means a newer attempt superseded this one and
    /// we drop the late error entirely.
    pub fn fail_biometric_unlock(
        &mut self,
        path: PathBuf,
        generation: u64,
        error: BiometricError,
        cx: &mut Context<Self>,
    ) {
        // Superseded by a newer attempt (or already cleared)? Ignore.
        if !self.biometric_unlock_is_current(generation) {
            return;
        }
        // If the user has moved on (different vault now pending, vault
        // got unlocked another way), discard the late-arriving error so
        // we don't render a stale "Touch ID cancelled" against the new
        // screen.
        let still_active = matches!(
            &self.vault,
            VaultStatus::AwaitingPassword { path: p, .. } if p == &path
        );
        if !still_active {
            self.biometric_attempt = BiometricAttempt::Idle;
            return;
        }
        // The keychain item is gone or unusable: drop the enrolment so
        // the screen falls back to the "Enable Touch ID" checkbox. Only
        // remove the registry pointer once the keychain delete is
        // confirmed - an unconfirmed delete keeps the entry (re-`Forget`
        // -able) rather than orphaning the stored password.
        if matches!(
            error,
            BiometricError::Invalidated | BiometricError::NotFound
        ) && let Some(entry) = self.biometric_registry.get(&path).cloned()
            && self.biometric.forget(&entry.id).is_ok()
        {
            self.biometric_registry.remove(&path);
            let _ = crate::biometric::registry::save(&self.biometric_registry);
        }
        // macOS took the prompt away (the window lost focus): nothing to
        // report, the user never answered. The shell tries again once the
        // window is in front.
        if error == BiometricError::Interrupted {
            self.biometric_attempt = BiometricAttempt::Idle;
            cx.notify();
            return;
        }
        self.biometric_attempt = BiometricAttempt::Error {
            path,
            message: error.to_string(),
        };
        cx.notify();
    }

    /// Reset to `Idle` once the unlock succeeded (or the user
    /// navigated away). Called from `finish_open_attempt` and from
    /// `lock_vault` for hygiene.
    pub(super) fn clear_biometric_attempt(&mut self) {
        self.biometric_attempt = BiometricAttempt::Idle;
    }

    /// Public companion to `clear_biometric_attempt` for the
    /// "Touch ID resolved successfully but the user already moved on"
    /// case in `AppShell::submit_biometric_unlock`. No `cx.notify` -
    /// whatever screen is up now wasn't observing this attempt anyway.
    pub fn clear_biometric_attempt_public(&mut self) {
        self.clear_biometric_attempt();
    }

    /// Persist a brand-new enrolment for `path`, transactionally: the
    /// keychain write and the registry pointer must *both* land, or
    /// neither does. The password is held only on the caller's stack
    /// (via `&str`); we never copy it onto `AppState`.
    ///
    /// Failure modes are ordered so we never leave an orphan:
    /// 1. keychain write fails → registry untouched, return Err.
    /// 2. registry save fails → roll the keychain item back (delete it)
    ///    so a master password can't sit in the keychain with no
    ///    pointer, then return Err.
    ///
    /// The registry save is synchronous here (small file, rare action)
    /// precisely so step 2 can observe the result and roll back.
    pub fn complete_biometric_enrollment(
        &mut self,
        path: PathBuf,
        keyfile: Option<PathBuf>,
        password: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), BiometricError> {
        // Overwrite-aware: if there's already an enrolment for this
        // path, drop the old keychain item before installing the new
        // one. Confirm the delete; if it can't be confirmed, abort
        // rather than risk two keychain items for one vault.
        if let Some(prior) = self.biometric_registry.get(&path).cloned() {
            self.biometric.forget(&prior.id)?;
            self.biometric_registry.remove(&path);
        }
        let id = EnrollmentId::new_random();
        self.biometric.enroll(&id, password)?;

        // Tentatively install the pointer, then persist. On a persist
        // failure, roll back both the in-memory registry and the
        // keychain item so the next render shows "Enable Touch ID"
        // rather than a half-committed enrolment.
        self.biometric_registry.upsert(
            path.clone(),
            BiometricEnrollment {
                id: id.clone(),
                keyfile,
                enrolled_at: Local::now(),
            },
        );
        if let Err(err) = crate::biometric::registry::save(&self.biometric_registry) {
            self.biometric_registry.remove(&path);
            let _ = self.biometric.forget(&id);
            return Err(BiometricError::Backend(format!(
                "could not persist Touch ID enrolment: {err}"
            )));
        }

        self.pending_biometric_enrollment = false;
        cx.notify();
        Ok(())
    }

    /// Remove an enrolment in both the registry and the OS keychain.
    /// Idempotent - calling on an absent path is a no-op. Order is
    /// keychain-first: we only drop the registry pointer once the
    /// keychain delete is confirmed, so a failed delete keeps the
    /// entry visible (and re-`Forget`-able) instead of orphaning the
    /// stored master password. Returns whether the enrolment is fully
    /// gone.
    pub fn forget_biometric(&mut self, path: &Path, cx: &mut Context<Self>) -> bool {
        let Some(entry) = self.biometric_registry.get(path).cloned() else {
            return true; // nothing enrolled - already "forgotten"
        };
        if self.biometric.forget(&entry.id).is_err() {
            // Keychain item may still be present; keep the registry
            // pointer so the UI doesn't claim Touch ID is gone while
            // the password lingers in the keychain.
            cx.notify();
            return false;
        }
        self.biometric_registry.remove(path);
        let _ = crate::biometric::registry::save(&self.biometric_registry);
        cx.notify();
        true
    }
}
