//! Secret-aware clipboard primitives.
//!
//! The public token is deliberately just the platform pasteboard generation.
//! Callers can retain it for an auto-clear timer without retaining the copied
//! secret, a digest of it, or any other value derived from it.

use std::fmt;

#[cfg(target_os = "macos")]
#[path = "clipboard_macos.rs"]
mod platform;

#[cfg(not(target_os = "macos"))]
#[path = "clipboard_portable.rs"]
mod platform;

pub use platform::{clear_if_unchanged, write_secret_text};

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
