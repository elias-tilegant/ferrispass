//! The components the new screens are built from.
//!
//! Each one draws its look from the tokens and the text styles and gives
//! VoiceOver what it needs: a role, a name that the caller passes in or that
//! comes from a text key, never from a literal in here, and a stable element
//! id. The button is drawn on the unstyled `gpui_base::Button`; the text field
//! and the switch wrap the GPUI Kit controls, whose editing and toggling
//! behavior is worth keeping.

pub mod button;
pub(in crate::ui::ds) mod keycap;
pub mod shortcut;
pub mod switch;
pub mod text_field;

pub use button::{Button, ButtonKind, ButtonSize, IconButton};
pub use shortcut::Shortcut;
pub use switch::Switch;
pub use text_field::TextField;

use gpui::SharedString;

/// The identifier a control shows to assistive technology and UI tests,
/// `fp.<window>.<component>.<index>`.
///
/// Unlike an element id, it is visible outside the process. It is built
/// from names fixed in the code and a number, so an entry title, a user name
/// or a host can never end up in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessibilityId(SharedString);

impl AccessibilityId {
    /// `window` and `component` are names such as `quick` and `copy`,
    /// `index` tells controls of the same kind in one window apart.
    pub fn new(window: &'static str, component: &'static str, index: usize) -> Self {
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
    use std::path::{Path, PathBuf};
    use std::rc::Rc;

    use gpui::{
        AnyElement, App, AppContext as _, Context, Element as _, IntoElement, ParentElement as _,
        Render, Role, Styled as _, TestAppContext, VisualTestContext, Window, accesskit, canvas,
        div,
    };
    use gpui_component::{Root, Theme};

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
}

#[cfg(test)]
mod tests {
    use std::path::Path;

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

    /// Calls that hand a text to VoiceOver or show it, beside the `.child(`
    /// and `aria_label(` that `no_literals_in_render_code` looks at.
    const TEXT_CALLS: &[&str] = &[
        "accessibility_label(",
        "aria_description(",
        "aria_value(",
        "aria_placeholder(",
        "tooltip(",
        "Label::new(",
    ];

    /// The string literals `source` hands straight to one of `TEXT_CALLS`,
    /// with their line. Comments and the test module are left out.
    fn literal_texts(source: &str) -> Vec<(usize, String)> {
        let code = source
            .split("\n#[cfg(test)]\nmod tests")
            .next()
            .unwrap_or_default();
        let code: Vec<&str> = code
            .lines()
            .map(|line| line.split("//").next().unwrap_or_default())
            .collect();
        let code = code.join("\n");
        let mut found = Vec::new();
        for call in TEXT_CALLS {
            for (at, _) in code.match_indices(call) {
                let part_of_a_longer_name = code[..at]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_');
                let argument = code[at + call.len()..].trim_start();
                let literal = argument.starts_with('"')
                    || argument.starts_with("r\"")
                    || argument.starts_with("r#");
                if literal && !part_of_a_longer_name {
                    let line = code[..at].matches('\n').count() + 1;
                    let first_line = argument.lines().next().unwrap_or_default();
                    let text = first_line.trim_end_matches([')', ',', ';']);
                    found.push((line, text.to_string()));
                }
            }
        }
        found.sort();
        found
    }

    /// A name, description, value or tooltip in `ui::ds` comes from a
    /// parameter or a text key, never from a literal in the code.
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
#[cfg(test)]
mod tests {
    fn sample() { div().aria_description("in a test"); }
}
        "##;
        assert_eq!(
            literal_texts(source),
            [
                (2, "\"not available\"".to_string()),
                (3, "r#\"Copy\"#".to_string()),
                (10, "\"Server\"".to_string()),
            ]
        );
    }
}
