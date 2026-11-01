//! What locking, parking and result routing do today, written down before
//! this file is split up. Every test here goes red when the rule it names
//! is taken out of the code, and a refactoring has to keep all of them
//! green without editing them.
use super::*;
use crate::domain::VaultGroup;
use gpui::TestAppContext;

/// Runs inside `lock_vault_now` immediately before the active document is
/// dropped, so a test can see what was already revoked at that point.
pub(super) struct WipeWitness(Arc<dyn Fn() + Send + Sync>);

impl WipeWitness {
    pub(super) fn observe(witness: Option<&Self>) {
        if let Some(witness) = witness {
            (witness.0)();
        }
    }
}

impl std::fmt::Debug for WipeWitness {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("WipeWitness")
    }
}

fn document() -> VaultDocument {
    VaultDocument::new(
        Database::new(),
        VaultSnapshot::new(VaultGroup::default()),
        "pw".to_string(),
        None,
    )
}

/// An Auto-Type run that is still typing must lose its authorization
/// before the secrets it types from are gone, never after.
#[test]
fn lock_rotates_auto_type_token_before_dropping_secrets() {
    let mut state = AppState::default();
    state.open_for_test(PathBuf::from("/tmp/characterize-lock.kdbx"), document());
    let token = state.auto_type_cancellation();
    let revoked_at_wipe = Arc::new(Mutex::new(None));
    let witness_token = token.clone();
    let witness_slot = Arc::clone(&revoked_at_wipe);
    state.wipe_witness = Some(WipeWitness(Arc::new(move || {
        *witness_slot.lock().expect("witness slot") = Some(witness_token.is_cancelled());
    })));

    state.lock_vault_now();

    assert_eq!(
        *revoked_at_wipe.lock().expect("witness slot"),
        Some(true),
        "the Auto-Type token has to be revoked by the time the document is dropped"
    );
    assert!(token.is_cancelled());
    assert!(matches!(state.vault, VaultStatus::Empty));
    assert!(
        !state.auto_type_cancellation().is_cancelled(),
        "the next vault starts with a token of its own"
    );
}

/// One lock sweeps every decrypted session at once: the active one, every
/// parked one, and any document held back for a save.
#[test]
fn lock_clears_every_parked_and_deferred_session() {
    let mut state = AppState::default();
    let held = PathBuf::from("/tmp/characterize-held.kdbx");
    let parked = PathBuf::from("/tmp/characterize-parked.kdbx");
    let active = PathBuf::from("/tmp/characterize-active.kdbx");

    // A lock asked for while `held` was still being written keeps that
    // document back. The writer has let go of the file since.
    state.open_for_test(held.clone(), document());
    let held_session = state.active_vault_session_id.expect("held session");
    state
        .saves_in_flight
        .insert(held.clone(), QueuedSave::new(held_session));
    state.lock_requested_after_save = true;
    state.quarantine_sessions_for_deferred_lock();
    state.saves_in_flight.remove(&held);
    assert!(state.deferred_lock_sessions.contains_key(&held_session));

    state.open_for_test(parked.clone(), document());
    state.park_active();
    state.open_for_test(active, document());
    assert_eq!(state.parked_order, vec![parked]);

    state.lock_vault_now();

    assert!(matches!(state.vault, VaultStatus::Empty));
    assert!(state.parked.is_empty(), "a parked vault outlived the lock");
    assert!(state.parked_order.is_empty());
    assert!(
        state.deferred_lock_sessions.is_empty(),
        "a document held back for a save outlived the lock"
    );
    assert!(!state.has_any_unlocked());
}

/// A save or sync result belongs to the session that started the work,
/// even when the user has moved on to another vault in the meantime.
#[test]
fn results_land_in_the_session_that_dispatched_them() {
    let mut state = AppState::default();
    let active = PathBuf::from("/tmp/characterize-route-active.kdbx");
    let parked = PathBuf::from("/tmp/characterize-route-parked.kdbx");

    state.open_for_test(parked.clone(), document());
    let parked_session = state.active_vault_session_id.expect("parked session");
    let parked_sync = state.sync_session_for(&parked).expect("parked sync");
    state.park_active();
    state.open_for_test(active.clone(), document());
    let active_session = state.active_vault_session_id.expect("active session");

    assert!(!state.set_save_status_for_session(&parked, parked_session, SaveStatus::Saved));
    assert!(!state.route_sync_status_for_session(
        &parked,
        parked_sync,
        SyncStatus::Failed("offline".into()),
    ));

    let slot = state.parked.get(&parked).expect("still parked");
    assert_eq!(slot.save_status, SaveStatus::Saved);
    assert!(matches!(slot.sync_status, SyncStatus::Failed(_)));
    assert_eq!(
        state.save_status,
        SaveStatus::Idle,
        "the active vault saved nothing"
    );
    assert!(matches!(state.sync_status, SyncStatus::Disconnected));

    assert!(state.set_save_status_for_session(&active, active_session, SaveStatus::Saving));
    assert_eq!(state.save_status, SaveStatus::Saving);
    assert_eq!(
        state.parked.get(&parked).map(|slot| &slot.save_status),
        Some(&SaveStatus::Saved)
    );
}

/// A result from a session that has gone away changes nothing, even when
/// its path is open again or still parked.
#[test]
fn stale_session_results_change_nothing() {
    let mut state = AppState::default();
    let active = PathBuf::from("/tmp/characterize-stale-active.kdbx");
    let parked = PathBuf::from("/tmp/characterize-stale-parked.kdbx");
    let closed_long_ago = VaultSessionId::next();

    // The user ends the parked vault's sync relationship while an answer
    // from it is still on its way, then switches to another vault.
    state.open_for_test(parked.clone(), document());
    let ended_relationship = state.sync_session_for(&parked).expect("parked sync");
    state.end_sync_relationship(Some(&parked));
    state.park_active();
    state.open_for_test(active.clone(), document());
    // A healthy status would clear this, so it shows whether one got in.
    state
        .deferred_conflicts
        .insert(parked.clone(), "remote-revision".into());

    for target in [&parked, &active] {
        assert!(!state.set_save_status_for_session(
            target,
            closed_long_ago,
            SaveStatus::Failed("stale".into()),
        ));
    }
    assert!(!state.route_sync_status_for_session(&parked, ended_relationship, SyncStatus::Idle,));
    assert!(!state.route_sync_status_for_session(
        &active,
        SyncSession {
            vault: closed_long_ago,
            binding: 0,
        },
        SyncStatus::Idle,
    ));

    let slot = state.parked.get(&parked).expect("still parked");
    assert_eq!(slot.save_status, SaveStatus::Idle);
    assert!(matches!(slot.sync_status, SyncStatus::Disconnected));
    assert_eq!(state.save_status, SaveStatus::Idle);
    assert!(matches!(state.sync_status, SyncStatus::Disconnected));
    assert!(state.deferred_conflicts.contains_key(&parked));
}

/// The save coordinator is keyed by path, so a second decrypted session
/// for the same file would let one of them overwrite the other.
#[gpui::test]
fn never_two_sessions_for_one_file(cx: &mut TestAppContext) {
    let first = PathBuf::from("/tmp/characterize-one-session.kdbx");
    let other = PathBuf::from("/tmp/characterize-other-session.kdbx");
    let state = cx.new(|_| {
        let mut state = AppState::default();
        state.open_for_test(first.clone(), document());
        state
    });
    let first_session = state.read_with(cx, |state, _| state.active_vault_session_id);

    // Open: asking for its password again changes nothing.
    state.update(cx, |state, cx| state.request_password(first.clone(), cx));
    state.read_with(cx, |state, _| {
        assert!(state.is_active_vault(&first), "the open vault was replaced");
        assert_eq!(state.active_vault_session_id, first_session);
        assert!(state.parked.is_empty());
    });

    // Parked: asking for its password brings the same session back.
    state.update(cx, |state, _| {
        state.park_active();
        state.open_for_test(other.clone(), document());
    });
    state.update(cx, |state, cx| state.request_password(first.clone(), cx));
    state.read_with(cx, |state, _| {
        assert!(
            state.is_active_vault(&first),
            "the parked vault was not brought back"
        );
        assert_eq!(state.active_vault_session_id, first_session);
        assert_eq!(state.parked_order, vec![other.clone()]);
    });

    // Switching back queued a write of the recent-vaults list, and that
    // goes to the real Application Support folder of whoever runs the
    // tests. Drop it before the harness runs what is left.
    cx.dispatcher.drain_tasks();
}

/// Asking to lock while work is not on disk yet revokes access at once
/// but keeps the document until it is. Either reason holds the document
/// back on its own: a save that is still being written, or changes whose
/// last save failed.
#[gpui::test]
fn unsaved_work_quarantines_instead_of_dropping(cx: &mut TestAppContext) {
    for save_running in [true, false] {
        let case = if save_running {
            "save still being written"
        } else {
            "last save failed"
        };
        let path = PathBuf::from("/tmp/characterize-quarantine.kdbx");
        let state = cx.new(|_| {
            let mut state = AppState::default();
            state.open_for_test(path.clone(), document());
            let session = state.active_vault_session_id.expect("session");
            if save_running {
                state
                    .saves_in_flight
                    .insert(path.clone(), QueuedSave::new(session));
            } else {
                state.mark_vault_session_unpersisted(session);
            }
            state
        });
        let session = state
            .read_with(cx, |state, _| state.active_vault_session_id)
            .expect("session");

        let locked_at_once = state.update(cx, |state, cx| state.lock_vault(cx));

        assert!(!locked_at_once, "{case}: the lock has to wait for the save");
        state.read_with(cx, |state, _| {
            assert!(
                matches!(state.vault_status(), VaultStatus::LockedPendingSave),
                "{case}: the vault does not wait for its save"
            );
            assert!(
                state.vault_browser().is_none(),
                "{case}: the vault is still readable"
            );
            assert!(!state.has_any_unlocked(), "{case}");
            assert_eq!(
                state
                    .deferred_lock_sessions
                    .get(&session)
                    .map(|held| &held.path),
                Some(&path),
                "{case}: the unsaved document was dropped"
            );
        });

        // After a failed save the lock tries once more, and that write
        // would land in /tmp. Drop it unrun, together with the timer the
        // lock starts for a save that hangs.
        cx.dispatcher.drain_tasks();
    }
}
