//! App-wide services. They work the same with or without a window in front:
//! the clipboard lives in a GPUI global, the idle clock in a static, and the
//! global shortcuts in a GPUI global with their own poll loop.

pub mod activity;
pub mod clipboard;
pub mod hotkeys;
