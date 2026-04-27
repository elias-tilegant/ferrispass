use crate::{
    app::{
        AppState, UnlockPrompt, VaultSummary,
        actions::{APP_CONTEXT, CancelUnlock, LockVault, OpenVault, SubmitPassword},
    },
    keepass::KeePassRepository,
    ui::vault_browser::{render_group_tree, render_vault_browser},
};
use gpui::{
    AnyElement, AppContext as _, ClickEvent, Context, Entity, InteractiveElement as _,
    IntoElement as _, ParentElement as _, PathPromptOptions, Render, Styled as _, Subscription,
    Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, StyledExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use std::path::{Path, PathBuf};

pub struct AppShell {
    state: Entity<AppState>,
    password_input: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl AppShell {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let password_input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Master password")
        });

        let _subscriptions = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            cx.subscribe_in(&password_input, window, Self::on_password_input_event),
        ];

        Self {
            state,
            password_input,
            _subscriptions,
        }
    }

    fn on_action_open_vault(&mut self, _: &OpenVault, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt_for_vault_path(window, cx);
    }

    fn on_action_submit_password(
        &mut self,
        _: &SubmitPassword,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.submit_password(window, cx);
    }

    fn on_action_cancel_unlock(
        &mut self,
        _: &CancelUnlock,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_unlock(window, cx);
    }

    fn on_action_lock_vault(&mut self, _: &LockVault, _: &mut Window, cx: &mut Context<Self>) {
        self.state.update(cx, |state, cx| state.lock_vault(cx));
    }

    fn on_password_input_event(
        &mut self,
        _: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::PressEnter { .. }) {
            self.submit_password(window, cx);
        }
    }

    fn prompt_for_vault_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Select a KeePass database".into()),
        });

        let shell = cx.entity();
        cx.spawn_in(window, async move |_, window| {
            let path = paths.await.ok()?.ok()??.first()?.clone();

            window
                .update(|window, cx| {
                    shell.update(cx, |shell, cx| {
                        shell.select_vault_path(path, window, cx);
                    })
                })
                .ok()
        })
        .detach();
    }

    fn select_vault_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if !is_kdbx_path(&path) {
            self.state.update(cx, |state, cx| {
                state.fail_vault_selection(
                    Some(path),
                    "Selected file is not a .kdbx database.",
                    cx,
                );
            });
            return;
        }

        self.password_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });

        self.state
            .update(cx, |state, cx| state.request_password(path, cx));
    }

    fn submit_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.state.read(cx).pending_unlock_path() else {
            return;
        };

        let password = self.password_input.read(cx).value().to_string();
        if password.is_empty() {
            self.state.update(cx, |state, cx| {
                state.set_unlock_error("Enter the master password.", cx)
            });
            self.password_input
                .update(cx, |input, cx| input.focus(window, cx));
            return;
        }

        self.password_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.state
            .update(cx, |state, cx| state.begin_open(path.clone(), cx));

        let state = self.state.downgrade();
        let open_task = cx.background_spawn(async move {
            let result = KeePassRepository::open_with_password(&path, &password)
                .map_err(|error| error.to_string());

            (path, result)
        });

        cx.spawn(async move |_, cx| {
            let (path, result) = open_task.await;
            let _ = state.update(cx, |state, cx| {
                state.finish_open_attempt(path, result, cx);
            });
        })
        .detach();
    }

    fn cancel_unlock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.state.read(cx).pending_unlock_path().is_none() {
            return;
        }

        self.password_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.state.update(cx, |state, cx| state.lock_vault(cx));
    }
}

impl Render for AppShell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let summary = self.state.read(cx).summary();
        let unlock_prompt = self.state.read(cx).unlock_prompt();
        let browser = self.state.read(cx).vault_browser();

        let base = div()
            .key_context(APP_CONTEXT)
            .on_action(cx.listener(Self::on_action_open_vault))
            .on_action(cx.listener(Self::on_action_submit_password))
            .on_action(cx.listener(Self::on_action_cancel_unlock))
            .on_action(cx.listener(Self::on_action_lock_vault))
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                h_flex()
                    .size_full()
                    .child(render_sidebar(&summary, browser.as_ref(), &self.state, cx))
                    .child(render_workspace(&summary, browser, &self.state, cx)),
            );

        if let Some(prompt) = unlock_prompt {
            base.child(render_unlock_overlay(&self.password_input, prompt, cx))
        } else {
            base
        }
    }
}

fn render_sidebar(
    summary: &VaultSummary,
    browser: Option<&crate::app::VaultBrowserModel>,
    state: &Entity<AppState>,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    v_flex()
        .h_full()
        .w(px(286.))
        .flex_shrink_0()
        .bg(cx.theme().sidebar)
        .text_color(cx.theme().sidebar_foreground)
        .border_r_1()
        .border_color(cx.theme().sidebar_border)
        .child(
            v_flex()
                .gap_3()
                .p_4()
                .border_b_1()
                .border_color(cx.theme().sidebar_border)
                .child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(div().text_lg().font_semibold().child("STC KeePass"))
                        .child(status_badge(&summary.status, cx)),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .overflow_hidden()
                        .child(summary.subtitle.clone()),
                ),
        )
        .child(match browser {
            Some(browser) => render_group_tree(browser, state, cx),
            None => render_default_navigation(summary, cx),
        })
        .child(div().flex_1())
        .child(
            v_flex()
                .gap_3()
                .p_4()
                .border_t_1()
                .border_color(cx.theme().sidebar_border)
                .child(stat_row("Entries", summary.entries, cx))
                .child(stat_row("Groups", summary.groups, cx)),
        )
        .into_any_element()
}

fn render_workspace(
    summary: &VaultSummary,
    browser: Option<crate::app::VaultBrowserModel>,
    state: &Entity<AppState>,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let content = if let Some(browser) = browser {
        render_vault_browser(browser, state, cx)
    } else if summary.is_busy {
        render_opening_state(summary, cx)
    } else {
        render_empty_state(summary, cx)
    };

    v_flex()
        .flex_1()
        .min_w(px(0.))
        .bg(cx.theme().background)
        .child(
            h_flex()
                .h(px(64.))
                .px_5()
                .border_b_1()
                .border_color(cx.theme().border)
                .justify_between()
                .child(
                    v_flex()
                        .gap_1()
                        .child(div().text_lg().font_semibold().child("Vault"))
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(summary.status.clone()),
                        ),
                )
                .child(if summary.is_open {
                    lock_button("toolbar-lock-vault", cx).into_any_element()
                } else {
                    open_button("toolbar-open-vault", summary.is_busy, cx).into_any_element()
                }),
        )
        .child(content)
        .into_any_element()
}

fn render_default_navigation(summary: &VaultSummary, cx: &mut Context<AppShell>) -> AnyElement {
    v_flex()
        .gap_1()
        .p_3()
        .child(nav_item("All entries", summary.is_open, cx))
        .child(nav_item("Groups", false, cx))
        .child(nav_item("Favorites", false, cx))
        .into_any_element()
}

fn render_empty_state(summary: &VaultSummary, cx: &mut Context<AppShell>) -> AnyElement {
    v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_4()
        .p_6()
        .child(
            v_flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_2xl()
                        .font_semibold()
                        .child(summary.title.clone()),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(summary.subtitle.clone()),
                ),
        )
        .child(open_button("empty-open-vault", false, cx))
        .into_any_element()
}

fn render_opening_state(summary: &VaultSummary, cx: &mut Context<AppShell>) -> AnyElement {
    v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_3()
        .p_6()
        .child(
            div()
                .text_2xl()
                .font_semibold()
                .child(summary.title.clone()),
        )
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(summary.subtitle.clone()),
        )
        .into_any_element()
}

fn open_button(id: &'static str, disabled: bool, cx: &mut Context<AppShell>) -> Button {
    Button::new(id)
        .primary()
        .label("Open vault")
        .disabled(disabled)
        .on_click(cx.listener(|shell, _: &ClickEvent, window, cx| {
            shell.prompt_for_vault_path(window, cx);
        }))
}

fn lock_button(id: &'static str, cx: &mut Context<AppShell>) -> Button {
    Button::new(id)
        .outline()
        .label("Lock vault")
        .on_click(cx.listener(|shell, _: &ClickEvent, _, cx| {
            shell.state.update(cx, |state, cx| state.lock_vault(cx));
        }))
}

fn nav_item(label: &'static str, selected: bool, cx: &mut Context<AppShell>) -> AnyElement {
    h_flex()
        .h(px(34.))
        .w_full()
        .px_3()
        .rounded(px(6.))
        .text_sm()
        .when(selected, |this| {
            this.bg(cx.theme().sidebar_accent)
                .text_color(cx.theme().sidebar_accent_foreground)
        })
        .when(!selected, |this| {
            this.text_color(cx.theme().sidebar_foreground.opacity(0.76))
        })
        .child(label)
        .into_any_element()
}

fn status_badge(status: &str, cx: &mut Context<AppShell>) -> AnyElement {
    div()
        .px_2()
        .py_1()
        .rounded(px(999.))
        .border_1()
        .border_color(cx.theme().border)
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(status.to_string())
        .into_any_element()
}

fn stat_row(label: &'static str, value: usize, cx: &mut Context<AppShell>) -> AnyElement {
    h_flex()
        .justify_between()
        .text_sm()
        .child(div().text_color(cx.theme().muted_foreground).child(label))
        .child(div().font_medium().child(value.to_string()))
        .into_any_element()
}

fn render_unlock_overlay(
    password_input: &Entity<InputState>,
    prompt: UnlockPrompt,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    div()
        .absolute()
        .top_0()
        .right_0()
        .bottom_0()
        .left_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(cx.theme().background.opacity(0.78))
        .p_6()
        .child(
            v_flex()
                .w(px(440.))
                .max_w_full()
                .gap_4()
                .rounded(px(8.))
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().popover)
                .text_color(cx.theme().popover_foreground)
                .shadow_lg()
                .p_5()
                .child(
                    v_flex()
                        .gap_1()
                        .child(div().text_xl().font_semibold().child("Unlock vault"))
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(prompt.file_name),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground.opacity(0.8))
                                .overflow_hidden()
                                .child(prompt.display_path),
                        ),
                )
                .child(Input::new(password_input).mask_toggle().cleanable(true))
                .when_some(prompt.error, |this, error| {
                    this.child(div().text_sm().text_color(cx.theme().red).child(error))
                })
                .child(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .child(cancel_button(cx))
                        .child(unlock_button(cx)),
                ),
        )
        .into_any_element()
}

fn unlock_button(cx: &mut Context<AppShell>) -> Button {
    Button::new("unlock-vault")
        .primary()
        .label("Unlock")
        .on_click(cx.listener(|shell, _: &ClickEvent, window, cx| {
            shell.submit_password(window, cx);
        }))
}

fn cancel_button(cx: &mut Context<AppShell>) -> Button {
    Button::new("cancel-unlock")
        .outline()
        .label("Cancel")
        .on_click(cx.listener(|shell, _: &ClickEvent, window, cx| {
            shell.cancel_unlock(window, cx);
        }))
}

fn is_kdbx_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("kdbx"))
}
