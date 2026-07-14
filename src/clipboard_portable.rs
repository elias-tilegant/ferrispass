//! Portable placeholder for platforms without a native secret pasteboard.
//!
//! The GPUI integration can catch `Unsupported` and delegate to the normal
//! platform clipboard until an equivalent concealed/host-only backend exists.

use super::{ClipboardChangeCount, ClipboardError, ClipboardResult};

pub fn write_secret_text(_: &str) -> ClipboardResult<ClipboardChangeCount> {
    Err(ClipboardError::Unsupported)
}

pub fn clear_if_unchanged(_: ClipboardChangeCount) -> ClipboardResult<bool> {
    Err(ClipboardError::Unsupported)
}
