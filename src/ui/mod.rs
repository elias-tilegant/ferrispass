// The old shell, screens and widgets read their colors from the deprecated
// palette shim until they are rebuilt; new code reads `cx.ds()`.
#[allow(deprecated)]
mod app_shell;
pub mod ds;
pub mod errors;
pub mod icons;
pub mod palette;
#[allow(deprecated)]
pub mod screens;
pub mod theme;
#[allow(deprecated)]
pub mod widgets;

pub use app_shell::AppShell;
