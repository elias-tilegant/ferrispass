//! The result list: the rows that match a search, the same in the quick
//! panel, the command palette and the main window's list.
//!
//! The rows sit in GPUI's `uniform_list`, which builds only the rows in view
//! and measures the first to place the others, so all rows of a list are as
//! high as the first. `uniform_list` writes no accessibility node of its
//! own, so the list wraps it in a box that does: a list box named with the
//! name the caller gives. The rows are options inside it.
//!
//! The list draws the selection it is given and places every row itself,
//! so a row's place, the size of the list and whether it is selected always
//! agree with the list. Moving the selection is for the screen that binds
//! the keys, ↑ and ↓ and ⌘1 to ⌘9, through [`move_selection`] and
//! [`select_by_number`], which never wrap around and never go past the end.
//! When the selection changes, the list scrolls the selected row into view.
//!
//! The list also notes when it first built rows for a frame, in
//! [`ResultsDrawn`], for the panel, which holds Return back until the
//! results have been on screen for a moment.

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;
use std::time::Instant;

use gpui::{
    App, ElementId, InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce, Role,
    ScrollStrategy, SharedString, StatefulInteractiveElement as _, Styled as _, UniformList,
    UniformListScrollHandle, Window, div, uniform_list,
};

use super::Row;

/// Which row of a list is selected, if any, in a list of how many rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListSelection {
    index: Option<usize>,
    len: usize,
}

impl ListSelection {
    /// The row at `index` in a list of `len` rows. An index past the end
    /// selects the last row, and an empty list has no selection.
    pub fn new(index: Option<usize>, len: usize) -> Self {
        let index = match len {
            0 => None,
            len => index.map(|index| index.min(len - 1)),
        };
        Self { index, len }
    }

    /// The first row of a list of `len` rows, or none in an empty list:
    /// the selection of results that just arrived.
    pub fn first(len: usize) -> Self {
        Self::new(Some(0), len)
    }

    pub fn index(self) -> Option<usize> {
        self.index
    }

    pub fn len(self) -> usize {
        self.len
    }

    pub fn is_empty(self) -> bool {
        self.len == 0
    }
}

/// The selection `delta` rows further down, or up for a negative `delta`.
/// It stops at the first and at the last row and never wraps around: ↑ on
/// the first row keeps it, and ↓ on the last keeps that. With no row
/// selected, a move down counts from above the first row, so ↓ selects the
/// first, and a move up leaves nothing selected. An empty list keeps no
/// selection.
pub fn move_selection(selection: ListSelection, delta: isize) -> ListSelection {
    let ListSelection { index, len } = selection;
    let moved = match index {
        _ if len == 0 => return selection,
        Some(index) => index.saturating_add_signed(delta),
        None if delta > 0 => delta.unsigned_abs() - 1,
        None => return selection,
    };
    ListSelection::new(Some(moved.min(len - 1)), len)
}

/// The selection after ⌘ and `number`: 1 to 9 select the first nine rows.
/// A number past the end of the list, or outside 1 to 9, leaves the
/// selection as it was.
pub fn select_by_number(selection: ListSelection, number: u8) -> ListSelection {
    match usize::from(number).checked_sub(1) {
        Some(index) if number <= 9 && index < selection.len => {
            ListSelection::new(Some(index), selection.len)
        }
        _ => selection,
    }
}

/// When a result list first built rows for a frame, on GPUI's clock.
///
/// The panel keeps one for the results it shows and clears it when they
/// change, and holds Return back until the results have been on screen for
/// a moment. Only the first frame counts: a list drawn again because the
/// pointer moved or a one-time code ticked leaves the time as it was, so
/// Return is not held back for ever.
#[derive(Clone, Debug, Default)]
pub struct ResultsDrawn(Rc<Cell<Option<Instant>>>);

impl ResultsDrawn {
    pub fn new() -> Self {
        Self::default()
    }

    /// When the list first built rows for a frame since this was made or
    /// last cleared, or `None` if it has not yet.
    pub fn at(&self) -> Option<Instant> {
        self.0.get()
    }

    /// Forgets the time, for results that have changed.
    pub fn clear(&self) {
        self.0.set(None);
    }

    fn note(&self, now: Instant) {
        if self.0.get().is_none() {
            self.0.set(Some(now));
        }
    }
}

type RenderRow = Rc<dyn Fn(usize, &mut Window, &mut App) -> Row>;

/// A list of results, each one a [`Row`].
#[derive(IntoElement)]
pub struct ResultList {
    id: ElementId,
    name: SharedString,
    count: usize,
    render_row: RenderRow,
    selected: Option<usize>,
    drawn: Option<ResultsDrawn>,
}

impl ResultList {
    /// A list of `count` rows. `name` is what VoiceOver calls the list, such
    /// as "Results" from a text key. `render_row` builds the row at an index,
    /// counted from 0; the list asks only for the rows in view. Panics if
    /// the name is empty, in every build.
    pub fn new(
        id: impl Into<ElementId>,
        name: impl Into<SharedString>,
        count: usize,
        render_row: impl Fn(usize, &mut Window, &mut App) -> Row + 'static,
    ) -> Self {
        let name = name.into();
        assert!(!name.is_empty(), "a result list needs a name");
        Self {
            id: id.into(),
            name,
            count,
            render_row: Rc::new(render_row),
            selected: None,
            drawn: None,
        }
    }

    /// The selected row, as [`move_selection`] and [`select_by_number`]
    /// leave it, or none. An index past the end of the list, such as one
    /// kept from longer results, selects no row, so the list neither marks
    /// one nor scrolls past its end.
    pub fn selected(mut self, index: Option<usize>) -> Self {
        self.selected = index.filter(|index| *index < self.count);
        self
    }

    /// Notes in `drawn` when the list first builds rows for a frame.
    pub fn report_drawn(mut self, drawn: &ResultsDrawn) -> Self {
        self.drawn = Some(drawn.clone());
        self
    }

    /// The row at `index` as the list draws it: placed in the list, and
    /// selected if it is the selected row.
    #[cfg(test)]
    pub(super) fn row_at(&self, index: usize, window: &mut Window, cx: &mut App) -> Row {
        build_row(
            &self.render_row,
            index,
            self.count,
            self.selected,
            window,
            cx,
        )
    }

    /// Every row of the list as the list draws it, then the list box as
    /// `render` returns it, each read as the tests read it.
    #[cfg(test)]
    pub(crate) fn parts(
        self,
        window: &mut Window,
        cx: &mut App,
    ) -> Vec<super::probe::ComponentPart> {
        use super::probe::part;
        let mut parts: Vec<_> = (0..self.count)
            .map(|index| {
                let row = self.row_at(index, window, cx);
                part("row", RenderOnce::render(row, window, cx))
            })
            .collect();
        parts.push(part("list", RenderOnce::render(self, window, cx)));
        parts
    }

    /// The rows in GPUI's `uniform_list`, which has no role and so writes
    /// no node: the rows' nodes are options of the list box around it.
    fn rows(&self, scroll: &UniformListScrollHandle) -> UniformList {
        let render_row = self.render_row.clone();
        let (count, selected) = (self.count, self.selected);
        let drawn = self.drawn.clone();
        uniform_list(
            (self.id.clone(), "rows"),
            count,
            move |range: Range<usize>, window, cx| {
                if let Some(drawn) = drawn.as_ref().filter(|_| !range.is_empty()) {
                    drawn.note(cx.background_executor().now());
                }
                range
                    .map(|index| build_row(&render_row, index, count, selected, window, cx))
                    .collect::<Vec<_>>()
            },
        )
        .track_scroll(scroll)
        .size_full()
    }
}

/// The caller's row at `index`, placed by the list and selected if it is
/// the selected row.
fn build_row(
    render_row: &RenderRow,
    index: usize,
    count: usize,
    selected: Option<usize>,
    window: &mut Window,
    cx: &mut App,
) -> Row {
    render_row(index, window, cx).in_list(index, count, selected == Some(index))
}

impl RenderOnce for ResultList {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let scroll = window
            .use_keyed_state((self.id.clone(), "scroll"), cx, |_, _| {
                UniformListScrollHandle::new()
            })
            .read(cx)
            .clone();
        // The list scrolls only when the selection changes, so a list
        // scrolled by the pointer stays where it was put.
        let shown = window.use_keyed_state((self.id.clone(), "selected"), cx, |_, _| None);
        if *shown.read(cx) != self.selected {
            if let Some(index) = self.selected {
                scroll.scroll_to_item(index, ScrollStrategy::Nearest);
            }
            let selected = self.selected;
            shown.update(cx, |shown, _| *shown = selected);
        }
        let rows = self.rows(&scroll);
        div()
            .id(self.id)
            .role(Role::ListBox)
            .aria_label(self.name)
            .size_full()
            .child(rows)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::time::Duration;

    use gpui::{
        Modifiers, ScrollDelta, ScrollWheelEvent, TestAppContext, TouchPhase, VisualTestContext,
        point, px,
    };
    use gpui_component::IconName;

    use super::super::{RowParts, Tile, VaultBadge, probe};
    use super::*;
    use crate::ui::ds::typography::u;

    fn text(value: &'static str) -> SharedString {
        SharedString::new_static(value)
    }

    /// The row a caller would build for `index`, in a list of `count`.
    fn row(index: usize, count: usize) -> Row {
        Row::new(
            ("result", index),
            index,
            count,
            RowParts::new(text("Web login"), text("Salesforce")).vault(text("Personal")),
            Tile::new(IconName::Globe),
            VaultBadge::Personal,
        )
    }

    fn list(count: usize) -> ResultList {
        ResultList::new("results", text("Results"), count, move |index, _, _| {
            row(index, count)
        })
    }

    /// The result list is one node, a list box with the name it was given,
    /// and the `uniform_list` inside it has no role, so GPUI writes no node
    /// for it and the rows are options of the list box.
    #[gpui::test]
    fn the_list_is_a_list_box_with_its_name(cx: &mut TestAppContext) {
        let (role, node, id, rows_role) = probe::in_frame(cx, |window, cx| {
            let list = list(3).selected(Some(1));
            let scroll = UniformListScrollHandle::new();
            let (rows_role, _) = probe::node(list.rows(&scroll));
            let element = RenderOnce::render(list, window, cx).into_element();
            let (role, node) = probe::node_of(&element);
            (role, node, probe::element_id(element), rows_role)
        });
        assert_eq!(role, Some(Role::ListBox));
        assert_eq!(
            id,
            Some(ElementId::from("results")),
            "an id, so it has a node"
        );
        assert_eq!(node.label(), Some("Results"));
        assert_eq!(node.description(), None);
        assert_eq!(node.value(), None);
        assert_eq!(
            rows_role, None,
            "the uniform list writes no node of its own"
        );
    }

    /// The list places every row it draws, whatever the row was built
    /// with: its place counted from 1, the size of the list, and whether it
    /// is the selected row.
    #[gpui::test]
    fn the_list_places_and_selects_its_rows(cx: &mut TestAppContext) {
        let nodes = probe::in_frame(cx, |window, cx| {
            // Every row built as the first of one, selected.
            let list = ResultList::new("results", text("Results"), 3, |_, _, _| {
                row(0, 1).selected(true)
            })
            .selected(Some(2));
            (0..3)
                .map(|index| {
                    let row = list.row_at(index, window, cx);
                    probe::node(RenderOnce::render(row, window, cx)).1
                })
                .collect::<Vec<_>>()
        });
        let read: Vec<_> = nodes
            .iter()
            .map(|node| {
                (
                    node.position_in_set(),
                    node.size_of_set(),
                    node.is_selected(),
                )
            })
            .collect();
        assert_eq!(
            read,
            [
                (Some(1), Some(3), Some(false)),
                (Some(2), Some(3), Some(false)),
                (Some(3), Some(3), Some(true)),
            ]
        );
    }

    /// A selection past the end of the list, such as one kept from longer
    /// results, selects no row, and the list does not scroll for it: a
    /// list of forty with the forty-sixth selected stays at its top.
    #[gpui::test]
    fn a_selection_past_the_end_selects_no_row(cx: &mut TestAppContext) {
        const ROWS: usize = 40;
        let selected = probe::in_frame(cx, |window, cx| {
            let list = list(ROWS).selected(Some(45));
            (0..ROWS)
                .map(|index| {
                    let row = list.row_at(index, window, cx);
                    probe::node(RenderOnce::render(row, window, cx))
                        .1
                        .is_selected()
                })
                .collect::<Vec<_>>()
        });
        assert_eq!(selected, [Some(false); ROWS]);

        let built = Rc::new(RefCell::new(Vec::new()));
        let seen = built.clone();
        let cx = probe::window(cx, move |_, _| {
            let seen = seen.clone();
            let list = ResultList::new("results", text("Results"), ROWS, move |index, _, _| {
                seen.borrow_mut().push(index);
                row(index, ROWS)
            })
            .selected(Some(45));
            div().h(u(200.)).child(list).into_any_element()
        });
        built.borrow_mut().clear();
        redraw(cx);
        let rows = built.borrow().clone();
        assert!(rows.contains(&0), "the list stays at its top: {rows:?}");
        assert!(
            !rows.contains(&(ROWS - 1)),
            "no scroll to the end: {rows:?}"
        );
    }

    #[gpui::test]
    #[should_panic(expected = "a result list needs a name")]
    fn a_result_list_needs_a_name_in_every_build(_: &mut TestAppContext) {
        let _ = ResultList::new("results", SharedString::default(), 0, |index, _, _| {
            row(index, 1)
        });
    }

    /// Moving the selection stops at both ends and never wraps around.
    #[test]
    fn move_selection_never_wraps() {
        let at = |index: Option<usize>, len: usize| ListSelection::new(index, len);
        for (from, delta, to) in [
            (at(Some(0), 5), -1, Some(0)),
            (at(Some(0), 5), 1, Some(1)),
            (at(Some(2), 5), -1, Some(1)),
            (at(Some(4), 5), 1, Some(4)),
            (at(Some(3), 5), 1, Some(4)),
            (at(Some(1), 5), -5, Some(0)),
            (at(Some(1), 5), 9, Some(4)),
            (at(Some(1), 5), isize::MIN, Some(0)),
            (at(Some(1), 5), isize::MAX, Some(4)),
            (at(Some(2), 5), 0, Some(2)),
            (at(Some(0), 1), 1, Some(0)),
            (at(Some(0), 1), -1, Some(0)),
            // Nothing selected: down counts from above the first row.
            (at(None, 5), 1, Some(0)),
            (at(None, 5), 3, Some(2)),
            (at(None, 5), 9, Some(4)),
            (at(None, 5), -1, None),
            (at(None, 5), 0, None),
            // An empty list keeps no selection.
            (at(None, 0), 1, None),
            (at(None, 0), -1, None),
            (at(Some(0), 0), 1, None),
        ] {
            let moved = move_selection(from, delta);
            assert_eq!(moved.index(), to, "{from:?} moved by {delta}");
            assert_eq!(moved.len(), from.len(), "{from:?} moved by {delta}");
        }
    }

    /// A selection made for a longer list ends at the last row, and an
    /// empty list has none.
    #[test]
    fn a_selection_stays_inside_its_list() {
        assert_eq!(ListSelection::new(Some(7), 3).index(), Some(2));
        assert_eq!(ListSelection::new(Some(0), 0).index(), None);
        assert_eq!(ListSelection::first(4).index(), Some(0));
        assert_eq!(ListSelection::first(0).index(), None);
        assert!(ListSelection::first(0).is_empty());
    }

    /// ⌘1 to ⌘9 select the first nine rows. A number past the end of the
    /// list, or outside 1 to 9, leaves the selection as it was.
    #[test]
    fn select_by_number_ignores_numbers_past_the_end() {
        let at = |index: Option<usize>, len: usize| ListSelection::new(index, len);
        for (from, number, to) in [
            (at(Some(0), 5), 1, Some(0)),
            (at(Some(0), 5), 3, Some(2)),
            (at(Some(4), 5), 5, Some(4)),
            (at(Some(1), 5), 6, Some(1)),
            (at(Some(1), 5), 9, Some(1)),
            (at(None, 5), 2, Some(1)),
            (at(None, 5), 7, None),
            (at(Some(0), 12), 9, Some(8)),
            (at(Some(0), 12), 10, Some(0)),
            (at(Some(3), 12), 0, Some(3)),
            (at(Some(3), 12), u8::MAX, Some(3)),
            (at(None, 0), 1, None),
        ] {
            let selected = select_by_number(from, number);
            assert_eq!(selected.index(), to, "{from:?}, ⌘{number}");
            assert_eq!(selected.len(), from.len(), "{from:?}, ⌘{number}");
        }
    }

    fn redraw(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    /// The list notes the time of the first frame that drew its rows, on
    /// GPUI's clock, and keeps it through the frames after; cleared, it
    /// notes the next one. A list without rows notes nothing.
    #[gpui::test]
    fn the_list_reports_when_it_drew(cx: &mut TestAppContext) {
        let drawn = ResultsDrawn::new();
        let count = Rc::new(Cell::new(0));
        let (shown, rows) = (drawn.clone(), count.clone());
        let cx = probe::window(cx, move |_, _| {
            list(rows.get()).report_drawn(&shown).into_any_element()
        });
        let now = |cx: &mut VisualTestContext| cx.update(|_, cx| cx.background_executor().now());

        redraw(cx);
        assert_eq!(drawn.at(), None, "no rows, nothing drawn");

        count.set(3);
        cx.executor().advance_clock(Duration::from_secs(1));
        let first = now(cx);
        redraw(cx);
        assert_eq!(drawn.at(), Some(first), "the first frame with rows");

        cx.executor().advance_clock(Duration::from_secs(1));
        redraw(cx);
        assert_eq!(drawn.at(), Some(first), "a later frame changes nothing");

        drawn.clear();
        assert_eq!(drawn.at(), None);
        cx.executor().advance_clock(Duration::from_secs(1));
        let again = now(cx);
        redraw(cx);
        assert_eq!(drawn.at(), Some(again), "cleared, the next frame counts");
    }

    /// When the selection changes, the list scrolls the selected row into
    /// view; while it stays, the list stays where the pointer scrolled it.
    #[gpui::test]
    fn the_selected_row_scrolls_into_view(cx: &mut TestAppContext) {
        const ROWS: usize = 40;
        let built = Rc::new(RefCell::new(Vec::new()));
        let selected = Rc::new(Cell::new(Some(0)));
        let (seen, chosen) = (built.clone(), selected.clone());
        let cx = probe::window(cx, move |_, _| {
            let seen = seen.clone();
            let list = ResultList::new("results", text("Results"), ROWS, move |index, _, _| {
                seen.borrow_mut().push(index);
                row(index, ROWS)
            })
            .selected(chosen.get());
            div().h(u(200.)).child(list).into_any_element()
        });
        let drawn = |cx: &mut VisualTestContext| {
            built.borrow_mut().clear();
            redraw(cx);
            built.borrow().clone()
        };

        assert!(!drawn(cx).contains(&30), "row 30 is out of view at first");
        selected.set(Some(30));
        assert!(drawn(cx).contains(&30), "the selected row is drawn");

        cx.simulate_event(ScrollWheelEvent {
            position: point(px(10.), px(10.)),
            delta: ScrollDelta::Pixels(point(px(0.), px(5000.))),
            modifiers: Modifiers::default(),
            touch_phase: TouchPhase::Moved,
        });
        let rows = drawn(cx);
        assert!(rows.contains(&0), "scrolled back to the top: {rows:?}");
        assert!(!rows.contains(&30), "and it stays there: {rows:?}");
    }
}
