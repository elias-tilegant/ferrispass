//! Coverage for the multi-vault session machinery: park, unpark,
//! and the lock-clears-all-parked semantics. The cx-bearing public
//! methods (`switch_to_unlocked`, `lock_vault`, …) can't be hit
//! without a gpui test harness, so we drill straight into the
//! private park/unpark helpers and the `parked` map. Save completion
//! routing is exercised through the pure session-aware status helper.
use super::*;
use crate::domain::VaultGroup;
use keepass::{Database, db::fields};
use std::path::PathBuf;
use std::rc::Rc;

fn fresh_open(state: &mut AppState, path: PathBuf, password: &str) {
    let document = VaultDocument::new(
        Database::new(),
        VaultSnapshot::new(VaultGroup::default()),
        password.to_string(),
        None,
    );
    state.open_for_test(path, document);
}

#[test]
fn conflict_state_debug_omits_databases_and_remote_revision() {
    let database_secret = "conflict-database-secret";
    let etag_secret = "conflict-etag-secret";
    let mut local_db = Database::new();
    local_db
        .root_mut()
        .add_entry()
        .set_unprotected(fields::NOTES, database_secret);
    let state = ConflictState {
        remote_db: local_db.clone(),
        local_db,
        remote_etag: etag_secret.into(),
        report: ConflictReport::default(),
        picks: crate::keepass::merge::Resolutions::default(),
        base_generation: 7,
        session: sync_session_of(VaultSessionId::next()),
    };

    let rendered = format!("{state:?}");

    assert!(!rendered.contains(database_secret));
    assert!(!rendered.contains(etag_secret));
    assert!(rendered.contains("base_generation: 7"));
}

#[test]
fn connect_operation_gate_rejects_stale_publication() {
    let gate = ConnectOperationGate::default();
    let stale = gate.advance();
    let current = gate.advance();

    assert_eq!(gate.commit_if_current(stale, || "stale"), None);
    assert_eq!(
        gate.commit_if_current(current, || "current"),
        Some("current")
    );
}

#[test]
fn connect_operation_gate_orders_cleanup_before_new_publication() {
    let gate = ConnectOperationGate::default();
    gate.reserve_cleanup();
    gate.reserve_cleanup();
    let current = gate.advance();
    let worker_gate = gate.clone();
    let events = Arc::new(Mutex::new(Vec::new()));
    let worker_events = Arc::clone(&events);
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        started_tx.send(()).expect("started");
        let result = worker_gate.commit_if_current(current, || {
            worker_events.lock().expect("events").push("publication");
            "published"
        });
        done_tx.send(result).expect("result");
    });

    started_rx.recv().expect("worker started");
    gate.run_reserved_cleanup(|| {
        events.lock().expect("events").push("cleanup-1");
    });
    gate.run_reserved_cleanup(|| {
        events.lock().expect("events").push("cleanup-2");
    });

    assert_eq!(
        done_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("publication unblocked"),
        Some("published")
    );
    worker.join().expect("worker joined");
    assert_eq!(
        *events.lock().expect("events"),
        ["cleanup-1", "cleanup-2", "publication"]
    );
}

#[test]
fn park_then_unpark_round_trips_the_document() {
    let mut state = AppState::default();
    let path = PathBuf::from("/tmp/round.kdbx");
    fresh_open(&mut state, path.clone(), "pw-A");
    state.save_status = SaveStatus::Saved;

    state.park_active();

    assert!(matches!(state.vault, VaultStatus::Empty));
    assert_eq!(state.parked.len(), 1);
    assert_eq!(state.parked_order, vec![path.clone()]);
    // Park took the save_status with it.
    assert_eq!(state.save_status, SaveStatus::Idle);
    assert_eq!(
        state.parked.get(&path).map(|s| s.save_status.clone()),
        Some(SaveStatus::Saved),
    );

    assert!(state.unpark(&path));
    assert!(matches!(&state.vault, VaultStatus::Open { path: p, .. } if p == &path));
    assert!(state.parked.is_empty());
    assert!(state.parked_order.is_empty());
    assert_eq!(state.save_status, SaveStatus::Saved);
}

#[test]
fn rotating_auto_type_context_revokes_only_the_previous_generation() {
    let mut state = AppState::default();
    let previous = state.auto_type_cancellation();

    state.rotate_auto_type_context();

    assert!(previous.is_cancelled());
    assert!(!state.auto_type_cancellation().is_cancelled());
}

#[test]
fn deferred_lock_revokes_the_open_auto_type_context() {
    let mut state = AppState::default();
    fresh_open(&mut state, PathBuf::from("/tmp/auto-type-dirty.kdbx"), "pw");
    let session_id = state.active_vault_session_id.expect("session id");
    state.mark_vault_session_unpersisted(session_id);
    let previous = state.auto_type_cancellation();

    state.quarantine_sessions_for_deferred_lock();

    assert!(previous.is_cancelled());
    assert!(matches!(state.vault, VaultStatus::LockedPendingSave));
    assert!(!state.auto_type_cancellation().is_cancelled());
}

#[test]
fn park_active_is_noop_when_no_vault_is_open() {
    let mut state = AppState::default();
    state.park_active();
    assert!(matches!(state.vault, VaultStatus::Empty));
    assert!(state.parked.is_empty());
}

#[test]
fn parked_order_records_oldest_first() {
    let mut state = AppState::default();
    let a = PathBuf::from("/tmp/a.kdbx");
    let b = PathBuf::from("/tmp/b.kdbx");

    fresh_open(&mut state, a.clone(), "pw-A");
    state.park_active();
    fresh_open(&mut state, b.clone(), "pw-B");
    state.park_active();

    assert_eq!(state.parked_order, vec![a, b]);
}

#[test]
fn unpark_unknown_path_returns_false_and_changes_nothing() {
    let mut state = AppState::default();
    let path = PathBuf::from("/tmp/never-parked.kdbx");
    assert!(!state.unpark(&path));
    assert!(matches!(state.vault, VaultStatus::Empty));
}

#[test]
fn unlocked_paths_includes_active_and_parked() {
    let mut state = AppState::default();
    let parked_path = PathBuf::from("/tmp/parked.kdbx");
    let active_path = PathBuf::from("/tmp/active.kdbx");

    fresh_open(&mut state, parked_path.clone(), "pw");
    state.park_active();
    fresh_open(&mut state, active_path.clone(), "pw");

    let paths = state.unlocked_paths();
    assert!(paths.contains(&parked_path));
    assert!(paths.contains(&active_path));
    assert_eq!(paths.len(), 2);
    assert!(state.has_any_unlocked());
}

#[test]
fn has_any_unlocked_false_when_everything_locked() {
    let state = AppState::default();
    assert!(!state.has_any_unlocked());
}

// -- Routing helpers (apply_save_status / apply_sync_status /
// with_sync_binding_mut_for / replace_document_for / database_clone_for /
// snapshot_sync_inputs). These power the High #1 / Medium #1 fixes
// where a save or sync that finishes after the user has switched
// away must land on the originating vault, not whatever's active now.

use crate::sync::auth::AccessToken;
use crate::sync::config::{SyncConfig, SyncProvider};
use std::time::{Duration, SystemTime};

fn fake_binding(email: &str) -> SyncBinding {
    SyncBinding {
        config: SyncConfig {
            schema_version: 2,
            provider: SyncProvider::SharePoint,
            account_email: email.to_string(),
            site_id: "site".into(),
            drive_id: "drive".into(),
            item_id: "item".into(),
            last_etag: "etag-0".into(),
            local_path: PathBuf::from("/tmp/whatever.kdbx"),
            remote_url: "https://example.invalid/foo.kdbx".into(),
            authenticated_at: None,
            remote_bookmark: None,
            uploaded_local_revision: None,
        },
        access_token: AccessToken {
            access_token: "token-0".into(),
            refresh_token: "refresh-0".into(),
            expires_at: SystemTime::now() + Duration::from_secs(3600),
        },
    }
}

/// The sync session a test's callback would have captured.
fn sync_session(state: &AppState, path: &Path) -> SyncSession {
    state
        .sync_session_for(path)
        .expect("the vault is open in this test")
}

fn sync_session_of(vault: VaultSessionId) -> SyncSession {
    SyncSession { vault, binding: 0 }
}

fn fake_binding_for(email: &str, local_path: PathBuf, item_id: &str) -> SyncBinding {
    SyncBinding {
        config: SyncConfig {
            schema_version: 2,
            provider: SyncProvider::SharePoint,
            account_email: email.to_string(),
            site_id: "site".into(),
            drive_id: "drive".into(),
            item_id: item_id.into(),
            last_etag: "etag-0".into(),
            local_path,
            remote_url: format!("https://example.invalid/{item_id}.kdbx"),
            authenticated_at: None,
            remote_bookmark: None,
            uploaded_local_revision: None,
        },
        access_token: AccessToken {
            access_token: "token-0".into(),
            refresh_token: "refresh-0".into(),
            expires_at: SystemTime::now() + Duration::from_secs(3600),
        },
    }
}

#[test]
fn pending_sync_installs_after_matching_unlock_without_stomping_parked_vault() {
    let mut state = AppState::default();
    let vault_a = PathBuf::from("/tmp/a.kdbx");
    let vault_b = PathBuf::from("/tmp/b.kdbx");

    fresh_open(&mut state, vault_a.clone(), "pw-A");
    state.sync = Some(fake_binding_for(
        "alice@example.invalid",
        vault_a.clone(),
        "item-a",
    ));
    state.sync_status = SyncStatus::Idle;

    state.pending_sync = Some(PendingSync {
        local_path: vault_b.clone(),
        binding: fake_binding_for("alice@example.invalid", vault_b.clone(), "item-b"),
    });
    state.park_active();
    fresh_open(&mut state, vault_b.clone(), "pw-B");
    assert!(state.install_pending_sync_for(&vault_b));

    assert!(matches!(&state.vault, VaultStatus::Open { path, .. } if path == &vault_b));
    assert_eq!(
        state.sync.as_ref().map(|b| b.config.item_id.as_str()),
        Some("item-b")
    );
    assert!(state.pending_sync.is_none());
    assert_eq!(
        state
            .parked
            .get(&vault_a)
            .and_then(|session| session.sync.as_ref())
            .map(|b| b.config.item_id.as_str()),
        Some("item-a")
    );
}

#[test]
fn request_password_for_other_path_drops_pending_sync() {
    let mut state = AppState::default();
    let pending_path = PathBuf::from("/tmp/pending.kdbx");
    let other_path = PathBuf::from("/tmp/other.kdbx");
    state.pending_sync = Some(PendingSync {
        local_path: pending_path.clone(),
        binding: fake_binding_for("alice@example.invalid", pending_path, "item-pending"),
    });

    state.clear_pending_sync_unless(&other_path);

    assert!(state.pending_sync.is_none());
}

#[test]
fn has_open_sync_remote_checks_active_parked_and_pending() {
    let mut state = AppState::default();
    let active_path = PathBuf::from("/tmp/active.kdbx");
    let parked_path = PathBuf::from("/tmp/parked.kdbx");
    let pending_path = PathBuf::from("/tmp/pending.kdbx");

    fresh_open(&mut state, parked_path.clone(), "pw-parked");
    state.sync = Some(fake_binding_for(
        "alice@example.invalid",
        parked_path,
        "item-parked",
    ));
    state.park_active();
    fresh_open(&mut state, active_path.clone(), "pw-active");
    state.sync = Some(fake_binding_for(
        "alice@example.invalid",
        active_path,
        "item-active",
    ));
    state.pending_sync = Some(PendingSync {
        local_path: pending_path.clone(),
        binding: fake_binding_for("alice@example.invalid", pending_path, "item-pending"),
    });

    assert!(state.has_open_sync_remote("drive", "item-active"));
    assert!(state.has_open_sync_remote("drive", "item-parked"));
    assert!(state.has_open_sync_remote("drive", "item-pending"));
    assert!(!state.has_open_sync_remote("drive", "item-missing"));
}

#[test]
fn apply_save_status_routes_to_parked_vault() {
    // The High #1 / Medium-related guarantee: a save that finishes
    // after the user has switched away marks the *saving* vault, not
    // whoever is now active.
    let mut state = AppState::default();
    let saved_path = PathBuf::from("/tmp/saved.kdbx");
    let active_path = PathBuf::from("/tmp/active.kdbx");

    fresh_open(&mut state, saved_path.clone(), "pw");
    let saved_session_id = state.active_vault_session_id.expect("session id");
    state.park_active();
    fresh_open(&mut state, active_path.clone(), "pw");
    // Active vault starts Idle; parked vault starts Idle too.
    assert_eq!(state.save_status, SaveStatus::Idle);

    // Background save for the parked session finished after the switch.
    // The pure setter returns false because no active redraw is needed.
    assert!(!state.set_save_status_for_session(&saved_path, saved_session_id, SaveStatus::Saved,));
    // Active stays Idle:
    assert_eq!(state.save_status, SaveStatus::Idle);
    // Parked vault recorded Saved:
    assert_eq!(
        state.parked.get(&saved_path).unwrap().save_status,
        SaveStatus::Saved,
    );
}

#[test]
fn vault_session_ids_are_monotonic_across_reopens() {
    let mut state = AppState::default();
    let path = PathBuf::from("/tmp/reopen.kdbx");

    fresh_open(&mut state, path.clone(), "pw-old");
    let old = state.active_vault_session_id.expect("old session");
    fresh_open(&mut state, path, "pw-new");
    let new = state.active_vault_session_id.expect("new session");

    assert!(new.0 > old.0);
}

#[test]
fn stale_save_result_cannot_mark_reopened_same_path_as_saved() {
    let mut state = AppState::default();
    let path = PathBuf::from("/tmp/same-path.kdbx");

    fresh_open(&mut state, path.clone(), "pw-old");
    let stale_session = state.active_vault_session_id.expect("old session");

    // Simulate closing and reopening the same file before the old
    // background callback resolves.
    fresh_open(&mut state, path.clone(), "pw-new");
    let current_session = state.active_vault_session_id.expect("new session");
    state.save_status = SaveStatus::Idle;

    assert!(!state.set_save_status_for_session(&path, stale_session, SaveStatus::Saved,));
    assert_eq!(state.save_status, SaveStatus::Idle);

    assert!(state.set_save_status_for_session(&path, current_session, SaveStatus::Saved,));
    assert_eq!(state.save_status, SaveStatus::Saved);
}

#[test]
fn invalid_vault_selection_does_not_replace_an_open_document() {
    let mut state = AppState::default();
    let open_path = PathBuf::from("/tmp/open.kdbx");
    fresh_open(&mut state, open_path.clone(), "pw");
    let session_id = state.active_vault_session_id.expect("session id");

    state.record_vault_selection_failure(
        Some(PathBuf::from("/tmp/not-a-vault.txt")),
        "Selected file is not a .kdbx database.".into(),
    );

    assert!(matches!(
        &state.vault,
        VaultStatus::Open { path, .. } if path == &open_path
    ));
    assert_eq!(state.active_vault_session_id, Some(session_id));
    assert_eq!(
        state.take_pending_notice().as_deref(),
        Some("Selected file is not a .kdbx database.")
    );
}

#[test]
fn queued_sync_replaces_all_old_state_for_a_reopened_session() {
    let old_session = VaultSessionId::next();
    let new_session = VaultSessionId::next();
    let old_bytes = Arc::new(vec![1]);
    let new_bytes = Arc::new(vec![2]);
    let mut queued = QueuedSync::new(sync_session_of(old_session));

    queued.queue(QueuedSyncRequest {
        session: sync_session_of(old_session),
        interactive: false,
        bytes: Some(old_bytes.clone()),
    });
    queued.queue(QueuedSyncRequest {
        session: sync_session_of(old_session),
        interactive: true,
        bytes: None,
    });
    let same_session = queued.pending.as_ref().expect("queued request");
    assert!(same_session.interactive);
    assert!(same_session.bytes.is_none());

    queued.queue(QueuedSyncRequest {
        session: sync_session_of(new_session),
        interactive: false,
        bytes: Some(new_bytes.clone()),
    });
    let reopened = queued.pending.as_ref().expect("replacement request");
    assert_eq!(queued.owner, sync_session_of(old_session));
    assert_eq!(reopened.session, sync_session_of(new_session));
    assert!(!reopened.interactive);
    assert!(
        reopened
            .bytes
            .as_ref()
            .is_some_and(|b| Arc::ptr_eq(b, &new_bytes))
    );
}

#[test]
fn stale_sync_callback_cannot_mutate_reopened_same_path_binding() {
    let mut state = AppState::default();
    let path = PathBuf::from("/tmp/same-sync-path.kdbx");

    fresh_open(&mut state, path.clone(), "pw-old");
    state.sync = Some(fake_binding("old@example.invalid"));
    let stale_session = state.active_vault_session_id.expect("old session");

    fresh_open(&mut state, path.clone(), "pw-new");
    state.sync = Some(fake_binding("new@example.invalid"));
    let current_session = state.active_vault_session_id.expect("new session");

    state.with_sync_binding_mut_for_session(&path, sync_session_of(stale_session), |binding| {
        binding.config.last_etag = "stale-etag".into();
    });
    assert_eq!(
        state.sync.as_ref().expect("binding").config.last_etag,
        "etag-0"
    );
    assert!(!state.rebind_sync_for_session(
        &path,
        sync_session_of(stale_session),
        fake_binding("stale-rebind@example.invalid"),
    ));
    assert_eq!(
        state.sync.as_ref().expect("binding").config.account_email,
        "new@example.invalid"
    );

    state.with_sync_binding_mut_for_session(&path, sync_session_of(current_session), |binding| {
        binding.config.last_etag = "current-etag".into();
    });
    assert_eq!(
        state.sync.as_ref().expect("binding").config.last_etag,
        "current-etag"
    );
    assert!(state.rebind_sync_for_session(
        &path,
        sync_session_of(current_session),
        fake_binding("current-rebind@example.invalid"),
    ));
    assert_eq!(
        state.sync.as_ref().expect("binding").config.account_email,
        "current-rebind@example.invalid"
    );
}

/// The iCloud publish callback used to write `state.sync` directly.
/// `vault_session_is_current` is also true for a parked vault, so a
/// publish that finished after the user pressed Cmd+O installed vault
/// A's binding on vault B, and B's next save uploaded its own bytes over
/// A's remote file.
#[test]
fn a_binding_for_a_parked_vault_never_lands_on_the_active_one() {
    let mut state = AppState::default();
    let published = PathBuf::from("/tmp/published.kdbx");
    let switched_to = PathBuf::from("/tmp/switched-to.kdbx");

    fresh_open(&mut state, published.clone(), "pw-a");
    let publishing_session = state.active_vault_session_id.expect("session a");
    let generation = state.connect_operations.advance();
    state.park_active();

    fresh_open(&mut state, switched_to.clone(), "pw-b");
    state.sync = Some(fake_binding_for(
        "b@example.invalid",
        switched_to.clone(),
        "item-b",
    ));

    // The production callback body, not the routing helper it calls: a
    // regression that writes `state.sync` directly has to fail here.
    state.apply_icloud_publish_result(
        &published,
        sync_session_of(publishing_session),
        generation,
        Ok(Some(
            fake_binding_for("a@example.invalid", published.clone(), "item-a").config,
        )),
    );

    assert_eq!(
        state
            .parked
            .get(&published)
            .and_then(|session| session.sync.as_ref())
            .map(|binding| binding.config.item_id.as_str()),
        Some("item-a"),
        "the binding belongs to the vault that was published"
    );
    assert_eq!(
        state
            .sync
            .as_ref()
            .map(|binding| binding.config.item_id.as_str()),
        Some("item-b"),
        "the vault on screen keeps its own remote file"
    );
}

/// Publishing writes the file but runs no revision check, so the vault is
/// connected, not verified-in-sync. Claiming "Synced just now" hid a
/// pending upload behind a green pill.
#[test]
fn a_status_for_a_parked_vault_never_lands_on_the_active_one() {
    let mut state = AppState::default();
    let published = PathBuf::from("/tmp/published.kdbx");
    let switched_to = PathBuf::from("/tmp/switched-to.kdbx");

    fresh_open(&mut state, published.clone(), "pw-a");
    let publishing_session = state.active_vault_session_id.expect("session a");
    state.sync_status = SyncStatus::Connecting;
    state.park_active();

    fresh_open(&mut state, switched_to, "pw-b");
    state.sync_status = SyncStatus::Disconnected;

    // Run the restore completion the way the spawn closure does. A
    // regression that assigns `sync_status` directly has to fail here.
    let outcome = state.apply_sync_binding_restore(
        &published,
        sync_session_of(publishing_session),
        Ok((
            fake_binding_for("a@example.invalid", published.clone(), "item-a").config,
            AccessToken::provider_placeholder(),
        )),
    );
    assert!(outcome.bound, "the parked vault still owns its session");
    assert!(
        !outcome.repaint,
        "a parked vault does not repaint the window"
    );

    assert!(
        matches!(
            state
                .parked
                .get(&published)
                .map(|session| &session.sync_status),
            Some(SyncStatus::Idle)
        ),
        "the published vault records the new status"
    );
    assert!(
        matches!(state.sync_status, SyncStatus::Disconnected),
        "the vault on screen is untouched"
    );
}

/// A save chains a cloud push. Gating that on a live binding meant a save
/// finishing while the restore had not landed never reached the push path
/// at all, so it was never even queued: written locally, and nothing ever
/// sent it.
#[test]
fn a_save_during_a_pending_restore_still_reaches_the_cloud_path() {
    let mut state = AppState::default();
    let path = PathBuf::from("/tmp/restoring.kdbx");
    fresh_open(&mut state, path.clone(), "pw");

    // No binding yet, and no configuration either: a local-only vault has
    // nowhere to push and must not hold vault bytes for a push that
    // cannot happen.
    state.sync_status = SyncStatus::Disconnected;
    assert!(!state.save_should_reach_the_cloud(&path));

    // The restore is in flight. This is the case that was dropped.
    state.sync_status = SyncStatus::Restoring;
    assert!(state.save_should_reach_the_cloud(&path));

    // A failed restore is the same situation, one tick later.
    state.sync_status = SyncStatus::Failed("network".into());
    assert!(state.save_should_reach_the_cloud(&path));

    // An expired sign-in needs the user before anything can be sent.
    state.sync_status = SyncStatus::Reconnect { detail: None };
    assert!(!state.save_should_reach_the_cloud(&path));

    // And with a binding it is the ordinary case.
    state.sync = Some(fake_binding_for("a@example.invalid", path.clone(), "item"));
    state.sync_status = SyncStatus::Idle;
    assert!(state.save_should_reach_the_cloud(&path));
}

/// A pull rewrites the local file, and every save re-encrypts with a
/// fresh seed, so the file's digest can never match the last one we sent.
/// Reading that difference as an unsent edit would upload after every
/// single pull, which is the loop the fast-forward path exists to avoid.
///
/// This drives the production decision, not the comparison alone: an
/// earlier version of this test checked only the pure function, and
/// removing the branch that records the baseline left it green.
#[test]
fn a_fast_forward_records_the_merged_file_as_the_new_baseline() {
    let dir = tempfile::tempdir().expect("tempdir");
    let vault = dir.path().join("pulled.kdbx");
    let merged_bytes = b"re-encrypted after the pull";

    let mut state = AppState::default();
    fresh_open(&mut state, vault.clone(), "pw");
    let session_id = state.active_vault_session_id.expect("session");
    let mut binding = fake_binding_for("a@example.invalid", vault.clone(), "item");
    binding.config.uploaded_local_revision =
        Some(crate::sync::config::local_revision(b"as uploaded"));
    state.sync = Some(binding);

    let config = state
        .adopt_fast_forward_baseline(
            &vault,
            sync_session_of(session_id),
            "etag-after".into(),
            merged_bytes,
        )
        .expect("the vault is still open, so there is a binding to update");

    assert_eq!(config.last_etag, "etag-after");
    assert_eq!(
        config.uploaded_local_revision.as_deref(),
        Some(crate::sync::config::local_revision(merged_bytes).as_str()),
        "the merged file is the new baseline"
    );

    // And the vault is no longer read as holding an unsent edit.
    std::fs::write(&vault, merged_bytes).expect("write vault");
    assert!(!AppState::local_is_ahead_of_the_cloud(&vault, &config));
}

/// The other half of the same rule: a file that differs from what was
/// sent is an unsent edit, whatever the remote etag says. That is the
/// case a push dropped by a lock, a quit or a crash leaves behind.
#[test]
fn a_file_that_was_never_uploaded_is_recognised_as_ahead() {
    let dir = tempfile::tempdir().expect("tempdir");
    let vault = dir.path().join("ahead.kdbx");
    std::fs::write(&vault, b"saved locally after the last upload").expect("write vault");

    let mut config = fake_binding_for("a@example.invalid", vault.clone(), "item").config;

    config.uploaded_local_revision = None;
    assert!(
        AppState::local_is_ahead_of_the_cloud(&vault, &config),
        "a config from before this field existed cannot say the file was sent, \
         and guessing that it was is how an edit gets stranded"
    );

    config.uploaded_local_revision = Some(crate::sync::config::local_revision(
        b"saved locally after the last upload",
    ));
    assert!(
        !AppState::local_is_ahead_of_the_cloud(&vault, &config),
        "what is on disk is what was sent"
    );

    std::fs::write(&vault, b"edited again, never sent").expect("write vault");
    assert!(
        AppState::local_is_ahead_of_the_cloud(&vault, &config),
        "the file moved and the upload did not"
    );
}

/// A conflict the user has not answered yet reports itself as failed, and
/// a failed sync is retried with a full push. Every tick therefore sent
/// the same bytes, took the same 412, downloaded the same remote and
/// deferred again: the most expensive request this app makes, on a timer,
/// for as long as the user did not look.
fn heavy_cost() -> KdfCost {
    KdfCost {
        variant: crate::keepass::limits::Argon2Variant::Argon2id,
        memory: 256 * 1024 * 1024,
        iterations: 160,
        parallelism: 4,
    }
}

/// A new attempt refetches the remote, so neither the cost notice nor a
/// merge still decrypting from the last one may stay live: both belong
/// to a remote revision that is being replaced.
#[test]
fn a_new_sync_attempt_withdraws_the_remote_cost_notice() {
    let dir = tempfile::tempdir().expect("tempdir");
    let vault = dir.path().join("heavy.kdbx");
    let mut state = AppState::default();
    fresh_open(&mut state, vault.clone(), "pw");

    state
        .remote_kdf_confirmations
        .insert(vault.clone(), heavy_cost());
    state.remote_merges_in_flight.insert(vault.clone(), 7);
    state.route_sync_status(&vault, SyncStatus::Syncing);
    assert!(state.remote_kdf_confirmation_for_active().is_none());
    assert!(
        state.remote_merges_in_flight.is_empty(),
        "an older merge must not land in the middle of a new attempt"
    );

    state
        .remote_kdf_confirmations
        .insert(vault.clone(), heavy_cost());
    state.remote_merges_in_flight.insert(vault.clone(), 8);
    state.route_sync_status(
        &vault,
        SyncStatus::Synced {
            at: chrono::Local::now(),
            auto_merged: 0,
        },
    );
    assert!(state.remote_kdf_confirmations.is_empty());
    assert!(
        state.remote_merges_in_flight.is_empty(),
        "an older merge must not land on top of a healthy sync"
    );
}

/// A failed open for vault A that comes back after the user moved on to
/// B must not touch B's pre-check.
#[test]
fn a_cost_note_applies_only_to_the_vault_being_unlocked() {
    let mut state = AppState::default();
    state.vault = VaultStatus::AwaitingPassword {
        path: PathBuf::from("/vaults/b.kdbx"),
        keyfile: None,
        error: None,
    };
    assert!(state.is_unlocking(Path::new("/vaults/b.kdbx")));
    assert!(!state.is_unlocking(Path::new("/vaults/a.kdbx")));

    state.vault = VaultStatus::Opening {
        path: PathBuf::from("/vaults/b.kdbx"),
    };
    assert!(state.is_unlocking(Path::new("/vaults/b.kdbx")));
}

#[test]
fn a_deferred_conflict_does_not_retry_itself_every_tick() {
    let dir = tempfile::tempdir().expect("tempdir");
    let vault = dir.path().join("deferred.kdbx");
    let mut state = AppState::default();
    fresh_open(&mut state, vault.clone(), "pw");

    state
        .deferred_conflicts
        .insert(vault.clone(), "etag-at-defer".into());
    state.route_sync_status(&vault, SyncStatus::Failed("Remote conflict".into()));
    assert!(
        state.deferred_conflicts.contains_key(&vault),
        "the wait survives the status that describes it"
    );

    // Locking and disconnecting assign a status directly rather than
    // routing one, so they clear it themselves.
    state
        .deferred_conflicts
        .insert(vault.clone(), "etag-at-defer".into());
    state.end_sync_relationship(Some(&vault));
    assert!(!state.deferred_conflicts.contains_key(&vault));
    state
        .deferred_conflicts
        .insert(vault.clone(), "etag-at-defer".into());
    state.lock_vault_now();
    assert!(state.deferred_conflicts.is_empty());

    fresh_open(&mut state, vault.clone(), "pw");

    // Anything else means the wait is over: the overlay opened, the sync
    // succeeded, or the user disconnected.
    for status in [
        SyncStatus::Syncing,
        SyncStatus::Idle,
        SyncStatus::Synced {
            at: chrono::Local::now(),
            auto_merged: 0,
        },
        SyncStatus::Disconnected,
    ] {
        state
            .deferred_conflicts
            .insert(vault.clone(), "etag-at-defer".into());
        state.route_sync_status(&vault, status);
        assert!(
            !state.deferred_conflicts.contains_key(&vault),
            "and ends with anything that is not that status"
        );
    }
}

/// Disconnecting one provider and connecting another keeps the vault
/// open, so its session id does not move. An answer still in flight from
/// the provider the user left was accepted as an answer from the one they
/// moved to: a binding, a status, or a resolved conflict installed
/// against the wrong remote, with no history anywhere to recover it from.
#[test]
fn an_answer_from_the_provider_the_user_left_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let vault = dir.path().join("moved.kdbx");
    let mut state = AppState::default();
    fresh_open(&mut state, vault.clone(), "pw");
    state.sync = Some(fake_binding_for(
        "a@example.invalid",
        vault.clone(),
        "item-a",
    ));

    // What a callback started against the first provider carries.
    let in_flight = sync_session(&state, &vault);
    assert!(state.sync_session_is_current(&vault, in_flight));

    state.end_sync_relationship(Some(&vault));
    state.sync = Some(fake_binding_for(
        "b@example.invalid",
        vault.clone(),
        "item-b",
    ));

    assert!(
        !state.sync_session_is_current(&vault, in_flight),
        "the relationship it belongs to is over"
    );
    assert!(
        state.vault_session_is_current(&vault, in_flight.vault),
        "while the vault it belongs to is the same one, which is the trap"
    );
    assert!(
        !state.rebind_sync_for_session(
            &vault,
            in_flight,
            fake_binding_for("a@example.invalid", vault.clone(), "item-a"),
        ),
        "so it cannot install the old provider's binding"
    );
    assert_eq!(
        state.sync.as_ref().expect("binding").config.item_id,
        "item-b",
        "and the one the user chose is untouched"
    );
    assert!(
        state
            .record_upload_baseline(&vault, in_flight, "etag-a".into(), "rev-a".into())
            .is_none(),
        "nor report its upload as this one's"
    );

    // And it says nothing about any other vault, in either direction. A
    // push in flight for one the user is not looking at belongs to that
    // vault's own relationship; refusing it removed its queue entry and
    // left it on `Syncing` while automatic sync skipped it as busy.
    let other = dir.path().join("other.kdbx");
    state.park_active();
    fresh_open(&mut state, other.clone(), "pw");
    let parked_session = sync_session(&state, &vault);
    let active_session = sync_session(&state, &other);

    state.end_sync_relationship(Some(&vault));
    assert!(
        state.sync_session_is_current(&other, active_session),
        "disconnecting the parked vault does not end the active one's"
    );
    assert!(
        !state.sync_session_is_current(&vault, parked_session),
        "while the parked one's is over"
    );

    let active_session = sync_session(&state, &other);
    let parked_session = sync_session(&state, &vault);
    state.end_sync_relationship(Some(&other));
    assert!(
        state.sync_session_is_current(&vault, parked_session),
        "and disconnecting the active vault does not end the parked one's"
    );
    assert!(!state.sync_session_is_current(&other, active_session));
}

/// A successful upload has to record the bytes it sent, not just the
/// revision the server returned.
///
/// This drives the production path, not the comparison alone: the test
/// next to it checked only `local_is_ahead_of_the_cloud`, and removing
/// the line that records the baseline left it green while every later
/// tick re-uploaded a file the cloud already had.
#[test]
fn a_successful_upload_records_what_it_sent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let vault = dir.path().join("pushed.kdbx");
    let sent = b"encrypted bytes as uploaded";
    std::fs::write(&vault, sent).expect("write vault");

    let mut state = AppState::default();
    fresh_open(&mut state, vault.clone(), "pw");
    let session_id = state.active_vault_session_id.expect("session");
    state.sync = Some(fake_binding_for("a@example.invalid", vault.clone(), "item"));

    let config = state
        .record_upload_baseline(
            &vault,
            sync_session_of(session_id),
            "etag-from-server".into(),
            crate::sync::config::local_revision(sent),
        )
        .expect("the vault is open, so there is a binding to update");

    assert_eq!(config.last_etag, "etag-from-server");
    assert!(
        !AppState::local_is_ahead_of_the_cloud(&vault, &config),
        "what is on disk is exactly what was just sent"
    );

    // An upload that lands after the user switched vaults belongs to the
    // one it was started for, not to the one on screen.
    state.park_active();
    let other = dir.path().join("other.kdbx");
    fresh_open(&mut state, other.clone(), "pw");
    state.sync = Some(fake_binding_for("b@example.invalid", other, "other-item"));

    let parked = state
        .record_upload_baseline(
            &vault,
            sync_session_of(session_id),
            "etag-late".into(),
            crate::sync::config::local_revision(sent),
        )
        .expect("the parked vault still has its binding");
    assert_eq!(parked.last_etag, "etag-late");
    assert_eq!(
        state
            .sync
            .as_ref()
            .expect("active binding")
            .config
            .last_etag,
        "etag-0",
        "and the vault on screen is untouched"
    );
}

/// The branch this replaces lived inside a spawn closure, so nothing
/// could reach it: a fix to it was lost to a stray `git checkout` and
/// only an outside reviewer noticed. Each row is a way to lose an edit.
#[test]
fn a_pull_only_merges_silently_when_there_is_nothing_to_decide() {
    use crate::keepass::merge::{EntryConflict, EntryView, GroupConflict};

    let nothing = ConflictReport::default();
    let entry_conflict = ConflictReport {
        conflicts: vec![EntryConflict {
            id: "e1".into(),
            local: EntryView::default(),
            remote: EntryView::default(),
            fields: Vec::new(),
        }],
        ..Default::default()
    };
    let group_conflict = ConflictReport {
        group_conflicts: vec![GroupConflict {
            id: "g1".into(),
            name: "Banking".into(),
            path: Vec::new(),
            fields: Vec::new(),
        }],
        ..Default::default()
    };

    let settings_conflict = ConflictReport {
        metadata_conflict: Some(crate::keepass::merge::MetadataConflict { fields: Vec::new() }),
        ..Default::default()
    };

    let active_and_asked = |report: &ConflictReport| RemoteMergePlan::of(report, true, true, false);

    assert_eq!(active_and_asked(&nothing), RemoteMergePlan::AutoMerge);
    assert_eq!(active_and_asked(&entry_conflict), RemoteMergePlan::Ask);
    assert_eq!(
        active_and_asked(&group_conflict),
        RemoteMergePlan::Ask,
        "a group conflict alone must reach the user, not the default side"
    );
    assert_eq!(
        active_and_asked(&settings_conflict),
        RemoteMergePlan::Ask,
        "and so must a settings conflict: the default side is uploaded"
    );

    // A parked vault, a background pull and an open editor each defer
    // rather than seizing the screen, and none of them merges silently.
    for (active, interactive, editing) in [
        (false, true, false),
        (true, false, false),
        (true, true, true),
    ] {
        assert_eq!(
            RemoteMergePlan::of(&settings_conflict, active, interactive, editing),
            RemoteMergePlan::Defer
        );
        assert_eq!(
            RemoteMergePlan::of(&group_conflict, active, interactive, editing),
            RemoteMergePlan::Defer
        );
        assert_eq!(
            RemoteMergePlan::of(&entry_conflict, active, interactive, editing),
            RemoteMergePlan::Defer
        );
        assert_eq!(
            RemoteMergePlan::of(&nothing, active, interactive, editing),
            RemoteMergePlan::AutoMerge,
            "with nothing to decide there is nothing to defer"
        );
    }
}

/// Two vaults can each be waiting for their binding restore. With one
/// shared slot the second queued push evicted the first, and the first
/// vault's restore then consumed and dropped the survivor: both edits
/// stayed on one machine while the UI reported everything in sync.
#[test]
fn a_queued_push_survives_another_vault_queueing_and_restoring() {
    let mut state = AppState::default();
    let vault_a = PathBuf::from("/tmp/queue-a.kdbx");
    let vault_b = PathBuf::from("/tmp/queue-b.kdbx");

    fresh_open(&mut state, vault_a.clone(), "pw-a");
    let session_a = state.active_vault_session_id.expect("session a");
    state.queue_push_awaiting_restore(
        &vault_a,
        QueuedSyncRequest {
            session: sync_session_of(session_a),
            interactive: false,
            bytes: Some(Arc::new(b"vault-a-bytes".to_vec())),
        },
    );
    state.park_active();

    fresh_open(&mut state, vault_b.clone(), "pw-b");
    let session_b = state.active_vault_session_id.expect("session b");
    state.queue_push_awaiting_restore(
        &vault_b,
        QueuedSyncRequest {
            session: sync_session_of(session_b),
            interactive: false,
            bytes: Some(Arc::new(b"vault-b-bytes".to_vec())),
        },
    );

    let replayed_a = state
        .take_push_awaiting_restore(&vault_a)
        .expect("vault A's push is still queued");
    assert_eq!(replayed_a.session, sync_session_of(session_a));
    assert_eq!(
        replayed_a.bytes.as_deref().map(Vec::as_slice),
        Some(b"vault-a-bytes".as_slice()),
        "vault A replays its own bytes"
    );

    let replayed_b = state
        .take_push_awaiting_restore(&vault_b)
        .expect("vault B's push survived vault A's restore");
    assert_eq!(replayed_b.session, sync_session_of(session_b));
    assert!(
        state.pushes_awaiting_restore.is_empty(),
        "a replayed push is not queued twice"
    );
}

/// A queued push carries the published KDBX ciphertext. A derived `Debug`
/// would print the whole vault into any log line or panic message.
#[test]
fn a_queued_push_debug_omits_the_vault_bytes() {
    let request = QueuedSyncRequest {
        session: sync_session_of(VaultSessionId::next()),
        interactive: true,
        bytes: Some(Arc::new(b"KDBX-ciphertext-marker".to_vec())),
    };

    let rendered = format!("{request:?}");

    assert!(!rendered.contains("KDBX-ciphertext-marker"));
    assert!(
        rendered.contains("22"),
        "byte length stays visible: {rendered}"
    );
    assert!(rendered.contains("interactive: true"), "{rendered}");
}

/// A queued push holds vault ciphertext. If the vault was locked and
/// reopened meanwhile, replaying it would upload bytes from a session
/// that no longer exists.
#[test]
fn a_queued_push_from_a_dead_session_is_dropped() {
    let mut state = AppState::default();
    let path = PathBuf::from("/tmp/relocked.kdbx");

    fresh_open(&mut state, path.clone(), "pw");
    let first_session = state.active_vault_session_id.expect("first session");
    state.queue_push_awaiting_restore(
        &path,
        QueuedSyncRequest {
            session: sync_session_of(first_session),
            interactive: false,
            bytes: Some(Arc::new(b"stale".to_vec())),
        },
    );

    // Same path, new session: the vault was locked and unlocked again.
    fresh_open(&mut state, path.clone(), "pw");

    assert!(
        state.take_push_awaiting_restore(&path).is_none(),
        "a push from the previous session is not replayed"
    );
    assert!(
        state.pushes_awaiting_restore.is_empty(),
        "and its ciphertext is not kept"
    );
}

/// The auto-sync tick recovers from `Failed` by re-uploading the whole
/// vault. Under a provider rate limit that fired every tick and kept the
/// limit alive, so a throttle has to park the vault until its window ends.
#[test]
fn a_throttle_parks_the_vault_until_its_window_passes() {
    let mut state = AppState::default();
    let path = PathBuf::from("/tmp/throttled.kdbx");

    assert!(!state.sync_is_backing_off(&path), "nothing to wait for yet");

    state.record_sync_throttle(&path, Some(Duration::from_secs(120)));
    assert!(state.sync_is_backing_off(&path));

    // A window already in the past no longer blocks.
    state.sync_backoff.insert(
        path.clone(),
        SyncBackoff {
            until: SystemTime::now() - Duration::from_secs(1),
            attempts: 1,
        },
    );
    assert!(!state.sync_is_backing_off(&path));
}

/// Without a `Retry-After` we pick the wait ourselves, and repeat
/// throttles have to grow it: a fixed-rate retry against a rate limit is
/// the behaviour that caused the problem.
#[test]
fn a_self_chosen_backoff_grows_and_stops_at_the_ceiling() {
    let mut state = AppState::default();
    let path = PathBuf::from("/tmp/throttled.kdbx");

    let mut waits = Vec::new();
    for _ in 0..6 {
        state.record_sync_throttle(&path, None);
        let backoff = state.sync_backoff.get(&path).copied().expect("armed");
        waits.push(
            backoff
                .until
                .duration_since(SystemTime::now())
                .unwrap_or_default()
                .as_secs(),
        );
    }
    assert!(waits[0] >= SYNC_BACKOFF_MIN.as_secs() - 1, "{waits:?}");
    assert!(
        waits[1] > waits[0],
        "second throttle waits longer: {waits:?}"
    );
    assert!(
        waits.iter().all(|w| *w <= SYNC_BACKOFF_MAX.as_secs()),
        "never past the ceiling: {waits:?}"
    );
}

/// A successful sync means the provider let us through, so the window is
/// over. The clear lives at the status choke point, not at each success.
#[test]
fn a_healthy_status_ends_the_backoff_window() {
    let mut state = AppState::default();
    let path = PathBuf::from("/tmp/throttled.kdbx");
    fresh_open(&mut state, path.clone(), "pw");

    state.record_sync_throttle(&path, Some(Duration::from_secs(600)));
    assert!(state.sync_is_backing_off(&path));

    state.route_sync_status(&path, SyncStatus::Idle);
    assert!(!state.sync_is_backing_off(&path));

    state.record_sync_throttle(&path, Some(Duration::from_secs(600)));
    state.route_sync_status(&path, SyncStatus::Failed("still broken".into()));
    assert!(
        state.sync_is_backing_off(&path),
        "another failure must not reset the window"
    );
}

/// `move_entry` deliberately leaves `selected_entry_id` pointing at an
/// entry that is no longer visible, because `vault_browser` falls back to
/// the first row. Anything reading the raw id disagreed with the panel:
/// after dragging the selected entry into another group the panel showed
/// one entry and the TOTP box showed a different entry's code.
#[test]
fn the_shown_entry_follows_the_list_not_a_stale_selection() {
    let mut state = AppState::default();
    let path = PathBuf::from("/tmp/shown.kdbx");
    fresh_open(&mut state, path, "pw");

    let visible = vec![
        VaultEntry {
            id: "visible".into(),
            ..VaultEntry::default()
        },
        VaultEntry {
            id: "also-visible".into(),
            ..VaultEntry::default()
        },
    ];
    let VaultStatus::Open {
        selected_entry_id,
        visible_entries,
        ..
    } = &mut state.vault
    else {
        unreachable!("just opened");
    };
    *visible_entries = Rc::new(visible);
    *selected_entry_id = Some("also-visible".into());
    assert_eq!(state.shown_entry_id(), Some("also-visible"));

    let VaultStatus::Open {
        selected_entry_id, ..
    } = &mut state.vault
    else {
        unreachable!()
    };
    // The selection was moved out of the current group.
    *selected_entry_id = Some("moved-away".into());
    assert_eq!(
        state.shown_entry_id(),
        Some("visible"),
        "falls back to the row the detail panel renders"
    );
}

/// A save that has settled into `Failed` never completes on its own. The
/// quit handler cannot borrow AppState, so it reads this signal to tell a
/// running save from a stuck one and stop vetoing forever.
#[test]
fn a_settled_save_failure_is_published_for_the_quit_handler() {
    let mut state = AppState::default();
    let path = PathBuf::from("/tmp/stalled.kdbx");
    fresh_open(&mut state, path.clone(), "pw");
    let session_id = state.active_vault_session_id.expect("session");

    state.set_save_status_for_session(&path, session_id, SaveStatus::Saving);
    state.publish_stalled_save();
    assert_eq!(stalled_save_vault(), None, "a running save is not stalled");

    state.set_save_status_for_session(
        &path,
        session_id,
        SaveStatus::Failed("volume disappeared".into()),
    );
    state.publish_stalled_save();
    assert_eq!(stalled_save_vault().as_deref(), Some("stalled.kdbx"));

    // A writer picking the work back up clears it again.
    state
        .saves_in_flight
        .insert(path.clone(), QueuedSave::new(session_id));
    state.publish_stalled_save();
    assert_eq!(stalled_save_vault(), None);
}

#[test]
fn deferred_lock_waits_for_dirty_session_and_writer_slot() {
    let mut state = AppState::default();
    let path = PathBuf::from("/tmp/queued-save.kdbx");
    fresh_open(&mut state, path.clone(), "pw");
    let session_id = state.active_vault_session_id.expect("session id");

    state.mark_vault_session_unpersisted(session_id);
    state
        .saves_in_flight
        .insert(path.clone(), QueuedSave::new(session_id));
    state.lock_requested_after_save = true;
    state.quarantine_sessions_for_deferred_lock();

    assert!(matches!(state.vault, VaultStatus::LockedPendingSave));
    assert!(!state.finish_deferred_lock_if_ready());
    assert!(!state.has_any_unlocked());
    assert!(state.unlocked_paths().is_empty());
    assert!(state.vault_browser().is_none());
    assert!(state.database_clone_for(&path).is_none());
    assert!(state.snapshot_sync_inputs(&path).is_none());
    assert!(!state.vault_session_is_current(&path, session_id));
    assert!(state.save_session_is_retained(&path, session_id));
    assert!(state.save_payload_for_session(&path, session_id).is_some());

    // Even after the payload is known durable, the writer slot itself
    // must be released before decrypted state can be discarded.
    state.mark_vault_session_persisted(session_id);
    assert!(!state.finish_deferred_lock_if_ready());
    state.saves_in_flight.remove(&path);

    assert!(state.finish_deferred_lock_if_ready());
    assert!(!state.has_any_unlocked());
    assert!(state.active_vault_session_id.is_none());
    assert!(state.deferred_lock_sessions.is_empty());
    assert!(matches!(state.vault, VaultStatus::Empty));
}

#[test]
fn failed_save_keeps_deferred_lock_armed_but_inaccessible() {
    let mut state = AppState::default();
    let path = PathBuf::from("/tmp/failed-save.kdbx");
    fresh_open(&mut state, path.clone(), "pw");
    let session_id = state.active_vault_session_id.expect("session id");

    state.mark_vault_session_unpersisted(session_id);
    state.set_save_status_for_session(&path, session_id, SaveStatus::Failed("disk full".into()));
    state.lock_requested_after_save = true;
    state.quarantine_sessions_for_deferred_lock();

    assert!(!state.finish_deferred_lock_if_ready());
    assert!(matches!(state.vault, VaultStatus::LockedPendingSave));
    assert!(!state.has_any_unlocked());
    assert!(state.vault_browser().is_none());
    assert!(state.unlocked_paths().is_empty());
    assert!(state.lock_requested_after_save);
    assert!(state.unpersisted_vault_sessions.contains(&session_id));
    assert!(state.save_payload_for_session(&path, session_id).is_some());
    assert_eq!(state.save_status, SaveStatus::Failed("disk full".into()));
}

#[test]
fn discard_deferred_saves_unwedges_a_permanently_failing_lock() {
    let mut state = AppState::default();
    let path = PathBuf::from("/tmp/wedged-save.kdbx");
    fresh_open(&mut state, path.clone(), "pw");
    let session_id = state.active_vault_session_id.expect("session id");

    state.mark_vault_session_unpersisted(session_id);
    state.set_save_status_for_session(&path, session_id, SaveStatus::Failed("volume gone".into()));
    state.lock_requested_after_save = true;
    state.quarantine_sessions_for_deferred_lock();

    assert!(state.can_discard_deferred_saves());
    assert!(state.discard_deferred_saves_and_lock_now());

    assert!(matches!(state.vault, VaultStatus::Empty));
    assert!(state.deferred_lock_sessions.is_empty());
    assert!(state.unpersisted_vault_sessions.is_empty());
    assert!(!state.has_unpersisted_save_work());
    assert!(!state.lock_requested_after_save);
    assert_eq!(state.save_status, SaveStatus::Idle);

    // Not offered while the vault is healthy or a save could still land.
    assert!(!state.can_discard_deferred_saves());
}

#[test]
fn discard_aborts_every_writing_slot_before_releasing_deferred_state() {
    let mut state = AppState::default();
    let path = PathBuf::from("/tmp/writing-save.kdbx");
    fresh_open(&mut state, path.clone(), "pw");
    let session_id = state.active_vault_session_id.expect("session id");

    state.mark_vault_session_unpersisted(session_id);
    let queued = QueuedSave::new(session_id);
    let abort = queued.abort.clone();
    state.saves_in_flight.insert(path, queued);
    state.lock_requested_after_save = true;
    state.quarantine_sessions_for_deferred_lock();
    state.deferred_lock_since = Some(
        std::time::Instant::now()
            .checked_sub(DEFERRED_LOCK_STUCK_AFTER + std::time::Duration::from_secs(1))
            .expect("representable deadline"),
    );

    assert!(state.can_discard_deferred_saves());
    assert!(state.discard_deferred_saves_and_lock_now());

    assert!(matches!(state.vault, VaultStatus::Empty));
    assert!(state.deferred_lock_sessions.is_empty());
    assert!(state.unpersisted_vault_sessions.is_empty());
    assert!(state.saves_in_flight.is_empty());
    assert!(abort.try_abort(), "the worker remains safely aborted");
    assert!(!abort.publication_started());
}

#[test]
fn mixed_discard_keeps_all_state_when_one_writer_is_publishing() {
    let mut state = AppState::default();
    let writing_path = PathBuf::from("/tmp/mixed-writing-save.kdbx");
    let publishing_path = PathBuf::from("/tmp/mixed-publishing-save.kdbx");

    fresh_open(&mut state, writing_path.clone(), "writing-pw");
    let writing_session = state.active_vault_session_id.expect("writing session");
    state.mark_vault_session_unpersisted(writing_session);
    let writing = QueuedSave::new(writing_session);
    let writing_abort = writing.abort.clone();
    state.saves_in_flight.insert(writing_path.clone(), writing);
    state.park_active();

    fresh_open(&mut state, publishing_path.clone(), "publishing-pw");
    let publishing_session = state.active_vault_session_id.expect("publishing session");
    state.mark_vault_session_unpersisted(publishing_session);
    let publishing = QueuedSave::new(publishing_session);
    let publishing_abort = publishing.abort.clone();
    assert!(publishing_abort.try_begin_publish());
    state
        .saves_in_flight
        .insert(publishing_path.clone(), publishing);

    state.lock_requested_after_save = true;
    state.quarantine_sessions_for_deferred_lock();
    state.deferred_lock_since = Some(
        std::time::Instant::now()
            .checked_sub(DEFERRED_LOCK_STUCK_AFTER + std::time::Duration::from_secs(1))
            .expect("representable deadline"),
    );

    assert!(
        !state.can_discard_deferred_saves(),
        "the UI must not offer an action once publication is known"
    );
    assert!(!state.discard_deferred_saves_and_lock_now());

    assert!(matches!(state.vault, VaultStatus::LockedPendingSave));
    assert!(state.lock_requested_after_save);
    assert!(state.deferred_save_is_publishing());
    assert_eq!(state.deferred_lock_sessions.len(), 2);
    assert!(state.deferred_lock_sessions.contains_key(&writing_session));
    assert!(
        state
            .deferred_lock_sessions
            .contains_key(&publishing_session)
    );
    assert!(state.unpersisted_vault_sessions.contains(&writing_session));
    assert!(
        state
            .unpersisted_vault_sessions
            .contains(&publishing_session)
    );
    assert_eq!(state.saves_in_flight.len(), 2);
    assert!(
        state
            .save_payload_for_session(&writing_path, writing_session)
            .is_some()
    );
    assert!(
        state
            .save_payload_for_session(&publishing_path, publishing_session)
            .is_some()
    );
    assert!(has_unpersisted_vault_saves());

    assert!(writing_abort.try_abort(), "the writing sibling was aborted");
    assert!(!writing_abort.publication_started());
    assert!(publishing_abort.publication_started());
    assert!(!publishing_abort.try_abort());
}

#[test]
fn deferred_lock_quarantines_all_dirty_sessions_and_drops_clean_ones() {
    let mut state = AppState::default();
    let dirty_parked = PathBuf::from("/tmp/dirty-parked.kdbx");
    let clean_parked = PathBuf::from("/tmp/clean-parked.kdbx");
    let dirty_active = PathBuf::from("/tmp/dirty-active.kdbx");

    fresh_open(&mut state, dirty_parked.clone(), "pw-a");
    let parked_session = state.active_vault_session_id.expect("parked session");
    state.mark_vault_session_unpersisted(parked_session);
    state.park_active();

    fresh_open(&mut state, clean_parked.clone(), "pw-clean");
    let clean_session = state.active_vault_session_id.expect("clean session");
    state.park_active();

    fresh_open(&mut state, dirty_active.clone(), "pw-b");
    let active_session = state.active_vault_session_id.expect("active session");
    state.mark_vault_session_unpersisted(active_session);
    state.lock_requested_after_save = true;

    state.quarantine_sessions_for_deferred_lock();

    assert!(matches!(state.vault, VaultStatus::LockedPendingSave));
    assert!(state.parked.is_empty());
    assert!(state.parked_order.is_empty());
    assert!(state.unlocked_paths().is_empty());
    assert_eq!(state.deferred_lock_sessions.len(), 2);
    assert_eq!(
        state
            .deferred_lock_sessions
            .get(&parked_session)
            .map(|session| &session.path),
        Some(&dirty_parked)
    );
    assert_eq!(
        state
            .deferred_lock_sessions
            .get(&active_session)
            .map(|session| &session.path),
        Some(&dirty_active)
    );
    assert!(!state.deferred_lock_sessions.contains_key(&clean_session));
}

#[test]
fn failed_merge_never_overwrites_newer_quarantined_mutations() {
    let mut state = AppState::default();
    let path = PathBuf::from("/tmp/merged-retry.kdbx");
    fresh_open(&mut state, path.clone(), "pw");
    let session_id = state.active_vault_session_id.expect("session id");
    state.mark_vault_session_unpersisted(session_id);
    state.lock_requested_after_save = true;
    state.quarantine_sessions_for_deferred_lock();
    let base_generation = state
        .document_generation_for_session(&path, session_id)
        .expect("quarantined generation");

    assert!(!state.install_failed_merge_for_deferred_retry(
        &path,
        sync_session_of(session_id),
        base_generation,
        Some(session_id),
        Database::new(),
    ));
    assert_eq!(
        state.document_generation_for_session(&path, session_id),
        Some(base_generation)
    );

    state
        .deferred_lock_sessions
        .get_mut(&session_id)
        .expect("quarantined session")
        .document
        .replace_database(Database::new());
    let newer_generation = state
        .document_generation_for_session(&path, session_id)
        .expect("newer quarantined generation");
    assert!(newer_generation > base_generation);
    assert!(!state.install_failed_merge_for_deferred_retry(
        &path,
        sync_session_of(session_id),
        base_generation,
        None,
        Database::new(),
    ));
    assert_eq!(
        state.document_generation_for_session(&path, session_id),
        Some(newer_generation)
    );

    assert!(state.install_failed_merge_for_deferred_retry(
        &path,
        sync_session_of(session_id),
        newer_generation,
        None,
        Database::new(),
    ));
    assert!(
        state
            .document_generation_for_session(&path, session_id)
            .is_some_and(|updated| updated > newer_generation)
    );
    assert!(state.save_payload_for_session(&path, session_id).is_some());
}

#[test]
fn invalid_selection_cannot_replace_locked_save_retry_state() {
    let mut state = AppState::default();
    let path = PathBuf::from("/tmp/retry-after-selection.kdbx");
    fresh_open(&mut state, path.clone(), "pw");
    let session_id = state.active_vault_session_id.expect("session id");
    state.mark_vault_session_unpersisted(session_id);
    state.set_save_status_for_session(&path, session_id, SaveStatus::Failed("disk full".into()));
    state.lock_requested_after_save = true;
    state.quarantine_sessions_for_deferred_lock();

    state.record_vault_selection_failure(
        Some(PathBuf::from("/tmp/not-a-vault.txt")),
        "Selected file is not a .kdbx database.".into(),
    );

    assert!(matches!(state.vault, VaultStatus::LockedPendingSave));
    assert!(state.lock_requested_after_save);
    assert!(state.save_session_is_retained(&path, session_id));
    assert!(state.save_payload_for_session(&path, session_id).is_some());
    assert_eq!(
        state.take_pending_notice().as_deref(),
        Some("Selected file is not a .kdbx database.")
    );
}

#[test]
fn snapshot_sync_inputs_works_for_active_and_parked() {
    let mut state = AppState::default();
    let parked_path = PathBuf::from("/tmp/parked.kdbx");
    let active_path = PathBuf::from("/tmp/active.kdbx");

    // Active vault with binding A.
    fresh_open(&mut state, parked_path.clone(), "pw-parked");
    state.sync = Some(fake_binding("parked@example.invalid"));
    state.park_active();

    // Active vault swapped in, with binding B.
    fresh_open(&mut state, active_path.clone(), "pw-active");
    state.sync = Some(fake_binding("active@example.invalid"));

    // snapshot_sync_inputs against the parked path returns the
    // parked vault's binding - not the active one. This is the
    // contract sync_now_for_path relies on.
    let (parked_config, _) = state.snapshot_sync_inputs(&parked_path).expect("parked");
    assert_eq!(parked_config.account_email, "parked@example.invalid");

    let (active_config, _) = state.snapshot_sync_inputs(&active_path).expect("active");
    assert_eq!(active_config.account_email, "active@example.invalid");

    // Unknown path → None.
    assert!(
        state
            .snapshot_sync_inputs(&PathBuf::from("/tmp/nowhere.kdbx"))
            .is_none()
    );
}

#[test]
fn with_sync_binding_mut_for_targets_correct_slot() {
    // Repro for the High #1 ETag-write race: after a switch, an
    // upload callback that finishes for vault A must update A's
    // binding, not the now-active B.
    let mut state = AppState::default();
    let saving_path = PathBuf::from("/tmp/saving.kdbx");
    let active_path = PathBuf::from("/tmp/active.kdbx");

    fresh_open(&mut state, saving_path.clone(), "pw");
    state.sync = Some(fake_binding("saver@example.invalid"));
    state.park_active();
    fresh_open(&mut state, active_path.clone(), "pw");
    state.sync = Some(fake_binding("active@example.invalid"));

    // Simulate the upload callback writing a fresh etag for the
    // saving (parked) vault.
    state.with_sync_binding_mut_for(&saving_path, |b| {
        b.config.last_etag = "etag-new".into();
    });

    // Saving (parked) vault picked up the new etag.
    assert_eq!(
        state
            .parked
            .get(&saving_path)
            .and_then(|p| p.sync.as_ref())
            .map(|b| b.config.last_etag.clone()),
        Some("etag-new".into()),
    );
    // Active vault is untouched.
    assert_eq!(
        state.sync.as_ref().unwrap().config.last_etag,
        "etag-0".to_string(),
    );
}

#[test]
fn lock_vault_clears_parked_and_resets_sync_fields() {
    let mut state = AppState::default();
    let parked_path = PathBuf::from("/tmp/parked.kdbx");

    fresh_open(&mut state, parked_path.clone(), "pw");
    state.sync = Some(fake_binding("user@example.invalid"));
    state.sync_status = SyncStatus::Idle;
    state.park_active();

    // Open another vault as active.
    fresh_open(&mut state, PathBuf::from("/tmp/active.kdbx"), "pw");
    state.sync = Some(fake_binding("user2@example.invalid"));
    state.sync_status = SyncStatus::Idle;

    state.lock_vault_now();

    assert!(state.parked.is_empty());
    assert!(state.parked_order.is_empty());
    assert!(state.sync.is_none());
    assert!(!state.has_any_unlocked());
    assert!(state.active_vault_session_id.is_none());
}
