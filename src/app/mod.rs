pub mod actions;
pub mod assets;

mod recents;
mod state;
pub(crate) mod time;

pub use recents::{RecentEntry, RecentsError};
pub use state::{
    AppState, ConflictState, ConnectFlow, CopyValueKind, LibrarySelection, Overlay,
    SaveStatus, SyncBinding, SyncStatus, UnlockPrompt, VaultBrowserModel, VaultStatus,
    VaultSummary,
};

use crate::ui::{AppShell, theme as ui_theme};
use gpui::{
    App, AppContext as _, Context, SharedString, Styled as _, TitlebarOptions, WindowBounds,
    WindowOptions, px, size,
};
use gpui_component::{ActiveTheme as _, Root};

const WINDOW_TITLE: &str = "KeePass RS";

pub fn run() {
    let application = gpui_platform::application().with_assets(assets::AppAssets::new());

    application.run(|cx| {
        let fonts = assets::font_bytes();
        if !fonts.is_empty() {
            let _ = cx.text_system().add_fonts(fonts);
        }

        gpui_component::init(cx);
        ui_theme::init_from_system(cx);
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
            let app_state = cx.new(|_| AppState::with_resume());
            let shell = cx.new(|cx| AppShell::new(app_state, window, cx));

            cx.new(|cx: &mut Context<Root>| Root::new(shell, window, cx).bg(cx.theme().background))
        })
        .expect("failed to open KeePass RS window");
    })
    .detach();
}
