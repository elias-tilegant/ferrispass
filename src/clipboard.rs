//! Secret-aware clipboard primitives.
//!
//! The public token is deliberately just the platform pasteboard generation.
//! Callers can retain it for an auto-clear timer without retaining the copied
//! secret, a digest of it, or any other value derived from it.
//!
//! Nothing here reads what is on the pasteboard. The functions look at the
//! generation, at the pasteboard types and at the access setting, which macOS
//! hands out without asking the user; reading contents is what shows the
//! "paste from other apps" prompt. [`Pasteboard`] is the surface the app's
//! clipboard service works through, and it has no method that reads contents.

use std::fmt;

#[cfg(target_os = "macos")]
#[path = "clipboard_macos.rs"]
mod platform;

#[cfg(not(target_os = "macos"))]
#[path = "clipboard_portable.rs"]
mod platform;

pub use platform::{
    access_behavior, change_count, clear_if_unchanged, has_own_marker, write_secret_text,
};

/// Opaque generation of the pasteboard contents written by FerrisPass.
///
/// Store this value, not the copied text, until the auto-clear timer fires.
/// Passing it to [`clear_if_unchanged`] prevents the timer from erasing text
/// that the user copied after the secret.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClipboardChangeCount(isize);

impl ClipboardChangeCount {
    pub(crate) const fn from_raw(value: isize) -> Self {
        Self(value)
    }

    pub(crate) const fn into_raw(self) -> isize {
        self.0
    }
}

/// Clipboard failures never contain the text that was being copied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardError {
    /// This platform has no native secret-aware implementation yet. The UI may
    /// delegate to its portable clipboard implementation as a fallback.
    Unsupported,
    /// AppKit rejected one of the values while constructing the pasteboard
    /// item. The general pasteboard was not touched.
    ItemConstructionFailed,
    /// The platform rejected the completed pasteboard item.
    WriteFailed,
}

impl fmt::Display for ClipboardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => write!(f, "secret-aware clipboard is not supported"),
            Self::ItemConstructionFailed => write!(f, "could not prepare clipboard item"),
            Self::WriteFailed => write!(f, "could not write to clipboard"),
        }
    }
}

impl std::error::Error for ClipboardError {}

pub type ClipboardResult<T> = Result<T, ClipboardError>;

/// What the user allows FerrisPass to do with the general pasteboard, as set
/// under Privacy & Security, "Paste from Other Apps". Writing is never
/// restricted; the setting is about reading what other apps put there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessBehavior {
    /// FerrisPass never read the pasteboard. The first read would show the
    /// prompt.
    Default,
    /// Every read shows the prompt.
    Ask,
    /// Reads go through without a prompt.
    AlwaysAllow,
    /// Reads are refused without a prompt.
    AlwaysDeny,
}

/// The pasteboard as the clipboard service sees it.
///
/// There is deliberately no method that returns contents. The service can
/// write a secret, ask whether our marker type is present, read the access
/// setting and the generation, and clear what it wrote; it cannot see what
/// anyone else copied.
pub trait Pasteboard {
    /// The access setting. Reading it never shows a prompt.
    fn access_behavior(&self) -> AccessBehavior;
    /// Whether the pasteboard holds an item with FerrisPass's private marker
    /// type. Looks at types only.
    fn has_own_marker(&self) -> bool;
    /// The current generation.
    fn change_count(&self) -> ClipboardChangeCount;
    /// Write a secret with every privacy marker and return the generation
    /// that write created. On error nothing of `text` is on the pasteboard.
    fn write_secret(&mut self, text: &str) -> ClipboardResult<ClipboardChangeCount>;
    /// Clear the pasteboard if its generation is still `expected`. Returns
    /// whether it cleared.
    fn clear_if_unchanged(&mut self, expected: ClipboardChangeCount) -> bool;
}

/// The general pasteboard of this Mac.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemPasteboard;

impl Pasteboard for SystemPasteboard {
    fn access_behavior(&self) -> AccessBehavior {
        access_behavior()
    }

    fn has_own_marker(&self) -> bool {
        has_own_marker()
    }

    fn change_count(&self) -> ClipboardChangeCount {
        change_count()
    }

    fn write_secret(&mut self, text: &str) -> ClipboardResult<ClipboardChangeCount> {
        write_secret_text(text)
    }

    fn clear_if_unchanged(&mut self, expected: ClipboardChangeCount) -> bool {
        clear_if_unchanged(expected).unwrap_or(false)
    }
}

/// A pasteboard for tests that keeps a ledger of every call.
///
/// It also has [`FakePasteboard::read_content`], which is not part of
/// [`Pasteboard`]. Code that only holds a `dyn Pasteboard` cannot reach it,
/// and `content_reads` staying at zero shows that no other route exists.
#[cfg(test)]
pub(crate) mod fake {
    use super::{
        AccessBehavior, ClipboardChangeCount, ClipboardError, ClipboardResult, Pasteboard,
    };
    use std::cell::{Ref, RefCell};
    use std::rc::Rc;

    #[derive(Debug)]
    pub(crate) struct Ledger {
        pub behavior: AccessBehavior,
        pub generation: isize,
        pub text: Option<String>,
        pub marked: bool,
        pub fail_writes: bool,
        pub access_behavior_calls: usize,
        pub has_own_marker_calls: usize,
        pub change_count_calls: usize,
        pub writes: usize,
        pub clear_attempts: usize,
        pub clears: usize,
        pub content_reads: usize,
        /// Copied by "another app" right after the next marker check.
        pub copy_after_marker_check: Option<String>,
    }

    /// Clones share one ledger, so a test keeps a handle while the service
    /// owns another.
    #[derive(Clone, Debug)]
    pub(crate) struct FakePasteboard(Rc<RefCell<Ledger>>);

    impl FakePasteboard {
        pub(crate) fn new(behavior: AccessBehavior) -> Self {
            Self(Rc::new(RefCell::new(Ledger {
                behavior,
                generation: 100,
                text: None,
                marked: false,
                fail_writes: false,
                access_behavior_calls: 0,
                has_own_marker_calls: 0,
                change_count_calls: 0,
                writes: 0,
                clear_attempts: 0,
                clears: 0,
                content_reads: 0,
                copy_after_marker_check: None,
            })))
        }

        pub(crate) fn ledger(&self) -> Ref<'_, Ledger> {
            self.0.borrow()
        }

        /// Another app, or the user, copies something.
        pub(crate) fn foreign_write(&self, text: &str) {
            let mut ledger = self.0.borrow_mut();
            ledger.generation += 1;
            ledger.text = Some(text.to_string());
            ledger.marked = false;
        }

        /// What an earlier run of FerrisPass left behind after a crash.
        pub(crate) fn leave_own_copy(&self, text: &str) {
            let mut ledger = self.0.borrow_mut();
            ledger.generation += 1;
            ledger.text = Some(text.to_string());
            ledger.marked = true;
        }

        /// Another app copies `text` just after the next marker check.
        pub(crate) fn copy_after_marker_check(&self, text: &str) {
            self.0.borrow_mut().copy_after_marker_check = Some(text.to_string());
        }

        pub(crate) fn fail_writes(&self, fail: bool) {
            self.0.borrow_mut().fail_writes = fail;
        }

        /// Reads the contents and counts the read. Not reachable through
        /// [`Pasteboard`].
        pub(crate) fn read_content(&self) -> Option<String> {
            let mut ledger = self.0.borrow_mut();
            ledger.content_reads += 1;
            ledger.text.clone()
        }
    }

    impl Pasteboard for FakePasteboard {
        fn access_behavior(&self) -> AccessBehavior {
            let mut ledger = self.0.borrow_mut();
            ledger.access_behavior_calls += 1;
            ledger.behavior
        }

        fn has_own_marker(&self) -> bool {
            let mut ledger = self.0.borrow_mut();
            ledger.has_own_marker_calls += 1;
            let marked = ledger.marked;
            if let Some(text) = ledger.copy_after_marker_check.take() {
                ledger.generation += 1;
                ledger.text = Some(text);
                ledger.marked = false;
            }
            marked
        }

        fn change_count(&self) -> ClipboardChangeCount {
            let mut ledger = self.0.borrow_mut();
            ledger.change_count_calls += 1;
            ClipboardChangeCount::from_raw(ledger.generation)
        }

        fn write_secret(&mut self, text: &str) -> ClipboardResult<ClipboardChangeCount> {
            let mut ledger = self.0.borrow_mut();
            if ledger.fail_writes {
                return Err(ClipboardError::WriteFailed);
            }
            ledger.generation += 1;
            ledger.text = Some(text.to_string());
            ledger.marked = true;
            ledger.writes += 1;
            Ok(ClipboardChangeCount::from_raw(ledger.generation))
        }

        fn clear_if_unchanged(&mut self, expected: ClipboardChangeCount) -> bool {
            let mut ledger = self.0.borrow_mut();
            ledger.clear_attempts += 1;
            if ledger.generation != expected.into_raw() {
                return false;
            }
            // Clearing is a change of its own, as with NSPasteboard.
            ledger.generation += 1;
            ledger.text = None;
            ledger.marked = false;
            ledger.clears += 1;
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn change_count_is_opaque_but_comparable() {
        let first = ClipboardChangeCount::from_raw(41);
        let same = ClipboardChangeCount::from_raw(41);
        let later = ClipboardChangeCount::from_raw(42);

        assert_eq!(first, same);
        assert_ne!(first, later);
    }

    #[test]
    fn errors_never_embed_caller_data() {
        let secret = "not-part-of-any-error";
        for error in [
            ClipboardError::Unsupported,
            ClipboardError::ItemConstructionFailed,
            ClipboardError::WriteFailed,
        ] {
            assert!(!error.to_string().contains(secret));
        }
    }
}
