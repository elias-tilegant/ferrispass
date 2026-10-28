pub mod errors;
pub mod icons;
mod legacy;
pub mod palette;
pub mod theme;
pub mod widgets;

pub use legacy::screens;
pub use legacy::shell::AppShell;

// The screens still name the shell by its old path.
use legacy::shell as app_shell;
