//! Global hotkey registration and event plumbing.
//!
//! `global-hotkey` (Tauri team) wraps the macOS `RegisterEventHotKey`
//! Carbon API behind a safe interface. Constraints worth knowing:
//!
//! - The `GlobalHotKeyManager` MUST be created on a thread that has a
//!   running Core Foundation event loop. On macOS that's the main
//!   thread. GPUI runs its event loop on the main thread, so we
//!   instantiate the manager from inside `AppShell::new` (which itself
//!   runs main-threaded).
//! - The manager owns OS resources and unregisters its hotkey on
//!   drop. We hold it in `HotkeyListener` so the `Task` that owns the
//!   service controls registration lifetime.
//! - Events arrive on a global crossbeam channel. We don't `recv()`
//!   blocking (which would hang the executor); we `try_recv` on a
//!   periodic timer driven by the GPUI background executor. ~30 Hz
//!   feels instant to the user and costs essentially nothing.

use std::str::FromStr;

use global_hotkey::{
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    hotkey::HotKey,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum HotkeyError {
    /// User-typed combo isn't a valid hotkey string. We surface this in
    /// the Settings UI when the user types something `global-hotkey`
    /// can't parse — better than silently registering nothing and
    /// having the feature appear broken.
    #[error("invalid hotkey combo \"{combo}\": {source}")]
    Parse {
        combo: String,
        #[source]
        source: global_hotkey::hotkey::HotKeyParseError,
    },
    /// macOS refused the registration. Usually means the combo is
    /// already taken by another global hotkey (Spotlight, a window
    /// manager, a different password manager).
    #[error("OS rejected hotkey registration: {0}")]
    Register(String),
    /// The hotkey manager itself failed to initialise. Shouldn't
    /// happen in practice — the only documented cause is calling
    /// from a thread without a CF event loop, which we control for.
    #[error("could not create hotkey manager: {0}")]
    Init(String),
}

/// The default combo. ⌃⌥⌘V matches KeePassXC's macOS default and
/// avoids the well-known reserved combos (Spotlight ⌘Space, Mission
/// Control ⌃↑, Dock toggle ⌘⌥D).
pub const DEFAULT_HOTKEY: &str = "ctrl+alt+super+KeyV";

/// Owns the OS hotkey registration. Drop = unregister.
///
/// Construction has two failure modes worth distinguishing for the
/// UI: a bad combo (user fixable in Settings) vs. an OS conflict
/// (different remediation: change the combo or quit the conflicting
/// app). The error enum splits them.
pub struct HotkeyListener {
    manager: GlobalHotKeyManager,
    hotkey: HotKey,
}

impl HotkeyListener {
    /// Register `combo` with the OS. Must run on the main thread (see
    /// module docs); enforced by the caller — we don't try to thread-
    /// check here because GPUI's only realistic call site is
    /// `AppShell::new`, which is itself main-threaded.
    pub fn register(combo: &str) -> Result<Self, HotkeyError> {
        let hotkey = parse_combo(combo)?;
        let manager = GlobalHotKeyManager::new()
            .map_err(|e| HotkeyError::Init(e.to_string()))?;
        manager
            .register(hotkey)
            .map_err(|e| HotkeyError::Register(e.to_string()))?;
        Ok(Self { manager, hotkey })
    }

    /// The `id` field of the registered hotkey, so the receiver loop
    /// can filter to our hotkey if other parts of the app ever
    /// register one (today they don't, but the channel is process-
    /// global so being defensive is cheap).
    pub fn id(&self) -> u32 {
        self.hotkey.id()
    }
}

impl Drop for HotkeyListener {
    fn drop(&mut self) {
        // Best-effort: if the OS rejects unregistration (e.g. the
        // process is mid-teardown and the Carbon dispatch table is
        // already gone), there's nothing we can do about it.
        let _ = self.manager.unregister(self.hotkey);
    }
}

/// Validate (but don't register) a hotkey string. Used by the Settings
/// UI to surface a parse error without taking the OS-registration
/// hit on every keystroke the user types into the field.
pub fn parse_combo(combo: &str) -> Result<HotKey, HotkeyError> {
    HotKey::from_str(combo).map_err(|source| HotkeyError::Parse {
        combo: combo.to_string(),
        source,
    })
}

/// Drain pending hotkey events, returning `true` if our hotkey fired
/// in the `Pressed` direction. Designed to be called from a GPUI
/// background-timer loop — non-blocking, drains all queued events
/// so a backlog can't accumulate, and ignores Released events
/// (we only act on the press to avoid double-firing).
pub fn poll_pressed(expected_id: u32) -> bool {
    let mut fired = false;
    while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
        if event.id == expected_id && event.state == HotKeyState::Pressed {
            fired = true;
        }
    }
    fired
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_combo() {
        // Sanity: the constant we ship as a default must itself parse.
        // If we ever change `DEFAULT_HOTKEY` to something invalid this
        // test fails before users hit it at startup.
        assert!(parse_combo(DEFAULT_HOTKEY).is_ok());
    }

    #[test]
    fn rejects_invalid_combo() {
        assert!(parse_combo("not a real hotkey").is_err());
        // Empty string — would silently fail on register, surface
        // the error at parse time instead.
        assert!(parse_combo("").is_err());
    }

    #[test]
    fn parses_lowercase_and_alternate_modifier_names() {
        // global-hotkey accepts these spellings; we lean on the crate
        // for full coverage but pin a few representative combos so a
        // crate upgrade that breaks them shows up in CI.
        assert!(parse_combo("ctrl+alt+KeyA").is_ok());
        assert!(parse_combo("shift+super+KeyZ").is_ok());
    }
}
