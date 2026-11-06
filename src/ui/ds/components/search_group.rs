//! The search group: a search field over a result list, under one node.
//!
//! The quick panel, the command palette and the main window each show a
//! search field over a list of results. Keyboard focus stays in the field,
//! so typing goes on while ↑ and ↓ move the selection, and the selected row
//! tells GPUI that it is the active descendant, the element VoiceOver
//! treats as focused. GPUI acts on that only while an ancestor of the row
//! has focus, and the field stands next to the list, not around it.
//!
//! So the group is a node around both, a group with a name, and tracks the
//! focus handle of the field's GPUI Kit input. While the field has focus,
//! the group is the focused node of the tree, an ancestor of every row, and
//! the selected row's claim holds. The Kit input keeps its own node inside
//! the group. The Kit keeps keyboard focus on an inner element of the input
//! that has no node, so without the group no node of the tree has focus
//! while someone types in the field.
//!
//! GPUI puts a focus handle into the tab order once for every element that
//! tracks it, and the Kit input tracks the field's handle too. The group's
//! copy of the handle is no tab stop, so Tab and Shift-Tab reach the field
//! once, as without the group.

use gpui::{
    App, Div, ElementId, FocusHandle, InteractiveElement as _, IntoElement, ParentElement as _,
    RenderOnce, Role, SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, Window,
    div, prelude::FluentBuilder as _, px,
};

use super::{ResultList, TextField};
use crate::ui::ds::tokens::dimension;
use crate::ui::ds::typography::u;

/// A search field and its result list, under one node.
#[derive(IntoElement)]
pub struct SearchGroup {
    id: ElementId,
    name: SharedString,
    description: Option<SharedString>,
    field: TextField,
    list: ResultList,
}

impl SearchGroup {
    /// `name` is what VoiceOver calls the group, such as "Search" from a
    /// text key. Panics if it is empty, in every build.
    pub fn new(
        id: impl Into<ElementId>,
        name: impl Into<SharedString>,
        field: TextField,
        list: ResultList,
    ) -> Self {
        let name = name.into();
        assert!(!name.is_empty(), "a search group needs a name");
        Self {
            id: id.into(),
            name,
            description: None,
            field,
            list,
        }
    }

    /// What VoiceOver reads after the group's name, such as the panel
    /// footer's sentence while the list is empty and no row can carry it.
    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }
}

/// The group's own node: a group with its name and description that tracks
/// `focus`, the field's handle, as no tab stop of its own.
///
/// A clone of a focus handle takes whether it is a tab stop from what GPUI
/// keeps for the handle, not from the handle it was cloned from, and
/// `track_focus` keeps a clone. So the tab stop is turned off while the group
/// takes its clone, and turned back to what it was right after, before the
/// Kit input clones the handle for itself.
fn group(
    id: ElementId,
    name: SharedString,
    description: Option<SharedString>,
    focus: &FocusHandle,
) -> Stateful<Div> {
    let tab_stop = focus.tab_stop;
    let off = focus.clone().tab_stop(false);
    let group = div()
        .id(id)
        .role(Role::Group)
        .aria_label(name)
        .when_some(description, |this, text| this.aria_description(text))
        .track_focus(&off);
    let _ = off.tab_stop(tab_stop);
    group
}

impl RenderOnce for SearchGroup {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus = self.field.focus_handle(cx);
        group(self.id, self.name, self.description, &focus)
            .flex()
            .flex_col()
            .gap(u(dimension::SPACE_8))
            .size_full()
            .child(self.field)
            .child(div().flex_1().min_h(px(0.)).child(self.list))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use gpui::{
        AppContext as _, Entity, Focusable as _, Modifiers, TestAppContext, VisualTestContext,
        accesskit,
    };
    use gpui_component::IconName;
    use gpui_component::input::InputState;

    use super::super::{Button, Row, RowParts, Tile, VaultBadge, probe};
    use super::*;

    fn text(value: &'static str) -> SharedString {
        SharedString::new_static(value)
    }

    /// A list of `count` rows; the selected row shows a ghost button with
    /// its verb when `verbs` is set, as in the panel.
    fn results(count: usize, verbs: bool) -> ResultList {
        ResultList::new("results", text("Results"), count, move |index, _, _| {
            let row = Row::new(
                ("result", index),
                index,
                count,
                RowParts::new(text("Web login"), text("Salesforce")).vault(text("Personal")),
                Tile::new(IconName::Globe),
                VaultBadge::Personal,
            );
            if verbs {
                row.verb(Button::ghost(("open", index), text("Open")))
            } else {
                row
            }
        })
    }

    /// The group is a node of its own: a group with its name, which
    /// assistive technology can focus, and the description it is given.
    #[gpui::test]
    fn the_group_is_a_named_node_that_takes_focus(cx: &mut TestAppContext) {
        let read = probe::in_frame(cx, |window, cx| {
            let state = cx.new(|cx| InputState::new(window, cx));
            let field = TextField::new("query", &state, text("Search"));
            let plain = SearchGroup::new("search", text("Search"), field, results(2, false));
            let element = RenderOnce::render(plain, window, cx).into_element();
            let (role, node) = probe::node_of(&element);
            let id = probe::element_id(element);

            let field = TextField::new("query", &state, text("Search"));
            let described = SearchGroup::new("search", text("Search"), field, results(0, false))
                .description(text("No matches. Return: search the web."));
            let (_, described) = probe::node(RenderOnce::render(described, window, cx));
            (role, node, id, described)
        });
        let (role, node, id, described) = read;
        assert_eq!(role, Some(Role::Group));
        assert_eq!(
            id,
            Some(ElementId::from("search")),
            "an id, so it has a node"
        );
        assert_eq!(node.label(), Some("Search"));
        assert!(
            node.supports_action(accesskit::Action::Focus),
            "it takes focus"
        );
        assert_eq!(node.description(), None);
        assert_eq!(node.value(), None, "the group repeats nothing of the field");
        assert_eq!(
            described.description(),
            Some("No matches. Return: search the web.")
        );
    }

    #[gpui::test]
    #[should_panic(expected = "a search group needs a name")]
    fn a_search_group_needs_a_name_in_every_build(cx: &mut TestAppContext) {
        probe::in_frame(cx, |window, cx| {
            let state = cx.new(|cx| InputState::new(window, cx));
            let field = TextField::new("query", &state, text("Search"));
            let _ = SearchGroup::new("search", SharedString::default(), field, results(1, false));
        });
    }

    /// What a test window shows: a box that takes focus, the search field
    /// over a list of three rows, in a group or in a plain box, and another
    /// box that takes focus. The input's state and the two boxes' handles
    /// live as long as the window.
    #[derive(Clone)]
    struct Harness {
        state: Entity<InputState>,
        before: FocusHandle,
        after: FocusHandle,
    }

    fn harness(
        cx: &mut TestAppContext,
        grouped: bool,
        verbs: bool,
        selected: Rc<Cell<Option<usize>>>,
    ) -> (&mut VisualTestContext, Harness) {
        let kept: Rc<RefCell<Option<Harness>>> = Rc::default();
        let made = kept.clone();
        let cx = probe::window(cx, move |window, cx| {
            let mut made = made.borrow_mut();
            let parts = made.get_or_insert_with(|| Harness {
                state: cx.new(|cx| InputState::new(window, cx)),
                before: cx.focus_handle().tab_stop(true),
                after: cx.focus_handle().tab_stop(true),
            });
            let field = TextField::new("query", &parts.state, text("Search"));
            let list = results(3, verbs).selected(selected.get());
            let middle = if grouped {
                SearchGroup::new("search", text("Search"), field, list).into_any_element()
            } else {
                div()
                    .flex()
                    .flex_col()
                    .size_full()
                    .child(field)
                    .child(div().flex_1().min_h(px(0.)).child(list))
                    .into_any_element()
            };
            div()
                .flex()
                .flex_col()
                .child(div().id("before").track_focus(&parts.before).h(px(20.)))
                .child(div().h(px(240.)).child(middle))
                .child(div().id("after").track_focus(&parts.after).h(px(20.)))
                .into_any_element()
        });
        let harness = kept.borrow().clone().expect("the window was drawn");
        (cx, harness)
    }

    fn redraw(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    /// What has focus: the box before, the field, the box after, or
    /// something else, which here is the verb of the selected row.
    fn focused(cx: &mut VisualTestContext, harness: &Harness) -> &'static str {
        cx.update(|window, cx| {
            let field = harness.state.focus_handle(cx);
            match window.focused(cx) {
                Some(handle) if handle == harness.before => "before",
                Some(handle) if handle == field => "field",
                Some(handle) if handle == harness.after => "after",
                Some(_) => "verb",
                None => "nothing",
            }
        })
    }

    /// One walk through the window: focus on the box before, then `keys`,
    /// typing a letter whenever the field has focus. For every key it notes
    /// where focus went and whether GPUI reported focus in or focus out for
    /// the field's handle, and the text in the field at the end.
    fn walk(
        cx: &mut TestAppContext,
        grouped: bool,
        verbs: bool,
        keys: &[&str],
    ) -> (Vec<(&'static str, bool, bool)>, SharedString) {
        let (cx, harness) = harness(cx, grouped, verbs, Rc::new(Cell::new(Some(0))));
        // GPUI reports focus in and out only in the active window.
        cx.update(|window, _| window.activate_window());
        cx.run_until_parked();
        let events = Rc::new(RefCell::new((false, false)));
        let (went_in, went_out) = (events.clone(), events.clone());
        let _listeners = cx.update(|window, cx| {
            let field = harness.state.focus_handle(cx);
            (
                window.on_focus_in(&field, cx, move |_, _| went_in.borrow_mut().0 = true),
                window.on_focus_out(&field, cx, move |_, _, _| went_out.borrow_mut().1 = true),
            )
        });
        cx.update(|window, cx| window.focus(&harness.before, cx));
        redraw(cx);
        *events.borrow_mut() = (false, false);
        let mut steps = Vec::new();
        for key in keys {
            cx.simulate_keystrokes(key);
            redraw(cx);
            let stop = focused(cx, &harness);
            if stop == "field" {
                cx.simulate_input("n");
            }
            let (focus_in, focus_out) = std::mem::take(&mut *events.borrow_mut());
            steps.push((stop, focus_in, focus_out));
        }
        let typed = cx.update(|_, cx| harness.state.read(cx).value());
        (steps, typed)
    }

    /// With and without the group, Tab takes focus from the box before to
    /// the field and on to the box after, Shift-Tab takes it back the same
    /// way, the field takes what is typed, and GPUI reports focus in and
    /// focus out for the field's handle on the same steps. The group tracks
    /// the field's handle too, but is no tab stop of its own, so Shift-Tab
    /// leaves the field instead of finding the same handle again.
    #[gpui::test]
    fn typing_and_tab_work_as_without_the_group(cx: &mut TestAppContext) {
        let keys = ["tab", "tab", "shift-tab", "shift-tab"];
        let (with, typed_with) = walk(cx, true, false, &keys);
        let (without, typed_without) = walk(cx, false, false, &keys);
        assert_eq!(with, without, "the steps with the group and without it");
        assert_eq!(
            with,
            [
                ("field", true, false),
                ("after", false, true),
                ("field", true, false),
                ("before", false, true),
            ]
        );
        assert_eq!((typed_with.as_ref(), typed_without.as_ref()), ("nn", "nn"));
    }

    /// With a ghost button on the selected row, Tab and Shift-Tab still
    /// stop at the same places in the same order with the group as without
    /// it, and the field takes what is typed. What changes is focus in and
    /// focus out for the field's handle between the field and the button:
    /// the group tracks the handle and holds the button, so GPUI counts the
    /// button as inside the field's handle and reports focus out only when
    /// focus leaves the group. Focus and blur of the handle itself, which
    /// the Kit input listens to, follow the stops, which are the same.
    #[gpui::test]
    fn a_control_in_the_list_is_inside_the_fields_handle(cx: &mut TestAppContext) {
        let keys = ["tab", "tab", "tab", "shift-tab", "shift-tab", "shift-tab"];
        let (with, typed_with) = walk(cx, true, true, &keys);
        let (without, typed_without) = walk(cx, false, true, &keys);
        let stops = |steps: &[(&'static str, bool, bool)]| -> Vec<&'static str> {
            steps.iter().map(|(stop, ..)| *stop).collect()
        };
        assert_eq!(stops(&with), stops(&without));
        assert_eq!(
            stops(&with),
            ["field", "verb", "after", "verb", "field", "before"]
        );
        assert_eq!(
            without,
            [
                ("field", true, false),
                ("verb", false, true),
                ("after", false, false),
                ("verb", false, false),
                ("field", true, false),
                ("before", false, true),
            ]
        );
        assert_eq!(
            with,
            [
                ("field", true, false),
                ("verb", false, false),
                ("after", false, true),
                ("verb", true, false),
                ("field", false, false),
                ("before", false, true),
            ]
        );
        assert_eq!((typed_with.as_ref(), typed_without.as_ref()), ("nn", "nn"));
    }

    /// A click in the group that no control inside it takes for itself,
    /// here on a row, puts focus into the field: the group tracks the
    /// field's handle. Without the group the same click leaves focus where
    /// it was.
    #[gpui::test]
    fn the_group_tracks_the_fields_focus_handle(cx: &mut TestAppContext) {
        for grouped in [true, false] {
            let (cx, harness) = harness(cx, grouped, false, Rc::new(Cell::new(Some(0))));
            cx.update(|window, cx| window.focus(&harness.before, cx));
            redraw(cx);
            let row = cx.debug_bounds("fp-row").expect("a row").center();
            cx.simulate_click(row, Modifiers::default());
            redraw(cx);
            let expected = if grouped { "field" } else { "before" };
            assert_eq!(focused(cx, &harness), expected, "grouped: {grouped}");
        }
    }

    /// While the field keeps keyboard focus, moving the selection changes
    /// which row is selected and makes the active-descendant claim, and
    /// typing still goes into the field. Whether VoiceOver then speaks the
    /// row needs a device: the test platform never turns accessibility on.
    #[gpui::test]
    fn the_selected_row_is_the_active_descendant_and_the_focus_stays_in_the_search_field(
        cx: &mut TestAppContext,
    ) {
        let selected = Rc::new(Cell::new(Some(0)));
        let (cx, harness) = harness(cx, true, false, selected.clone());
        cx.update(|window, cx| {
            let field = harness.state.focus_handle(cx);
            window.focus(&field, cx);
        });
        redraw(cx);
        for (step, index) in [0, 1, 2, 1].into_iter().enumerate() {
            selected.set(Some(index));
            redraw(cx);
            cx.simulate_input("x");
            redraw(cx);
            assert_eq!(focused(cx, &harness), "field", "step {step}");
            let rows = probe::in_frame(cx, move |window, cx| {
                let list = results(3, false).selected(Some(index));
                (0..3)
                    .map(|row| {
                        let row = list.row_at(row, window, cx);
                        let claims = row.claims_active_descendant();
                        let node = probe::node(RenderOnce::render(row, window, cx)).1;
                        (node.is_selected(), claims)
                    })
                    .collect::<Vec<_>>()
            });
            let expected: Vec<_> = (0..3)
                .map(|row| (Some(row == index), row == index))
                .collect();
            assert_eq!(rows, expected, "step {step}: selected and claiming");
        }
        let typed = cx.update(|_, cx| harness.state.read(cx).value());
        assert_eq!(typed.as_ref(), "xxxx");
    }
}
