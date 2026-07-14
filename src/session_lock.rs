//! macOS session-lock notifications.
//!
//! AppKit covers system/display sleep and fast-user switching. macOS exposes
//! the immediate lock-screen and screensaver transitions through distributed
//! notifications. The Objective-C callbacks only enqueue an opaque event; the
//! GPUI task that consumes it performs the actual vault lock on the UI thread.

#![allow(unsafe_code)]

use std::ptr::NonNull;

use async_channel::{Receiver, Sender};
use block2::RcBlock;
use objc2::{
    AnyThread, DefinedClass, define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject},
    sel,
};
use objc2_app_kit::{
    NSWorkspace, NSWorkspaceDidWakeNotification, NSWorkspaceScreensDidSleepNotification,
    NSWorkspaceScreensDidWakeNotification, NSWorkspaceSessionDidBecomeActiveNotification,
    NSWorkspaceSessionDidResignActiveNotification, NSWorkspaceWillSleepNotification,
};
use objc2_foundation::{
    NSDistributedNotificationCenter, NSNotification, NSNotificationCenter, NSNotificationName,
    NSNotificationSuspensionBehavior, NSString,
};

const SCREEN_LOCKED_NOTIFICATION: &str = "com.apple.screenIsLocked";
const SCREEN_UNLOCKED_NOTIFICATION: &str = "com.apple.screenIsUnlocked";
const SCREENSAVER_STARTED_NOTIFICATION: &str = "com.apple.screensaver.didstart";
const SCREENSAVER_STOPPED_NOTIFICATION: &str = "com.apple.screensaver.didstop";

type Observer = Retained<ProtocolObject<dyn NSObjectProtocol>>;

struct SessionLockObserverIvars {
    sender: Sender<()>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements. The only ivar is a
    // thread-safe channel sender because distributed notifications may arrive
    // on a non-main thread.
    #[unsafe(super(NSObject))]
    #[name = "FerrisPassSessionLockObserver"]
    #[ivars = SessionLockObserverIvars]
    struct SessionLockObserver;

    impl SessionLockObserver {
        #[unsafe(method(sessionDidLock:))]
        fn session_did_lock(&self, _notification: &NSNotification) {
            let _ = self.ivars().sender.try_send(());
        }
    }

    // SAFETY: NSObjectProtocol has no additional implementation requirements.
    unsafe impl NSObjectProtocol for SessionLockObserver {}
);

impl SessionLockObserver {
    fn new(sender: Sender<()>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(SessionLockObserverIvars { sender });
        // SAFETY: invokes NSObject's designated initializer on a newly
        // allocated instance whose Rust ivars have already been initialized.
        unsafe { msg_send![super(this), init] }
    }
}

/// Retained OS registrations plus an async stream of coalesced lock events.
pub struct SessionLockMonitor {
    receiver: Receiver<()>,
    workspace_center: Retained<NSNotificationCenter>,
    workspace_observers: Vec<Observer>,
    distributed_center: Retained<NSDistributedNotificationCenter>,
    distributed_observer: Retained<SessionLockObserver>,
}

impl SessionLockMonitor {
    pub fn new() -> Self {
        // A single pending event is enough: locking is idempotent, and sleep
        // commonly emits several notifications in quick succession.
        let (sender, receiver) = async_channel::bounded(1);

        let workspace_center = NSWorkspace::sharedWorkspace().notificationCenter();
        // SAFETY: these are immutable AppKit notification-name constants.
        // Post-event notifications are deliberate fail-safes: FerrisPass may
        // be suspended while the matching sleep/resign event is delivered.
        let workspace_names = unsafe {
            [
                NSWorkspaceWillSleepNotification,
                NSWorkspaceDidWakeNotification,
                NSWorkspaceScreensDidSleepNotification,
                NSWorkspaceScreensDidWakeNotification,
                NSWorkspaceSessionDidResignActiveNotification,
                NSWorkspaceSessionDidBecomeActiveNotification,
            ]
        };
        let workspace_observers = workspace_names
            .into_iter()
            .map(|name| observe(&workspace_center, name, &sender))
            .collect();

        let distributed_center = NSDistributedNotificationCenter::defaultCenter();
        let distributed_observer = SessionLockObserver::new(sender.clone());
        for name in [
            SCREEN_LOCKED_NOTIFICATION,
            SCREEN_UNLOCKED_NOTIFICATION,
            SCREENSAVER_STARTED_NOTIFICATION,
            SCREENSAVER_STOPPED_NOTIFICATION,
        ] {
            let name = NSString::from_str(name);
            add_distributed_observer(&distributed_center, &distributed_observer, &name);
        }

        Self {
            receiver,
            workspace_center,
            workspace_observers,
            distributed_center,
            distributed_observer,
        }
    }

    pub fn events(&self) -> Receiver<()> {
        self.receiver.clone()
    }
}

impl Drop for SessionLockMonitor {
    fn drop(&mut self) {
        for observer in self.workspace_observers.drain(..) {
            // SAFETY: every token was returned by this exact center, remains
            // retained through this call, and is removed only once.
            remove_observer(&self.workspace_center, &observer);
        }
        remove_distributed_observer(&self.distributed_center, &self.distributed_observer);
    }
}

fn observe(
    center: &NSNotificationCenter,
    name: &NSNotificationName,
    sender: &Sender<()>,
) -> Observer {
    let sender = sender.clone();
    let callback = RcBlock::new(move |_notification: NonNull<NSNotification>| {
        // Never block an OS notification thread. A full channel means an
        // equivalent lock request is already pending.
        let _ = sender.try_send(());
    });

    // SAFETY: the notification name and center are valid Objective-C objects;
    // the copied block captures only a thread-safe async-channel sender. The
    // returned token is retained by SessionLockMonitor and explicitly removed.
    unsafe { center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &callback) }
}

fn remove_observer(center: &NSNotificationCenter, observer: &Observer) {
    let observer: &ProtocolObject<dyn NSObjectProtocol> = observer.as_ref();
    let observer: &AnyObject = observer.as_ref();
    // SAFETY: the token was returned by this center, is still retained, and
    // SessionLockMonitor removes each token at most once.
    unsafe { center.removeObserver(observer) };
}

fn add_distributed_observer(
    center: &NSDistributedNotificationCenter,
    observer: &SessionLockObserver,
    name: &NSNotificationName,
) {
    // SAFETY: `sessionDidLock:` is implemented above with the Objective-C
    // signature expected for a notification selector. The center retains the
    // observer registration while SessionLockMonitor retains the Rust object.
    unsafe {
        center.addObserver_selector_name_object_suspensionBehavior(
            observer,
            sel!(sessionDidLock:),
            Some(name),
            None,
            NSNotificationSuspensionBehavior::DeliverImmediately,
        )
    };
}

fn remove_distributed_observer(
    center: &NSDistributedNotificationCenter,
    observer: &SessionLockObserver,
) {
    // SAFETY: removes every registration made for this retained observer from
    // the same distributed center. It is called exactly once during Drop.
    unsafe { center.removeObserver_name_object(observer, None, None) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_callback_forwards_without_blocking() {
        let center = NSNotificationCenter::new();
        let name = NSString::from_str("dev.ferrispass.session-lock-test");
        let (sender, receiver) = async_channel::bounded(1);
        let observer = observe(&center, &name, &sender);

        // SAFETY: the test name is a valid NSString and the object filter is
        // intentionally absent, matching the registration above.
        unsafe { center.postNotificationName_object(&name, None) };
        assert_eq!(receiver.try_recv(), Ok(()));

        // SAFETY: `observer` came from `center` and is removed once.
        remove_observer(&center, &observer);
    }

    #[test]
    fn monitor_forwards_pre_and_post_workspace_events() {
        let monitor = SessionLockMonitor::new();
        // SAFETY: these are immutable AppKit notification-name constants.
        let names = unsafe {
            [
                NSWorkspaceWillSleepNotification,
                NSWorkspaceDidWakeNotification,
                NSWorkspaceScreensDidSleepNotification,
                NSWorkspaceScreensDidWakeNotification,
                NSWorkspaceSessionDidResignActiveNotification,
                NSWorkspaceSessionDidBecomeActiveNotification,
            ]
        };

        for name in names {
            // SAFETY: the test posts a valid AppKit name to the exact center
            // on which the monitor registered, without an object filter.
            unsafe {
                monitor
                    .workspace_center
                    .postNotificationName_object(name, None)
            };
            assert_eq!(monitor.receiver.try_recv(), Ok(()));
        }
    }

    #[test]
    fn duplicate_notifications_are_coalesced() {
        let center = NSNotificationCenter::new();
        let name = NSString::from_str("dev.ferrispass.session-lock-coalesce-test");
        let (sender, receiver) = async_channel::bounded(1);
        let observer = observe(&center, &name, &sender);

        // SAFETY: same valid test-only notification contract as above.
        unsafe {
            center.postNotificationName_object(&name, None);
            center.postNotificationName_object(&name, None);
        }
        assert_eq!(receiver.try_recv(), Ok(()));
        assert!(receiver.try_recv().is_err());

        // SAFETY: `observer` came from `center` and is removed once.
        remove_observer(&center, &observer);
    }

    #[test]
    fn selector_observer_forwards_notifications() {
        let center = NSNotificationCenter::new();
        let name = NSString::from_str("dev.ferrispass.session-lock-distributed-test");
        let (sender, receiver) = async_channel::bounded(1);
        let observer = SessionLockObserver::new(sender);
        let observer_object: &AnyObject = observer.as_ref();
        // SAFETY: the selector is implemented by the retained observer with
        // the standard single-NSNotification argument.
        unsafe {
            center.addObserver_selector_name_object(
                observer_object,
                sel!(sessionDidLock:),
                Some(&name),
                None,
            )
        }
        // SAFETY: valid test name and no object filter.
        unsafe { center.postNotificationName_object(&name, None) };
        assert_eq!(receiver.try_recv(), Ok(()));

        // SAFETY: removes the retained observer once from its registering
        // center.
        unsafe { center.removeObserver(observer_object) };
    }
}
