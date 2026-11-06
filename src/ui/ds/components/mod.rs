//! The components the new screens are built from.
//!
//! Each one draws its look from the tokens and the text styles and gives
//! VoiceOver what it needs: a role, a name that the caller passes in or that
//! comes from a text key, never from a literal in here, and a stable element
//! id. The button is drawn on the unstyled `gpui_base::Button`; the text field
//! and the switch wrap the GPUI Kit controls, whose editing and toggling
//! behavior is worth keeping. All three show the same keyboard focus ring,
//! from `focus_ring.rs`, and the text field and the switch turn the Kit's
//! own ring off for it.
//!
//! The type tile, the chip and the vault badge are parts of a row and write
//! no node there; the row's name says what they show. Shown on their own,
//! each sits in a [`Standalone`], which gives it a node with a role and a
//! name. The row itself is one node, an option in a list box, named after
//! its parts through [`RowParts::accessible_name`].
//!
//! The secret field and the one-time code ring are built from several
//! elements, and the secret or the code may reach none of them while it is
//! hidden or while no assistive technology runs. Under `cfg(test)` each
//! lists every element it builds, and its containers note how many
//! children they hold, so the tests read each element and find one that is
//! not on the list.
//!
//! The panel footer is no node and takes no focus; the one node in it is
//! the button that opens its menu. The panel hands what it says to
//! VoiceOver as the description of the element that has focus.

mod accessibility_id;
pub mod button;
pub mod chip;
mod focus_ring;
pub(in crate::ui::ds) mod keycap;
pub mod otp;
pub mod panel_footer;
pub mod row;
pub mod secret_field;
pub mod shortcut;
pub mod switch;
pub mod text_field;
pub mod tile;
pub mod vault_badge;

use gpui::{
    AnyElement, App, Div, ElementId, InteractiveElement as _, IntoElement, ParentElement as _,
    RenderOnce, Role, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
};

pub use accessibility_id::AccessibilityId;
pub use button::{Button, ButtonKind, ButtonSize, IconButton};
pub use chip::{Chip, ChipTone};
pub use otp::{OtpRing, OtpSize};
pub use panel_footer::{FooterTone, PanelFooter};
pub use row::{Row, RowDensity, RowParts};
pub use secret_field::{CopyState, SecretField};
pub use shortcut::Shortcut;
pub use switch::Switch;
pub use text_field::TextField;
pub use tile::Tile;
pub use vault_badge::{VaultBadge, initial_for};

/// A component shown on its own rather than inside a row, such as the type
/// tile in the head of an entry's details: a node of its own, with a role
/// and a name, around what the component draws.
#[derive(IntoElement)]
pub struct Standalone {
    id: ElementId,
    role: Role,
    name: SharedString,
    body: AnyElement,
}

impl Standalone {
    /// Panics on an empty name, in every build: VoiceOver reads the node by
    /// its name, since a glyph, a dot or a mark such as "PRD" says nothing
    /// on its own.
    fn new(id: ElementId, role: Role, name: SharedString, body: AnyElement) -> Self {
        assert!(!name.is_empty(), "a component on its own needs a name");
        Self {
            id,
            role,
            name,
            body,
        }
    }
}

impl RenderOnce for Standalone {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .id(self.id)
            .role(self.role)
            .aria_label(self.name)
            .flex()
            .flex_none()
            .child(self.body)
    }
}

/// A container of a composed component, such as the secret field or the
/// one-time code ring. In the
/// tests it notes how many children it holds once it is laid out, so that a
/// test can hold the component to the parts it lists for the tests; outside
/// the tests it is the container as it was.
fn counted(container: Div, name: &'static str) -> Div {
    #[cfg(test)]
    let container = container.on_children_prepainted(move |children, window, _| {
        probe::note_children(window, name, children.len())
    });
    #[cfg(not(test))]
    let _ = name;
    container
}

/// The count behind the rule of one copper surface per view.
///
/// Every copper button reports here while it renders, in debug builds only.
/// After the frame the count for its window is kept, and a frame with more
/// than one copper surface says so on stderr. The test `one_copper_per_view`
/// reads the count for a whole view.
#[cfg(debug_assertions)]
pub mod copper {
    use std::collections::HashMap;

    use gpui::{App, Global, Subscription, Window, WindowId};

    #[derive(Default)]
    struct CopperCounter {
        windows: HashMap<WindowId, Tally>,
        /// Drops the count of a window when it closes.
        _closed: Option<Subscription>,
    }

    impl Global for CopperCounter {}

    #[derive(Default)]
    struct Tally {
        /// Copper surfaces in the frame being drawn.
        drawing: usize,
        /// Copper surfaces in the last frame that had any.
        last: usize,
        /// Whether the end of the current frame is already scheduled.
        pending: bool,
    }

    fn counter(cx: &mut App) -> &mut CopperCounter {
        if !cx.has_global::<CopperCounter>() {
            let closed = cx.on_window_closed(|cx, window| {
                cx.default_global::<CopperCounter>().windows.remove(&window);
            });
            cx.set_global(CopperCounter {
                windows: HashMap::new(),
                _closed: Some(closed),
            });
        }
        cx.global_mut::<CopperCounter>()
    }

    /// Called by a copper surface while it renders.
    pub(in crate::ui::ds) fn note(window: &Window, cx: &mut App) {
        let id = window.window_handle().window_id();
        let tally = counter(cx).windows.entry(id).or_default();
        tally.drawing += 1;
        if !tally.pending {
            tally.pending = true;
            // Deferred work runs once the update that draws the frame is
            // done, so this sees every surface of the frame.
            cx.defer(move |cx| finish_frame(id, cx));
        }
    }

    fn finish_frame(id: WindowId, cx: &mut App) {
        let Some(tally) = counter(cx).windows.get_mut(&id) else {
            return;
        };
        tally.last = std::mem::take(&mut tally.drawing);
        tally.pending = false;
        if tally.last > 1 {
            eprintln!(
                "ferrispass: one frame drew {} copper surfaces in a window, the design allows one",
                tally.last
            );
        }
    }

    /// How many copper surfaces `window` drew in its last frame that had
    /// any. A frame without copper leaves the count as it was.
    pub fn surfaces(window: &Window, cx: &App) -> usize {
        cx.try_global::<CopperCounter>()
            .and_then(|counter| counter.windows.get(&window.window_handle().window_id()))
            .map_or(0, |tally| tally.last)
    }
}

/// Helpers for the component tests: a window to draw components in, and a
/// way to read what an element tells assistive technology.
#[cfg(test)]
pub(in crate::ui::ds) mod probe {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::ops::Range;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;

    use gpui::{
        AnyElement, App, AppContext as _, Bounds, Context, Edges, Element, ElementId, Hsla,
        IntoElement, Modifiers, ParentElement as _, Pixels, Render, Role, Styled as _,
        TestAppContext, VisualTestContext, Window, WindowId, accesskit, canvas, div, px,
    };
    use gpui_component::{Root, Theme};

    use crate::ui::ds::ActiveTokens as _;

    type Build = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

    struct Harness {
        build: Build,
    }

    impl Render for Harness {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child((self.build)(window, cx))
        }
    }

    /// A window under the Kit's `Root`, which sets the rem size from the
    /// text size as the app does, showing what `build` returns.
    pub fn window(
        cx: &mut TestAppContext,
        build: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    ) -> &mut VisualTestContext {
        cx.update(|cx| {
            if !cx.has_global::<Theme>() {
                gpui_component::init(cx);
            }
        });
        let build: Build = Rc::new(build);
        let (_, cx) = cx.add_window_view(|window, cx| {
            let harness = cx.new(|_| Harness { build });
            Root::new(harness, window, cx)
        });
        cx.run_until_parked();
        cx
    }

    /// Runs `probe` while a frame is drawn, where components can render, and
    /// returns its result.
    pub fn in_frame<R: 'static>(
        cx: &mut TestAppContext,
        probe: impl Fn(&mut Window, &mut App) -> R + 'static,
    ) -> R {
        let slot = Rc::new(RefCell::new(None));
        let filled = slot.clone();
        let probe = Rc::new(probe);
        let cx = window(cx, move |_, _| {
            let filled = filled.clone();
            let probe = probe.clone();
            canvas(
                move |_, window, cx| *filled.borrow_mut() = Some(probe(window, cx)),
                |_, _, _, _| {},
            )
            .size_full()
            .into_any_element()
        });
        cx.update(|window, cx| window.draw(cx).clear(cx));
        slot.take().expect("the frame ran the probe")
    }

    /// The accessibility node `element` writes: its role and the properties
    /// AccessKit hands to VoiceOver.
    pub fn node(element: impl IntoElement) -> (Option<Role>, accesskit::Node) {
        node_of(&element.into_element())
    }

    /// The same, read without taking `element`, so that a test can still
    /// draw the element it read.
    pub fn node_of(element: &impl Element) -> (Option<Role>, accesskit::Node) {
        let role = element.a11y_role();
        let mut node = accesskit::Node::new(role.unwrap_or(Role::Unknown));
        element.write_a11y_info(&mut node);
        (role, node)
    }

    /// The element id `element` carries. Without one, GPUI writes no node
    /// for it in the real tree, whatever its role.
    pub fn element_id(element: impl IntoElement) -> Option<ElementId> {
        element.into_element().id()
    }

    /// One element a composed component builds, and what it tells assistive
    /// technology: its role, its element id and its node. Read before the
    /// element goes into its container, since an element whose type GPUI
    /// has erased reports no role and writes nothing.
    pub struct ComponentPart {
        pub name: &'static str,
        pub role: Option<Role>,
        pub id: Option<ElementId>,
        pub node: accesskit::Node,
    }

    impl ComponentPart {
        /// The texts the node hands to assistive technology, by name.
        pub fn texts(&self) -> [(&'static str, Option<&str>); 6] {
            [
                ("label", self.node.label()),
                ("value", self.node.value()),
                ("description", self.node.description()),
                ("tooltip", self.node.tooltip()),
                ("author id", self.node.author_id()),
                ("placeholder", self.node.placeholder()),
            ]
        }
    }

    /// Reads `element` as the part called `name`.
    pub fn part(name: &'static str, element: impl IntoElement) -> ComponentPart {
        let element = element.into_element();
        let (role, node) = node_of(&element);
        ComponentPart {
            name,
            role,
            id: element.id(),
            node,
        }
    }

    thread_local! {
        /// How many children each counted container held when its window
        /// last laid it out, on this thread.
        static CHILDREN: RefCell<HashMap<(WindowId, &'static str), usize>> =
            RefCell::new(HashMap::new());
    }

    /// Called by a counted container once its children are laid out.
    pub fn note_children(window: &Window, container: &'static str, count: usize) {
        let window = window.window_handle().window_id();
        CHILDREN.with_borrow_mut(|children| {
            children.insert((window, container), count);
        });
    }

    /// How many children the counted containers of `window` held in its last
    /// frame, together.
    pub fn counted_children(window: &Window) -> usize {
        let window = window.window_handle().window_id();
        CHILDREN.with_borrow(|children| {
            children
                .iter()
                .filter(|((drawn_in, _), _)| *drawn_in == window)
                .map(|(_, count)| count)
                .sum()
        })
    }

    /// Each line of `file` in `src/ui/ds/components` that has `call` on
    /// it, once for every time it does, read through `code_only`, so
    /// comments and the file's tests do not count. A call that starts with
    /// a letter counts only where no letter, digit or underscore comes
    /// right before it: `id(` is found in `.id(` and in
    /// `InteractiveElement::id(`, not in `accessibility_id(`, and
    /// `Text::new(` not in `StyledText::new(`.
    pub fn calls_in(file: &str, call: &str) -> Vec<String> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/ui/ds/components")
            .join(file);
        let source = std::fs::read_to_string(path).expect(file);
        let code = code_only(&source);
        let in_word = |c: char| c.is_alphanumeric() || c == '_';
        let word = call.starts_with(in_word);
        code.lines()
            .enumerate()
            .flat_map(|(at, line)| {
                line.match_indices(call)
                    .filter(move |(start, _)| !word || !line[..*start].ends_with(in_word))
                    .map(move |_| format!("{file}:{}: {}", at + 1, line.trim()))
            })
            .collect()
    }

    /// The edges a window painted in its last frame: each quad with a
    /// visible border, with its bounds, its color and its widths in pixels.
    /// GPUI paints an edge without a surface as strips that share the
    /// bounds and the widths of the edge.
    fn edges(cx: &mut VisualTestContext) -> Vec<(Bounds<Pixels>, Hsla, Edges<Pixels>)> {
        cx.update(|window, _| {
            let scale = window.scale_factor();
            window
                .painted_quads()
                .into_iter()
                .filter(|quad| !quad.border_color.is_transparent())
                .map(|quad| {
                    (
                        quad.bounds.map(|value| px(value.0 / scale)),
                        quad.border_color,
                        quad.border_widths.map(|width| px(width.0 / scale)),
                    )
                })
                .filter(|(_, _, widths)| widths.any(|width| *width > Pixels::ZERO))
                .collect()
        })
    }

    /// The edges `cx` paints now that it did not paint in `before`.
    fn added_edges(
        cx: &mut VisualTestContext,
        before: &[(Bounds<Pixels>, Hsla, Edges<Pixels>)],
    ) -> Vec<(Bounds<Pixels>, Hsla, Edges<Pixels>)> {
        edges(cx)
            .into_iter()
            .filter(|edge| !before.contains(edge))
            .collect()
    }

    /// Whether two bounds lie within a pixel of each other: painting snaps
    /// to device pixels.
    fn near(a: Bounds<Pixels>, b: Bounds<Pixels>) -> bool {
        [
            (a.left(), b.left()),
            (a.top(), b.top()),
            (a.right(), b.right()),
            (a.bottom(), b.bottom()),
        ]
        .into_iter()
        .all(|(x, y)| (x - y).abs() <= px(1.))
    }

    /// Checks the keyboard focus ring of the element marked `control`, the
    /// first tab stop of the window, which holds a second tab stop after
    /// it.
    ///
    /// A key press while nothing has focus shows no ring. Taking the first
    /// tab stop, as Tab does, shows one ring, around the control: it ends
    /// 3 px out, it is the only edge focus adds to the frame, 2 px wide in
    /// `focus_ring`, and 1 px lies between it and the control. A ring the
    /// GPUI Kit draws, a tinted edge or a wider band, is an edge focus adds
    /// too, and so is a ring around a control without focus. Taking the
    /// next tab stop takes the ring off the control. Then clicks the middle
    /// of the control and checks that focus from a click shows no ring.
    /// Returns the bounds of the control.
    pub fn assert_keyboard_focus_ring(
        cx: &mut VisualTestContext,
        control: &'static str,
    ) -> Bounds<Pixels> {
        assert_eq!(
            cx.debug_bounds("fp-focus-ring"),
            None,
            "nothing has focus yet"
        );
        let unfocused = edges(cx);
        // With nothing focused yet, a key press reaches no handler, so the
        // first tab stop is taken directly after it, as Tab would.
        cx.simulate_keystrokes("tab");
        cx.update(|window, cx| {
            assert!(window.last_input_was_keyboard());
            assert!(window.focused(cx).is_none(), "a key press takes no focus");
            window.draw(cx).clear(cx);
        });
        assert_eq!(
            cx.debug_bounds("fp-focus-ring"),
            None,
            "a key press shows no ring while no control has focus"
        );
        cx.update(|window, cx| window.focus_next(cx));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let bounds = cx.debug_bounds(control).expect("the control");
        let ring = cx.debug_bounds("fp-focus-ring").expect("a ring after Tab");
        let color = cx.update(|_, cx| cx.ds().colors.focus_ring.hsla);
        let added = added_edges(cx, &unfocused);
        assert!(!added.is_empty(), "the ring is painted");
        assert!(
            added
                .iter()
                .all(|(_, painted, width)| *painted == color && *width == Edges::all(px(2.))),
            "focus adds the ring alone, 2 px wide in focus_ring: {added:?}"
        );
        let (first, _, width) = &added[0];
        assert!(
            added.iter().all(|(painted, _, _)| near(*painted, *first)),
            "focus adds one ring, around the control that has focus: {added:?}"
        );
        assert_eq!(ring, bounds.dilate(px(3.)), "the ring ends 3 px out");
        let gaps = [
            bounds.left() - (ring.left() + width.left),
            bounds.top() - (ring.top() + width.top),
            (ring.right() - width.right) - bounds.right(),
            (ring.bottom() - width.bottom) - bounds.bottom(),
        ];
        assert!(
            gaps.iter().all(|gap| (*gap - px(1.)).abs() < px(0.01)),
            "1 px between the control and the ring: {gaps:?}"
        );

        let focused = cx.update(|window, cx| window.focused(cx));
        cx.update(|window, cx| window.focus_next(cx));
        let moved = cx.update(|window, cx| window.focused(cx)) != focused;
        assert!(moved, "the window has a second tab stop after the control");
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let left: Vec<_> = added_edges(cx, &unfocused)
            .into_iter()
            .filter(|(painted, _, _)| near(*painted, ring))
            .collect();
        assert!(
            left.is_empty(),
            "the control shows no ring once focus moves on: {left:?}"
        );

        cx.simulate_click(bounds.center(), Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(
            cx.debug_bounds("fp-focus-ring"),
            None,
            "a click shows no ring"
        );
        bounds
    }

    /// `code` from its first `{` to the brace that closes it. Meant for code
    /// without comments; a brace inside a literal counts like any other.
    pub fn braced(code: &str) -> &str {
        let open = code.find('{').expect("an opening brace");
        let mut depth = 0usize;
        for (offset, c) in code[open..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return &code[open..=open + offset];
                    }
                }
                _ => {}
            }
        }
        &code[open..]
    }

    /// The arguments of the call whose opening parenthesis is at `open` in
    /// `code`, trimmed, up to the parenthesis that closes it. A comma splits
    /// arguments only outside nested brackets and outside literals; a
    /// trailing comma adds no argument. Meant for code without comments.
    pub fn call_arguments(code: &str, open: usize) -> Vec<&str> {
        let parts = parts(code);
        let bytes = code.as_bytes();
        let mut arguments = Vec::new();
        let mut depth = 0usize;
        let mut start = open + 1;
        for at in open..bytes.len() {
            if parts[at] != Part::Code {
                continue;
            }
            match bytes[at] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        arguments.push(code[start..at].trim());
                        break;
                    }
                }
                b',' if depth == 1 => {
                    arguments.push(code[start..at].trim());
                    start = at + 1;
                }
                _ => {}
            }
        }
        if arguments.last().is_some_and(|last| last.is_empty()) {
            arguments.pop();
        }
        arguments
    }

    /// Every `.rs` file under `dir`, for the scans over the source.
    pub fn rust_files(dir: &Path, files: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries {
            let path = entry.expect("directory entry").path();
            if path.is_dir() {
                rust_files(&path, files);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                files.push(path);
            }
        }
    }

    /// `source` without its comments and its inline test modules, for the
    /// scans over the code. Line breaks stay, so a line in the result is the
    /// same line in the file.
    ///
    /// String and character literals stay as they are written, and a `//`
    /// or `/*` inside one starts no comment. An inline test module is
    /// `#[cfg(test)]` followed by `mod tests {`, and it is left out up to
    /// the brace that closes it, so code after it still counts. A test
    /// module declared as `mod tests;` leaves the rest of its file alone.
    pub fn code_only(source: &str) -> String {
        let parts = parts(source);
        // The source with comments and literals blanked out, byte for byte,
        // so that braces and attributes inside them do not count.
        let structure: Vec<u8> = source
            .bytes()
            .zip(&parts)
            .map(|(byte, part)| {
                if *part == Part::Code || byte == b'\n' {
                    byte
                } else {
                    b' '
                }
            })
            .collect();
        let structure = String::from_utf8(structure).expect("blanking keeps whole characters");
        let mut keep: Vec<bool> = parts.iter().map(|part| *part != Part::Comment).collect();
        for module in inline_test_modules(&structure) {
            keep[module].fill(false);
        }
        let code: Vec<u8> = source
            .bytes()
            .zip(keep)
            .filter(|(byte, keep)| *keep || *byte == b'\n')
            .map(|(byte, _)| byte)
            .collect();
        String::from_utf8(code).expect("comments and test modules are whole characters")
    }

    /// What a byte of a source file belongs to.
    #[derive(Clone, Copy, PartialEq)]
    enum Part {
        Code,
        Comment,
        Literal,
    }

    /// Which bytes of `source` are code, which are comments, and which are
    /// string or character literals.
    fn parts(source: &str) -> Vec<Part> {
        let bytes = source.as_bytes();
        let at = |i: usize| bytes.get(i).copied();
        let mut parts = vec![Part::Code; bytes.len()];
        let mut i = 0;
        while i < bytes.len() {
            let start = i;
            let part = match bytes[i] {
                b'/' if at(i + 1) == Some(b'/') => {
                    i = source[i..].find('\n').map_or(bytes.len(), |end| i + end);
                    Part::Comment
                }
                b'/' if at(i + 1) == Some(b'*') => {
                    // Block comments nest.
                    let mut depth = 0;
                    while i < bytes.len() {
                        if bytes[i] == b'/' && at(i + 1) == Some(b'*') {
                            depth += 1;
                            i += 2;
                        } else if bytes[i] == b'*' && at(i + 1) == Some(b'/') {
                            depth -= 1;
                            i += 2;
                            if depth == 0 {
                                break;
                            }
                        } else {
                            i += 1;
                        }
                    }
                    Part::Comment
                }
                b'"' => {
                    i = string_end(bytes, i + 1);
                    Part::Literal
                }
                b'\'' => match char_literal_end(source, i) {
                    Some(end) => {
                        i = end;
                        Part::Literal
                    }
                    // A lifetime or a label.
                    None => {
                        i += 1;
                        Part::Code
                    }
                },
                b'r' | b'b' | b'c' if !(i > 0 && is_identifier(bytes[i - 1])) => {
                    match raw_string_end(source, i) {
                        Some(end) => {
                            i = end;
                            Part::Literal
                        }
                        None => {
                            i += 1;
                            Part::Code
                        }
                    }
                }
                _ => {
                    i += 1;
                    Part::Code
                }
            };
            let end = i.min(bytes.len());
            parts[start..end].fill(part);
            i = end;
        }
        parts
    }

    /// Whether `byte` can be part of an identifier. The bytes of characters
    /// beyond ASCII count: outside literals and comments, only identifiers
    /// hold them.
    fn is_identifier(byte: u8) -> bool {
        byte.is_ascii_alphanumeric() || byte == b'_' || !byte.is_ascii()
    }

    /// Where the string whose text starts at `i` ends, after its closing
    /// quote. An escaped quote does not end it.
    pub fn string_end(bytes: &[u8], mut i: usize) -> usize {
        while i < bytes.len() {
            match bytes[i] {
                b'\\' => i += 2,
                b'"' => return i + 1,
                _ => i += 1,
            }
        }
        bytes.len()
    }

    /// Where the character literal that starts with the quote at `i` ends,
    /// or `None` when that quote starts a lifetime or a label.
    fn char_literal_end(source: &str, i: usize) -> Option<usize> {
        let rest = &source[i + 1..];
        let first = rest.chars().next()?;
        if first == '\\' {
            let escaped = rest[1..].chars().next()?;
            let after = 1 + escaped.len_utf8();
            return rest[after..].find('\'').map(|end| i + 1 + after + end + 1);
        }
        let after = first.len_utf8();
        rest[after..].starts_with('\'').then_some(i + 1 + after + 1)
    }

    /// Where the raw string that starts at `i` (`r"`, `r#"`, `br"`, `cr"`
    /// and so on) ends, or `None` when no raw string starts there.
    fn raw_string_end(source: &str, i: usize) -> Option<usize> {
        let bytes = source.as_bytes();
        let mut at = i;
        if matches!(bytes.get(at), Some(b'b' | b'c')) {
            at += 1;
        }
        if bytes.get(at) != Some(&b'r') {
            return None;
        }
        at += 1;
        let hashes = bytes[at..].iter().take_while(|byte| **byte == b'#').count();
        at += hashes;
        if bytes.get(at) != Some(&b'"') {
            return None;
        }
        let body = at + 1;
        let closing = format!("\"{}", "#".repeat(hashes));
        Some(
            source[body..]
                .find(&closing)
                .map_or(source.len(), |end| body + end + closing.len()),
        )
    }

    /// The byte ranges of the inline test modules in `structure`, the
    /// source with its comments and literals blanked out: from
    /// `#[cfg(test)]` to the brace that closes the `mod tests {` after it.
    fn inline_test_modules(structure: &str) -> Vec<Range<usize>> {
        const ATTRIBUTE: &str = "#[cfg(test)]";
        let mut modules = Vec::new();
        let mut from = 0;
        while let Some(start) = structure[from..].find(ATTRIBUTE).map(|at| from + at) {
            from = start + ATTRIBUTE.len();
            let Some(open) = test_module_body(&structure[from..]).map(|at| from + at) else {
                continue;
            };
            let mut depth = 0usize;
            let end = structure[open..]
                .char_indices()
                .find_map(|(offset, c)| {
                    match c {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                return Some(open + offset + 1);
                            }
                        }
                        _ => {}
                    }
                    None
                })
                .unwrap_or(structure.len());
            modules.push(start..end);
            from = end;
        }
        modules
    }

    /// Where the opening brace is when `text` starts with `mod tests {`,
    /// with any whitespace around the words.
    fn test_module_body(text: &str) -> Option<usize> {
        let after_mod = text.trim_start().strip_prefix("mod")?;
        let name = after_mod.trim_start();
        if name.len() == after_mod.len() {
            return None;
        }
        let body = name.strip_prefix("tests")?.trim_start();
        body.starts_with('{').then_some(text.len() - body.len())
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    /// Calls that hand a text to VoiceOver or show it, beside the `.child(`
    /// and `aria_label(` that `no_literals_in_render_code` looks at, each
    /// with the place of that text among its arguments, counted from 0.
    /// The other arguments, such as an element id or a key string, may be
    /// literals.
    const TEXT_CALLS: &[(&str, usize)] = &[
        ("accessibility_label(", 0),
        ("aria_description(", 0),
        ("aria_value(", 0),
        ("aria_placeholder(", 0),
        ("tooltip(", 0),
        ("Label::new(", 0),
        // The label or name of a component.
        ("Button::new(", 2),
        ("Button::primary(", 1),
        ("Button::secondary(", 1),
        ("Button::quiet(", 1),
        ("Button::ghost(", 1),
        ("Button::danger(", 1),
        ("Button::large(", 1),
        ("IconButton::new(", 2),
        ("Shortcut::from_keystroke(", 1),
        ("TextField::new(", 2),
        ("Switch::new(", 1),
        ("Chip::new(", 1),
        ("SecretField::new(", 1),
        // The footer's line about what to connect to first, and the notes
        // of its tones.
        ("connect_first(", 0),
        ("FooterTone::Offline(", 0),
        ("FooterTone::Tie(", 0),
        ("FooterTone::UpdateReady(", 0),
        ("VaultBadge::team(", 0),
        ("VaultBadge::other(", 0),
        // The name of a component on its own. `standalone(` covers the
        // vault badge's `badge.standalone(id, name)`; in the tile's and the
        // chip's the argument there is the glyph or the tone.
        ("Standalone::new(", 2),
        ("Tile::standalone(", 2),
        ("Chip::standalone(", 2),
        ("Chip::standalone(", 3),
        ("standalone(", 1),
        // The parts of a row's name, which the row also shows. `Row::new`
        // takes no text of its own.
        ("RowParts::new(", 0),
        ("RowParts::new(", 1),
        ("role_word(", 0),
        ("customer(", 0),
        ("client(", 0),
        ("user(", 0),
        ("vault(", 0),
        // The sentences a component adds to its name.
        ("description(", 0),
        ("note(", 0),
        ("error(", 0),
    ];

    /// The string literals `source` hands straight to one of `TEXT_CALLS`
    /// as its text, with the line of the call. Comments and inline test
    /// modules are left out.
    fn literal_texts(source: &str) -> Vec<(usize, String)> {
        let code = probe::code_only(source);
        let mut found = Vec::new();
        for (call, place) in TEXT_CALLS {
            for (at, _) in code.match_indices(call) {
                let part_of_a_longer_name = code[..at]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_');
                if part_of_a_longer_name {
                    continue;
                }
                let arguments = probe::call_arguments(&code, at + call.len() - 1);
                let Some(argument) = arguments.get(*place) else {
                    continue;
                };
                let literal = argument.starts_with('"')
                    || argument.starts_with("r\"")
                    || argument.starts_with("r#");
                if literal {
                    let line = code[..at].matches('\n').count() + 1;
                    let first_line = argument.lines().next().unwrap_or_default();
                    found.push((line, first_line.to_string()));
                }
            }
        }
        found.sort();
        found
    }

    /// A label, name, description, sentence, value or tooltip in `ui::ds`
    /// comes from a parameter or a text key, never from a literal in the
    /// code.
    #[test]
    fn no_literal_names_descriptions_or_tooltips() {
        let ds = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui/ds");
        let mut files = Vec::new();
        probe::rust_files(&ds, &mut files);
        assert!(
            files.len() > 10,
            "the scan found only {} files",
            files.len()
        );
        let mut problems = Vec::new();
        for file in &files {
            let source = std::fs::read_to_string(file).expect("read source");
            for (line, literal) in literal_texts(&source) {
                problems.push(format!("{}:{line}: {literal}", file.display()));
            }
        }
        assert!(
            problems.is_empty(),
            "texts for VoiceOver come from a parameter or tr!:\n{}",
            problems.join("\n")
        );
    }

    #[test]
    fn the_text_scan_finds_literals_in_code_only() {
        let source = r##"
            div().aria_description("not available")
                .accessibility_label(
                    r#"Copy"#,
                )
                .aria_value(sentence)
                // .tooltip("in a comment")
                .my_aria_value("not this one")
                .when_some(text, |this, text| this.aria_placeholder(text));
            Label::new("Server");
            div().child(link("https://example.com")).tooltip("Open the link");
#[cfg(test)]
mod tests {
    fn sample() { div().aria_description("in a test"); }
}
            Button::new("open", ButtonKind::Primary, "Open website");
            Button::primary("sign-in", "Sign in");
            Button::secondary("other", "Other vault");
            Button::quiet("why", "Why?");
            Button::ghost("open", "Open");
            Button::danger("forget", "Forget");
            super::Button::large("unlock", "Unlock");
            IconButton::new("copy", IconName::Copy, "Copy password");
            Shortcut::from_keystroke("cmd-c", "Password");
            TextField::new("server", &state, "Server");
            Switch::new("autolock", "Lock when idle");
            Button::primary("save", label).description("Production system");
            Switch::new("autolock", label).note("After four minutes without input.");
            TextField::new("server", &state, label).error("The address needs a port.");
            TextField::new(
                "server",
                &state,
                "Server address",
            );
            Button::primary("sign-in", tr!("quick.sign_in")).shortcut("enter");
            IconButton::new("copy", IconName::Copy, name);
            Shortcut::from_keystroke("cmd-c", word);
            KitSwitch::new("autolock");
            gpui_base::Button::new("save");
            Button::primary("row,", label).my_note("not this one");
            fn note(mut self, sentence: impl Into<SharedString>) -> Self { self }
            Chip::new(ChipTone::Production, "PRD");
            Chip::new(ChipTone::Production, prd);
            Chip::standalone("role", ChipTone::Production, "PRD", "Production system");
            Tile::standalone("kind", IconName::Globe, "Web login");
            VaultBadge::team("B", color).standalone("vault", "Team Beratung");
            VaultBadge::other("K");
            Standalone::new(id, Role::Image, "Image", body);
            Tile::standalone("kind", IconName::Globe, name);
            fn standalone(self, id: impl Into<ElementId>, name: impl Into<SharedString>) {}
            RowParts::new("SAP system", "NHP 100")
                .role_word("Production system")
                .customer("Nordhafen AG")
                .client("Client 100")
                .user("N.WEISS")
                .vault("Team Beratung");
            RowParts::new(kind, title).role_word(role).user(user).vault(vault);
            fn user(mut self, user: impl Into<SharedString>) -> Self { self }
            Row::new("row", 0, 1, parts, tile, badge).description("Return: sign in");
            SecretField::new("password", "Password").value(secret);
            PanelFooter::new("footer", open).connect_first("Connect first: Nordhafen GP");
            footer.tone(FooterTone::Offline("As of 13:40"));
            footer.tone(FooterTone::Tie("2 identical matches"));
            footer.tone(FooterTone::UpdateReady("Update ready"));
            footer.tone(FooterTone::Offline(note)).connect_first(via);
        "##;
        assert_eq!(
            literal_texts(source),
            [
                (2, "\"not available\"".to_string()),
                (3, "r#\"Copy\"#".to_string()),
                (10, "\"Server\"".to_string()),
                (11, "\"Open the link\"".to_string()),
                (16, "\"Open website\"".to_string()),
                (17, "\"Sign in\"".to_string()),
                (18, "\"Other vault\"".to_string()),
                (19, "\"Why?\"".to_string()),
                (20, "\"Open\"".to_string()),
                (21, "\"Forget\"".to_string()),
                (22, "\"Unlock\"".to_string()),
                (23, "\"Copy password\"".to_string()),
                (24, "\"Password\"".to_string()),
                (25, "\"Server\"".to_string()),
                (26, "\"Lock when idle\"".to_string()),
                (27, "\"Production system\"".to_string()),
                (28, "\"After four minutes without input.\"".to_string()),
                (29, "\"The address needs a port.\"".to_string()),
                (30, "\"Server address\"".to_string()),
                (42, "\"PRD\"".to_string()),
                (44, "\"PRD\"".to_string()),
                (44, "\"Production system\"".to_string()),
                (45, "\"Web login\"".to_string()),
                (46, "\"B\"".to_string()),
                (46, "\"Team Beratung\"".to_string()),
                (47, "\"K\"".to_string()),
                (48, "\"Image\"".to_string()),
                (51, "\"NHP 100\"".to_string()),
                (51, "\"SAP system\"".to_string()),
                (52, "\"Production system\"".to_string()),
                (53, "\"Nordhafen AG\"".to_string()),
                (54, "\"Client 100\"".to_string()),
                (55, "\"N.WEISS\"".to_string()),
                (56, "\"Team Beratung\"".to_string()),
                (59, "\"Return: sign in\"".to_string()),
                (60, "\"Password\"".to_string()),
                (61, "\"Connect first: Nordhafen GP\"".to_string()),
                (62, "\"As of 13:40\"".to_string()),
                (63, "\"2 identical matches\"".to_string()),
                (64, "\"Update ready\"".to_string()),
            ]
        );
    }

    /// Text in the components comes in a text style, and no text style is
    /// below 11 points (`nothing_below_11_points` in `typography.rs`). So a
    /// component sets no text size of its own: neither GPUI's size setters
    /// nor the GPUI Kit's sizes, which go down to 11.4 px at text size 13.
    /// The scan reads the files through `code_only`, so comments and inline
    /// test modules do not count.
    #[test]
    fn nothing_below_11_points_in_the_new_components() {
        // Built from pieces so that the list does not find itself.
        let setters = [
            concat!(".text_", "size("),
            concat!(".text_", "xs("),
            concat!(".text_", "sm("),
            concat!(".text_", "base("),
            concat!(".text_", "lg("),
            concat!(".text_", "xl("),
            concat!(".text_", "2xl("),
            concat!(".text_", "3xl("),
            concat!(".x", "small("),
            concat!(".", "small("),
            concat!(".with_", "size("),
            concat!(".font_", "size("),
        ];
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui/ds/components");
        let mut files = Vec::new();
        probe::rust_files(&dir, &mut files);
        assert!(files.len() > 8, "the scan found only {} files", files.len());
        let mut found = Vec::new();
        for file in &files {
            let source = std::fs::read_to_string(file).expect("read source");
            for (number, line) in probe::code_only(&source).lines().enumerate() {
                if setters.iter().any(|setter| line.contains(setter)) {
                    found.push(format!(
                        "{}:{}: {}",
                        file.display(),
                        number + 1,
                        line.trim()
                    ));
                }
            }
        }
        assert!(
            found.is_empty(),
            "set text with .text_style(TextStyle::..), never below 11 points:\n{}",
            found.join("\n")
        );
        for style in crate::ui::ds::typography::TextStyle::ALL {
            assert!(style.size() >= 11.0, "{style:?} is {} pt", style.size());
        }
    }

    /// The secret and the one-time codes of the tests below.
    const SECRET: &str = "sp-test-7Qx!2026";
    const CODE: &str = "739114";
    const NEXT_CODE: &str = "205881";

    /// `secret` and the pieces of it that would give it away: all of it, and
    /// its groups of three and of four, alone and in a row with spaces.
    fn giveaways(secret: &str) -> Vec<String> {
        let chars: Vec<char> = secret.chars().collect();
        let mut found = vec![secret.to_string()];
        for size in [3, 4] {
            let groups: Vec<String> = chars
                .chunks(size)
                .map(|group| group.iter().collect())
                .collect();
            found.push(groups.join(" "));
            found.extend(
                groups
                    .into_iter()
                    .filter(|group| group.chars().count() == size),
            );
        }
        found
    }

    fn field(revealed: bool, assistive: bool, copied: CopyState, handlers: bool) -> SecretField {
        let field = SecretField::new("password", SharedString::new_static("Password"))
            .value(zeroize::Zeroizing::new(SECRET.to_string()))
            .revealed(revealed)
            .assistive(assistive)
            .copied(copied)
            .accessibility_id(AccessibilityId::new("quick", "password", 0));
        if handlers {
            field.on_copy(|_, _, _| {}).on_toggle_reveal(|_, _, _| {})
        } else {
            field
        }
    }

    fn ring(remaining: u64, masked: bool, assistive: bool) -> OtpRing {
        OtpRing::new(
            "otp",
            std::time::Duration::from_secs(remaining),
            std::time::Duration::from_secs(30),
        )
        .code(zeroize::Zeroizing::new(CODE.to_string()))
        .next_code(zeroize::Zeroizing::new(NEXT_CODE.to_string()))
        .masked(masked)
        .assistive(assistive)
    }

    /// Where `parts` hand any piece of `secrets` to assistive technology,
    /// other than `allowed`: the part, the property and the exact text that
    /// may hold it.
    fn leaks(
        case: &str,
        parts: &[probe::ComponentPart],
        secrets: &[&str],
        allowed: Option<(&str, &str, &str)>,
    ) -> Vec<String> {
        let mut found = Vec::new();
        for part in parts {
            for (property, said) in part.texts() {
                let Some(said) = said else {
                    continue;
                };
                if allowed == Some((part.name, property, said)) {
                    continue;
                }
                for secret in secrets {
                    for piece in giveaways(secret) {
                        if said.contains(&piece) {
                            found.push(format!(
                                "{case}: the {property} of the {} holds {piece:?}: {said:?}",
                                part.name
                            ));
                        }
                    }
                }
            }
        }
        found
    }

    /// No secret and no one-time code reaches assistive technology while it
    /// is hidden or while none runs. Every part the secret field and the
    /// ring build, read one by one, holds neither the secret nor the code,
    /// nor a group of three or four of either, in its label, value,
    /// description, tooltip, author id or placeholder: the field hidden
    /// without and with assistive technology and revealed without it, in
    /// every copy state, with and without its handlers, and the ring masked
    /// without and with assistive technology and unmasked without it, with
    /// and without the next code. Revealed with assistive technology, the
    /// secret is the value of the value button and nothing else, and the
    /// code the value of the timer; the next code never reaches the tree.
    #[gpui::test]
    fn no_secret_in_ax_tree(cx: &mut gpui::TestAppContext) {
        let mut found = Vec::new();
        let mut shown = 0;
        for (revealed, assistive) in [(false, false), (false, true), (true, false), (true, true)] {
            for copied in CopyState::ALL {
                for handlers in [true, false] {
                    let parts = probe::in_frame(cx, move |window, cx| {
                        field(revealed, assistive, copied, handlers).parts(window, cx)
                    });
                    let case = format!(
                        "field, revealed: {revealed}, assistive: {assistive}, {copied:?}, handlers: {handlers}"
                    );
                    let allowed =
                        (revealed && assistive && handlers).then_some(("value", "value", SECRET));
                    if let Some((part, property, value)) = allowed {
                        let holds = parts.iter().any(|read| {
                            read.name == part && read.texts().contains(&(property, Some(value)))
                        });
                        assert!(holds, "{case}: the value button holds the secret");
                        shown += 1;
                    }
                    found.extend(leaks(&case, &parts, &[SECRET], allowed));
                }
            }
        }
        for (masked, assistive) in [(true, false), (true, true), (false, false), (false, true)] {
            for remaining in [18, 3] {
                let parts = probe::in_frame(cx, move |window, cx| {
                    ring(remaining, masked, assistive).parts(window, cx)
                });
                let case =
                    format!("ring, masked: {masked}, assistive: {assistive}, {remaining} s left");
                let allowed = (!masked && assistive).then_some(("timer", "value", "739 114"));
                if allowed.is_some() {
                    let timer = parts
                        .iter()
                        .find(|part| part.name == "timer")
                        .expect("the timer");
                    assert_eq!(timer.node.value(), Some("739 114"), "{case}");
                    shown += 1;
                }
                if !masked && remaining == 3 {
                    assert!(
                        parts.iter().any(|part| part.name == "next_code"),
                        "{case}: the next code shows"
                    );
                }
                found.extend(leaks(&case, &parts, &[CODE, NEXT_CODE], allowed));
            }
        }
        assert_eq!(shown, 3 + 2, "the cases that may show a secret");
        assert!(
            found.is_empty(),
            "a secret reaches assistive technology:\n{}",
            found.join("\n")
        );
    }

    /// The revealed secret and the code are drawn as text without an id:
    /// neither `secret_field.rs` nor `otp.rs` calls `text!`, `Text::new`,
    /// `with_id`, `InteractiveText` or the GPUI Kit's `Label::new`, which
    /// each give a text an id or a node of its own, so GPUI writes the text
    /// as a label with the secret as its value. `Text::new_inaccessible` is
    /// allowed. The files are read through `code_only`, so comments and their
    /// tests do not count; such a call made through a macro or a function in
    /// another file, or written through a renamed import, is past it.
    #[test]
    fn secret_text_has_no_role() {
        let mut found = Vec::new();
        for file in ["secret_field.rs", "otp.rs"] {
            for call in [
                "text!(",
                "Text::new(",
                "with_id(",
                "InteractiveText",
                "Label::new(",
            ] {
                found.extend(probe::calls_in(file, call));
            }
        }
        assert!(
            found.is_empty(),
            "draw a secret as plain or styled text without an id:\n{}",
            found.join("\n")
        );
    }

    /// Neither the secret field nor the ring shows a tooltip. GPUI's tooltip
    /// is a view of its own that opens under the pointer, not a property of
    /// the node `no_secret_in_ax_tree` reads, so a tooltip could show the
    /// secret past the dots and past that test. The files are read through
    /// `code_only`, so comments and their tests do not count; a tooltip set
    /// through a renamed import, a macro or a function in another file is
    /// past it.
    #[test]
    fn the_secret_components_show_no_tooltip() {
        let mut found = Vec::new();
        for file in ["secret_field.rs", "otp.rs"] {
            for call in ["tooltip(", "hoverable_tooltip(", "Tooltip"] {
                found.extend(probe::calls_in(file, call));
            }
        }
        assert!(
            found.is_empty(),
            "a secret component shows a tooltip:\n{}",
            found.join("\n")
        );
    }

    /// How many children the counted containers of `build` held, with the
    /// element `build` returns itself, in a window of its own.
    fn drawn_elements(
        cx: &mut gpui::TestAppContext,
        build: impl Fn() -> AnyElement + 'static,
    ) -> usize {
        let cx = probe::window(cx, move |_, _| build());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        1 + cx.update(|window, _| probe::counted_children(window))
    }

    /// Every element the secret field and the ring put into their containers
    /// is one of their parts, which `no_secret_in_ax_tree` reads: the parts
    /// each lists are as many as the elements it draws, the one it returns
    /// and the children its containers hold, in each state. The count
    /// covers the containers the two files build; what a button of their
    /// own draws inside it is the button's.
    #[gpui::test]
    fn every_rendered_child_is_a_part(cx: &mut gpui::TestAppContext) {
        for revealed in [false, true] {
            for handlers in [false, true] {
                for with_value in [false, true] {
                    let build = move || {
                        let field = field(revealed, false, CopyState::Idle, handlers);
                        if with_value {
                            field
                        } else {
                            SecretField::new("password", SharedString::new_static("Password"))
                                .revealed(revealed)
                        }
                    };
                    let parts =
                        probe::in_frame(cx, move |window, cx| build().parts(window, cx).len());
                    let drawn = drawn_elements(cx, move || build().into_any_element());
                    assert_eq!(
                        drawn, parts,
                        "field, revealed: {revealed}, handlers: {handlers}, value: {with_value}"
                    );
                }
            }
        }
        for masked in [true, false] {
            for remaining in [18, 3] {
                for with_code in [false, true] {
                    let build = move || {
                        if with_code {
                            ring(remaining, masked, false)
                        } else {
                            OtpRing::new(
                                "otp",
                                std::time::Duration::from_secs(remaining),
                                std::time::Duration::from_secs(30),
                            )
                            .masked(masked)
                        }
                    };
                    let parts =
                        probe::in_frame(cx, move |window, cx| build().parts(window, cx).len());
                    let drawn = drawn_elements(cx, move || build().into_any_element());
                    assert_eq!(
                        drawn, parts,
                        "ring, masked: {masked}, {remaining} s left, code: {with_code}"
                    );
                }
            }
        }
    }

    /// Whether a type has `Debug` or `Display`, read without requiring
    /// either: the constant of the inherent impl applies where its bound
    /// holds, the trait's everywhere else.
    struct Printable<T>(std::marker::PhantomData<T>);

    trait NotPrintable {
        const DEBUG: bool = false;
        const DISPLAY: bool = false;
    }

    impl<T> NotPrintable for Printable<T> {}

    impl<T: std::fmt::Debug> Printable<T> {
        const DEBUG: bool = true;
    }

    impl<T: std::fmt::Display> Printable<T> {
        const DISPLAY: bool = true;
    }

    /// Neither the secret field nor the ring can be printed with what it
    /// holds: neither has `Debug` or `Display`, which for the secret's
    /// `Zeroizing<String>` would write the secret, and neither file calls a
    /// macro that prints, writes or logs. The files are read through
    /// `code_only`, so comments and their tests do not count; a macro
    /// called through a renamed import or from another file is past it.
    #[test]
    fn neither_secret_component_can_be_printed() {
        assert_eq!(
            [
                <Printable<zeroize::Zeroizing<String>>>::DEBUG,
                <Printable<String>>::DISPLAY,
            ],
            [true, true],
            "the test sees both traits"
        );
        let printable: Vec<&str> = [
            (
                "SecretField",
                <Printable<SecretField>>::DEBUG,
                <Printable<SecretField>>::DISPLAY,
            ),
            (
                "OtpRing",
                <Printable<OtpRing>>::DEBUG,
                <Printable<OtpRing>>::DISPLAY,
            ),
        ]
        .into_iter()
        .filter(|(_, debug, display)| *debug || *display)
        .map(|(name, _, _)| name)
        .collect();
        assert!(printable.is_empty(), "printable: {printable:?}");

        let mut found = Vec::new();
        for file in ["secret_field.rs", "otp.rs"] {
            for call in [
                "print!(",
                "println!(",
                "eprint!(",
                "eprintln!(",
                "dbg!(",
                "write!(",
                "writeln!(",
                "log::",
                "tracing::",
            ] {
                found.extend(probe::calls_in(file, call));
            }
        }
        assert!(
            found.is_empty(),
            "a secret component prints, writes or logs:\n{}",
            found.join("\n")
        );
    }
}
