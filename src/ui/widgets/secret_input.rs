use std::collections::HashMap;

use gpui::{
    App, Entity, Global, InteractiveElement as _, IntoElement, Menu, MenuItem, ParentElement as _,
    RenderOnce, WeakEntity, Window, div, prelude::FluentBuilder as _,
};
use gpui_component::{
    input::{self, Input, InputContentType, InputState},
    native_menu::NativeMenu,
};

/// Remembers whether a shared input was protected on its previous paint.
/// Synchronizing only on protection transitions preserves an explicit reveal
/// made through the mask toggle across ordinary re-renders.
#[derive(Default)]
struct SecretInputMaskRegistry {
    protected: HashMap<WeakEntity<InputState>, bool>,
}

impl Global for SecretInputMaskRegistry {}

/// The only items a protected field's context menu offers: putting a value in
/// and selecting it. No Copy or Cut, so the menu cannot export the secret.
/// A plain `gpui::Menu` so a test can read the items back; GPUI Kit turns it
/// into the native menu item for item.
pub(crate) fn protected_context_menu() -> Menu {
    Menu::new("").items([
        MenuItem::action("Paste", input::Paste),
        MenuItem::separator(),
        MenuItem::action("Select All", input::SelectAll),
    ])
}

/// Password-style input that prevents GPUI's normal Copy/Cut handlers from
/// exporting a protected value as a plain clipboard item.
#[derive(IntoElement)]
pub struct SecretInput {
    state: Entity<InputState>,
    protected: bool,
    content_type: Option<InputContentType>,
}

impl SecretInput {
    pub fn new(
        state: &Entity<InputState>,
        protected: bool,
        content_type: Option<InputContentType>,
    ) -> Self {
        Self {
            state: state.clone(),
            protected,
            content_type,
        }
    }
}

impl RenderOnce for SecretInput {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state_key = self.state.downgrade();
        let previous_protection = {
            let registry = cx.default_global::<SecretInputMaskRegistry>();
            registry
                .protected
                .retain(|state, _| state.upgrade().is_some());
            registry.protected.insert(state_key, self.protected)
        };

        if previous_protection != Some(self.protected) {
            self.state.update(cx, |state, cx| {
                state.set_masked(self.protected, window, cx);
            });
        }

        let input = Input::new(&self.state)
            .when_some(self.content_type, |input, content_type| {
                input.content_type(content_type)
            })
            .when(self.protected, |input| {
                input
                    .mask_toggle()
                    .context_menu(|_, _, _| NativeMenu::from(protected_context_menu()))
            });

        // The library blocks Copy and Cut only while the field is masked. After a
        // reveal through the mask toggle `masked` is off and `protected` is not,
        // and this capture is the only thing between the value and the clipboard.
        div()
            .when(self.protected, |this| {
                this.capture_action(|_: &input::Copy, _, cx| cx.stop_propagation())
                    .capture_action(|_: &input::Cut, _, cx| cx.stop_propagation())
            })
            .child(input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        Action as _, AppContext as _, ClipboardItem, Context, Focusable as _, Render, Styled as _,
        TestAppContext, WindowHandle,
    };
    use gpui_component::Root;

    const SECRET: &str = "correct horse battery staple";

    struct Form {
        field: Entity<InputState>,
        protected: bool,
    }

    impl Render for Form {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .child(SecretInput::new(&self.field, self.protected, None))
        }
    }

    /// A window with one `SecretInput` holding `SECRET`, focused.
    fn open_form(
        cx: &mut TestAppContext,
        protected: bool,
    ) -> (WindowHandle<Root>, Entity<InputState>) {
        cx.update(gpui_component::init);
        let mut field = None;
        let window = cx.add_window(|window, cx| {
            let state = cx.new(|cx| {
                let mut state = InputState::new(window, cx);
                state.set_value(SECRET, window, cx);
                state
            });
            field = Some(state.clone());
            let form = cx.new(|_| Form {
                field: state,
                protected,
            });
            Root::new(form, window, cx)
        });
        cx.run_until_parked();
        let field = field.expect("field state");
        window
            .update(cx, |_, window, cx| {
                field.update(cx, |state, cx| state.focus(window, cx));
            })
            .unwrap();
        cx.run_until_parked();
        (window, field)
    }

    fn masked(cx: &TestAppContext, field: &Entity<InputState>) -> bool {
        field.read_with(cx, |state, _| state.presentation().is_masked())
    }

    fn value(cx: &TestAppContext, field: &Entity<InputState>) -> String {
        field.read_with(cx, |state, _| state.value().to_string())
    }

    fn focused(
        cx: &mut TestAppContext,
        window: WindowHandle<Root>,
        field: &Entity<InputState>,
    ) -> bool {
        window
            .update(cx, |_, window, cx| {
                field.read(cx).focus_handle(cx).is_focused(window)
            })
            .unwrap()
    }

    /// Every way the app lets a focused field copy or cut: the keys, and the
    /// Edit menu, which dispatches the same actions to the focused element.
    #[derive(Clone, Copy, Debug)]
    enum ExportPath {
        CopyKeys,
        CopyMenu,
        CutKeys,
        CutMenu,
    }

    const EXPORT_PATHS: [ExportPath; 4] = [
        ExportPath::CopyKeys,
        ExportPath::CopyMenu,
        ExportPath::CutKeys,
        ExportPath::CutMenu,
    ];

    /// Selects the whole value and exports it along `path`. Asserts that the
    /// field is still focused afterwards: a path that moved the focus away
    /// would leave the clipboard empty for the wrong reason.
    fn export(
        cx: &mut TestAppContext,
        window: WindowHandle<Root>,
        field: &Entity<InputState>,
        path: ExportPath,
    ) {
        match path {
            ExportPath::CopyKeys => cx.simulate_keystrokes(window.into(), "cmd-a cmd-c"),
            ExportPath::CutKeys => cx.simulate_keystrokes(window.into(), "cmd-a cmd-x"),
            ExportPath::CopyMenu => {
                cx.dispatch_action(window.into(), input::SelectAll);
                cx.dispatch_action(window.into(), input::Copy);
            }
            ExportPath::CutMenu => {
                cx.dispatch_action(window.into(), input::SelectAll);
                cx.dispatch_action(window.into(), input::Cut);
            }
        }
        assert!(focused(cx, window, field), "{path:?} moved the focus away");
    }

    /// The harness itself: an unprotected field copies and cuts along every
    /// path, so an empty clipboard in the tests below means blocked, not
    /// unreachable.
    #[gpui::test]
    fn unprotected_field_exports_along_every_path(cx: &mut TestAppContext) {
        let (window, field) = open_form(cx, false);
        assert!(!masked(cx, &field));
        for path in EXPORT_PATHS {
            cx.write_to_clipboard(ClipboardItem::new_string(String::new()));
            export(cx, window, &field, path);
            assert_eq!(
                cx.read_from_clipboard().and_then(|item| item.text()),
                Some(SECRET.to_string()),
                "{path:?} does not reach the field"
            );
            // Cut empties the field; put the value back for the next path.
            window
                .update(cx, |_, window, cx| {
                    field.update(cx, |state, cx| state.set_value(SECRET, window, cx));
                })
                .unwrap();
            cx.run_until_parked();
        }
    }

    fn assert_exports_nothing(
        cx: &mut TestAppContext,
        window: WindowHandle<Root>,
        field: &Entity<InputState>,
    ) {
        for path in EXPORT_PATHS {
            export(cx, window, field, path);
            assert_eq!(
                cx.read_from_clipboard(),
                None,
                "{path:?} exported the value"
            );
            assert_eq!(value(cx, field), SECRET, "{path:?} removed the value");
        }
    }

    /// T0.1 (a): protected and masked.
    #[gpui::test]
    fn masked_protected_field_exports_nothing(cx: &mut TestAppContext) {
        let (window, field) = open_form(cx, true);
        assert!(masked(cx, &field), "a protected field starts masked");
        assert_exports_nothing(cx, window, &field);
    }

    /// T0.1 (b): protected, revealed through the mask toggle. Here the
    /// library no longer blocks anything; only the capture in `render` does.
    #[gpui::test]
    fn revealed_protected_field_exports_nothing(cx: &mut TestAppContext) {
        let (window, field) = open_form(cx, true);
        window
            .update(cx, |_, window, cx| {
                // What the eye button of `mask_toggle` calls.
                field.update(cx, |state, cx| state.toggle_masked(window, cx));
            })
            .unwrap();
        cx.run_until_parked();
        assert!(
            !masked(cx, &field),
            "the reveal must survive the re-render, or this test proves nothing"
        );
        assert_exports_nothing(cx, window, &field);
    }

    /// Checks the menu the protected field hands to `Input::context_menu`.
    /// Whether `render` actually hands it over is not observable headless:
    /// `NativeMenu::show` goes straight to AppKit and is a no-op in the test
    /// window. That half stays in the manual protocol (right-click on a
    /// protected field shows only Paste and Select All).
    #[test]
    fn protected_context_menu_offers_only_paste_and_select_all() {
        let items: Vec<String> = protected_context_menu()
            .items
            .iter()
            .map(|item| match item {
                MenuItem::Action { name, action, .. } => format!("{name}={}", action.name()),
                MenuItem::Separator => "-".to_string(),
                MenuItem::Submenu(menu) => format!("submenu {}", menu.name),
                MenuItem::SystemMenu(_) => "system menu".to_string(),
            })
            .collect();
        assert_eq!(
            items,
            [
                format!("Paste={}", input::Paste.name()),
                "-".to_string(),
                format!("Select All={}", input::SelectAll.name()),
            ]
        );
    }
}
