pub mod actions;

mod state;

pub use state::{
    AppState, CopyValueKind, UnlockPrompt, VaultBrowserModel, VaultStatus, VaultSummary,
};

use crate::ui::AppShell;
use gpui::{
    App, AppContext as _, Context, SharedString, Styled as _, TitlebarOptions, WindowBounds,
    WindowOptions, px, size,
};
use gpui_component::{ActiveTheme as _, Root};

const WINDOW_TITLE: &str = "STC KeePass";

pub fn run() {
    gpui_platform::application().run(|cx| {
        gpui_component::init(cx);
        actions::init(cx);
        open_main_window(cx);
    });
}

fn open_main_window(cx: &mut App) {
    let window_bounds = WindowBounds::centered(size(px(1120.), px(760.)), cx);

    cx.spawn(async move |cx| {
        let window_options = WindowOptions {
            window_bounds: Some(window_bounds),
            window_min_size: Some(size(px(860.), px(560.))),
            titlebar: Some(TitlebarOptions {
                title: Some(SharedString::from(WINDOW_TITLE)),
                ..TitlebarOptions::default()
            }),
            ..WindowOptions::default()
        };

        cx.open_window(window_options, |window, cx| {
            let app_state = cx.new(|_| AppState::default());
            let shell = cx.new(|cx| AppShell::new(app_state, window, cx));

            cx.new(|cx: &mut Context<Root>| Root::new(shell, window, cx).bg(cx.theme().background))
        })
        .expect("failed to open STC KeePass window");
    })
    .detach();
}
