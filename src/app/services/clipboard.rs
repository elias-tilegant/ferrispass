//! The one way a secret reaches the clipboard.
//!
//! Every copy of a password, a username, a URL, a TOTP code or a custom field
//! goes through [`ClipboardService::write_secret`]. The service lives in a
//! GPUI global rather than in a window, so a caller without a window copies
//! and clears the same way the main window does.
//!
//! The rules, each held by a test at the end of this file:
//!
//! 1. `write_secret` is the only way in. When the secret-aware write fails,
//!    nothing is written, a copy of ours that is still on the clipboard is
//!    cleared, and the caller gets the error. There is no fallback to a plain
//!    clipboard item that a history tool or Universal Clipboard would keep
//!    (`write_failure_fails_closed`).
//! 2. The caller passes the clear delay, from the user's setting. `None`
//!    ("Never") starts no timer, but the service still remembers the copy, so
//!    a lock can clear it (`never_setting_keeps_ownership_for_lock`).
//! 3. A new copy replaces the previous copy's timer. A timer that fires for a
//!    copy that is no longer the current one does nothing
//!    (`timer_clears_only_its_own_write`).
//! 4. Clearing always compares the pasteboard generation first. If anyone
//!    copied something after us, the clipboard stays as it is
//!    (`clear_leaves_newer_clipboard_alone`).
//! 5. Quitting clears our copy twice over: `request_quit` calls
//!    [`clear_owned`] before it asks the platform to quit, and
//!    [`install_quit_hook`] registers the same clear with `on_app_quit` for
//!    a quit that does not come through that action
//!    (`quit_clears_only_unchanged_own_copy`).
//! 6. At launch, before the first window, [`clear_own_copy_on_start`]
//!    removes a copy an earlier run left behind, a crash or a forced quit
//!    for example. It reads the access setting first and goes no further
//!    unless the user chose "Always Allow" for FerrisPass: with any other
//!    setting it does not even ask for the types on the pasteboard. With
//!    "Always Allow" it clears only when our marker type is there, and only
//!    if nothing was copied between that check and the clear. The decision
//!    is [`start_action`] (`start_clears_only_own_marked_copy`).
//! 7. Neither the service nor the launch check reads what is on the
//!    clipboard. [`Pasteboard`] has no method for it, a test pasteboard
//!    counts every read (`start_path_never_reads_content`), and a scan in
//!    `tests/architecture.rs` keeps the content readers of `NSPasteboard`
//!    out of the clipboard code outside its tests.
//! 8. Every lock ends in [`clear_owned`]: the main window's lock, and the
//!    shell's state observer, which wipes whenever the open vault closes or
//!    changes. Closing the window and the idle and screen locks get there
//!    through the observer (`lock_clears_clipboard_ownership` in
//!    `src/ui/legacy/shell/clipboard_tests.rs`).
//!
//! The service remembers the generation its write created, never the secret.
//!
//! Why the launch check is this careful: macOS asks the user before an app
//! reads another app's clipboard contents. A password manager that shows that
//! prompt at every launch teaches people to click it away. Asking for types
//! and the access setting shows nothing, and with any setting other than
//! "Always Allow" the check does not even do that much. A copy left behind
//! under those settings stays until something else is copied; it is still
//! concealed, transient and kept to this Mac.
//!
//! The marker type is the only sign of ownership a new launch has: the
//! generation of an earlier run's write is not kept anywhere. Any app can put
//! an item with that type on the clipboard, and with "Always Allow" a launch
//! clears it. That costs the other app the item it labelled as ours and
//! nothing else, and a clipboard history tool that puts one of our items
//! back gets it cleared, which is the point.

use crate::clipboard::{
    AccessBehavior, ClipboardChangeCount, ClipboardError, Pasteboard, SystemPasteboard,
};
#[cfg(not(target_os = "macos"))]
use gpui::ClipboardItem;
use gpui::{App, AppContext as _, Context, Entity, EventEmitter, Global, Subscription, Task};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

/// How often observers re-render while a clear is pending, so a countdown
/// can show whole seconds.
const COUNTDOWN_TICK: Duration = Duration::from_secs(1);

/// What changed, for anything that shows the countdown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardEvent {
    /// A secret was copied. `deadline` is when it will be cleared, `None`
    /// when the user chose "Never".
    Written { deadline: Option<Instant> },
    /// The service no longer holds a copy: it cleared it, or something newer
    /// had replaced it and the clipboard was left alone.
    Cleared,
}

/// Identity of the copy the service wrote. It holds neither the copied value
/// nor a digest of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OwnedCopy {
    /// The pasteboard generation our write created.
    Native(ClipboardChangeCount),
    /// GPUI's plain clipboard has no generation, so a counter stands in.
    #[cfg(not(target_os = "macos"))]
    Portable(u64),
}

pub struct ClipboardService {
    pasteboard: Box<dyn Pasteboard>,
    /// The copy this service wrote and has not cleared yet.
    owned: Option<OwnedCopy>,
    /// When `clear_task` fires. `None` while no timer runs.
    deadline: Option<Instant>,
    /// Holding the task keeps the timer alive; replacing it cancels it.
    clear_task: Option<Task<()>>,
    /// Notifies observers once a second while a clear is pending.
    countdown_tick: Option<Task<()>>,
    /// The `on_app_quit` registration from [`install_quit_hook`].
    quit_hook: Option<Subscription>,
    #[cfg(not(target_os = "macos"))]
    next_portable_copy: u64,
}

/// What the launch check does with the clipboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartAction {
    /// The access setting is not "Always Allow": nothing past the setting
    /// was looked at.
    Untouched,
    /// "Always Allow", but our marker type is not on the clipboard.
    NotOurs,
    /// "Always Allow" and our marker type is there: clear it, unless
    /// something was copied meanwhile.
    ClearOwnCopy,
}

/// The one service, reachable from anywhere that has an `App`.
struct SharedClipboard(Entity<ClipboardService>);

impl Global for SharedClipboard {}

impl EventEmitter<ClipboardEvent> for ClipboardService {}

impl ClipboardService {
    fn new(pasteboard: Box<dyn Pasteboard>) -> Self {
        Self {
            pasteboard,
            owned: None,
            deadline: None,
            clear_task: None,
            countdown_tick: None,
            quit_hook: None,
            #[cfg(not(target_os = "macos"))]
            next_portable_copy: 0,
        }
    }

    /// The service on the general pasteboard, installed on first use.
    pub fn handle(cx: &mut App) -> Entity<Self> {
        match cx.try_global::<SharedClipboard>() {
            Some(shared) => shared.0.clone(),
            None => Self::install(cx, Box::new(SystemPasteboard)),
        }
    }

    fn install(cx: &mut App, pasteboard: Box<dyn Pasteboard>) -> Entity<Self> {
        let service = cx.new(|_| Self::new(pasteboard));
        cx.set_global(SharedClipboard(service.clone()));
        service
    }

    /// Installs the service on a test pasteboard. Has to run before anything
    /// calls [`Self::handle`], or the real pasteboard would already be in.
    #[cfg(test)]
    pub(crate) fn install_for_test(cx: &mut App, pasteboard: Box<dyn Pasteboard>) -> Entity<Self> {
        assert!(
            cx.try_global::<SharedClipboard>().is_none(),
            "the clipboard service is already installed"
        );
        Self::install(cx, pasteboard)
    }

    /// Put `value` on the clipboard and clear it after `clear_after`.
    ///
    /// A new copy replaces the previous one and its timer. On error nothing
    /// was written and no copy of ours is left on the clipboard.
    pub fn write_secret(
        &mut self,
        value: Zeroizing<String>,
        clear_after: Option<Duration>,
        cx: &mut Context<Self>,
    ) -> Result<(), ClipboardError> {
        let copy = match self.pasteboard.write_secret(&value) {
            Ok(generation) => OwnedCopy::Native(generation),
            #[cfg(not(target_os = "macos"))]
            Err(ClipboardError::Unsupported) => self.write_portable(&value, cx),
            Err(error) => {
                // Fail closed: a native write error must never downgrade to a
                // normal pasteboard item that clipboard history or Universal
                // Clipboard can retain. Clear any previous copy we own.
                self.clear_owned(cx);
                return Err(error);
            }
        };
        drop(value);
        self.owned = Some(copy);
        self.start_timer(copy, clear_after, cx);
        cx.emit(ClipboardEvent::Written {
            deadline: self.deadline,
        });
        cx.notify();
        Ok(())
    }

    /// Clear our copy if it is still the newest thing on the clipboard, and
    /// forget it either way. Cancels the timer.
    pub fn clear_owned(&mut self, cx: &mut Context<Self>) {
        self.clear_task = None;
        self.countdown_tick = None;
        self.deadline = None;
        let Some(copy) = self.owned.take() else {
            return;
        };
        match copy {
            OwnedCopy::Native(generation) => {
                self.pasteboard.clear_if_unchanged(generation);
            }
            #[cfg(not(target_os = "macos"))]
            OwnedCopy::Portable(_) => {
                // GPUI has no portable clipboard generation. Clearing is
                // unconditional here, but only while our copy is still the
                // one the service owns; no secret is kept for comparison.
                cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
            }
        }
        cx.emit(ClipboardEvent::Cleared);
        cx.notify();
    }

    /// When the pending clear fires. `None` when nothing is pending or the
    /// user chose "Never".
    pub fn deadline(&self) -> Option<Instant> {
        self.deadline
    }

    /// Whether the service holds a copy it will clear on lock or quit.
    pub fn has_pending(&self) -> bool {
        self.owned.is_some()
    }

    #[cfg(not(target_os = "macos"))]
    fn write_portable(&mut self, value: &str, cx: &mut Context<Self>) -> OwnedCopy {
        cx.write_to_clipboard(ClipboardItem::new_string(value.to_string()));
        self.next_portable_copy = self.next_portable_copy.wrapping_add(1);
        OwnedCopy::Portable(self.next_portable_copy)
    }

    fn start_timer(
        &mut self,
        copy: OwnedCopy,
        clear_after: Option<Duration>,
        cx: &mut Context<Self>,
    ) {
        // Replacing the slots drops the previous timer and tick, which
        // cancels them.
        self.clear_task = None;
        self.countdown_tick = None;
        self.deadline = None;
        // "Never", or a delay too long to represent: no timer. The copy stays
        // owned, so a lock still clears it.
        let Some((clear_after, deadline)) =
            clear_after.and_then(|after| Instant::now().checked_add(after).map(|at| (after, at)))
        else {
            return;
        };
        self.deadline = Some(deadline);
        self.countdown_tick = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(COUNTDOWN_TICK).await;
                let ticking = this
                    .update(cx, |service, cx| {
                        if service.deadline.is_none() {
                            return false;
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !ticking {
                    break;
                }
            }
        }));
        self.clear_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(clear_after).await;
            let _ = this.update(cx, |service, cx| service.clear_after_timer(copy, cx));
        }));
    }

    /// The timer started for `copy` fired. A replaced timer can still be woken
    /// before its task is dropped, so it may only clear the copy it was
    /// started for.
    fn clear_after_timer(&mut self, copy: OwnedCopy, cx: &mut Context<Self>) {
        if self.owned != Some(copy) {
            return;
        }
        self.clear_owned(cx);
    }
}

/// Clear our copy from anywhere in the app. Before the first copy the service
/// may not be installed yet, and then there is nothing to clear.
pub fn clear_owned(cx: &mut App) {
    let Some(service) = cx
        .try_global::<SharedClipboard>()
        .map(|shared| shared.0.clone())
    else {
        return;
    };
    service.update(cx, |service, cx| service.clear_owned(cx));
}

/// Clear our copy when the app quits, however the quit started. GPUI runs
/// the hook from the platform's terminate callback; installing it twice
/// registers it once.
pub fn install_quit_hook(cx: &mut App) {
    ClipboardService::handle(cx).update(cx, |service, cx| {
        if service.quit_hook.is_none() {
            service.quit_hook = Some(cx.on_app_quit(|service, cx| {
                service.clear_owned(cx);
                async {}
            }));
        }
    });
}

/// Remove a copy an earlier run left on the clipboard. Runs once at launch,
/// before the first window; see rule 6 at the top of this file.
pub fn clear_own_copy_on_start(cx: &mut App) -> StartAction {
    ClipboardService::handle(cx).update(cx, |service, _| check_at_launch(&mut *service.pasteboard))
}

/// The launch decision. Only "Always Allow" lets the check look at the
/// clipboard at all, so `has_own_marker` is called for that setting alone.
pub fn start_action(
    behavior: AccessBehavior,
    has_own_marker: impl FnOnce() -> bool,
) -> StartAction {
    if behavior != AccessBehavior::AlwaysAllow {
        return StartAction::Untouched;
    }
    if has_own_marker() {
        StartAction::ClearOwnCopy
    } else {
        StartAction::NotOurs
    }
}

/// The launch check over any pasteboard: [`start_action`] with the marker
/// check, then the clear it asks for.
fn check_at_launch(pasteboard: &mut dyn Pasteboard) -> StartAction {
    let behavior = pasteboard.access_behavior();
    let mut generation = None;
    let action = start_action(behavior, || {
        // Taken before the marker check. A copy made between the two moves
        // the generation on, and the clear below then leaves it alone.
        generation = Some(pasteboard.change_count());
        pasteboard.has_own_marker()
    });
    if let (StartAction::ClearOwnCopy, Some(generation)) = (action, generation) {
        pasteboard.clear_if_unchanged(generation);
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard::AccessBehavior;
    use crate::clipboard::fake::FakePasteboard;
    use gpui::TestAppContext;
    use std::cell::RefCell;
    use std::rc::Rc;

    const TEN_SECONDS: Duration = Duration::from_secs(10);

    fn install(cx: &mut TestAppContext, pasteboard: &FakePasteboard) -> Entity<ClipboardService> {
        cx.update(|cx| ClipboardService::install_for_test(cx, Box::new(pasteboard.clone())))
    }

    fn copy(
        cx: &mut TestAppContext,
        service: &Entity<ClipboardService>,
        text: &str,
        clear_after: Option<Duration>,
    ) -> Result<(), ClipboardError> {
        service.update(cx, |service, cx| {
            service.write_secret(Zeroizing::new(text.to_string()), clear_after, cx)
        })
    }

    fn pending(cx: &mut TestAppContext, service: &Entity<ClipboardService>) -> bool {
        service.read_with(cx, |service, _| service.has_pending())
    }

    fn text(pasteboard: &FakePasteboard) -> Option<String> {
        pasteboard.ledger().text.clone()
    }

    #[gpui::test]
    fn write_failure_fails_closed(cx: &mut TestAppContext) {
        let pasteboard = FakePasteboard::new(AccessBehavior::Default);
        let service = install(cx, &pasteboard);
        copy(cx, &service, "first-secret", Some(TEN_SECONDS)).expect("first copy");

        pasteboard.fail_writes(true);
        let result = copy(cx, &service, "second-secret", Some(TEN_SECONDS));

        assert_eq!(result, Err(ClipboardError::WriteFailed));
        assert_eq!(
            pasteboard.ledger().writes,
            1,
            "the failed write wrote nothing"
        );
        assert_eq!(
            text(&pasteboard),
            None,
            "the earlier copy has to go when a newer one fails"
        );
        assert_eq!(pasteboard.ledger().clears, 1);
        service.read_with(cx, |service, _| {
            assert!(!service.has_pending());
            assert_eq!(service.deadline(), None, "no countdown for a failed copy");
        });
    }

    #[gpui::test]
    fn timer_clears_only_its_own_write(cx: &mut TestAppContext) {
        let pasteboard = FakePasteboard::new(AccessBehavior::Default);
        let service = install(cx, &pasteboard);

        copy(cx, &service, "first", Some(TEN_SECONDS)).expect("first copy");
        let first = OwnedCopy::Native(ClipboardChangeCount::from_raw(
            pasteboard.ledger().generation,
        ));
        copy(cx, &service, "second", Some(Duration::from_secs(30))).expect("second copy");

        // The first copy's ten seconds pass. Its timer was replaced.
        cx.executor().advance_clock(Duration::from_secs(11));
        cx.run_until_parked();
        assert_eq!(text(&pasteboard).as_deref(), Some("second"));
        assert!(pending(cx, &service));

        // A timer for the first copy that still gets to run changes nothing.
        service.update(cx, |service, cx| service.clear_after_timer(first, cx));
        assert_eq!(text(&pasteboard).as_deref(), Some("second"));
        assert!(pending(cx, &service));
        assert_eq!(pasteboard.ledger().clear_attempts, 0);

        // The second copy's own timer clears it.
        cx.executor().advance_clock(Duration::from_secs(20));
        cx.run_until_parked();
        assert_eq!(text(&pasteboard), None);
        assert!(!pending(cx, &service));
        assert_eq!(service.read_with(cx, |service, _| service.deadline()), None);
    }

    #[gpui::test]
    fn never_setting_keeps_ownership_for_lock(cx: &mut TestAppContext) {
        let pasteboard = FakePasteboard::new(AccessBehavior::Default);
        let service = install(cx, &pasteboard);

        copy(cx, &service, "secret", None).expect("copy");
        service.read_with(cx, |service, _| {
            assert!(service.has_pending(), "\"Never\" still owns the copy");
            assert_eq!(service.deadline(), None, "\"Never\" shows no countdown");
        });

        cx.executor()
            .advance_clock(Duration::from_secs(24 * 60 * 60));
        cx.run_until_parked();
        assert_eq!(text(&pasteboard).as_deref(), Some("secret"));
        assert_eq!(pasteboard.ledger().clear_attempts, 0);

        // What a lock does.
        cx.update(clear_owned);
        assert_eq!(text(&pasteboard), None);
        assert!(!pending(cx, &service));
    }

    #[gpui::test]
    fn clear_leaves_newer_clipboard_alone(cx: &mut TestAppContext) {
        let pasteboard = FakePasteboard::new(AccessBehavior::Default);
        let service = install(cx, &pasteboard);

        copy(cx, &service, "secret", Some(TEN_SECONDS)).expect("copy");
        pasteboard.foreign_write("a note");
        service.update(cx, |service, cx| service.clear_owned(cx));
        assert_eq!(text(&pasteboard).as_deref(), Some("a note"));
        assert_eq!(pasteboard.ledger().clears, 0);
        assert!(
            !pending(cx, &service),
            "the copy is no longer ours to clear"
        );

        // The same when the timer is the one clearing.
        copy(cx, &service, "secret", Some(TEN_SECONDS)).expect("copy");
        pasteboard.foreign_write("another note");
        cx.executor().advance_clock(Duration::from_secs(11));
        cx.run_until_parked();
        assert_eq!(text(&pasteboard).as_deref(), Some("another note"));
        assert_eq!(pasteboard.ledger().clears, 0);
        assert_eq!(pasteboard.ledger().clear_attempts, 2);
        assert!(!pending(cx, &service));
    }

    const EVERY_BEHAVIOR: [AccessBehavior; 4] = [
        AccessBehavior::Default,
        AccessBehavior::Ask,
        AccessBehavior::AlwaysAllow,
        AccessBehavior::AlwaysDeny,
    ];

    /// The clipboard as a launch finds it: a copy an earlier run left behind,
    /// or another app's text.
    fn at_launch(behavior: AccessBehavior, ours: bool) -> FakePasteboard {
        let pasteboard = FakePasteboard::new(behavior);
        if ours {
            pasteboard.leave_own_copy("left-behind-secret");
        } else {
            pasteboard.foreign_write("someone else's text");
        }
        pasteboard
    }

    #[test]
    fn start_path_never_reads_content() {
        // The ledger does count reads: one through the test pasteboard shows.
        let witness = FakePasteboard::new(AccessBehavior::AlwaysAllow);
        witness.read_content();
        assert_eq!(witness.ledger().content_reads, 1);

        for behavior in EVERY_BEHAVIOR {
            for ours in [true, false] {
                let pasteboard = at_launch(behavior, ours);
                check_at_launch(&mut pasteboard.clone());
                let ledger = pasteboard.ledger();
                assert_eq!(ledger.content_reads, 0, "{behavior:?}, ours: {ours}");
                assert_eq!(
                    ledger.writes, 0,
                    "{behavior:?}, ours: {ours}: the launch check never writes"
                );
            }
        }
    }

    #[test]
    fn start_clears_only_own_marked_copy() {
        // "Always Allow" and our marker: cleared.
        let pasteboard = at_launch(AccessBehavior::AlwaysAllow, true);
        assert_eq!(
            check_at_launch(&mut pasteboard.clone()),
            StartAction::ClearOwnCopy
        );
        assert_eq!(text(&pasteboard), None);
        assert_eq!(pasteboard.ledger().clears, 1);

        // "Always Allow" and another app's text: looked at, left alone.
        let pasteboard = at_launch(AccessBehavior::AlwaysAllow, false);
        assert_eq!(
            check_at_launch(&mut pasteboard.clone()),
            StartAction::NotOurs
        );
        assert_eq!(
            text(&pasteboard).as_deref(),
            Some("someone else's text"),
            "another app's clipboard was cleared at launch"
        );
        assert_eq!(pasteboard.ledger().has_own_marker_calls, 1);
        assert_eq!(pasteboard.ledger().clear_attempts, 0);

        // Our marker, but another app copies right after the check: the
        // generation taken before the check no longer matches.
        let pasteboard = at_launch(AccessBehavior::AlwaysAllow, true);
        pasteboard.copy_after_marker_check("copied during the check");
        assert_eq!(
            check_at_launch(&mut pasteboard.clone()),
            StartAction::ClearOwnCopy
        );
        assert_eq!(
            text(&pasteboard).as_deref(),
            Some("copied during the check")
        );
        assert_eq!(pasteboard.ledger().clears, 0);

        // Any other setting: the setting is read and nothing else, whatever
        // is on the clipboard.
        for behavior in [
            AccessBehavior::Default,
            AccessBehavior::Ask,
            AccessBehavior::AlwaysDeny,
        ] {
            for ours in [true, false] {
                let pasteboard = at_launch(behavior, ours);
                let before = text(&pasteboard);
                assert_eq!(
                    check_at_launch(&mut pasteboard.clone()),
                    StartAction::Untouched,
                    "{behavior:?}, ours: {ours}"
                );
                assert_eq!(text(&pasteboard), before, "{behavior:?}, ours: {ours}");
                let ledger = pasteboard.ledger();
                assert_eq!(ledger.access_behavior_calls, 1);
                assert_eq!(
                    ledger.has_own_marker_calls, 0,
                    "{behavior:?}, ours: {ours}: asked for the pasteboard types"
                );
                assert_eq!(ledger.change_count_calls, 0, "{behavior:?}, ours: {ours}");
                assert_eq!(ledger.clear_attempts, 0, "{behavior:?}, ours: {ours}");
            }
        }
    }

    #[gpui::test]
    fn launch_check_uses_the_service_pasteboard(cx: &mut TestAppContext) {
        let pasteboard = at_launch(AccessBehavior::AlwaysAllow, true);
        install(cx, &pasteboard);
        assert_eq!(
            cx.update(clear_own_copy_on_start),
            StartAction::ClearOwnCopy
        );
        assert_eq!(text(&pasteboard), None);
    }

    #[gpui::test]
    fn quit_clears_only_unchanged_own_copy(cx: &mut TestAppContext) {
        let pasteboard = FakePasteboard::new(AccessBehavior::Default);
        let service = install(cx, &pasteboard);
        cx.update(install_quit_hook);

        // Cmd+Q and the Quit menu item end in `actions::quit`, which clears
        // before it asks the platform to quit. The test platform's quit does
        // nothing, so only that direct clear can have run here.
        copy(cx, &service, "secret", None).expect("copy");
        cx.update(crate::app::actions::quit);
        assert_eq!(text(&pasteboard), None, "quitting left our copy behind");
        assert!(!pending(cx, &service));

        // Something copied after us stays.
        copy(cx, &service, "secret", None).expect("copy");
        pasteboard.foreign_write("a note");
        cx.update(crate::app::actions::quit);
        assert_eq!(text(&pasteboard).as_deref(), Some("a note"));

        // Any quit runs the app's quit handlers, and the hook clears there.
        copy(cx, &service, "secret", None).expect("copy");
        cx.quit();
        assert_eq!(
            text(&pasteboard),
            None,
            "the quit hook left our copy behind"
        );
    }

    #[gpui::test]
    fn events_announce_the_countdown(cx: &mut TestAppContext) {
        let pasteboard = FakePasteboard::new(AccessBehavior::Default);
        let service = install(cx, &pasteboard);
        let events = Rc::new(RefCell::new(Vec::new()));
        cx.update(|cx| {
            let events = events.clone();
            cx.subscribe(&service, move |_, event: &ClipboardEvent, _| {
                events.borrow_mut().push(*event);
            })
            .detach();
        });

        copy(cx, &service, "secret", Some(TEN_SECONDS)).expect("copy");
        copy(cx, &service, "secret", None).expect("copy");
        service.update(cx, |service, cx| service.clear_owned(cx));
        cx.run_until_parked();

        let events = events.borrow();
        assert!(
            matches!(
                events.as_slice(),
                [
                    ClipboardEvent::Written { deadline: Some(_) },
                    ClipboardEvent::Written { deadline: None },
                    ClipboardEvent::Cleared,
                ]
            ),
            "unexpected events: {events:?}"
        );
    }
}
