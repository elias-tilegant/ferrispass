//! Checks that need the real macOS text system: the bundled mono font loads
//! and sets every digit at the same width, Medium of the system font is a
//! face of its own rather than Regular again, and without the bundled fonts
//! secrets and the Kit theme fall back to Menlo.
//!
//! The test platform behind `#[gpui::test]` has a text system that knows no
//! fonts and gives every font the same id, so these checks build the macOS
//! platform headless instead, without a window, and run on its text system.
//! macOS lets that platform be created on the main thread only, and libtest
//! runs every test on a thread of its own. So this file has `harness = false`
//! in `Cargo.toml` and runs its checks from `main`, one after the other.
//!
//! Like libtest, it takes test names as filters, `--skip <name>`, `--list`,
//! and `--ignored` or `--include-ignored` for checks marked as ignored.

#[cfg(target_os = "macos")]
fn main() {
    macos::run();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    println!("The text system checks run on macOS only.");
}

#[cfg(target_os = "macos")]
mod macos {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::Arc;

    use ferrispass::app::assets::font_bytes;
    use ferrispass::app::settings::{TextSize, ThemeChoice};
    use ferrispass::ui::ds;
    use ferrispass::ui::ds::typography::{
        self, Family, MEDIUM, MONO_FALLBACK, MONO_FAMILY, MONO_MEDIUM, TextStyle, UI_FAMILY,
    };
    use gpui::{
        App, Font, FontFeatures, FontId, FontRun, FontWeight, PlatformTextSystem, SharedString,
        TestAppContext, TestDispatcher, px,
    };
    use gpui_component::Theme;

    /// One check: its name, the reason it is ignored if it is, and the code.
    struct Check {
        name: &'static str,
        ignored: Option<&'static str>,
        run: fn(&mut App, &Arc<dyn PlatformTextSystem>),
    }

    const CHECKS: [Check; 3] = [
        Check {
            name: "atkinson_loads_and_digits_are_tabular",
            ignored: None,
            run: atkinson_loads_and_digits_are_tabular,
        },
        Check {
            name: "system_font_medium_is_its_own_face",
            ignored: None,
            run: system_font_medium_is_its_own_face,
        },
        Check {
            name: "without_the_mono_font_the_kit_gets_menlo",
            ignored: None,
            run: without_the_mono_font_the_kit_gets_menlo,
        },
    ];

    /// Set for the copy of this binary that starts without the bundled fonts.
    const WITHOUT_BUNDLED_FONTS: &str = "FERRISPASS_TEXT_SYSTEM_WITHOUT_BUNDLED_FONTS";

    /// What the command line asks for.
    #[derive(Default)]
    struct Selection {
        filters: Vec<String>,
        skip: Vec<String>,
        list: bool,
        only_ignored: bool,
        include_ignored: bool,
    }

    impl Selection {
        fn from_args() -> Self {
            let mut selection = Self::default();
            let mut args = std::env::args().skip(1);
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--list" => selection.list = true,
                    "--ignored" => selection.only_ignored = true,
                    "--include-ignored" => selection.include_ignored = true,
                    "--skip" => selection.skip.extend(args.next()),
                    // Options with a value that says nothing about which
                    // checks to run.
                    "--format" | "--color" | "--test-threads" | "--logfile" | "-Z" => {
                        args.next();
                    }
                    _ if arg.starts_with('-') => {}
                    _ => selection.filters.push(arg),
                }
            }
            selection
        }

        fn matches(&self, check: &Check) -> bool {
            (self.filters.is_empty()
                || self.filters.iter().any(|f| check.name.contains(f.as_str())))
                && !self.skip.iter().any(|s| check.name.contains(s.as_str()))
        }

        fn runs(&self, check: &Check) -> bool {
            match check.ignored {
                Some(_) => self.only_ignored || self.include_ignored,
                None => !self.only_ignored,
            }
        }
    }

    pub fn run() {
        if std::env::var_os(WITHOUT_BUNDLED_FONTS).is_some() {
            start_without_bundled_fonts();
            return;
        }
        let selection = Selection::from_args();
        let checks: Vec<&Check> = CHECKS.iter().filter(|c| selection.matches(c)).collect();
        if selection.list {
            for check in &checks {
                println!("{}: test", check.name);
            }
            return;
        }

        println!("\nrunning {} tests", checks.len());
        let (mut passed, mut failed, mut ignored) = (Vec::new(), Vec::new(), Vec::new());
        let to_run: Vec<&Check> = checks
            .into_iter()
            .filter(|check| {
                if selection.runs(check) {
                    true
                } else {
                    println!(
                        "test {} ... ignored, {}",
                        check.name,
                        check.ignored.unwrap_or("")
                    );
                    ignored.push(check.name);
                    false
                }
            })
            .collect();

        if !to_run.is_empty() {
            let platform = gpui_platform::current_platform(true);
            let text = platform.text_system();
            let cx =
                TestAppContext::build_with_text_system(TestDispatcher::new(0), None, text.clone());
            // What the app does at startup, in the same order. That order is
            // repeated here, not taken from `app::run`; a unit test next to
            // the text styles holds `app::run` to it.
            cx.update(|cx| {
                cx.text_system()
                    .add_fonts(font_bytes())
                    .expect("add the bundled fonts");
                gpui_component::init(cx);
                ds::init(ThemeChoice::Light, TextSize::Standard, cx);
            });
            for check in to_run {
                let outcome = catch_unwind(AssertUnwindSafe(|| {
                    cx.update(|cx| (check.run)(cx, &text));
                }));
                match outcome {
                    Ok(()) => {
                        println!("test {} ... ok", check.name);
                        passed.push(check.name);
                    }
                    Err(_) => {
                        println!("test {} ... FAILED", check.name);
                        failed.push(check.name);
                    }
                }
            }
        }

        let result = if failed.is_empty() { "ok" } else { "FAILED" };
        if !failed.is_empty() {
            println!("\nfailures:");
            for name in &failed {
                println!("    {name}");
            }
        }
        println!(
            "\ntest result: {result}. {} passed; {} failed; {} ignored\n",
            passed.len(),
            failed.len(),
            ignored.len()
        );
        if !failed.is_empty() {
            std::process::exit(101);
        }
    }

    fn font(family: &str, weight: FontWeight, features: FontFeatures) -> Font {
        Font {
            weight,
            features,
            ..gpui::font(SharedString::from(family.to_string()))
        }
    }

    /// The width of `text` laid out on one line at `size` points.
    fn width(system: &Arc<dyn PlatformTextSystem>, text: &str, size: f32, id: FontId) -> f32 {
        let runs = [FontRun {
            len: text.len(),
            font_id: id,
        }];
        f32::from(system.layout_line(text, px(size), &runs).width)
    }

    fn atkinson_loads_and_digits_are_tabular(cx: &mut App, system: &Arc<dyn PlatformTextSystem>) {
        assert!(
            cx.text_system()
                .all_font_names()
                .iter()
                .any(|name| name == MONO_FAMILY),
            "the text system does not list {MONO_FAMILY} after the fonts were added"
        );
        assert_eq!(
            typography::mono_family(cx),
            MONO_FAMILY,
            "the startup check fell back"
        );
        assert_eq!(TextStyle::Secret.family().name(), MONO_FAMILY);
        assert_eq!(
            Theme::global(cx).mono_font_family.as_ref(),
            MONO_FAMILY,
            "the Kit theme's mono font"
        );

        // Both bundled weights load by name, each as a face of its own.
        let plain = FontFeatures::default();
        let regular = system
            .font_id(&font(MONO_FAMILY, FontWeight::NORMAL, plain.clone()))
            .expect("Regular of the mono font");
        let medium = system
            .font_id(&font(MONO_FAMILY, MONO_MEDIUM, plain))
            .expect("Medium of the mono font");
        assert_ne!(regular, medium, "Medium of the mono font is Regular again");

        // The one-time code style, with its tabular figures: every digit, and
        // the letters a digit could be taken for, at the same width.
        let style = TextStyle::Code;
        assert_eq!(style.family(), Family::Mono);
        assert!(style.tabular());
        let id = system
            .font_id(&font(
                style.family().name(),
                style.weight(),
                typography::tabular_figures(),
            ))
            .expect("the font of the code style");
        let sample = "0123456789O1lI|";
        let zero = width(system, "0", style.size(), id);
        assert!(zero > 0.0, "0 has no width");
        for character in sample.chars() {
            let each = width(system, &character.to_string(), style.size(), id);
            assert!(
                (each - zero).abs() < 0.001,
                "{character} is {each} wide, 0 is {zero}"
            );
        }
        let line = width(system, "0O1lI|", style.size(), id);
        assert!(
            (line - 6.0 * zero).abs() < 0.01,
            "0O1lI| is {line} wide, six times 0 is {}",
            6.0 * zero
        );
    }

    fn system_font_medium_is_its_own_face(cx: &mut App, system: &Arc<dyn PlatformTextSystem>) {
        let plain = FontFeatures::default();
        // `.SystemUIFont` loads as itself. A family GPUI cannot find ends up
        // in Helvetica, the first family of GPUI's own fallback list a Mac
        // has.
        let regular = system
            .font_id(&font(UI_FAMILY, FontWeight::NORMAL, plain.clone()))
            .expect("the system font did not load by name");
        let helvetica =
            cx.text_system()
                .resolve_font(&font("Helvetica", FontWeight::NORMAL, plain.clone()));
        assert_ne!(regular, helvetica, "the system font is Helvetica");
        // The Kit renames the UI family to the one it lands on, and writes
        // that to its log, only when the two differ. It kept the name.
        assert_eq!(Theme::global(cx).font_family.as_ref(), UI_FAMILY);

        // The way a view asks for a face: resolve, with the fallback list.
        let face = |weight: FontWeight| {
            cx.text_system()
                .resolve_font(&font(UI_FAMILY, weight, plain.clone()))
        };
        let regular = face(FontWeight::NORMAL);
        let medium = face(MEDIUM);
        let semibold = face(FontWeight::SEMIBOLD);
        assert_ne!(medium, regular, "weight {} lands on Regular", MEDIUM.0);
        assert_ne!(medium, semibold, "weight {} lands on Semibold", MEDIUM.0);
        assert_ne!(regular, semibold);

        // And so does every UI text style.
        for style in TextStyle::ALL {
            if style.family() != Family::Ui {
                continue;
            }
            let expected = match style.weight() {
                weight if weight == FontWeight::NORMAL => regular,
                weight if weight == MEDIUM => medium,
                weight if weight == FontWeight::SEMIBOLD => semibold,
                weight => panic!(
                    "{style:?} has weight {}, which this check does not know",
                    weight.0
                ),
            };
            assert_eq!(face(style.weight()), expected, "{style:?}");
        }
    }

    /// The mono family is decided once per process, and in this one the
    /// bundled fonts are in. So the check starts this binary again, without
    /// them, and that copy runs [`start_without_bundled_fonts`].
    fn without_the_mono_font_the_kit_gets_menlo(_: &mut App, _: &Arc<dyn PlatformTextSystem>) {
        let program = std::env::current_exe().expect("the path of this test binary");
        let output = std::process::Command::new(program)
            .env(WITHOUT_BUNDLED_FONTS, "1")
            .output()
            .expect("start this test binary again");
        assert!(
            output.status.success(),
            "the start without the bundled fonts failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// The startup of an app whose bundled fonts did not load: the same steps
    /// as in `run`, without adding the fonts. Secrets fall back to Menlo, and
    /// so does the Kit theme after every change. Menlo is also the Kit's own
    /// default for mono text, which the Kit replaces when a Mac does not have
    /// it, so this is the one place the family the Kit ends up with shows. A
    /// failed assertion ends the process with an error, which the check
    /// reports with this process's stderr.
    fn start_without_bundled_fonts() {
        let platform = gpui_platform::current_platform(true);
        let cx = TestAppContext::build_with_text_system(
            TestDispatcher::new(0),
            None,
            platform.text_system(),
        );
        cx.update(|cx| {
            assert!(
                !cx.text_system()
                    .all_font_names()
                    .iter()
                    .any(|name| name == MONO_FAMILY),
                "{MONO_FAMILY} is installed on this Mac, so it cannot go missing"
            );
            gpui_component::init(cx);
            ds::init(ThemeChoice::Light, TextSize::Standard, cx);
            assert_eq!(
                typography::mono_family(cx),
                MONO_FALLBACK,
                "the startup check"
            );
            assert_eq!(TextStyle::Secret.family().name(), MONO_FALLBACK);
            for choice in [ThemeChoice::Light, ThemeChoice::Dark] {
                ds::apply_choice(choice, TextSize::Standard, None, cx);
                assert_eq!(
                    Theme::global(cx).mono_font_family.as_ref(),
                    MONO_FALLBACK,
                    "the Kit theme's mono font, {choice:?}"
                );
            }
        });
    }
}
