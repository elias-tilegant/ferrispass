use std::collections::HashMap;

use gpui::{
    App, Entity, Global, InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce,
    WeakEntity, Window, div, prelude::FluentBuilder as _,
};
use gpui_component::input::{self, Input, InputContentType, InputState};

/// Remembers whether a shared input was protected on its previous paint.
/// Synchronizing only on protection transitions preserves an explicit reveal
/// made through the mask toggle across ordinary re-renders.
#[derive(Default)]
struct SecretInputMaskRegistry {
    protected: HashMap<WeakEntity<InputState>, bool>,
}

impl Global for SecretInputMaskRegistry {}

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
                input.mask_toggle().context_menu(|menu, _, _| {
                    menu.menu("Paste", Box::new(input::Paste))
                        .separator()
                        .menu("Select All", Box::new(input::SelectAll))
                })
            });

        div()
            .when(self.protected, |this| {
                this.capture_action(|_: &input::Copy, _, cx| cx.stop_propagation())
                    .capture_action(|_: &input::Cut, _, cx| cx.stop_propagation())
            })
            .child(input)
    }
}
