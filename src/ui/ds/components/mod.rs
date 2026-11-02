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

mod accessibility_id;
pub mod button;
mod focus_ring;
pub(in crate::ui::ds) mod keycap;
pub mod shortcut;
pub mod switch;
pub mod text_field;

pub use accessibility_id::AccessibilityId;
pub use button::{Button, ButtonKind, ButtonSize, IconButton};
pub use shortcut::Shortcut;
pub use switch::Switch;
pub use text_field::TextField;

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
    use std::ops::Range;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;

    use gpui::{
        AnyElement, App, AppContext as _, Bounds, Context, Edges, Element as _, Hsla, IntoElement,
        Modifiers, ParentElement as _, Pixels, Render, Role, Styled as _, TestAppContext,
        VisualTestContext, Window, accesskit, canvas, div, px,
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
        let element = element.into_element();
        let role = element.a11y_role();
        let mut node = accesskit::Node::new(role.unwrap_or(Role::Unknown));
        element.write_a11y_info(&mut node);
        (role, node)
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
            ]
        );
    }
}
