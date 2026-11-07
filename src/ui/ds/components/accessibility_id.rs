//! The identifier a control shows to assistive technology and UI tests.
//!
//! It lives in a file of its own so that its field is private to this file:
//! code anywhere else, the components next to it included, cannot build one
//! as a tuple and has to go through a constructor here, which today is
//! [`AccessibilityId::new`] alone.
//!
//! This is also the one file in `ui::ds` that hands a text to GPUI's
//! `accessibility_id`, through [`AccessibilityId::apply`] and
//! [`AccessibilityId::apply_to_input`]. GPUI's method takes any text, an
//! entry title too, so a component that calls it directly would be past the
//! type; `raw_accessibility_ids_are_applied_in_one_file` keeps such calls
//! out of the rest of the design system.

use gpui::{SharedString, StatefulInteractiveElement};
use gpui_component::input::Input;

/// The identifier a control shows to assistive technology and UI tests,
/// `fp.<window>.<component>.<index>`.
///
/// Unlike an element id, it is visible outside the process. It is built
/// from names fixed in the code and a number, so an entry title, a user name
/// or a host does not end up in it. Three things hold that:
///
/// - The compiler. The field is private to this file, so a tuple
///   construction anywhere else does not compile and `new` is the only
///   constructor. `new` is visible in `ui::ds` only, so a call from anywhere
///   else, `src/app` or `examples/` for instance, does not compile either.
///   It takes `&'static str` for the names, which keeps a borrowed title out
///   but not a leaked one.
/// - `an_accessibility_id_takes_literal_names_only`. It checks that `new` is
///   still visible in `ui::ds` only and takes `&'static str`, and that every
///   call written `AccessibilityId::new` in a file under `src/ui/ds` passes
///   string literals for the window and the component. It leaves out
///   comments and inline `#[cfg(test)] mod tests { }` blocks, and reads the
///   rest of the file, after a `//` inside a string or a `mod tests;` too.
/// - `raw_accessibility_ids_are_applied_in_one_file`. GPUI's
///   `accessibility_id` and the GPUI Kit input's take any text, so a
///   component calling them directly could hand them a title. The test finds
///   every call to a method of that name, and to `set_author_id`, in the
///   files under `src/ui/ds` and allows them in this file only, where
///   [`AccessibilityId::apply`] and [`AccessibilityId::apply_to_input`] hand
///   them the id. It reads the files the same way.
///
/// None of them catches a second constructor added in this file, a call to
/// `new` written another way (through a renamed import or a type alias,
/// `Self::new`, spaces in the path, or a macro), a call to GPUI's method
/// whose name a macro puts together, or a file outside `src/ui/ds` that
/// becomes part of `ui::ds` through `#[path]` or `include!`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessibilityId(SharedString);

impl AccessibilityId {
    /// `window` and `component` are names such as `quick` and `copy`,
    /// `index` tells controls of the same kind in one window apart.
    // Only the tests build an id so far.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(in crate::ui::ds) fn new(
        window: &'static str,
        component: &'static str,
        index: usize,
    ) -> Self {
        Self(format!("fp.{window}.{component}.{index}").into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Puts this id on `element`, where assistive technology and UI tests
    /// read it. The components call this rather than GPUI's
    /// `accessibility_id`, which takes any text.
    pub(in crate::ui::ds) fn apply<E: StatefulInteractiveElement>(&self, element: E) -> E {
        element.accessibility_id(self.0.clone())
    }

    /// Puts this id on the GPUI Kit's text input, which takes it through a
    /// method of its own that takes any text as well.
    pub(in crate::ui::ds) fn apply_to_input(&self, input: Input) -> Input {
        input.accessibility_id(self.0.clone())
    }
}

impl From<AccessibilityId> for SharedString {
    fn from(id: AccessibilityId) -> Self {
        id.0
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use gpui::{AppContext as _, InteractiveElement as _, RenderOnce, TestAppContext, div};
    use gpui_component::input::InputState;

    use super::super::{Button, TextField, probe};
    use super::*;

    #[test]
    fn accessibility_ids_name_the_window_the_component_and_a_number() {
        assert_eq!(
            AccessibilityId::new("quick", "copy", 2).as_str(),
            "fp.quick.copy.2"
        );
        let shared: SharedString = AccessibilityId::new("main", "lock", 0).into();
        assert_eq!(shared.as_ref(), "fp.main.lock.0");
    }

    /// The declaration of `AccessibilityId::new` in this file, on one line,
    /// from its visibility to the end of its parameters.
    fn constructor() -> String {
        let code = probe::code_only(include_str!("accessibility_id.rs"));
        let code = code.split_whitespace().collect::<Vec<_>>().join(" ");
        let start = code
            .find("impl AccessibilityId {")
            .expect("the impl of AccessibilityId");
        let code = &code[start..];
        let name = code.find("fn new(").expect("its constructor");
        let start = code[..name].rfind("pub").unwrap_or(name);
        let close = name + code[name..].find(')').expect("the end of its parameters");
        code[start..=close].to_string()
    }

    /// The types of the parameters in `constructor`, the declaration of
    /// `AccessibilityId::new`.
    fn parameter_types(constructor: &str) -> Vec<String> {
        let (_, parameters) = constructor.split_once("fn new(").expect("its parameters");
        parameters
            .trim_end_matches(')')
            .split(',')
            .map(str::trim)
            .filter(|parameter| !parameter.is_empty())
            .map(|parameter| match parameter.split_once(':') {
                Some((_, kind)) => kind.trim().to_string(),
                None => parameter.to_string(),
            })
            .collect()
    }

    /// What follows a plain string literal and the comma after it, when
    /// `text` starts with them. An escaped quote does not end the literal.
    fn literal_then_comma(text: &str) -> Option<&str> {
        let text = text.trim_start();
        if !text.starts_with('"') {
            return None;
        }
        let end = probe::string_end(text.as_bytes(), 1);
        text.get(end..)?.trim_start().strip_prefix(',')
    }

    /// The calls to `AccessibilityId::new` in `source` that do not pass
    /// string literals for the window and the component, with their line.
    /// Comments and inline test modules are left out.
    fn ids_from_other_than_literals(source: &str) -> Vec<(usize, String)> {
        let code = probe::code_only(source);
        let mut found = Vec::new();
        for (at, call) in code.match_indices("AccessibilityId::new") {
            let literals = code[at + call.len()..]
                .trim_start()
                .strip_prefix('(')
                .and_then(literal_then_comma)
                .and_then(literal_then_comma)
                .is_some();
            if !literals {
                let line = code[..at].matches('\n').count() + 1;
                let text = code.lines().nth(line - 1).unwrap_or_default().trim();
                found.push((line, text.to_string()));
            }
        }
        found
    }

    /// An accessibility id is built from names written in the code, so an
    /// entry title or a user name cannot end up in it. Outside this file
    /// the compiler leaves only the constructors declared here, and `new`
    /// is the only one; outside `ui::ds` a call to it does not compile. The
    /// `&'static str` it takes keeps a borrowed title out, but not a leaked
    /// one: `Box::leak` turns any text into a `&'static str`. So the test
    /// reads the source: `new` is still visible in `ui::ds` only and takes
    /// `&'static str` for the window and the component, and every call
    /// written `AccessibilityId::new` in a file under `src/ui/ds` passes
    /// string literals for both. Comments and inline test modules
    /// (`#[cfg(test)] mod tests { }`) are left out.
    ///
    /// It reads text, not types: a second constructor added in this file, a
    /// call to `new` written another way (through a renamed import or a type
    /// alias, `Self::new`, spaces in the path, or a macro), or a file outside
    /// `src/ui/ds` that becomes part of `ui::ds` through `#[path]` or
    /// `include!`, is past it.
    #[test]
    fn an_accessibility_id_takes_literal_names_only() {
        let constructor = constructor();
        assert!(
            constructor.starts_with("pub(in crate::ui::ds) fn new("),
            "AccessibilityId::new is visible in ui::ds only, where the scan below reads every call: {constructor}"
        );
        assert_eq!(
            parameter_types(&constructor),
            ["&'static str", "&'static str", "usize"],
            "AccessibilityId::new takes names fixed in the code"
        );
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
            for (line, call) in ids_from_other_than_literals(&source) {
                problems.push(format!("{}:{line}: {call}", file.display()));
            }
        }
        assert!(
            problems.is_empty(),
            "an accessibility id takes string literals for the window and the component:\n{}",
            problems.join("\n")
        );
    }

    /// The places in `source` that hand a text to an author id without
    /// going through this file, with their line: a call to a method named
    /// `accessibility_id` or `set_author_id`, or such a method named as a
    /// path, as in `.map(Input::accessibility_id)`. GPUI's
    /// `StatefulInteractiveElement::accessibility_id`, the GPUI Kit input's
    /// method of that name and AccessKit's `Node::set_author_id` all take
    /// any text. A declaration (`fn accessibility_id(`), a field of that
    /// name and a longer name that contains it are no such place. Comments
    /// and inline test modules are left out.
    fn raw_ids(source: &str) -> Vec<(usize, String)> {
        let code = probe::code_only(source);
        let joins = |c: char| c.is_alphanumeric() || c == '_';
        let mut found = Vec::new();
        for name in ["accessibility_id", "set_author_id"] {
            for (at, _) in code.match_indices(name) {
                let before = &code[..at];
                let after = &code[at + name.len()..];
                if before.chars().next_back().is_some_and(joins)
                    || after.chars().next().is_some_and(joins)
                {
                    continue;
                }
                let declared = before
                    .trim_end()
                    .strip_suffix("fn")
                    .is_some_and(|rest| !rest.chars().next_back().is_some_and(joins));
                let called = after.trim_start().starts_with('(');
                let named = before.trim_end().ends_with("::");
                if !declared && (called || named) {
                    let line = before.matches('\n').count() + 1;
                    let text = code.lines().nth(line - 1).unwrap_or_default().trim();
                    found.push((line, text.to_string()));
                }
            }
        }
        found.sort();
        found
    }

    /// GPUI's `accessibility_id` takes any text, so a component could hand
    /// it an entry title past the type. In `ui::ds` only this file calls
    /// it, through `apply` and `apply_to_input`, which hand it an
    /// `AccessibilityId`; the components keep the id as that type until
    /// they apply it. The test reads every file under `src/ui/ds` other
    /// than this one and fails for any call to a method named
    /// `accessibility_id` or `set_author_id`, whatever its receiver, so
    /// inside the design system a component's own builder of that name is
    /// not called either. Comments and inline test modules are left out.
    #[test]
    fn raw_accessibility_ids_are_applied_in_one_file() {
        let ds = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui/ds");
        let this_file = ds.join("components/accessibility_id.rs");
        let mut files = Vec::new();
        probe::rust_files(&ds, &mut files);
        assert!(
            files.len() > 10,
            "the scan found only {} files",
            files.len()
        );
        assert!(files.contains(&this_file), "the scan reads this folder");
        let mut problems = Vec::new();
        for file in files.iter().filter(|file| **file != this_file) {
            let source = std::fs::read_to_string(file).expect("read source");
            for (line, call) in raw_ids(&source) {
                problems.push(format!("{}:{line}: {call}", file.display()));
            }
        }
        assert!(
            problems.is_empty(),
            "apply an accessibility id with AccessibilityId::apply or apply_to_input:\n{}",
            problems.join("\n")
        );
        let here = raw_ids(include_str!("accessibility_id.rs"));
        assert_eq!(
            here.len(),
            2,
            "apply and apply_to_input are the two places here: {here:?}"
        );
    }

    #[test]
    fn the_raw_id_scan_finds_calls_to_the_methods_that_take_any_text() {
        let source = r#"
            div().id("row").accessibility_id(title)
                . accessibility_id (format!("fp.{host}"));
            StatefulInteractiveElement::accessibility_id(div(), user);
            let input = input.when_some(title, Input::accessibility_id);
            node.set_author_id(title.to_string());
            pub fn accessibility_id(mut self, id: AccessibilityId) -> Self {
                self.accessibility_id = Some(id);
                Self { accessibility_id: None }
            }
            fn  accessibility_id(self) {}
            let text = node.author_id();
            div().my_accessibility_id(title);
            div().accessibility_id_of(title);
            // div().accessibility_id(title)
            let row = id.apply(div().id("row"));
#[cfg(test)]
mod tests {
    fn sample() { div().accessibility_id(title); }
}
            let after = button.accessibility_id(AccessibilityId::new("quick", "copy", 0));
        "#;
        assert_eq!(
            raw_ids(source),
            [
                (2, r#"div().id("row").accessibility_id(title)"#.to_string()),
                (
                    3,
                    r#". accessibility_id (format!("fp.{host}"));"#.to_string()
                ),
                (
                    4,
                    "StatefulInteractiveElement::accessibility_id(div(), user);".to_string()
                ),
                (
                    5,
                    "let input = input.when_some(title, Input::accessibility_id);".to_string()
                ),
                (6, "node.set_author_id(title.to_string());".to_string()),
                (
                    21,
                    r#"let after = button.accessibility_id(AccessibilityId::new("quick", "copy", 0));"#
                        .to_string()
                ),
            ]
        );
    }

    /// `apply` and `apply_to_input` put the id where assistive technology
    /// reads it, on GPUI's elements and on the GPUI Kit's input, and the
    /// button and the text field reach it through them.
    #[gpui::test]
    fn an_id_reaches_the_node_through_apply(cx: &mut TestAppContext) {
        let ids = probe::in_frame(cx, |window, cx| {
            let id = AccessibilityId::new("quick", "row", 3);
            let (_, element) = probe::node(id.apply(div().id("row")));
            let state = cx.new(|cx| InputState::new(window, cx));
            let input = id.apply_to_input(Input::new(&state));
            let (_, input) = probe::node(RenderOnce::render(input, window, cx));
            let button = Button::secondary("open", SharedString::new_static("Open"))
                .accessibility_id(AccessibilityId::new("quick", "open", 0))
                .into_base(window, cx);
            let (_, button) = probe::node(RenderOnce::render(button, window, cx));
            let field = TextField::new("user", &state, SharedString::new_static("User name"))
                .accessibility_id(AccessibilityId::new("quick", "user", 0));
            let (_, field) = probe::node(RenderOnce::render(field.input(cx), window, cx));
            [element, input, button, field].map(|node| node.author_id().map(str::to_string))
        });
        assert_eq!(
            ids,
            [
                Some("fp.quick.row.3".to_string()),
                Some("fp.quick.row.3".to_string()),
                Some("fp.quick.open.0".to_string()),
                Some("fp.quick.user.0".to_string()),
            ]
        );
    }

    #[test]
    fn the_id_scan_finds_calls_without_literal_names() {
        let source = r#"
            let a = AccessibilityId::new("quick", "copy", index);
            let b = super::AccessibilityId::new(
                "main",
                "lock",
                0,
            );
            let c = AccessibilityId::new(window, "copy", 0);
            let d = AccessibilityId::new("main", Box::leak(title.into_boxed_str()), 0);
            let e = AccessibilityId::new;
            // AccessibilityId::new(title, "copy", 0)
            /* AccessibilityId::new(title, "copy", 0) */
            let f = ("https://", AccessibilityId::new(window, "copy", 0));
            let g = ('"', "//", AccessibilityId::new(window, "copy", 0));
#[cfg(test)]
mod tests {
    fn sample() { AccessibilityId::new(title, "copy", 0); }
}
            let h = AccessibilityId::new(window, "copy", 0);
#[cfg(test)]
mod tests;
            let i = AccessibilityId::new(window, "copy", 0);
            let j = AccessibilityId::new("main", "\",".replace(',', title).leak(), 0);
        "#;
        assert_eq!(
            ids_from_other_than_literals(source),
            [
                (
                    8,
                    r#"let c = AccessibilityId::new(window, "copy", 0);"#.to_string()
                ),
                (
                    9,
                    r#"let d = AccessibilityId::new("main", Box::leak(title.into_boxed_str()), 0);"#
                        .to_string()
                ),
                (10, "let e = AccessibilityId::new;".to_string()),
                (
                    13,
                    r#"let f = ("https://", AccessibilityId::new(window, "copy", 0));"#.to_string()
                ),
                (
                    14,
                    r#"let g = ('"', "//", AccessibilityId::new(window, "copy", 0));"#.to_string()
                ),
                (
                    19,
                    r#"let h = AccessibilityId::new(window, "copy", 0);"#.to_string()
                ),
                (
                    22,
                    r#"let i = AccessibilityId::new(window, "copy", 0);"#.to_string()
                ),
                (
                    23,
                    r#"let j = AccessibilityId::new("main", "\",".replace(',', title).leak(), 0);"#
                        .to_string()
                ),
            ]
        );
    }
}
