//! macOS Accessibility (TCC) permission probe.
//!
//! Both halves of auto-type (reading the foreground window's title via
//! the AX framework, and synthesising keystrokes via `CGEventPost`)
//! require the host process to be in the *Accessibility* list under
//! System Settings → Privacy & Security. macOS gates access through
//! a single `AXIsProcessTrusted` bit per process — granted once, the
//! bit stays set across launches until the user revokes it.
//!
//! Our policy: probe before every auto-type attempt. Cheap (one C
//! call), and lets the UI surface a clear "grant access" message
//! instead of failing silently when enigo's keyboard events are
//! ignored by the OS.

/// `true` if the host process is trusted to use the macOS Accessibility
/// APIs. Always `true` on non-macOS targets so the rest of the auto-
/// type pipeline can stay platform-agnostic in its plumbing without
/// littering `cfg` everywhere.
pub fn is_trusted() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos_accessibility_client::accessibility::application_is_trusted()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// Prompt the user to grant Accessibility permission. Spawns the
/// standard macOS modal ("FerrisPass would like to control this
/// computer using accessibility features"), which links into the
/// Privacy pane. Returns the post-prompt trust state — but because
/// macOS won't re-check the bit in a running process, callers
/// should treat any subsequent auto-type attempt as the real test.
///
/// No-op on non-macOS (returns `true` to mirror `is_trusted`).
///
/// ⚠️ This dispatches a system-level user prompt. Only call it in
/// response to an explicit user action (toggling Auto-Type on, or
/// clicking the "Grant access" button) — never on app launch.
pub fn request_trust() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos_accessibility_client::accessibility::application_is_trusted_with_prompt()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}
