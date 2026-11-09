//! One Touch ID flow for the lock screen and the quick access panel.
//!
//! Both unlock a vault the same way: [`AppState::begin_biometric_unlock`]
//! stamps a new generation, and the keychain read behind the Touch ID prompt
//! runs in the background. As soon as macOS has answered, `on_answered` runs
//! on the foreground. With the password, the vault is then opened in the
//! background, and the answer is applied on the foreground with `begin_open`
//! and `finish_open_attempt`; a failed prompt goes to `fail_biometric_unlock`
//! right after `on_answered`. The prompt reads "Unlock <file name>". What
//! differs is who may start it and what happens around the answer:
//!
//! - The lock screen calls [`unlock_from_lock_screen`] when you click the
//!   Touch ID button or use its shortcut, and on its own when the screen
//!   comes up. If macOS withdraws that automatic prompt, the lock screen
//!   asks once more; that rule stays with the shell, in the `on_done` it
//!   passes.
//! - The quick access panel calls [`unlock_from_panel`], which takes a
//!   [`Confirmed`] token, and only [`confirm`] makes one: for Return and for
//!   a click on the unlock button. The panel opening, typing, the panel
//!   coming back to the front, a timer, the typed-ahead buffer expiring and
//!   macOS withdrawing a prompt make none, so none of them can raise the
//!   Touch ID prompt. The call uses the token up, so the next attempt needs
//!   a new confirmation. There is no second attempt either: a withdrawn
//!   prompt comes back as [`Answer::Interrupted`] and nothing else happens.
//!   And the panel never answers a cost question: while one is open, or the
//!   vault's header has not been read yet, no attempt starts. Nor does one
//!   start while an unlock is under way, from the prompt until the vault is
//!   open or the attempt has ended: a Return or a click then starts nothing,
//!   and `locked_panel::kind` says `Unlocking`.
//! - As soon as macOS has answered, before the vault is opened and before
//!   the answer is applied, `on_answered` runs on the foreground. The Touch
//!   ID dialog takes key focus from the panel and macOS does not give it
//!   back, so the panel makes itself key there and focuses the search field
//!   or the list. Opening the vault can take seconds with an expensive key
//!   derivation, and keys typed in that time must not go to another app.
//!   The next Return reaches the panel only after that. The lock screen has
//!   nothing to do there.
//!
//! The rules both share:
//!
//! - An answer from an attempt that a newer one replaced (its generation no
//!   longer matches) opens nothing and ends the attempt in flight
//!   ([`Answer::Stale`]). That is checked when the answer comes, so a stale
//!   password never goes on to the open, and again once the open is done,
//!   for an attempt replaced while the vault was being opened.
//! - The flow runs in a task this module detaches. Nobody holds a handle, so
//!   no caller can cancel an unlock by dropping something, and neither the
//!   shell nor the panel needs a field for it. The functions return only the
//!   generation of the attempt they started, or `None` when none started.
//! - The flow itself is private. The public functions are the only ways in,
//!   and none of them takes the origin as a plain value.

use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui::{App, AppContext as _, Context, Entity};

use crate::app::{AppState, BiometricAttempt, BiometricLaunch, VaultStatus};
use crate::biometric::{BiometricError, BiometricResult, RetrieveOptions};
use crate::keepass::KeePassRepository;
use crate::ui::errors;
use zeroize::Zeroizing;

/// How long the panel's gate waits, after Touch ID, for the app it is about
/// to fill to come back to the front. GlobalProtect took up to 1.3 seconds
/// after the Touch ID dialog closed. Nothing here waits; the code that fills
/// other apps reads this.
pub const TARGET_RETURN_WAIT: Duration = Duration::from_secs(2);

/// Where an unlock was asked for. Public so a reader of an [`Answer`] can
/// name it, but no public function takes it: the panel's way in is
/// [`unlock_from_panel`] with a token, the lock screen's
/// [`unlock_from_lock_screen`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnlockOrigin {
    LockScreen,
    Panel,
}

/// What happened in the quick access panel. Only the two that are the user
/// asking to unlock count as a confirmation ([`confirm`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelTrigger {
    /// Return, in the locked panel.
    ReturnKey,
    /// A click on the panel's unlock button.
    UnlockButtonClick,
    /// The panel opened.
    Opened,
    /// A key that goes into the typed-ahead buffer.
    Typed,
    /// The panel came back to the front.
    Activated,
    /// A timer fired.
    Timer,
    /// macOS withdrew the previous prompt.
    AfterInterrupted,
    /// The typed-ahead buffer expired.
    AfterExpiry,
}

/// Proof that [`confirm`] was asked with Return or the unlock button. Only
/// `confirm` makes one. The field is private, so no code outside this module
/// can build one, and it is neither `Clone` nor `Copy`: [`unlock_from_panel`]
/// takes it by value, so each attempt uses one up.
///
/// The token can't tell whether the key press was real; the code that asks
/// `confirm` has to be the panel's handler for that key or click.
/// `TOUCH_ID_CALL_SITES` in `tests/architecture.rs` lists every call of
/// `confirm` and of both ways into the flow with the function it is in.
///
/// Neither of these compiles:
///
/// ```compile_fail,E0603
/// let made_up = ferrispass::app::unlock::touch_id::Confirmed(());
/// ```
///
/// ```compile_fail,E0599
/// use ferrispass::app::unlock::touch_id::{PanelTrigger, confirm};
/// let token = confirm(PanelTrigger::ReturnKey).unwrap();
/// let twice = token.clone();
/// ```
#[derive(Debug)]
#[must_use = "a confirmation starts nothing until it goes to unlock_from_panel"]
pub struct Confirmed(());

/// The one table that turns something that happened in the panel into a
/// confirmation: Return and a click on the unlock button do, nothing else
/// does.
pub fn confirm(trigger: PanelTrigger) -> Option<Confirmed> {
    match trigger {
        PanelTrigger::ReturnKey | PanelTrigger::UnlockButtonClick => Some(Confirmed(())),
        PanelTrigger::Opened
        | PanelTrigger::Typed
        | PanelTrigger::Activated
        | PanelTrigger::Timer
        | PanelTrigger::AfterInterrupted
        | PanelTrigger::AfterExpiry => None,
    }
}

/// What runs on the foreground as soon as macOS has answered, before the
/// vault is opened and before the answer is applied.
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

/// The quick access panel's Touch ID unlock, for the token a confirmation
/// gave. `on_answered` runs as soon as macOS has answered, before the vault
/// is opened and before the answer is applied: the panel makes itself key
/// there. `on_done` gets the answer once it is applied. Returns the
/// generation of the attempt, or `None` when none started: no vault with an
/// enrolment on the unlock screen, a cost question open, the vault's header
/// not read yet, or an unlock already under way. A refused token is used up
/// all the same, and neither callback runs.
pub fn unlock_from_panel(
    token: Confirmed,
    state: Entity<AppState>,
    allow_device_passcode: bool,
    on_answered: OnAnswered,
    on_done: OnDone,
    cx: &mut App,
) -> Option<u64> {
    let Confirmed(()) = token;
    run(
        UnlockOrigin::Panel,
        state,
        allow_device_passcode,
        on_answered,
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
    // Nor does it start a second attempt while one is under way: the new
    // generation would make the first answer stale, that answer would end
    // the new attempt too, and the vault would stay locked after two
    // prompts answered with a finger.
    if origin == UnlockOrigin::Panel
        && (!cost_question_settled(state.read(cx)) || unlock_under_way(state.read(cx)))
    {
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
    // the background. `begin_open` comes only with the opened vault: until
    // then, the unlock screen stays as it is.
    let asked = cx.background_spawn(async move { store.retrieve(&id, &prompt, options) });

    let state = state.downgrade();
    cx.spawn(async move |cx| {
        let retrieved = asked.await;
        // macOS has answered. `on_answered` comes first, before the open: the
        // key derivation can take seconds, and the panel must not be without
        // key focus for that long.
        let Some((password, on_done)) = cx.update(|cx| {
            on_answered(cx);
            let next = state
                .update(cx, |state, cx| {
                    take_answer(retrieved, &path, generation, state, cx)
                })
                .unwrap_or(Err(Answer::Stale));
            match next {
                Ok(password) => Some((password, on_done)),
                Err(answer) => {
                    on_done(answer, cx);
                    None
                }
            }
        }) else {
            return;
        };

        let opened = cx.background_spawn(async move {
            let result = KeePassRepository::open(&path, &password, keyfile.as_deref());
            // `password` (Zeroizing) drops here, wiping the buffer.
            BiometricUnlockOutcome::Open {
                path,
                result,
                generation,
            }
        });
        let outcome = opened.await;
        cx.update(|cx| {
            let answer = state
                .update(cx, |state, cx| apply(outcome, state, cx))
                .unwrap_or(Answer::Stale);
            on_done(answer, cx);
        });
    })
    .detach();
    Some(generation)
}

/// What becomes of macOS's answer once `on_answered` has run. The password
/// goes on to the open, unless a newer attempt replaced this one while the
/// prompt was up: then it opens nothing and ends the attempt in flight, as
/// [`apply`] does with a stale answer. A failed prompt is applied at once.
fn take_answer(
    retrieved: BiometricResult<Zeroizing<String>>,
    path: &Path,
    generation: u64,
    state: &mut AppState,
    cx: &mut Context<AppState>,
) -> Result<Zeroizing<String>, Answer> {
    match retrieved {
        Ok(password) if state.biometric_unlock_is_current(generation) => Ok(password),
        // The password drops unused with `retrieved`, which wipes it.
        Ok(_) => {
            state.clear_biometric_attempt_public();
            Err(Answer::Stale)
        }
        Err(err) => Err(apply(
            BiometricUnlockOutcome::Failed {
                path: path.to_path_buf(),
                err,
                generation,
            },
            state,
            cx,
        )),
    }
}

/// Whether the vault waiting on the unlock screen has had its header read
/// and has no cost left to confirm.
pub(super) fn cost_question_settled(state: &AppState) -> bool {
    state.kdf_check_settled_for_pending() && state.kdf_confirmation_for_pending().is_none()
}

/// Whether an unlock is under way: a Touch ID attempt waits for its answer
/// or for its vault to open (the attempt stays in flight until the answer is
/// applied), or the vault is being opened with its password.
pub(super) fn unlock_under_way(state: &AppState) -> bool {
    matches!(state.biometric_attempt(), BiometricAttempt::InFlight { .. })
        || matches!(state.vault_status(), VaultStatus::Opening { .. })
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

/// Either-shaped result of a Touch ID unlock, applied on the foreground.
/// `Open` carries the KDF outcome so the success path runs the same
/// `finish_open_attempt` codepath as a password-typed unlock; `Failed`
/// carries the OS-side `BiometricError` so the Unlock screen can render the
/// right error message and (when applicable) clear an invalidated enrolment.
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
