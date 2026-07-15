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

/// Why the monitor fired. `Lock` means the session is genuinely going away
/// (sleep imminent, screen locked, fast user switch, screensaver started)
/// and must always lock the vault. `PostWake` events are trailing
/// fail-safes (DidWake, screen unlocked, …) delivered after the user is
/// already back — the consumer may apply a short grace window to those, but
/// never to `Lock`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionLockEvent {
    Lock,
    PostWake,
}

fn classify_distributed(name: &str) -> SessionLockEvent {
    if name == SCREEN_UNLOCKED_NOTIFICATION || name == SCREENSAVER_STOPPED_NOTIFICATION {
        SessionLockEvent::PostWake
    } else {
        SessionLockEvent::Lock
    }
}

type Observer = Retained<ProtocolObject<dyn NSObjectProtocol>>;

/// Channel sender plus a latch that survives a full channel. The bounded
/// queue is only a wakeup mechanism; the latch is the authoritative "a
/// genuine lock event happened" bit — `try_send` may drop an event when
/// the queue is full, but the latch cannot be lost.
#[derive(Clone)]
struct EventSink {
    sender: Sender<SessionLockEvent>,
    lock_latch: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl EventSink {
    fn dispatch(&self, event: SessionLockEvent) {
        if event == SessionLockEvent::Lock {
            self.lock_latch
                .store(true, std::sync::atomic::Ordering::Release);
        }
        // Never block an OS notification thread. A full channel still
        // wakes the consumer, which reads the latch.
        let _ = self.sender.try_send(event);
    }
}

struct SessionLockObserverIvars {
    sink: EventSink,
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
        fn session_did_lock(&self, notification: &NSNotification) {
            let name = notification.name();
            self.ivars()
                .sink
                .dispatch(classify_distributed(&name.to_string()));
        }
    }

    // SAFETY: NSObjectProtocol has no additional implementation requirements.
    unsafe impl NSObjectProtocol for SessionLockObserver {}
);

impl SessionLockObserver {
    fn new(sink: EventSink) -> Retained<Self> {
        let this = Self::alloc().set_ivars(SessionLockObserverIvars { sink });
        // SAFETY: invokes NSObject's designated initializer on a newly
        // allocated instance whose Rust ivars have already been initialized.
        unsafe { msg_send![super(this), init] }
    }
}

/// Retained OS registrations plus an async stream of typed lock events.
pub struct SessionLockMonitor {
    receiver: Receiver<SessionLockEvent>,
    lock_latch: std::sync::Arc<std::sync::atomic::AtomicBool>,
    workspace_center: Retained<NSNotificationCenter>,
    workspace_observers: Vec<Observer>,
    distributed_center: Retained<NSDistributedNotificationCenter>,
    distributed_observer: Retained<SessionLockObserver>,
}

impl SessionLockMonitor {
    pub fn new() -> Self {
        // The channel is only a wakeup; the latch carries the "genuine
        // lock happened" bit and survives even a full queue (eight queued
        // PostWake duplicates must never swallow a real Lock).
        let (sender, receiver) = async_channel::bounded(8);
        let sink = EventSink {
            sender,
            lock_latch: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };

        let workspace_center = NSWorkspace::sharedWorkspace().notificationCenter();
        // SAFETY: these are immutable AppKit notification-name constants.
        // Post-event notifications are deliberate fail-safes: FerrisPass may
        // be suspended while the matching sleep/resign event is delivered.
        let workspace_names = unsafe {
            [
                (NSWorkspaceWillSleepNotification, SessionLockEvent::Lock),
                (NSWorkspaceDidWakeNotification, SessionLockEvent::PostWake),
                (
                    NSWorkspaceScreensDidSleepNotification,
                    SessionLockEvent::Lock,
                ),
                (
                    NSWorkspaceScreensDidWakeNotification,
                    SessionLockEvent::PostWake,
                ),
                (
                    NSWorkspaceSessionDidResignActiveNotification,
                    SessionLockEvent::Lock,
                ),
                (
                    NSWorkspaceSessionDidBecomeActiveNotification,
                    SessionLockEvent::PostWake,
                ),
            ]
        };
        let workspace_observers = workspace_names
            .into_iter()
            .map(|(name, event)| observe(&workspace_center, name, event, &sink))
            .collect();

        let distributed_center = NSDistributedNotificationCenter::defaultCenter();
        let distributed_observer = SessionLockObserver::new(sink.clone());
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
            lock_latch: sink.lock_latch,
            workspace_center,
            workspace_observers,
            distributed_center,
            distributed_observer,
        }
    }

    pub fn events(&self) -> Receiver<SessionLockEvent> {
        self.receiver.clone()
    }

    /// Authoritative "a genuine lock event happened since the last check"
    /// bit. Consumers must call this after draining `events()` — a Lock
    /// dropped by a full channel is still recorded here.
    pub fn lock_latch(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        std::sync::Arc::clone(&self.lock_latch)
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
    event: SessionLockEvent,
    sink: &EventSink,
) -> Observer {
    let sink = sink.clone();
    let callback = RcBlock::new(move |_notification: NonNull<NSNotification>| {
        sink.dispatch(event);
    });

    // SAFETY: the notification name and center are valid Objective-C objects;
    // the copied block captures only a thread-safe sink (channel sender +
    // atomic latch). The returned token is retained by SessionLockMonitor
    // and explicitly removed.
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

    fn test_sink(capacity: usize) -> (EventSink, Receiver<SessionLockEvent>) {
        let (sender, receiver) = async_channel::bounded(capacity);
        (
            EventSink {
                sender,
                lock_latch: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            },
            receiver,
        )
    }

    #[test]
    fn notification_callback_forwards_without_blocking() {
        let center = NSNotificationCenter::new();
        let name = NSString::from_str("dev.ferrispass.session-lock-test");
        let (sink, receiver) = test_sink(8);
        let observer = observe(&center, &name, SessionLockEvent::Lock, &sink);

        // SAFETY: the test name is a valid NSString and the object filter is
        // intentionally absent, matching the registration above.
        unsafe { center.postNotificationName_object(&name, None) };
        assert_eq!(receiver.try_recv(), Ok(SessionLockEvent::Lock));

        // SAFETY: `observer` came from `center` and is removed once.
        remove_observer(&center, &observer);
    }

    #[test]
    fn monitor_types_pre_events_as_lock_and_post_events_as_wake() {
        let monitor = SessionLockMonitor::new();
        // SAFETY: these are immutable AppKit notification-name constants.
        let names = unsafe {
            [
                (NSWorkspaceWillSleepNotification, SessionLockEvent::Lock),
                (NSWorkspaceDidWakeNotification, SessionLockEvent::PostWake),
                (
                    NSWorkspaceScreensDidSleepNotification,
                    SessionLockEvent::Lock,
                ),
                (
                    NSWorkspaceScreensDidWakeNotification,
                    SessionLockEvent::PostWake,
                ),
                (
                    NSWorkspaceSessionDidResignActiveNotification,
                    SessionLockEvent::Lock,
                ),
                (
                    NSWorkspaceSessionDidBecomeActiveNotification,
                    SessionLockEvent::PostWake,
                ),
            ]
        };

        for (name, expected) in names {
            // SAFETY: the test posts a valid AppKit name to the exact center
            // on which the monitor registered, without an object filter.
            unsafe {
                monitor
                    .workspace_center
                    .postNotificationName_object(name, None)
            };
            assert_eq!(monitor.receiver.try_recv(), Ok(expected));
        }
    }

    #[test]
    fn queued_post_wake_never_drops_a_genuine_lock() {
        // Regression guard for the bounded(1) coalescing this replaced:
        // a pending PostWake must not swallow a subsequent Lock event.
        let center = NSNotificationCenter::new();
        let lock_name = NSString::from_str("dev.ferrispass.session-lock-hard");
        let wake_name = NSString::from_str("dev.ferrispass.session-lock-wake");
        // Capacity 1 on purpose: even when the queue is already full of
        // PostWake noise and the Lock's `try_send` is dropped, the latch
        // must still record it.
        let (sink, receiver) = test_sink(1);
        let lock_observer = observe(&center, &lock_name, SessionLockEvent::Lock, &sink);
        let wake_observer = observe(&center, &wake_name, SessionLockEvent::PostWake, &sink);

        // SAFETY: same valid test-only notification contract as above.
        unsafe {
            center.postNotificationName_object(&wake_name, None);
            center.postNotificationName_object(&lock_name, None);
        }
        assert_eq!(receiver.try_recv(), Ok(SessionLockEvent::PostWake));
        assert!(receiver.try_recv().is_err(), "queue was full — dropped");
        assert!(
            sink.lock_latch
                .swap(false, std::sync::atomic::Ordering::AcqRel),
            "latch must survive a dropped Lock send"
        );

        // SAFETY: both observers came from `center` and are removed once.
        remove_observer(&center, &lock_observer);
        remove_observer(&center, &wake_observer);
    }

    #[test]
    fn selector_observer_classifies_distributed_notifications() {
        let center = NSNotificationCenter::new();
        let (sink, receiver) = test_sink(8);
        let observer = SessionLockObserver::new(sink);
        let observer_object: &AnyObject = observer.as_ref();
        for name in [
            SCREEN_LOCKED_NOTIFICATION,
            SCREEN_UNLOCKED_NOTIFICATION,
            SCREENSAVER_STARTED_NOTIFICATION,
            SCREENSAVER_STOPPED_NOTIFICATION,
        ] {
            let name = NSString::from_str(name);
            // SAFETY: the selector is implemented by the retained observer
            // with the standard single-NSNotification argument.
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
        }
        assert_eq!(receiver.try_recv(), Ok(SessionLockEvent::Lock));
        assert_eq!(receiver.try_recv(), Ok(SessionLockEvent::PostWake));
        assert_eq!(receiver.try_recv(), Ok(SessionLockEvent::Lock));
        assert_eq!(receiver.try_recv(), Ok(SessionLockEvent::PostWake));

        // SAFETY: removes the retained observer once from its registering
        // center.
        unsafe { center.removeObserver(observer_object) };
    }
}
