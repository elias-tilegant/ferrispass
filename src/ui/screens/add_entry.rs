use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, StyledImage as _, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    Sizable as _,
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputContentType},
    slider::Slider,
    v_flex,
};

use crate::app::match_groups;
use crate::domain::VaultGroup;
use crate::ui::app_shell::AppShell;
use crate::ui::icons::AppIcon;
use crate::ui::palette;
use crate::ui::widgets::atoms::label;
use crate::ui::widgets::interaction::Interaction as _;
use crate::ui::widgets::secret_input::SecretInput;

pub fn render(shell: &AppShell, cx: &mut Context<AppShell>) -> AnyElement {
    let underlay = crate::ui::screens::vault::render(shell, cx);
    let modal = modal_card(shell, cx);

    div()
        .size_full()
        .relative()
        .child(underlay)
        .child(
            // Center the modal on both axes; on tall windows it sits ~natural
            // size near the middle, on short ones it caps at viewport height
            // and the form body inside scrolls. The 16 px padding around the
            // edges prevents the chrome from kissing the window border.
            //
            // `occlude()` blocks every mouse interaction - clicks, hover, AND
            // scroll-wheel - from reaching the underlay vault view sitting
            // behind us. Without it, scrolling on the dimmed background (or
            // past a modal scroll boundary) would bubble through to the
            // sidebar / entry list and shift the user's place under them.
            div()
                .absolute()
                .top_0()
                .right_0()
                .bottom_0()
                .left_0()
                .bg(palette::transparent_overlay())
                .occlude()
                .flex()
                .items_center()
                .justify_center()
                .p(px(16.))
                .child(modal),
        )
        .into_any_element()
}

fn modal_card(shell: &AppShell, cx: &mut Context<AppShell>) -> AnyElement {
    let title_input = shell.new_entry_title_input().clone();
    let username_input = shell.new_entry_username_input().clone();
    let password_input = shell.new_entry_password_input().clone();
    let url_input = shell.new_entry_url_input().clone();
    let notes_input = shell.new_entry_notes_input().clone();
    let otp_input = shell.new_entry_otp_input().clone();

    // Are we editing an existing entry, or adding a new one? The overlay
    // variant carries the entry id when in edit mode.
    let editing_id: Option<String> = match shell.state().read(cx).overlay() {
        crate::app::Overlay::EditEntry { entry_id } => Some(entry_id.clone()),
        _ => None,
    };
    let is_edit = editing_id.is_some();

    // Resolve the group the picker shows as chosen. Priority:
    //   1. user's explicit pick from the inline group picker, if any
    //   2. Edit: the group the entry is in; a pick replaces it and the
    //      entry moves there on save
    //   3. Add: the sidebar's currently-selected group, or the vault root
    //      when the user was viewing a tag / library filter with no
    //      specific group context
    let target_group_id = shell
        .new_entry_target_group_id()
        .map(ToOwned::to_owned)
        .or_else(|| {
            let browser = shell.state().read(cx).vault_browser()?;
            match &editing_id {
                Some(id) => browser.snapshot.find_entry_group_id(id),
                None => Some(match browser.selection {
                    crate::app::LibrarySelection::Group(id) => id,
                    _ => browser.snapshot.root.id.clone(),
                }),
            }
        });
    // Snapshot the root group so the inline picker can render the full
    // tree without re-borrowing AppState across the closure boundary
    // when each row's click listener fires. `None` while the vault is
    // closing/locked - the modal won't render in those states anyway,
    // but we still gate the picker on it.
    let root_group: Option<VaultGroup> = shell
        .state()
        .read(cx)
        .vault_browser()
        .map(|b| b.snapshot.root.clone());
    let recycle_bin_id: Option<String> = shell
        .state()
        .read(cx)
        .vault_browser()
        .and_then(|b| b.snapshot.recycle_bin_id.clone());
    let picker_open = shell.new_entry_picker_open();

    let target_group_label = if is_edit && shell.new_entry_target_group_id().is_none() {
        // For edits, show the entry's current parent group instead of where a
        // new entry would land.
        editing_id
            .as_ref()
            .and_then(|id| {
                let state = shell.state().read(cx);
                state
                    .vault_browser()
                    .and_then(|b| b.snapshot.find_entry(id).cloned())
                    .and_then(|e| e.group_path.last().cloned())
            })
            .unwrap_or_else(|| "Vault root".to_string())
    } else {
        target_group_id
            .as_ref()
            .and_then(|id| {
                let state = shell.state().read(cx);
                state
                    .vault_browser()
                    .and_then(|b| b.snapshot.find_group(id).map(|g| g.name.clone()))
            })
            .unwrap_or_else(|| "Vault root".to_string())
    };

    let cancel_button = div()
        .id("add-cancel")
        .h(px(30.))
        .px(px(12.))
        .rounded(px(6.))
        .bg(palette::panel())
        .border_1()
        .border_color(palette::border_strong())
        .text_sm()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(palette::text())
        .flex()
        .items_center()
        .justify_center()
        .hover_press(palette::border())
        .child("Cancel")
        .on_click(
            cx.listener(|shell: &mut AppShell, _: &ClickEvent, window, cx| {
                shell.request_close_entry_form(window, cx);
            }),
        );

    let save_label = if is_edit {
        "Save changes"
    } else {
        "Save entry"
    };
    let save_button = div()
        .id("add-save")
        .h(px(30.))
        .px(px(14.))
        .rounded(px(6.))
        .bg(palette::blue())
        .border_1()
        .border_color(palette::blue_hover())
        .text_sm()
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(palette::panel())
        .flex()
        .items_center()
        .justify_center()
        .gap_1p5()
        .child(
            gpui_component::Icon::from(gpui_component::IconName::Check)
                .with_size(gpui_component::Size::Size(px(13.)))
                .text_color(palette::panel()),
        )
        .child(save_label)
        .hover_press(palette::blue_hover())
        .on_click(
            cx.listener(|shell: &mut AppShell, _: &ClickEvent, window, cx| {
                shell.submit_entry_form(window, cx);
            }),
        );

    let generate_button_el = div()
        .id("add-generate")
        .pressable_dim()
        .child(generate_button_visual())
        .on_click(
            cx.listener(|shell: &mut AppShell, _: &ClickEvent, window, cx| {
                shell.generate_password(window, cx);
            }),
        );

    v_flex()
        .w(px(540.))
        // Cap at viewport height so the modal can't overflow the window on
        // short displays. `min_h(0)` lets the inner flex-1 body actually
        // shrink instead of forcing the modal to grow to its content size.
        .max_h_full()
        .min_h(px(0.))
        .bg(palette::panel())
        .border_1()
        .border_color(palette::border())
        .rounded(px(10.))
        .overflow_hidden()
        .child(
            h_flex()
                .gap_2p5()
                .items_center()
                .px_5()
                .py_4()
                .border_b_1()
                .border_color(palette::border())
                .child(
                    div()
                        .size(px(28.))
                        .rounded(px(6.))
                        .bg(palette::blue_soft())
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            gpui_component::Icon::from(gpui_component::IconName::Plus)
                                .with_size(gpui_component::Size::Size(px(14.)))
                                .text_color(palette::blue()),
                        ),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .gap_0p5()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui::FontWeight::BOLD)
                                .child(if is_edit { "Edit entry" } else { "New entry" }),
                        )
                        .child(target_group_chip(&target_group_label, picker_open, cx)),
                ),
        )
        .when(picker_open, |this| {
            this.child(group_picker_panel(
                shell,
                root_group.as_ref(),
                recycle_bin_id.as_deref(),
                target_group_id.as_deref(),
                cx,
            ))
        })
        .child(
            // Form body claims the remaining vertical space and scrolls
            // internally - header (above) and footer (below) stay pinned so
            // Cancel/Save are always reachable, even on a short window.
            v_flex()
                .id("add-entry-form-scroll")
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .gap_3p5()
                .px_5()
                .py_4()
                .child(
                    v_flex()
                        .gap_2()
                        .child(label("Title"))
                        .child(Input::new(&title_input))
                        // Inline, under the field it is about. A toast for a
                        // missing title floated away from the form and read
                        // like the result of an async operation.
                        .when_some(shell.entry_form_error(), |this, message| {
                            this.child(
                                div()
                                    .text_xs()
                                    .text_color(palette::red())
                                    .child(message.to_string()),
                            )
                        }),
                )
                .child(
                    v_flex()
                        .gap_2()
                        .child(label("Username"))
                        .child(Input::new(&username_input)),
                )
                .child(
                    v_flex()
                        .gap_2()
                        .child(label("Password"))
                        .child(
                            h_flex()
                                .gap_1p5()
                                .child(div().flex_1().child(SecretInput::new(
                                    &password_input,
                                    true,
                                    Some(if is_edit {
                                        InputContentType::Password
                                    } else {
                                        InputContentType::NewPassword
                                    }),
                                )))
                                .child(generate_button_el),
                        )
                        .child(generator_card(shell, cx)),
                )
                .child(
                    v_flex()
                        .gap_2()
                        .child(label("URL"))
                        .child(Input::new(&url_input)),
                )
                .child(
                    v_flex()
                        .gap_2()
                        .child(label("Notes"))
                        .child(Input::new(&notes_input)),
                )
                .child(
                    v_flex()
                        .gap_2()
                        .child(
                            h_flex()
                                .gap_1()
                                .items_baseline()
                                .child(label("2FA / TOTP"))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(palette::text_faint())
                                        .child("(otpauth URL or secret)"),
                                ),
                        )
                        .child(SecretInput::new(
                            &otp_input,
                            true,
                            Some(InputContentType::Password),
                        )),
                )
                .child(custom_fields_editor(shell, cx)),
        )
        .child(
            // Match the parent modal's `rounded(10)` on the bottom edge so
            // the footer's own background paints inside the same curve;
            // otherwise the gray fill peeks past the rounded corners.
            h_flex()
                .gap_2()
                .items_center()
                .px_5()
                .py_3()
                .bg(palette::sidebar())
                .border_t_1()
                .border_color(palette::border())
                .rounded_bl(px(10.))
                .rounded_br(px(10.))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .text_xs()
                        .text_color(if shell.entry_discard_armed() {
                            palette::red()
                        } else {
                            palette::text_muted()
                        })
                        .child(footer_hint(shell, cx)),
                )
                .child(cancel_button)
                .child(save_button),
        )
        .into_any_element()
}

/// Footer line under the form. Normally says where the entry lands; once a
/// discard is armed it says what the next Escape or Cancel will throw away,
/// because that used to happen silently on the first keystroke of Escape.
fn footer_hint(shell: &AppShell, cx: &Context<AppShell>) -> String {
    if shell.entry_discard_armed() {
        return "Discard your changes? Press Escape or Cancel again.".to_string();
    }
    shell
        .state()
        .read(cx)
        .summary()
        .provider
        .map(|provider| format!("Saves locally, syncs to {provider}"))
        .unwrap_or_else(|| "Saves locally to this vault".to_string())
}

/// Dynamic "Additional fields" section. One row per
/// `CustomFieldDraftInputs` in `AppShell::new_entry_custom_fields`,
/// plus a "+ Add field" button at the bottom. Each row has its own
/// pair of `Input` widgets, a lock toggle (the `protected` flag),
/// and a trash button to delete the row.
///
/// KeePass clients (KeePassXC's "Additional attributes") show these
/// 1:1 - the user can pick the same key conventions there.
fn custom_fields_editor(shell: &AppShell, cx: &mut Context<AppShell>) -> AnyElement {
    let rows = shell.new_entry_custom_fields();

    let mut col = v_flex().gap_2().child(
        h_flex()
            .gap_1()
            .items_baseline()
            .child(label("Additional fields"))
            .child(
                div()
                    .text_xs()
                    .text_color(palette::text_faint())
                    .child("(SAP_CONN, SAP_USER, SAP_LANG, …)"),
            ),
    );

    for row in rows {
        let id_for_remove = row.id;
        let id_for_lock = row.id;
        let key_input = row.key_input.clone();
        let value_input = row.value_input.clone();
        let protected = row.protected;
        let row_id_remove = SharedString::from(format!("cf-row-remove-{}", row.id));
        let row_id_lock = SharedString::from(format!("cf-row-lock-{}", row.id));

        col = col.child(
            h_flex()
                .gap_2()
                .items_center()
                .child(div().flex_1().min_w(px(0.)).child(Input::new(&key_input)))
                .child(div().flex_1().min_w(px(0.)).child(SecretInput::new(
                    &value_input,
                    protected,
                    protected.then_some(InputContentType::Password),
                )))
                .child(
                    // Lock toggle: clicking flips the `protected` flag.
                    // When set, the value is stored via `set_protected`
                    // on save - KeePassXC renders such fields with a
                    // `Protected="True"` attribute and masks them.
                    div()
                        .id(row_id_lock)
                        .h(px(28.))
                        .px_2()
                        .rounded(px(6.))
                        .border_1()
                        .border_color(if protected {
                            palette::blue_border()
                        } else {
                            palette::border()
                        })
                        .bg(if protected {
                            palette::blue_soft()
                        } else {
                            palette::sidebar()
                        })
                        .text_color(if protected {
                            palette::blue()
                        } else {
                            palette::text_muted()
                        })
                        .text_xs()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            gpui_component::Icon::from(if protected {
                                AppIcon::Key
                            } else {
                                AppIcon::Note
                            })
                            .with_size(gpui_component::Size::Size(px(12.))),
                        )
                        .hover_press(if protected {
                            palette::blue_border()
                        } else {
                            palette::border()
                        })
                        .on_click(cx.listener(
                            move |shell: &mut AppShell, _: &ClickEvent, _, cx| {
                                shell.toggle_custom_field_protected(id_for_lock, cx);
                            },
                        )),
                )
                .child(
                    div()
                        .id(row_id_remove)
                        .h(px(28.))
                        .px_2()
                        .rounded(px(6.))
                        .border_1()
                        .border_color(palette::border())
                        .bg(palette::sidebar())
                        .text_color(palette::text_muted())
                        .text_xs()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child("✕")
                        .hover(|s| s.text_color(palette::red()))
                        .pressable()
                        .on_click(cx.listener(
                            move |shell: &mut AppShell, _: &ClickEvent, window, cx| {
                                shell.remove_custom_field_row(id_for_remove, window, cx);
                            },
                        )),
                ),
        );
    }

    // Trailing actions row: generic "+ Add field" plus the SAP
    // quick-add. Both are styled like the existing buttons in the
    // modal (small chip, sidebar-tone bg) - the SAP one gets the
    // accent border to nudge users towards the structured path
    // when they want to wire up an SAP connection.
    col.child(
        h_flex()
            .gap_2()
            .child(
                div()
                    .id("cf-row-add")
                    .h(px(28.))
                    .px_3()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(palette::border())
                    .bg(palette::sidebar())
                    .text_color(palette::text_muted())
                    .text_xs()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child("+ Add field")
                    .hover_press(palette::border())
                    .on_click(
                        cx.listener(|shell: &mut AppShell, _: &ClickEvent, window, cx| {
                            shell.add_custom_field_row(window, cx);
                        }),
                    ),
            )
            .child(
                div()
                    .id("cf-row-add-sap")
                    .h(px(28.))
                    .px_3()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(palette::blue_border())
                    .bg(palette::blue_soft())
                    .text_color(palette::blue())
                    .text_xs()
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child("+ Add SAP connection")
                    .hover_press(palette::blue_border())
                    .on_click(
                        cx.listener(|shell: &mut AppShell, _: &ClickEvent, window, cx| {
                            shell.add_sap_connection_template(window, cx);
                        }),
                    ),
            ),
    )
    .into_any_element()
}

/// Live generator card: draggable length slider + four class checkboxes,
/// both wired to `AppShell` state. Strength label and bits update on every
/// drag/toggle (driven by `AppShell::observe(&gen_length_state)` for slider
/// moves and `cx.notify()` inside `toggle_gen_class` for checkbox clicks).
fn generator_card(shell: &AppShell, cx: &mut Context<AppShell>) -> AnyElement {
    let length = shell.gen_length(cx);
    let classes = shell.gen_classes();
    let bits = crate::keepass::password_gen::estimate_bits(length, classes);
    let strength = crate::keepass::password_gen::strength_from_bits(bits);
    let strength_label = strength.label();
    let strength_color = match strength {
        crate::domain::Strength::Weak => palette::red(),
        crate::domain::Strength::Fair => palette::yellow(),
        crate::domain::Strength::Strong => palette::green(),
    };

    let class_row = {
        let entries = [
            ("gen-class-upper", "A-Z", classes.upper, 0usize),
            ("gen-class-lower", "a-z", classes.lower, 1),
            ("gen-class-digits", "0-9", classes.digits, 2),
            ("gen-class-symbols", "!@#", classes.symbols, 3),
        ];
        let mut row = h_flex().gap_3p5();
        for (id, label_text, checked, idx) in entries {
            row = row.child(
                Checkbox::new(id)
                    .checked(checked)
                    .label(label_text)
                    .on_click(cx.listener(move |shell: &mut AppShell, _: &bool, _, cx| {
                        shell.toggle_gen_class(idx, cx);
                    })),
            );
        }
        row
    };

    v_flex()
        .gap_3()
        .p_3()
        .rounded(px(6.))
        .bg(palette::sidebar())
        .border_1()
        .border_color(palette::border())
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .text_xs()
                .text_color(palette::text_muted())
                .child(
                    h_flex().gap_1().child("Length:").child(
                        div()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(palette::text())
                            .child(length.to_string()),
                    ),
                )
                .child(
                    div()
                        .text_color(strength_color)
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(format!("{strength_label} · {bits} bits")),
                ),
        )
        .child(Slider::new(shell.gen_length_state()))
        .child(class_row)
        .into_any_element()
}

fn generate_button_visual() -> AnyElement {
    h_flex()
        .h(px(32.))
        .px(px(12.))
        .gap_1p5()
        .items_center()
        .rounded(px(6.))
        .bg(palette::orange())
        .border_1()
        .border_color(palette::orange_deep())
        .text_color(palette::panel())
        .text_sm()
        .font_weight(gpui::FontWeight::MEDIUM)
        .child(
            gpui_component::Icon::from(AppIcon::Refresh)
                .with_size(gpui_component::Size::Size(px(12.)))
                .text_color(palette::panel()),
        )
        .child("Generate")
        .into_any_element()
}

/// "in {Group} ▾" chip below the header. Toggles the inline group picker,
/// in Add mode to choose where the entry lands and in Edit mode to move it.
fn target_group_chip(label: &str, picker_open: bool, cx: &mut Context<AppShell>) -> AnyElement {
    let text: SharedString = format!("in {label}").into();
    h_flex()
        .id("add-entry-group-chip")
        .gap_1()
        .items_center()
        .rounded(px(4.))
        .px(px(4.))
        .text_xs()
        .text_color(palette::text_muted())
        .hover(|s| s.bg(palette::sidebar()).text_color(palette::text()))
        .pressable()
        .on_click(
            cx.listener(|shell: &mut AppShell, _: &ClickEvent, window, cx| {
                shell.toggle_new_entry_picker(window, cx);
            }),
        )
        .child(text)
        .child(
            gpui_component::Icon::from(if picker_open {
                gpui_component::IconName::ChevronUp
            } else {
                gpui_component::IconName::ChevronDown
            })
            .with_size(gpui_component::Size::Size(px(10.))),
        )
        .into_any_element()
}

/// Inline group-picker panel rendered below the header when expanded.
/// A search field on top filters the groups by name or path; without a
/// query the panel lists every group as an indented preorder tree (same
/// shape as the sidebar walk in `screens::vault::groups_section`). Click a
/// row, or press Enter for the first hit, and it becomes the target.
fn group_picker_panel(
    shell: &AppShell,
    root: Option<&VaultGroup>,
    recycle_bin_id: Option<&str>,
    selected_id: Option<&str>,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let filter_input = shell.new_entry_group_filter_input().clone();
    let query = filter_input.read(cx).value().to_string();
    let searching = !query.trim().is_empty();
    let rows = root
        .map(|root| match_groups(root, recycle_bin_id, &query))
        .unwrap_or_default();

    let selected_owned = selected_id.map(ToOwned::to_owned);
    let mut col = v_flex();
    if rows.is_empty() {
        col = col.child(
            div()
                .px(px(8.))
                .py(px(6.))
                .text_sm()
                .text_color(palette::text_muted())
                .child("No group matches."),
        );
    }
    for (idx, row) in rows.iter().enumerate() {
        let group = row.group;
        // While searching the list is flat: indentation would suggest a
        // nesting the filtered rows no longer have. The path says it instead.
        let indent = if searching { 0 } else { row.depth };
        let group_id = group.id.clone();
        let is_selected = selected_owned.as_deref() == Some(group.id.as_str());
        // Enter takes the first hit; mark it so the key has a visible target.
        let is_enter_target = searching && idx == 0;
        let row_id: SharedString = format!("add-entry-group-row-{idx}").into();
        let name: SharedString = group.name.clone().into();
        let path: Option<SharedString> =
            (searching && !row.parents.is_empty()).then(|| row.parents.join(" / ").into());
        col = col.child(
            h_flex()
                .id(row_id)
                .h(px(26.))
                .pl(px(8. + indent as f32 * 14.))
                .pr(px(10.))
                .items_center()
                .gap_2()
                .rounded(px(4.))
                .bg(if is_selected {
                    palette::blue_soft()
                } else if is_enter_target {
                    palette::panel()
                } else {
                    gpui::transparent_black()
                })
                .text_color(if is_selected {
                    palette::blue()
                } else {
                    palette::text()
                })
                .text_sm()
                .when(!is_selected, |this| this.hover(|s| s.bg(palette::panel())))
                .pressable()
                .on_click(
                    cx.listener(move |shell: &mut AppShell, _: &ClickEvent, window, cx| {
                        shell.set_new_entry_target_group(group_id.clone(), window, cx);
                    }),
                )
                .child(picker_row_icon(group, is_selected))
                .child(div().flex_shrink_0().max_w(px(240.)).truncate().child(name))
                .when_some(path, |this, path| {
                    this.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(palette::text_muted())
                            .child(path),
                    )
                })
                .when(is_enter_target, |this| {
                    this.child(
                        div()
                            .ml_auto()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(palette::text_faint())
                            .child("↩"),
                    )
                }),
        );
    }

    v_flex()
        .border_b_1()
        .border_color(palette::border())
        .bg(palette::sidebar())
        .child(
            div()
                .px_3()
                .pt_2()
                .child(Input::new(&filter_input).small().cleanable(true)),
        )
        .child(
            div()
                .id("add-entry-group-picker")
                .max_h(px(240.))
                .overflow_y_scroll()
                .px_3()
                .py_2()
                .child(col),
        )
        .into_any_element()
}

/// Leading icon for a row in the AddEntry group picker. Custom-icon
/// image (when the group has `Icon::Custom(_)`) takes priority over
/// the generic note glyph; image rendering uses `with_fallback` so a
/// corrupt blob silently falls back to the glyph instead of leaving
/// an empty slot.
fn picker_row_icon(group: &VaultGroup, is_selected: bool) -> AnyElement {
    let glyph_color = if is_selected {
        palette::blue()
    } else {
        palette::text_muted()
    };
    if let Some(image) = group.icon.as_ref() {
        return div()
            .id(SharedString::from(format!("picker-row-icon-{}", group.id)))
            .size(px(12.))
            .rounded(px(3.))
            .overflow_hidden()
            .child(
                gpui::img(image.0.clone())
                    .object_fit(gpui::ObjectFit::Cover)
                    .size(px(12.))
                    .with_fallback(move || {
                        gpui_component::Icon::from(AppIcon::Note)
                            .with_size(gpui_component::Size::Size(px(12.)))
                            .text_color(glyph_color)
                            .into_any_element()
                    }),
            )
            .into_any_element();
    }
    gpui_component::Icon::from(AppIcon::Note)
        .with_size(gpui_component::Size::Size(px(12.)))
        .text_color(glyph_color)
        .into_any_element()
}
