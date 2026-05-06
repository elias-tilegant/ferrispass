use gpui::{
    AnyElement, ClickEvent, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, div, prelude::FluentBuilder as _,
    px,
};
use gpui_component::{
    Sizable as _, WindowExt as _, checkbox::Checkbox, h_flex, input::Input, slider::Slider, v_flex,
};

use crate::domain::VaultGroup;
use crate::ui::app_shell::AppShell;
use crate::ui::icons::AppIcon;
use crate::ui::palette;
use crate::ui::widgets::atoms::label;

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
            // `occlude()` blocks every mouse interaction — clicks, hover, AND
            // scroll-wheel — from reaching the underlay vault view sitting
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

    // Resolve the destination group (only relevant for Add). Priority:
    //   1. user's explicit pick from the inline group picker, if any
    //   2. the sidebar's currently-selected group
    //   3. vault root (when the user was viewing a tag / library filter
    //      with no specific group context)
    let target_group_id = {
        let state = shell.state().read(cx);
        shell
            .new_entry_target_group_id()
            .map(ToOwned::to_owned)
            .or_else(|| {
                state.vault_browser().and_then(|b| match b.selection {
                    crate::app::LibrarySelection::Group(id) => Some(id),
                    _ => Some(b.snapshot.root.id.clone()),
                })
            })
    };
    // Snapshot the root group so the inline picker can render the full
    // tree without re-borrowing AppState across the closure boundary
    // when each row's click listener fires. `None` while the vault is
    // closing/locked — the modal won't render in those states anyway,
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
    let picker_open = shell.new_entry_picker_open() && !is_edit;

    let target_group_label = if is_edit {
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
        .child("Cancel")
        .on_click(
            cx.listener(|shell: &mut AppShell, _: &ClickEvent, window, cx| {
                shell.clear_entry_form(window, cx);
                shell.state().clone().update(cx, |state, cx| {
                    let _ = state.close_overlay(cx);
                });
            }),
        );

    let save_target_group_id = target_group_id.clone();
    let save_editing_id = editing_id.clone();
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
        .on_click(
            cx.listener(move |shell: &mut AppShell, _: &ClickEvent, window, cx| {
                let draft = shell.collect_entry_draft(cx);
                if draft.title.trim().is_empty() {
                    window.push_notification("Title is required to save the entry.", cx);
                    return;
                }
                let state = shell.state().clone();
                let result: Result<(), String> = match save_editing_id.as_ref() {
                    Some(entry_id) => state
                        .update(cx, |state, cx| state.update_entry(entry_id, draft, cx))
                        .map_err(|e| e.to_string()),
                    None => {
                        let Some(group_id) = save_target_group_id.clone() else {
                            window.push_notification("No destination group is selected.", cx);
                            return;
                        };
                        state
                            .update(cx, |state, cx| state.create_entry(&group_id, draft, cx))
                            .map(|_id| ())
                            .map_err(|e| e.to_string())
                    }
                };
                match result {
                    Ok(()) => {
                        shell.clear_entry_form(window, cx);
                        shell.state().clone().update(cx, |state, cx| {
                            let _ = state.close_overlay(cx);
                        });
                        let toast = if save_editing_id.is_some() {
                            "Changes saved."
                        } else {
                            "Entry saved."
                        };
                        window.push_notification(toast, cx);
                    }
                    Err(error) => {
                        window.push_notification(format!("Could not save entry: {error}"), cx);
                    }
                }
            }),
        );

    let generate_button_el = div()
        .id("add-generate")
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
                        .child(target_group_chip(
                            &target_group_label,
                            picker_open,
                            is_edit,
                            cx,
                        )),
                ),
        )
        .when(picker_open, |this| {
            this.child(group_picker_panel(
                root_group.as_ref(),
                recycle_bin_id.as_deref(),
                target_group_id.as_deref(),
                cx,
            ))
        })
        .child(
            // Form body claims the remaining vertical space and scrolls
            // internally — header (above) and footer (below) stay pinned so
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
                        .child(Input::new(&title_input)),
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
                                .child(div().flex_1().child(Input::new(&password_input)))
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
                        .child(Input::new(&otp_input)),
                ),
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
                        .text_xs()
                        .text_color(palette::text_muted())
                        .font_family("JetBrains Mono")
                        .child("Saves locally, syncs to OneDrive"),
                )
                .child(cancel_button)
                .child(save_button),
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

/// "in {Group} ▾" chip below the header. Clickable in Add mode (toggles
/// the inline group picker); rendered as plain muted text in Edit mode
/// because the entry's parent group can't be moved from this modal.
fn target_group_chip(
    label: &str,
    picker_open: bool,
    is_edit: bool,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let text: SharedString = format!("in {label}").into();
    if is_edit {
        return div()
            .text_xs()
            .text_color(palette::text_muted())
            .child(text)
            .into_any_element();
    }
    h_flex()
        .id("add-entry-group-chip")
        .gap_1()
        .items_center()
        .rounded(px(4.))
        .px(px(4.))
        .text_xs()
        .text_color(palette::text_muted())
        .hover(|s| s.bg(palette::sidebar()).text_color(palette::text()))
        .on_click(cx.listener(|shell: &mut AppShell, _: &ClickEvent, _, cx| {
            shell.toggle_new_entry_picker(cx);
        }))
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
/// Lists every group in the vault as an indented preorder tree (same
/// shape as the sidebar walk in `screens::vault::groups_section`); the
/// recycle bin is filtered out because creating a new entry there would
/// be confusing. Click a row → it becomes the new target.
fn group_picker_panel(
    root: Option<&VaultGroup>,
    recycle_bin_id: Option<&str>,
    selected_id: Option<&str>,
    cx: &mut Context<AppShell>,
) -> AnyElement {
    let mut flat: Vec<(usize, &VaultGroup)> = Vec::new();
    fn collect<'a>(
        group: &'a VaultGroup,
        depth: usize,
        recycle_bin_id: Option<&str>,
        out: &mut Vec<(usize, &'a VaultGroup)>,
    ) {
        if recycle_bin_id == Some(group.id.as_str()) {
            return;
        }
        out.push((depth, group));
        // Picker always shows the *whole* tree regardless of the
        // sidebar's collapse state — the user has to be able to pick
        // any group, even one they happen to have folded shut.
        for child in &group.groups {
            collect(child, depth + 1, recycle_bin_id, out);
        }
    }
    if let Some(root) = root {
        collect(root, 0, recycle_bin_id, &mut flat);
    }

    let selected_owned = selected_id.map(ToOwned::to_owned);
    let mut col = v_flex();
    for (idx, (depth, group)) in flat.iter().enumerate() {
        let depth = *depth;
        let group_id = group.id.clone();
        let is_selected = selected_owned.as_deref() == Some(group.id.as_str());
        let row_id: SharedString = format!("add-entry-group-row-{idx}").into();
        let name: SharedString = group.name.clone().into();
        col = col.child(
            h_flex()
                .id(row_id)
                .h(px(26.))
                .pl(px(8. + depth as f32 * 14.))
                .pr(px(10.))
                .items_center()
                .gap_2()
                .rounded(px(4.))
                .bg(if is_selected {
                    palette::blue_soft()
                } else {
                    gpui::transparent_black()
                })
                .text_color(if is_selected {
                    palette::blue()
                } else {
                    palette::text()
                })
                .text_sm()
                .when(!is_selected, |this| {
                    this.hover(|s| s.bg(palette::sidebar()))
                })
                .on_click(
                    cx.listener(move |shell: &mut AppShell, _: &ClickEvent, _, cx| {
                        shell.set_new_entry_target_group(group_id.clone(), cx);
                    }),
                )
                .child(
                    gpui_component::Icon::from(AppIcon::Note)
                        .with_size(gpui_component::Size::Size(px(12.)))
                        .text_color(if is_selected {
                            palette::blue()
                        } else {
                            palette::text_muted()
                        }),
                )
                .child(div().flex_1().min_w_0().truncate().child(name)),
        );
    }

    div()
        .id("add-entry-group-picker")
        .max_h(px(200.))
        .overflow_y_scroll()
        .px_3()
        .py_2()
        .border_b_1()
        .border_color(palette::border())
        .bg(palette::sidebar())
        .child(col)
        .into_any_element()
}
