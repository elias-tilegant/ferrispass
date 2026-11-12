//! The quick access panel: the window ⌃⌥⌘P opens over any app to search the
//! open vault and act on an entry without going to the main window.
//!
//! The rules it keeps, which its tests hold:
//!
//! 1. The panel never activates the app. It is a `PopUp` window, which macOS
//!    makes a non-activating panel: it takes the keyboard (`focus: true`)
//!    while the app the user works in stays in front, and closing it removes
//!    the window and activates nothing, so the keys go back to that app. The
//!    one way out of the panel that brings FerrisPass forward is
//!    `show_in_app` in `view.rs` (⌘O), on the user's word.
//! 2. Every copy goes through `ClipboardService::write_secret`, with the
//!    clear timer from the settings. The panel knows no other way to the
//!    pasteboard. A copy keeps the panel open; an action that hands over to
//!    another app, signing in or opening a website, closes it.
//! 3. Every action acts on the entry the panel selected, by its id, never on
//!    the main window's selection.
//! 4. Every key and click in the panel counts as activity for the auto-lock
//!    (`activity::touch(Panel)`); the shortcut that opens it counts in
//!    `services::hotkeys`.
//! 5. Cmd+W and Escape close the panel and nothing else: no vault is locked,
//!    the app is not hidden, and the main window stays as it was. Cmd+M
//!    minimizes nothing while the panel is in front.
//! 6. It closes when it loses the keyboard, as after a click elsewhere, and
//!    when the vault is locked. Locking also forgets the search.
//! 7. The window is built anew every time it opens; the search and the
//!    selection live in the model (`ui::vm::panel`) for two minutes.
//! 8. No notification, no live region, and no title, user name or host in a
//!    log line, an accessibility id or an error text. Ids are
//!    `fp.panel.<component>.<index>`.
//! 9. While the panel is open, the Auto-Type shortcut types nothing
//!    (`services::hotkeys`): the panel has the keyboard, so the user name
//!    and the password would land in its search field.

use gpui::{
    AnyWindowHandle, App, AppContext as _, Bounds, Entity, Global, Pixels, Subscription, Task,
    Window, WindowBounds, WindowKind, WindowOptions, point, px, size,
};
use gpui_component::Root;

use crate::app::{AppState, VaultStatus};
use crate::launch::{LaunchContext, LaunchError, LaunchHandle, LaunchTarget};
use crate::ui::vm::panel::PanelVm;

pub mod view;

use view::PanelView;

/// The panel's width, in points.
pub const WIDTH: f32 = 680.0;

/// The panel's height, in points: the most it may take.
pub const HEIGHT: f32 = 460.0;

/// The panel's window while it is open, and what outlives it.
struct PanelWindow {
    handle: Option<AnyWindowHandle>,
    vm: Entity<PanelVm>,
    state: Entity<AppState>,
    /// Launch files handed to SAP GUI, each removed after the cleanup time
    /// from the settings, or when the vault is locked. Drop = remove.
    launches: Vec<Task<()>>,
    _state_changes: Subscription,
}

impl Global for PanelWindow {}

/// The panel's model and the app's state, made the first time the panel is
/// asked for. `None` before the app has its state.
fn panel(cx: &mut App) -> Option<(Entity<PanelVm>, Entity<AppState>)> {
    if let Some(panel) = cx.try_global::<PanelWindow>() {
        return Some((panel.vm.clone(), panel.state.clone()));
    }
    let state = crate::app::with_shared_state(cx, |_, cx| cx.entity())?;
    let vm = cx.new(|_| PanelVm::default());
    let watched = vm.clone();
    let subscription = cx.observe(&state, move |state, cx| state_changed(&watched, &state, cx));
    cx.set_global(PanelWindow {
        handle: None,
        vm: vm.clone(),
        state: state.clone(),
        launches: Vec::new(),
        _state_changes: subscription,
    });
    Some((vm, state))
}

/// Locking closes the panel and forgets its search; any other change shows
/// in the open panel at once.
fn state_changed(vm: &Entity<PanelVm>, state: &Entity<AppState>, cx: &mut App) {
    let open = matches!(state.read(cx).vault_status(), VaultStatus::Open { .. });
    let was_open = vm.read(cx).vault_name().is_some();
    if was_open && !open {
        close(cx);
        cx.global_mut::<PanelWindow>().launches.clear();
        vm.update(cx, |vm, cx| {
            vm.forget();
            cx.notify();
        });
        return;
    }
    if is_open(cx) {
        vm.update(cx, |vm, cx| {
            vm.refresh(state.read(cx));
            cx.notify();
        });
    }
}

/// Whether the panel is open.
pub fn is_open(cx: &App) -> bool {
    cx.try_global::<PanelWindow>()
        .is_some_and(|panel| panel.handle.is_some())
}

/// Opens the panel when it is closed and closes it when it is open: what
/// the global shortcut does.
pub fn toggle(cx: &mut App) {
    if is_open(cx) {
        close(cx);
    } else {
        open(cx);
    }
}

/// Opens the panel over whatever app is in front, without activating
/// FerrisPass. A search from less than two minutes ago is still there.
pub fn open(cx: &mut App) {
    if is_open(cx) {
        return;
    }
    let Some((vm, state)) = panel(cx) else {
        return;
    };
    let now = cx.background_executor().now();
    vm.update(cx, |vm, cx| {
        vm.forget_if_stale(now);
        vm.refresh(state.read(cx));
    });
    let opened = cx.open_window(window_options(cx), |window, cx| {
        let view = cx.new(|cx| PanelView::new(vm, state, window, cx));
        cx.new(|cx| Root::new(view, window, cx))
    });
    if let Ok(handle) = opened {
        cx.global_mut::<PanelWindow>().handle = Some(handle.into());
    }
}

/// Closes the panel from outside its window: the shortcut, a lock. Removes
/// the window and activates nothing.
pub fn close(cx: &mut App) {
    if let Some(handle) = forget_window(cx) {
        let _ = handle.update(cx, |_, window, _| window.remove_window());
    }
}

/// Closes the panel from inside its window, whose handle cannot be updated
/// from there: Escape, Cmd+W, an action that hands over to another app, and
/// the panel losing the keyboard.
fn close_from(window: &mut Window, cx: &mut App) {
    forget_window(cx);
    window.remove_window();
}

/// Takes the open window's handle and notes when the panel closed, so its
/// search is kept for two minutes.
fn forget_window(cx: &mut App) -> Option<AnyWindowHandle> {
    if !cx.has_global::<PanelWindow>() {
        return None;
    }
    let panel = cx.global_mut::<PanelWindow>();
    let handle = panel.handle.take()?;
    let vm = panel.vm.clone();
    let now = cx.background_executor().now();
    vm.update(cx, |vm, _| vm.remember(now));
    Some(handle)
}

/// A non-activating panel that takes the keyboard, 680 by 460 points, in the
/// upper part of the main display, not movable and not resizable.
pub fn window_options(cx: &App) -> WindowOptions {
    let display = cx.primary_display();
    let bounds = display
        .as_ref()
        .map(|display| panel_bounds(display.bounds()))
        .unwrap_or_else(|| Bounds::centered(None, size(px(WIDTH), px(HEIGHT)), cx));
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        focus: true,
        show: true,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        display_id: display.map(|display| display.id()),
        ..WindowOptions::default()
    }
}

/// Where the panel goes on a display with these bounds: centred, its top a
/// sixth of the way down, and never larger than the display.
pub fn panel_bounds(display: Bounds<Pixels>) -> Bounds<Pixels> {
    let display_width = f32::from(display.size.width);
    let display_height = f32::from(display.size.height);
    let width = WIDTH.min(display_width);
    let height = HEIGHT.min(display_height);
    let x = f32::from(display.origin.x) + (display_width - width) / 2.0;
    let y = f32::from(display.origin.y) + (display_height / 6.0).min(display_height - height);
    Bounds {
        origin: point(px(x), px(y)),
        size: size(px(width), px(height)),
    }
}

/// Hands the entry to SAP GUI and keeps the launch file until the cleanup
/// time from the settings has passed or the vault is locked.
fn sign_in(
    target: LaunchTarget,
    context: LaunchContext<'_>,
    cx: &mut App,
) -> Result<(), LaunchError> {
    let handle = start(target, context)?;
    let cleanup = crate::app::with_shared_shell(cx, |shell, _| {
        shell.settings().launch_cleanup_secs_clamped()
    })
    .unwrap_or(crate::app::settings::DEFAULT_LAUNCH_CLEANUP_SECS);
    let wait = cx
        .background_executor()
        .timer(std::time::Duration::from_secs(u64::from(cleanup)));
    let task = cx.spawn(async move |_| {
        wait.await;
        drop(handle);
    });
    if cx.has_global::<PanelWindow>() {
        cx.global_mut::<PanelWindow>().launches.push(task);
    }
    Ok(())
}

#[cfg(not(test))]
fn start(target: LaunchTarget, context: LaunchContext<'_>) -> Result<LaunchHandle, LaunchError> {
    crate::launch::launch(target, context)
}

/// Unit tests never start SAP GUI: the launch is noted instead.
#[cfg(test)]
fn start(target: LaunchTarget, context: LaunchContext<'_>) -> Result<LaunchHandle, LaunchError> {
    tests::note_launch(context.entry.id.clone(), context.password.is_some());
    Ok(LaunchHandle {
        temp_file: None,
        launcher_id: target.id(),
    })
}

/// The open panel's window and the panel's model, for the tests.
#[cfg(test)]
pub(crate) fn for_test(cx: &App) -> (Option<AnyWindowHandle>, Option<Entity<PanelVm>>) {
    cx.try_global::<PanelWindow>()
        .map_or((None, None), |panel| (panel.handle, Some(panel.vm.clone())))
}

#[cfg(test)]
pub(crate) mod tests;
