//! Portable placeholder for platforms without a native secret pasteboard.
//!
//! The clipboard service catches `Unsupported` and falls back to GPUI's
//! plain clipboard until an equivalent concealed/host-only backend exists.
//! There is no marker and no access setting here, so the launch check never
//! looks at the clipboard on these platforms.

use super::{AccessBehavior, ClipboardChangeCount, ClipboardError, ClipboardResult};

pub fn write_secret_text(_: &str) -> ClipboardResult<ClipboardChangeCount> {
    Err(ClipboardError::Unsupported)
}

pub fn clear_if_unchanged(_: ClipboardChangeCount) -> ClipboardResult<bool> {
    Err(ClipboardError::Unsupported)
}

/// Never `AlwaysAllow`: the launch check leaves the clipboard alone.
pub fn access_behavior() -> AccessBehavior {
    AccessBehavior::Default
}

pub fn has_own_marker() -> bool {
    false
}

pub fn change_count() -> ClipboardChangeCount {
    ClipboardChangeCount::from_raw(0)
}
