//! The design gallery: every component of the design system in every state
//! it offers, in the four modes, at both text sizes and in both languages.
//!
//! ```sh
//! cargo run --example design_gallery
//! ```
//!
//! One window, 1120 by 760. It starts the way the app starts, without the
//! app's state, windows and services: its own code reads no settings, no
//! vault and no clipboard. The text fields on its pages are the Kit's
//! inputs and keep the Kit's copy, cut and paste.
//! `docs/redesign/p1a-30-design-gallery.md` says what each page shows and
//! how to switch the mode, the text size and the language.

mod gallery;
mod pages;
#[cfg(test)]
mod tests;

use ferrispass::app::assets::AppAssets;
use gpui::{
    AppContext as _, QuitMode, SharedString, TitlebarOptions, WindowBounds, WindowOptions, px, size,
};
use gpui_component::Root;

use gallery::Gallery;

fn main() {
    let application = gpui_platform::application()
        .with_quit_mode(QuitMode::LastWindowClosed)
        .with_assets(AppAssets::new());

    application.run(|cx| {
        gallery::init(cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1120.), px(760.)), cx)),
            titlebar: Some(TitlebarOptions {
                title: Some(SharedString::from("FerrisPass design gallery")),
                ..TitlebarOptions::default()
            }),
            ..WindowOptions::default()
        };
        cx.open_window(options, |window, cx| {
            let gallery = cx.new(|cx| Gallery::new(window, cx));
            cx.new(|cx| Root::new(gallery, window, cx))
        })
        .expect("failed to open the gallery window");
        cx.activate(true);
    });
}
