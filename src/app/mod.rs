pub mod actions;
pub mod assets;

mod recents;
mod search;
pub mod settings;
mod state;
pub mod sync_history;
pub(crate) mod time;

pub use recents::{RecentEntry, RecentsError};
pub use settings::AppSettings;
pub use state::{
    AppState, BiometricAttempt, BiometricLaunch, ConflictState, ConnectFlow, CopyValueKind,
    FaviconDownloadStatus, LibrarySelection, Overlay, SaveStatus, SyncBinding, SyncStatus,
    UnlockPrompt, VaultBrowserModel, VaultStatus, VaultSummary,
};
pub use sync_history::{SyncChangeKind, SyncHistoryEntry};

use crate::ui::{AppShell, theme as ui_theme};
use gpui::{
    App, AppContext as _, Context, SharedString, Styled as _, TitlebarOptions, WindowBounds,
    WindowOptions, px, size,
};
use gpui_component::{ActiveTheme as _, Root};

const APP_NAME: &str = "FerrisPass";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

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
                title: Some(SharedString::from(APP_NAME)),
                ..TitlebarOptions::default()
            }),
            ..WindowOptions::default()
        };

        cx.open_window(window_options, |window, cx| {
            let app_state = cx.new(|cx| {
                let mut state = AppState::with_resume();
                // Kick off the auto-update check at startup so a banner can
                // appear on the welcome screen if a newer release exists.
                // Reads settings directly because AppShell (which owns the
                // live AppSettings) hasn't been constructed yet at this
                // point in the bootstrap; the settings.json read is cheap
                // (small JSON, sync I/O).
                if settings::load().auto_update_check_enabled {
                    state.start_update_check(cx);
                }
                state
            });
            let shell = cx.new(|cx| AppShell::new(app_state, window, cx));

            cx.new(|cx: &mut Context<Root>| Root::new(shell, window, cx).bg(cx.theme().background))
        })
        .expect("failed to open FerrisPass window");
    })
    .detach();
}
