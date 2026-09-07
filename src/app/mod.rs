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
    FaviconDownloadStatus, LibrarySelection, Overlay, SaveStatus, SyncActivity, SyncBinding,
    SyncStatus, SyncTone, UnlockPrompt, VaultBrowserModel, VaultStatus, VaultSummary,
};
pub use sync_history::{SyncChangeKind, SyncHistoryEntry};

use crate::ui::{AppShell, theme as ui_theme};
use gpui::{
    App, AppContext as _, Context, Entity, QuitMode, SharedString, Styled as _, TitlebarOptions,
    WindowBounds, WindowOptions, px, size,
};
use gpui_component::{ActiveTheme as _, Root};

/// The one `AppState`, reachable from app-level handlers that have no window.
/// Closing the window locks the vault but keeps the process alive, so the
/// Dock, the Window menu and the Auto-Type hotkey all need a way back to it.
struct SharedAppState(Entity<AppState>);

impl gpui::Global for SharedAppState {}

/// The one `AppShell`. Cmd+W and Cmd+Q are app-level actions with no view in
/// hand, and both throw away an unsaved entry draft, which lives in the
/// shell's inputs rather than in `AppState`.
struct SharedAppShell(Entity<AppShell>);

impl gpui::Global for SharedAppShell {}

/// Run `f` against the one `AppState`, if the app has finished booting.
pub(crate) fn with_shared_state<R>(
    cx: &mut App,
    f: impl FnOnce(&mut AppState, &mut Context<AppState>) -> R,
) -> Option<R> {
    let state = cx.try_global::<SharedAppState>()?.0.clone();
    Some(state.update(cx, f))
}

/// Run `f` against the one `AppShell`, if the app has finished booting.
pub(crate) fn with_shared_shell<R>(
    cx: &mut App,
    f: impl FnOnce(&mut AppShell, &mut Context<AppShell>) -> R,
) -> Option<R> {
    let shell = cx.try_global::<SharedAppShell>()?.0.clone();
    Some(shell.update(cx, f))
}

/// Bring the window back after Cmd+W hid it. Also what a Dock click does.
/// The window survives the hide, so this only has to re-open one if something
/// unexpected destroyed it.
pub fn show_main_window(cx: &mut App) {
    if cx.windows().is_empty()
        && let Some(state) = cx
            .try_global::<SharedAppState>()
            .map(|shared| shared.0.clone())
    {
        open_main_window(cx, state, settings::load().window);
    }
    cx.activate(true);
}

const APP_NAME: &str = "FerrisPass";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn run() {
    let application = gpui_platform::application()
        .with_quit_mode(QuitMode::Explicit)
        .with_assets(assets::AppAssets::new());

    // Cmd+W locks the vault and closes the window without ending the process,
    // so clicking the Dock icon has to bring it back.
    application.on_reopen(show_main_window);

    application.run(|cx| {
        let fonts = assets::font_bytes();
        if !fonts.is_empty() {
            let _ = cx.text_system().add_fonts(fonts);
        }

        gpui_component::init(cx);
        // One read for both the theme and the saved window geometry: the
        // first paint has to be in the right appearance and the right place.
        let startup_settings = settings::load();
        ui_theme::init_from_settings(startup_settings.theme, cx);

        let app_state = cx.new(|cx| {
            let mut state = AppState::with_resume();
            if startup_settings.auto_update_check_enabled {
                state.start_update_check(cx);
            }
            state
        });

        actions::init(cx);
        // Held globally so a window closed with Cmd+W can be reopened from
        // the Dock against the same state, rather than restarting the app.
        cx.set_global(SharedAppState(app_state.clone()));
        open_main_window(cx, app_state, startup_settings.window);
    });
}

fn open_main_window(
    cx: &mut App,
    app_state: Entity<AppState>,
    saved: Option<crate::app::settings::WindowBoundsSetting>,
) {
    // A saved size and position, when it still makes sense. Anything smaller
    // than the window minimum, or with a non-finite coordinate from a
    // hand-edited file, falls back to a centred default rather than opening
    // something unusable.
    let displays: Vec<settings::WindowBoundsSetting> = cx
        .displays()
        .iter()
        .map(|display| {
            let bounds = display.bounds();
            settings::WindowBoundsSetting {
                x: f32::from(bounds.origin.x),
                y: f32::from(bounds.origin.y),
                width: f32::from(bounds.size.width),
                height: f32::from(bounds.size.height),
            }
        })
        .collect();
    let window_bounds = saved
        .filter(settings::WindowBoundsSetting::is_usable)
        .filter(|bounds| bounds.is_reachable_on(&displays))
        .map(|bounds| {
            WindowBounds::Windowed(gpui::Bounds {
                origin: gpui::point(px(bounds.x), px(bounds.y)),
                size: size(px(bounds.width), px(bounds.height)),
            })
        })
        .unwrap_or_else(|| WindowBounds::centered(size(px(1120.), px(760.)), cx));

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
            window.on_window_should_close(cx, actions::request_window_close);

            let shell = cx.new(|cx| AppShell::new(app_state, window, cx));
            // Reachable from the app-level Cmd+W / Cmd+Q handlers, which have
            // to ask the editor whether it holds an unsaved draft.
            cx.set_global(SharedAppShell(shell.clone()));

            cx.new(|cx: &mut Context<Root>| Root::new(shell, window, cx).bg(cx.theme().background))
        })
        .expect("failed to open FerrisPass window");
    })
    .detach();
}
