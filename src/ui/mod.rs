pub mod ds;
pub mod errors;
pub mod icons;
// The old shell, screens and widgets read their colors from the deprecated
// palette shim until they are rebuilt; new code reads `cx.ds()`.
#[allow(deprecated)]
mod legacy;
pub mod palette;
#[cfg(test)]
#[allow(deprecated)]
mod palette_tests;
pub mod vm;
#[allow(deprecated)]
pub mod widgets;
pub mod windows;

pub use legacy::screens;
pub use legacy::shell::AppShell;

// The screens still name the shell by its old path.
use legacy::shell as app_shell;
