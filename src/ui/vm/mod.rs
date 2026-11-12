//! View models for the windows in `ui::windows`. A view model holds what a
//! window shows and decides what its keys do, reading the app's state through
//! `AppState`'s functions. It draws nothing, so its rules can be tested
//! without a window.

pub mod panel;
