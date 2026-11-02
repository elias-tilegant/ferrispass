//! What macOS says about contrast, motion and VoiceOver.
//!
//! Read on demand rather than observed: `ds::apply_choice` asks each time it
//! runs, and the three calls are cheap.

/// The accessibility settings the design system follows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct A11yPrefs {
    /// Increase contrast, in the Display settings for accessibility.
    pub increase_contrast: bool,
    /// Reduce motion, in the same place.
    pub reduce_motion: bool,
    /// VoiceOver is running.
    pub voiceover: bool,
}

/// The settings as macOS reports them now.
#[cfg(target_os = "macos")]
pub fn read() -> A11yPrefs {
    let workspace = objc2_app_kit::NSWorkspace::sharedWorkspace();
    A11yPrefs {
        increase_contrast: workspace.accessibilityDisplayShouldIncreaseContrast(),
        reduce_motion: workspace.accessibilityDisplayShouldReduceMotion(),
        voiceover: workspace.isVoiceOverEnabled(),
    }
}

/// Other platforms have nothing to read here: everything off.
#[cfg(not(target_os = "macos"))]
pub fn read() -> A11yPrefs {
    A11yPrefs::default()
}
