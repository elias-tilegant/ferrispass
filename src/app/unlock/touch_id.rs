//! One Touch ID flow for the lock screen and the quick access panel.
//!
//! Both unlock a vault the same way: [`AppState::begin_biometric_unlock`]
//! stamps a new generation, the keychain read behind the Touch ID prompt and
//! the open run in the background, and the answer is applied on the
//! foreground with `begin_open` and `finish_open_attempt`, or
//! `fail_biometric_unlock`. The prompt reads "Unlock <file name>". What
//! differs is who may start it and what happens around the answer:
//!
//! - The lock screen calls [`unlock_from_lock_screen`] when you click the
//!   Touch ID button or use its shortcut, and on its own when the screen
//!   comes up. If macOS withdraws that automatic prompt, the lock screen
//!   asks once more; that rule stays with the shell, in the `on_done` it
//!   passes.
//! - Before the answer is applied, `on_answered` runs on the foreground. The
//!   lock screen has nothing to do there.
//!
//! The rules both share:
//!
//! - An answer from an attempt that a newer one replaced (its generation no
//!   longer matches) opens nothing and ends the attempt in flight
//!   ([`Answer::Stale`]).
//! - The flow runs in a task this module detaches. Nobody holds a handle, so
//!   no caller can cancel an unlock by dropping something, and neither the
//!   shell nor the panel needs a field for it. The functions return only the
//!   generation of the attempt they started, or `None` when none started.
//! - The flow itself is private. The public functions are the only ways in,
//!   and none of them takes the origin as a plain value.

use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui::{App, AppContext as _, Context, Entity};

use crate::app::{AppState, BiometricLaunch, VaultStatus};
use crate::biometric::{BiometricError, RetrieveOptions};
use crate::keepass::KeePassRepository;
use crate::ui::errors;

/// How long the panel's gate waits, after Touch ID, for the app it is about
/// to fill to come back to the front. GlobalProtect took up to 1.3 seconds
/// after the Touch ID dialog closed. Nothing here waits; the code that fills
/// other apps reads this.
pub const TARGET_RETURN_WAIT: Duration = Duration::from_secs(2);

/// Where an unlock was asked for. Public so a reader of an [`Answer`] can
/// name it, but no public function takes it: each origin has a function of
/// its own, such as [`unlock_from_lock_screen`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnlockOrigin {
    LockScreen,
    Panel,
}

/// What runs on the foreground as soon as macOS has answered, before the
/// answer is applied.
pub type OnAnswered = Box<dyn FnOnce(&mut App) + 'static>;

/// What runs once the answer is applied, with the answer.
pub type OnDone = Box<dyn FnOnce(Answer, &mut App) + 'static>;

/// How an attempt ended, handed to `on_done`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// The vault at this path is open.
    Opened(PathBuf),
    /// Touch ID gave the stored password, but the vault did not open: the
    /// password no longer fits, the file is gone, or the vault's cost has to
    /// be confirmed first. The unlock screen says which.
    NotOpened,
    /// Touch ID failed or was cancelled; the unlock screen shows why.
    Failed(BiometricError),
    /// macOS took the prompt away before anyone answered it (the app lost
    /// focus, or there was no way to show it). Not an answer from the user.
    Interrupted,
    /// A newer attempt replaced this one, or the user moved on.
    Stale,
}

/// The lock screen's Touch ID unlock. `on_done` gets the answer once it is
/// applied. Returns the generation of the attempt, or `None` when none
/// started (no vault waiting on the unlock screen, or no enrolment for it).
pub fn unlock_from_lock_screen(
    state: Entity<AppState>,
    allow_device_passcode: bool,
    on_done: OnDone,
    cx: &mut App,
) -> Option<u64> {
    run(
        UnlockOrigin::LockScreen,
        state,
        allow_device_passcode,
        Box::new(|_| {}),
        on_done,
        cx,
    )
}

/// The flow behind every public entry.
fn run(
    origin: UnlockOrigin,
    state: Entity<AppState>,
    allow_device_passcode: bool,
    on_answered: OnAnswered,
    on_done: OnDone,
    cx: &mut App,
) -> Option<u64> {
    // The panel never answers a cost question: the lock screen's click
    // counts as allowing the cost it shows, a Return in the panel must not.
    if origin == UnlockOrigin::Panel && !cost_question_settled(state.read(cx)) {
        return None;
    }
    // Only valid while a vault with an enrolment waits on the unlock screen;
    // `begin_biometric_unlock` checks that and stamps a new generation, which
    // makes any attempt still in flight stale.
    let BiometricLaunch {
        id,
        path,
        keyfile,
        store,
        generation,
    } = state.update(cx, |state, cx| state.begin_biometric_unlock(cx))?;
    let prompt = format!("Unlock {}", file_name_for_prompt(&path));
    let options = RetrieveOptions {
        allow_device_passcode,
    };

    // The keychain read blocks for as long as the prompt is up, so it runs in
    // the background, and the open with it. `begin_open` waits for the
    // answer: until macOS has one, the unlock screen stays as it is.
    let answered = cx.background_spawn(async move {
        match store.retrieve(&id, &prompt, options) {
            Ok(password) => {
                let result = KeePassRepository::open(&path, &password, keyfile.as_deref());
                // `password` (Zeroizing) drops here, wiping the buffer.
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

    let state = state.downgrade();
    cx.spawn(async move |cx| {
        let outcome = answered.await;
        cx.update(|cx| {
            on_answered(cx);
            let answer = state
                .update(cx, |state, cx| apply(outcome, state, cx))
                .unwrap_or(Answer::Stale);
            on_done(answer, cx);
        });
    })
    .detach();
    Some(generation)
}

/// Whether the vault waiting on the unlock screen has had its header read
/// and has no cost left to confirm.
fn cost_question_settled(state: &AppState) -> bool {
    state.kdf_check_settled_for_pending() && state.kdf_confirmation_for_pending().is_none()
}

/// Applies the answer to the state, or drops it when it is stale.
fn apply(
    outcome: BiometricUnlockOutcome,
    state: &mut AppState,
    cx: &mut Context<AppState>,
) -> Answer {
    match outcome {
        BiometricUnlockOutcome::Open {
            path,
            result,
            generation,
        } => {
            // A Touch ID prompt the user satisfied *after* cancelling or
            // starting another attempt (even for the same vault) carries a
            // stale generation; ignore it so it can't drive the current
            // screen.
            if !state.biometric_unlock_is_current(generation) {
                state.clear_biometric_attempt_public();
                return Answer::Stale;
            }
            let result = result.map_err(|error| {
                note_kdf_confirmation(state, &path, &error, cx);
                errors::open_message(&error)
            });
            state.begin_open(path.clone(), cx);
            let _ = state.finish_open_attempt(path.clone(), result, cx);
            let opened = matches!(
                state.vault_status(),
                VaultStatus::Open { path: open, .. } if *open == path
            );
            if opened {
                Answer::Opened(path)
            } else {
                Answer::NotOpened
            }
        }
        BiometricUnlockOutcome::Failed {
            path,
            err,
            generation,
        } => {
            if !state.biometric_unlock_is_current(generation) {
                return Answer::Stale;
            }
            let answer = if err == BiometricError::Interrupted {
                Answer::Interrupted
            } else {
                Answer::Failed(err.clone())
            };
            state.fail_biometric_unlock(path, generation, err, cx);
            answer
        }
    }
}

/// A failed open that turns out to be a cost question (the file changed
/// after the pre-check, or the pre-check had not landed yet) puts the notice
/// on the unlock screen instead of an error the user cannot act on.
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

/// Either-shaped result the biometric-unlock background task hands
/// back to the foreground update. `Open` carries the KDF outcome so
/// the success path runs the same `finish_open_attempt` codepath as a
/// password-typed unlock; `Failed` carries the OS-side
/// `BiometricError` so the Unlock screen can render the right error
/// message and (when applicable) clear an invalidated enrolment.
///
/// The `Open` variant is large (it owns a `VaultDocument`) while
/// `Failed` is small; we don't box it because the value is moved
/// exactly once per unlock and the surrounding unlock path already
/// passes `VaultDocument` by value.
#[allow(clippy::large_enum_variant)]
enum BiometricUnlockOutcome {
    Open {
        path: PathBuf,
        result: Result<crate::keepass::VaultDocument, keepass::db::DatabaseOpenError>,
        /// Generation of the launch that produced this outcome, so a
        /// stale resolution can be discarded.
        generation: u64,
    },
    Failed {
        path: PathBuf,
        err: crate::biometric::BiometricError,
        generation: u64,
    },
}

/// UI-friendly file name for the Touch ID prompt header. Strips the
/// `.kdbx` suffix so the prompt reads "Unlock my-vault" rather than
/// "Unlock my-vault.kdbx". Used only for human-facing strings - never
/// as a keychain account or any other security-relevant identifier.
fn file_name_for_prompt(path: &std::path::Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "vault".to_string())
}

#[cfg(test)]
mod tests;
