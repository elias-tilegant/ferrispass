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
    SyncTone, UnlockPrompt, VaultBrowserModel, VaultStatus, VaultSummary,
};
pub use sync_history::{SyncChangeKind, SyncHistoryEntry};

use crate::ui::{AppShell, theme as ui_theme};
use gpui::{
    App, AppContext as _, Context, Entity, QuitMode, SharedString, Styled as _, TitlebarOptions,
    WindowBounds, WindowOptions, px, size,
};
use gpui_component::{ActiveTheme as _, Root};

const APP_NAME: &str = "FerrisPass";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn run() {
    let application = gpui_platform::application()
        .with_quit_mode(QuitMode::Explicit)
        .with_assets(assets::AppAssets::new());

    application.run(|cx| {
        let fonts = assets::font_bytes();
        if !fonts.is_empty() {
            let _ = cx.text_system().add_fonts(fonts);
        }

        gpui_component::init(cx);
        ui_theme::init_from_system(cx);

        let app_state = cx.new(|cx| {
            let mut state = AppState::with_resume();
            // AppShell owns the live settings after bootstrap. Until then,
            // this small synchronous read decides whether startup should
            // schedule the first update check.
            if settings::load().auto_update_check_enabled {
                state.start_update_check(cx);
            }
            state
        });

        actions::init(cx);
        open_main_window(cx, app_state);
    });
}

fn open_main_window(cx: &mut App, app_state: Entity<AppState>) {
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
            window
                .on_window_should_close(cx, |window, cx| actions::request_window_close(window, cx));

            let shell = cx.new(|cx| AppShell::new(app_state, window, cx));

            cx.new(|cx: &mut Context<Root>| Root::new(shell, window, cx).bg(cx.theme().background))
        })
        .expect("failed to open FerrisPass window");
    })
    .detach();
}
