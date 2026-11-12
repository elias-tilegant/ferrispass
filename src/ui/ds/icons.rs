//! The glyphs of the design system.
//!
//! Each glyph is a line drawing of its own under `assets/icons/fp/`, on a
//! 16 point grid: `viewBox="0 0 16 16"`, a stroke of
//! [`dimension::STROKE_ICON`](super::tokens::dimension::STROKE_ICON) in
//! `currentColor`, round ends and corners, no fill. Dots are `h.01` segments
//! with a stroke of 2. Glyphs that stand for a state are filled: `star-fill`
//! for a favourite, and the five `print-*` shapes that make up the print of a
//! version. A file has no transforms and no CSS, and stays under 2 KB.
//!
//! [`glyph`] draws one with `gpui::svg()`, which paints the drawing as a mask
//! in the text color. The caller picks the color from a token with
//! `text_color`; without a text color GPUI paints nothing.
//!
//! Kit components draw Lucide glyphs on a 24 grid with a stroke of 2, which is
//! 1.33 at 16 points. `AppAssets` asks the app's files before the Kit's, so
//! the twelve Lucide glyphs the Kit's components use lie redrawn under
//! `assets/icons/` with the names the Kit loads, at the same weight as these.
//! They are Lucide's shapes moved onto the 16 grid, derived works under the
//! licence in `assets/licenses/ISC-Lucide.txt`.

use gpui::{SharedString, Styled as _, Svg, svg};
use gpui_component::IconNamed;

use super::typography::u;

macro_rules! glyphs {
    ($($variant:ident => $name:literal,)*) => {
        /// A glyph of the design system, one per file under
        /// `assets/icons/fp/`. Kit components take it wherever they take an
        /// icon, through [`IconNamed`].
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum FpIcon {
            $($variant,)*
        }

        impl FpIcon {
            const ALL: &'static [FpIcon] = &[$(FpIcon::$variant,)*];

            /// The name of the file, without its folder and extension.
            pub fn name(self) -> &'static str {
                match self {
                    $(FpIcon::$variant => $name,)*
                }
            }

            fn asset_path(self) -> &'static str {
                match self {
                    $(FpIcon::$variant => concat!("icons/fp/", $name, ".svg"),)*
                }
            }
        }
    };
}

glyphs! {
    // Kinds of entry.
    TypeSap => "type-sap",
    TypeRemote => "type-remote",
    TypeWeb => "type-web",
    TypeServer => "type-server",
    TypeNote => "type-note",
    // Vaults and the lock. The shell with a keyhole is the lock; a vault of
    // one's own is the shell without one.
    VaultPersonal => "vault-personal",
    VaultTeam => "vault-team",
    VaultOther => "vault-other",
    LockClosed => "lock-closed",
    LockOpen => "lock-open",
    Shell => "shell",
    // Status. Each has a shape of its own, so color never carries it alone.
    StatusOk => "status-ok",
    StatusAttention => "status-attention",
    StatusDecision => "status-decision",
    StatusDanger => "status-danger",
    // Navigation.
    AllEntries => "all-entries",
    Star => "star",
    StarFill => "star-fill",
    Clock => "clock",
    Customer => "customer",
    Security => "security",
    Trash => "trash",
    Folder => "folder",
    Tag => "tag",
    Sidebar => "sidebar",
    // Actions.
    Search => "search",
    Copy => "copy",
    Eye => "eye",
    EyeOff => "eye-off",
    OpenExternal => "open-external",
    Plus => "plus",
    Check => "check",
    Close => "close",
    More => "more",
    ChevronLeft => "chevron-left",
    ChevronRight => "chevron-right",
    ChevronDown => "chevron-down",
    Dice => "dice",
    Keyboard => "keyboard",
    Clipboard => "clipboard",
    Fingerprint => "fingerprint",
    Sliders => "sliders",
    Person => "person",
    File => "file",
    Cloud => "cloud",
    Key => "key",
    Link => "link",
    // The print of a version, filled.
    PrintTriangle => "print-triangle",
    PrintCircle => "print-circle",
    PrintSquare => "print-square",
    PrintDiamond => "print-diamond",
    PrintStar => "print-star",
}

impl IconNamed for FpIcon {
    fn path(self) -> SharedString {
        SharedString::new_static(self.asset_path())
    }
}

/// Every glyph, in the order of the list above.
pub fn all() -> &'static [FpIcon] {
    FpIcon::ALL
}

/// The three sizes a glyph is drawn at, in points at text size 13. They grow
/// with the text size like everything else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconSize {
    /// In chips and badges only.
    Chip12,
    /// Everywhere else.
    Standard16,
    /// On the lock screen, in onboarding and on large buttons.
    Large20,
}

impl IconSize {
    pub fn points(self) -> f32 {
        match self {
            Self::Chip12 => 12.0,
            Self::Standard16 => 16.0,
            Self::Large20 => 20.0,
        }
    }
}

/// `icon` at `size`, as a mask that GPUI fills with the text color. Set it
/// with `text_color` from a token: without one, nothing is painted.
pub fn glyph(icon: FpIcon, size: IconSize) -> Svg {
    svg()
        .path(icon.asset_path())
        .size(u(size.points()))
        .flex_none()
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::{Path, PathBuf};

    use gpui::{
        AnyElement, AssetSource as _, Hsla, InteractiveElement as _, IntoElement as _,
        ParentElement as _, TestAppContext, px,
    };
    use gpui_component::IconName;

    use super::super::components::probe;
    use super::super::components::{IconButton, Tile};
    use super::super::tokens::{LIGHT, dimension};
    use super::*;
    use crate::app::assets::AppAssets;

    /// The stroke of a dot, an `h.01` segment.
    const DOT_STROKE: f32 = 2.0;

    /// The largest file a glyph may be.
    const MAX_BYTES: usize = 2048;

    /// The Lucide glyphs Kit components draw, which the app draws again at
    /// its own weight.
    fn kit_glyphs() -> [IconName; 12] {
        [
            IconName::Eye,
            IconName::EyeOff,
            IconName::Close,
            IconName::ChevronDown,
            IconName::ChevronRight,
            IconName::Check,
            IconName::Inbox,
            IconName::Loader,
            IconName::Search,
            IconName::ExternalLink,
            IconName::Minus,
            IconName::Plus,
        ]
    }

    fn icons_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icons")
    }

    /// The `.svg` files directly in `dir`, by name without the extension.
    fn svg_files(dir: &Path) -> BTreeMap<String, PathBuf> {
        std::fs::read_dir(dir)
            .unwrap_or_else(|error| panic!("read {}: {error}", dir.display()))
            .map(|entry| entry.expect("directory entry").path())
            .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "svg"))
            .map(|path| {
                let stem = path.file_stem().expect("a file name");
                (stem.to_string_lossy().into_owned(), path)
            })
            .collect()
    }

    /// One element of an SVG file: its tag and its attributes in order.
    struct Element {
        tag: String,
        attributes: Vec<(String, String)>,
    }

    impl Element {
        fn get(&self, name: &str) -> Option<&str> {
            self.attributes
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
        }
    }

    /// The elements of `text`, in order, or what in it is not a plain
    /// element: text between the tags, a comment, a declaration, CSS.
    fn elements(text: &str) -> Result<Vec<Element>, String> {
        let mut found = Vec::new();
        let mut rest = text.trim();
        while !rest.is_empty() {
            let Some(after) = rest.strip_prefix('<') else {
                let end = rest.find('<').unwrap_or(rest.len());
                return Err(format!("text between tags: {:?}", &rest[..end]));
            };
            let end = after
                .find('>')
                .ok_or_else(|| format!("a tag that never closes: {after:?}"))?;
            let tag = &after[..end];
            rest = after[end + 1..].trim_start();
            if tag.starts_with('/') {
                continue;
            }
            if tag.starts_with('!') || tag.starts_with('?') {
                return Err(format!("not an element: <{tag}>"));
            }
            let tag = tag.trim_end_matches('/').trim();
            let (name, mut attributes_text) =
                tag.split_once(char::is_whitespace).unwrap_or((tag, ""));
            let mut attributes = Vec::new();
            loop {
                attributes_text = attributes_text.trim_start();
                if attributes_text.is_empty() {
                    break;
                }
                let (key, value) = attributes_text.split_once("=\"").ok_or_else(|| {
                    format!("an attribute without a quoted value: {attributes_text:?}")
                })?;
                let (value, after) = value
                    .split_once('"')
                    .ok_or_else(|| format!("an attribute that never closes: {value:?}"))?;
                attributes.push((key.trim().to_string(), value.to_string()));
                attributes_text = after;
            }
            found.push(Element {
                tag: name.to_string(),
                attributes,
            });
        }
        Ok(found)
    }

    /// Whether `d` draws nothing but dots: `M x y h.01`, once or more.
    fn only_dots(d: &str) -> bool {
        let tokens = path_tokens(d);
        !tokens.is_empty()
            && tokens.chunks(4).all(|dot| {
                dot.len() == 4
                    && dot[0] == "M"
                    && dot[1].parse::<f32>().is_ok()
                    && dot[2].parse::<f32>().is_ok()
                    && dot[3] == "h.01"
            })
    }

    /// The commands and numbers of path data, with the spacing and commas
    /// that do not change the path taken out. A dot's `h.01` stays one token.
    fn path_tokens(d: &str) -> Vec<String> {
        let mut tokens: Vec<String> = Vec::new();
        let mut number = String::new();
        let flush = |number: &mut String, tokens: &mut Vec<String>| {
            if !number.is_empty() {
                tokens.push(std::mem::take(number));
            }
        };
        for c in d.chars() {
            match c {
                c if c.is_ascii_alphabetic() && c != 'e' => {
                    flush(&mut number, &mut tokens);
                    tokens.push(c.to_string());
                }
                '-' if !number.ends_with('e') => {
                    flush(&mut number, &mut tokens);
                    number.push(c);
                }
                '.' if number.contains('.') => {
                    flush(&mut number, &mut tokens);
                    number.push(c);
                }
                c if c.is_ascii_digit() || c == '.' || c == 'e' => number.push(c),
                _ => flush(&mut number, &mut tokens),
            }
        }
        flush(&mut number, &mut tokens);
        // `h` and the `.01` after it form a dot.
        let mut joined = Vec::new();
        let mut tokens = tokens.into_iter().peekable();
        while let Some(token) = tokens.next() {
            if token == "h"
                && tokens
                    .peek()
                    .is_some_and(|next| next.parse::<f32>() == Ok(0.01))
            {
                tokens.next();
                joined.push("h.01".to_string());
            } else {
                joined.push(token);
            }
        }
        joined
    }

    fn is_stroke(value: &str, width: f32) -> bool {
        value.parse::<f32>() == Ok(width)
    }

    /// What in the file at `path` breaks the grid, one line each.
    fn grid_problems(path: &Path) -> Vec<String> {
        let name = path.file_name().expect("a file name").to_string_lossy();
        let text = std::fs::read_to_string(path).expect("read the glyph");
        let mut problems = Vec::new();
        let mut problem = |what: String| problems.push(format!("{name}: {what}"));
        if text.len() > MAX_BYTES {
            problem(format!("{} bytes, more than {MAX_BYTES}", text.len()));
        }
        let elements = match elements(&text) {
            Ok(elements) => elements,
            Err(error) => {
                problem(error);
                return problems;
            }
        };
        let Some(root) = elements.first().filter(|root| root.tag == "svg") else {
            problem("does not start with <svg>".to_string());
            return problems;
        };
        for (attribute, expected) in [
            ("viewBox", "0 0 16 16"),
            ("stroke", "currentColor"),
            ("stroke-linecap", "round"),
            ("stroke-linejoin", "round"),
        ] {
            if root.get(attribute) != Some(expected) {
                problem(format!(
                    "{attribute} is {:?}, not {expected:?}",
                    root.get(attribute)
                ));
            }
        }
        if !root
            .get("stroke-width")
            .is_some_and(|width| is_stroke(width, dimension::STROKE_ICON))
        {
            problem(format!(
                "stroke-width is {:?}, not {}",
                root.get("stroke-width"),
                dimension::STROKE_ICON
            ));
        }
        if !matches!(root.get("fill"), Some("none" | "currentColor")) {
            problem(format!("fill is {:?}", root.get("fill")));
        }
        for element in &elements {
            if !matches!(
                element.tag.as_str(),
                "svg"
                    | "g"
                    | "path"
                    | "circle"
                    | "rect"
                    | "ellipse"
                    | "line"
                    | "polyline"
                    | "polygon"
            ) {
                problem(format!("<{}> is not a shape", element.tag));
            }
            for (key, value) in &element.attributes {
                match key.as_str() {
                    "fill" | "stroke" | "color" if value != "currentColor" && value != "none" => {
                        problem(format!("<{}> {key}=\"{value}\"", element.tag));
                    }
                    "style" | "class" | "transform" => {
                        problem(format!("<{}> has {key}", element.tag));
                    }
                    _ => {}
                }
            }
            let d = element.get("d").unwrap_or_default();
            let dots = element.tag == "path" && only_dots(d);
            let has_dot = path_tokens(d).iter().any(|token| token == "h.01");
            if element.tag != "svg"
                && let Some(width) = element.get("stroke-width")
            {
                let allowed = is_stroke(width, dimension::STROKE_ICON)
                    || (dots && is_stroke(width, DOT_STROKE));
                if !allowed {
                    problem(format!(
                        "<{}> stroke-width=\"{width}\" on {d:?}",
                        element.tag
                    ));
                }
            }
            if has_dot
                && !(dots
                    && element
                        .get("stroke-width")
                        .is_some_and(|width| is_stroke(width, DOT_STROKE)))
            {
                problem(format!(
                    "dots go in a path of their own with a stroke of {DOT_STROKE}: {d:?}"
                ));
            }
        }
        problems
    }

    /// Every glyph keeps to the grid of the design system.
    #[test]
    fn glyph_grid() {
        let files = svg_files(&icons_dir().join("fp"));
        assert!(!files.is_empty(), "no glyphs found");
        let problems: Vec<String> = files
            .values()
            .flat_map(|path| grid_problems(path))
            .collect();
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    /// `CamelCase` to `kebab-case`, the way the variants are named after
    /// their files.
    fn kebab(variant: &str) -> String {
        let mut name = String::new();
        for c in variant.chars() {
            if c.is_ascii_uppercase() && !name.is_empty() {
                name.push('-');
            }
            name.push(c.to_ascii_lowercase());
        }
        name
    }

    /// Each file has its variant and each variant its file, named alike, and
    /// the app finds the file at the variant's path.
    #[test]
    fn glyph_enum_complete() {
        let files = svg_files(&icons_dir().join("fp"));
        let names: BTreeSet<&str> = all().iter().map(|icon| icon.name()).collect();
        assert_eq!(names.len(), all().len(), "a glyph is listed twice");
        let on_disk: BTreeSet<&str> = files.keys().map(String::as_str).collect();
        let without_variant: Vec<_> = on_disk.difference(&names).collect();
        let without_file: Vec<_> = names.difference(&on_disk).collect();
        assert!(
            without_variant.is_empty() && without_file.is_empty(),
            "files without a variant: {without_variant:?}, variants without a file: {without_file:?}"
        );
        let assets = AppAssets::new();
        for &icon in all() {
            assert_eq!(kebab(&format!("{icon:?}")), icon.name(), "{icon:?}");
            let path = icon.path();
            assert_eq!(path.as_ref(), format!("icons/fp/{}.svg", icon.name()));
            let loaded = assets
                .load(&path)
                .unwrap_or_else(|error| panic!("{path}: {error}"))
                .unwrap_or_else(|| panic!("{path} is not embedded"));
            let file = std::fs::read(&files[icon.name()]).expect("read the glyph");
            assert_eq!(loaded.as_ref(), file.as_slice(), "{path}");
        }
    }

    /// The shape of a glyph as a sequence of commands: each element's kind,
    /// and for a path its commands without their numbers, in either case.
    /// Two glyphs that differ only in their numbers count as one shape here.
    fn shape(path: &Path) -> Vec<String> {
        let text = std::fs::read_to_string(path).expect("read the glyph");
        elements(&text)
            .expect("the glyph parses")
            .into_iter()
            .skip(1)
            .map(|element| match element.get("d") {
                Some(d) => {
                    let commands: String = path_tokens(d)
                        .iter()
                        .filter(|token| token.starts_with(|c: char| c.is_ascii_alphabetic()))
                        .map(|token| token.to_ascii_uppercase())
                        .collect::<Vec<_>>()
                        .join(" ");
                    format!("{} {commands}", element.tag)
                }
                None => element.tag,
            })
            .collect()
    }

    /// Color never says a status alone: the four status glyphs differ in
    /// their shapes, compared by the commands that draw them.
    #[test]
    fn status_shapes_differ() {
        let dir = icons_dir().join("fp");
        let status = [
            FpIcon::StatusOk,
            FpIcon::StatusAttention,
            FpIcon::StatusDecision,
            FpIcon::StatusDanger,
        ];
        let shapes: Vec<(FpIcon, Vec<String>)> = status
            .iter()
            .map(|&icon| (icon, shape(&dir.join(format!("{}.svg", icon.name())))))
            .collect();
        for (at, (first, first_shape)) in shapes.iter().enumerate() {
            for (second, second_shape) in &shapes[at + 1..] {
                assert_ne!(
                    first_shape, second_shape,
                    "{first:?} and {second:?} have the same shape"
                );
            }
        }
    }

    /// The files directly under `assets/icons/` are exactly the Kit glyphs
    /// listed above, at the paths the Kit loads them from. The Kit ships each
    /// in the bundle the app falls back to, and the app's own file wins.
    #[test]
    fn kit_overrides_match_the_kit_names() {
        let files = svg_files(&icons_dir());
        let on_disk: BTreeSet<String> = files
            .keys()
            .map(|name| format!("icons/{name}.svg"))
            .collect();
        let kit: BTreeSet<String> = kit_glyphs()
            .into_iter()
            .map(|name| name.path().to_string())
            .collect();
        assert_eq!(
            on_disk, kit,
            "files under assets/icons/ and the Kit's names"
        );
        let app = AppAssets::new();
        for path in &kit {
            let shipped = gpui_kit_assets::Assets
                .load(path)
                .expect("the Kit's assets load")
                .unwrap_or_else(|| panic!("the Kit does not ship {path}"));
            let drawn = app
                .load(path)
                .expect("the app's assets load")
                .expect("the app has the file");
            let name = path.trim_start_matches("icons/");
            let file = std::fs::read(icons_dir().join(name)).expect("read the file");
            assert_eq!(
                drawn.as_ref(),
                file.as_slice(),
                "{path} is not the app's file"
            );
            assert_ne!(drawn.as_ref(), shipped.as_ref(), "{path} is the Kit's own");
        }
    }

    /// The redrawn Kit glyphs keep to the same grid as the app's own.
    #[test]
    fn kit_overrides_are_on_the_sixteen_grid() {
        let files = svg_files(&icons_dir());
        assert_eq!(files.len(), kit_glyphs().len());
        let problems: Vec<String> = files
            .values()
            .flat_map(|path| grid_problems(path))
            .collect();
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    /// Every glyph turns into a mask through GPUI's own SVG renderer: some
    /// of it covered, not all of it. `glyph` hands GPUI the color the caller
    /// set and lays the glyph out at its size, which grows with the text
    /// size. The test platform loads no assets, so what the mask looks like
    /// in a window is for the eye.
    #[gpui::test]
    fn a_glyph_renders_as_a_mask_in_the_text_color(cx: &mut TestAppContext) {
        let assets = AppAssets::new();
        let renderer = cx.update(|cx| cx.svg_renderer());
        for &icon in all() {
            let bytes = assets
                .load(&icon.path())
                .expect("the glyph loads")
                .expect("the glyph is embedded");
            let image = renderer
                .render_single_frame(&bytes, 1.0)
                .unwrap_or_else(|error| panic!("{icon:?} does not render: {error}"));
            let pixels = image.as_bytes(0).expect("one frame");
            let alpha: Vec<u8> = pixels
                .as_chunks::<4>()
                .0
                .iter()
                .map(|pixel| pixel[3])
                .collect();
            let covered = alpha.iter().filter(|&&a| a > 0).count();
            assert!(covered > 0, "{icon:?} draws nothing");
            assert!(covered < alpha.len(), "{icon:?} covers its whole square");
        }

        let color: Hsla = LIGHT.text_primary.hsla;
        let mut drawn = glyph(FpIcon::TypeSap, IconSize::Standard16).text_color(color);
        assert_eq!(drawn.style().text.color, Some(color));

        let cx = probe::window(cx, move |_, _| {
            gpui::div()
                .flex()
                .children([
                    glyph(FpIcon::TypeSap, IconSize::Chip12)
                        .text_color(color)
                        .debug_selector(|| "glyph-12".into()),
                    glyph(FpIcon::TypeSap, IconSize::Standard16)
                        .text_color(color)
                        .debug_selector(|| "glyph-16".into()),
                    glyph(FpIcon::TypeSap, IconSize::Large20)
                        .text_color(color)
                        .debug_selector(|| "glyph-20".into()),
                ])
                .into_any_element()
        });
        for (selector, size) in [
            ("glyph-12", IconSize::Chip12),
            ("glyph-16", IconSize::Standard16),
            ("glyph-20", IconSize::Large20),
        ] {
            let bounds = cx.debug_bounds(selector).expect(selector);
            let side = cx.update(|window, _| u(size.points()).to_pixels(window.rem_size()));
            for length in [bounds.size.width, bounds.size.height] {
                assert!(
                    (length - side).abs() <= px(1.),
                    "{selector}: {length:?}, not {side:?}"
                );
            }
        }
    }

    /// A tile and an icon button take a glyph as they take a Kit icon.
    #[gpui::test]
    fn fp_icons_fit_the_tile_and_the_icon_button(cx: &mut TestAppContext) {
        let cx = probe::window(cx, |_, _| -> AnyElement {
            gpui::div()
                .child(Tile::new(FpIcon::TypeSap))
                .child(IconButton::new(
                    "copy",
                    FpIcon::Copy,
                    SharedString::new_static("Copy password"),
                ))
                .into_any_element()
        });
        assert!(cx.debug_bounds("fp-tile").is_some(), "the tile is drawn");
    }
}
