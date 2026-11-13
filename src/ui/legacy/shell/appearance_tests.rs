//! How the shell reacts when macOS switches between light and dark.
use super::*;
use crate::app::settings::ThemeChoice;
use crate::ui::ds::{self, ActiveTokens};
use gpui::TestAppContext;
use gpui_component::Root;

/// Puts `applied` into effect, opens a shell whose theme setting is `chosen`
/// and returns whether the app is dark afterwards.
///
/// GPUI calls a window's appearance observers once right after the window
/// opens, the same call it makes when macOS switches. `chosen` is set in
/// memory only, which is where a new choice is until the settings file has
/// been written in the background.
fn dark_after_appearance_change(
    cx: &mut TestAppContext,
    chosen: ThemeChoice,
    applied: ThemeChoice,
) -> bool {
    cx.update(|cx| ds::apply_choice(applied, crate::app::settings::TextSize::Standard, None, cx));
    let state = cx.new(|_| AppState::default());
    cx.add_window(|window, cx| {
        let shell = cx.new(|cx| {
            let mut shell = AppShell::new(state.clone(), window, cx);
            shell.settings.theme = chosen;
            shell
        });
        Root::new(shell, window, cx)
    });
    cx.run_until_parked();
    cx.read(|cx| cx.ds().mode.is_dark())
}

/// A change in macOS goes by the theme the user picked last, even when the
/// settings file still has the one before: it is saved in the background,
/// and a failed save leaves it behind for good.
#[gpui::test]
fn appearance_change_follows_the_choice_in_memory(cx: &mut TestAppContext) {
    let _lock = ds::tests::lock_mode();
    cx.update(|cx| {
        gpui_component::init(cx);
        crate::app::actions::init(cx);
    });
    // The test platform always reports a light system appearance, so the
    // second case starts out dark to see the app follow it.
    assert!(
        dark_after_appearance_change(cx, ThemeChoice::Dark, ThemeChoice::Dark),
        "Dark was picked, so a change in macOS must leave the app dark"
    );
    assert!(
        !dark_after_appearance_change(cx, ThemeChoice::System, ThemeChoice::Dark),
        "System was picked, so the app must follow macOS to light"
    );
}
