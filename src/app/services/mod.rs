//! App-wide services. They work the same with or without a window in front:
//! the clipboard lives in a GPUI global, the idle clock in a static.

pub mod activity;
pub mod clipboard;
