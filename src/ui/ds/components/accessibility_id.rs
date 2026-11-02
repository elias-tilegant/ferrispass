//! The identifier a control shows to assistive technology and UI tests.
//!
//! It lives in a file of its own so that its field is private to this file:
//! code anywhere else, the components next to it included, cannot build one
//! as a tuple and has to go through a constructor here, which today is
//! [`AccessibilityId::new`] alone.

use gpui::SharedString;

/// The identifier a control shows to assistive technology and UI tests,
/// `fp.<window>.<component>.<index>`.
///
/// Unlike an element id, it is visible outside the process. It is built
/// from names fixed in the code and a number, so an entry title, a user name
/// or a host does not end up in it. Two things hold that:
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
///
/// Neither catches a second constructor added in this file, a call to `new`
/// written another way (through a renamed import or a type alias,
/// `Self::new`, spaces in the path, or a macro), or a file outside
/// `src/ui/ds` that becomes part of `ui::ds` through `#[path]` or
/// `include!`.
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
}

impl From<AccessibilityId> for SharedString {
    fn from(id: AccessibilityId) -> Self {
        id.0
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::super::probe;
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
