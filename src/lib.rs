pub mod app;
pub mod autotype;
pub mod biometric;
pub mod cli;
pub mod cli_install;
pub mod clipboard;
pub mod domain;
pub mod favicon;
pub mod i18n;
pub mod keepass;
pub mod launch;
#[cfg(target_os = "macos")]
pub(crate) mod session_lock;
pub mod sync;
pub mod ui;
pub mod update;

// The text catalogues in locales/, compiled into the crate. rust-i18n wants
// this at the crate root; everything else about text keys is in src/i18n/.
rust_i18n::i18n!("locales", fallback = "en");
