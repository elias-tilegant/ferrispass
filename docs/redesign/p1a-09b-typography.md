# P1a.09b-a: Text styles and the mono font

The first part of P1a.09b, on the `rebuild` branch. It brings the fonts and the text styles the new screens are built with. Nothing in the app uses the text styles yet; the components that do come next, on their own branch.

| Commit | What |
|---|---|
| `16e2286` | The mono font bundled, `ui::ds::typography`, the fallback, the Kit theme set on the new mono font |
| `505b1cf` | The checks on the real macOS text system |
| `c1e26d2` | Before the startup check, mono text names the bundled font instead of guessing the fallback |

## Which fonts ship, and why

- **The UI font is the macOS system font**, named `.SystemUIFont`. GPUI maps that name to the system font. Asking for "SF Pro", "SF Pro Text" or the CSS font stack the old theme used ends up in Helvetica instead: GPUI does not find those names and takes the first family of its own fallback list that a Mac has. The GPUI Kit theme set already named `.SystemUIFont` before this step.
- **Secrets are set in Atkinson Hyperlegible Mono**: passwords, generated passwords, one-time codes and the large type for typing a secret by hand. It is a monospace font made to tell apart characters that look alike, such as `0` and `O` or `1`, `l` and `I`. The app bundles Regular and Medium, the two weights the design uses, as the unchanged static TTFs of the upstream release. The upstream repository also has a variable font, but GPUI gets only one weight out of it. `design/README.md` names the repository, the commit and the Git blob of each file, so a copy can be checked, and says how to move to a newer release. The license is the SIL Open Font License 1.1, without a Reserved Font Name, in `assets/licenses/OFL-AtkinsonHyperlegibleMono.txt`. `scripts/build-mac.sh` copies every file in `assets/licenses/` into the app bundle.
- **JetBrains Mono stays** for the old screens, which name it directly, until they are rebuilt.

The GPUI Kit theme set (`assets/theme/ferrispass.theme.json`) now names Atkinson Hyperlegible Mono as its mono font. The regenerated file differs from the old one in those four lines only, one per theme.

## The text styles

`src/ui/ds/typography.rs` has the type scale as `TextStyle`. The numbers are the `typography` section of `design/tokens.json`, and `typography_matches_tokens` holds the code to it: sizes, line heights, weights, families, tabular figures, and the list of styles itself.

| Style | Size / line | Weight | Family | Used for |
|---|---|---|---|---|
| `Display` | 26 / 32 | Semibold | UI | |
| `Title1` | 22 / 28 | Semibold | UI | |
| `Search` | 20 / 26 | Regular | UI | the search field of the quick panel |
| `Title2` | 17 / 22 | Semibold | UI | |
| `Title3` | 15 / 20 | Semibold | UI | |
| `Headline` | 13 / 16 | Semibold | UI | buttons, section headers, the selected row |
| `RowTitle` | 13 / 16 | Medium | UI | a row title while the row is not selected |
| `Body` | 13 / 18 | Regular | UI | |
| `Callout` | 12 / 16 | Regular | UI | a second line, the caption of a field |
| `Caption` | 11 / 14 | Medium | UI | labels, counters, keycaps |
| `Secret` | 15 / 20 | Regular | mono | a secret, in groups of four |
| `Generator` | 20 / 26 | Regular | mono | a generated password |
| `Code` | 22 / 28 | Medium | mono | a one-time code, with tabular figures |
| `Typing` | 48 / 56 | Regular | mono | large type for typing a secret by hand |

Sizes are points at text size 13. `u(points)` turns a size into rems (`points / 13`), and the Kit sets the rem size from the text size, so the larger text size (15) scales text, heights and spacing together. `.text_style(TextStyle::Body)` on any element sets family, size, line height and weight, and for `Code` the `tnum` feature. A number that changes in another style, such as a counter in `Caption`, adds `typography::tabular_figures()` itself. Nothing is smaller than 11 points, and `nothing_below_11_points` checks the code and the JSON.

A family set by hand comes from the same module. `font_families_in_new_modules_come_from_typography` reads every `font_family` call in `src/ui/ds` and `src/ui/windows`, with its whole argument, and accepts only `typography::UI_FAMILY`, `typography::Family::Ui.name()`, `typography::Family::Mono.name()` or `typography::mono_family(cx)`. `MONO_FAMILY` and `MONO_FALLBACK` on their own are not accepted, since they would skip the startup check below.

## The weight that is really Medium

In the system font, `FontWeight(500)`, which CSS calls Medium, draws Regular. GPUI maps the weights of the system font onto CSS weights piecewise, and SF Medium sits near 530 on that scale. So `typography::MEDIUM` is `FontWeight(530.0)`, and `RowTitle` and `Caption` use it. In Atkinson Hyperlegible Mono, Medium is at 500 as usual, so `Code` uses `MONO_MEDIUM`, which is 500. `medium_is_530_for_the_system_font_and_500_for_mono` checks that no style asks either family for a weight it has no face for.

Whether 530 really hits a face of its own can only be seen with the macOS text system, so `system_font_medium_is_its_own_face` asks it (see below): 400, 530 and 600 resolve to three different faces, and every UI text style to the face its weight names.

## When the mono font is missing

GPUI draws a family it cannot find in Helvetica, which is no font for a password. So `ui::ds::init` checks at startup, after the app added its fonts, whether the text system lists Atkinson Hyperlegible Mono. If it does not, the app writes one line to stderr and sets mono text in Menlo, which every Mac has:

```
ferrispass: the bundled font Atkinson Hyperlegible Mono did not load, using Menlo
```

The answer holds for the rest of the process. `typography::mono_family(cx)` returns it, `.text_style()` uses it for the mono styles, and every theme change puts it into the Kit theme, which otherwise would name the font from the theme set whether it loaded or not. Menlo only ever comes from that check: before it has run, the mono styles name the bundled font, as the design and the theme set do. `ds::init` runs it before the first window opens, so nothing is drawn before it, and after the app added its fonts: `the_app_adds_its_fonts_before_the_design_system_starts` reads `app::run` and fails when anything from `ui::ds` comes first. If adding the bundled fonts fails, the app now writes that to stderr too, instead of dropping the error.

Menlo is also the GPUI Kit's own default for mono text, and the Kit swaps its default for another installed mono font when the text system does not list it. A Mac has Menlo, so the Kit keeps it. The test platform has no fonts at all, so there the Kit theme ends up on `.SystemUIFont`, and the unit tests can only check that the bundled font is not left in the Kit theme; the exact family is checked on the real text system.

The app has no log file yet, so both lines are only visible when the app is started from a terminal (`FerrisPass.app/Contents/MacOS/FerrisPass`).

## Checks on the real text system

The test platform behind `#[gpui::test]` knows no fonts at all and gives every font the same id, so it cannot tell whether a font loaded or which face a weight lands on. `tests/text_system.rs` builds the macOS platform headless instead, without a window, adds the bundled fonts and runs `ui::ds::init` the way the app does at startup:

- `atkinson_loads_and_digits_are_tabular`: the text system lists Atkinson Hyperlegible Mono, the startup check did not fall back, the Kit theme names it, Regular and Medium load as two faces, and in the `Code` style every digit and the characters `O`, `1`, `l`, `I` and `|` are as wide as `0`.
- `system_font_medium_is_its_own_face`: `.SystemUIFont` loads by name and is not Helvetica, the Kit kept the name, and 400, 530 and 600 are three faces. The Kit renames the UI family, and writes a line about it to its log, only when the font lands on another family. The app installs no logger for the Kit, so a Helvetica in that line would never be seen; this check is where it shows.
- `without_the_mono_font_the_kit_gets_menlo`: the mono family is decided once per process, so this check starts the test binary a second time, and that copy runs the startup without adding the bundled fonts. There the startup check falls back, the `Secret` style names Menlo, and after a change to light and to dark the Kit theme names exactly Menlo. If Atkinson Hyperlegible Mono is installed on the Mac itself, it cannot go missing, and the check says so and fails.

macOS lets the platform be created on the main thread only, and libtest runs each test on a thread of its own, even with `--test-threads=1`. So the file has `harness = false` in `Cargo.toml` and runs its checks from its own `main`. It prints the same lines as libtest and takes test names, `--skip` and `--list` the same way, so `cargo test golden` leaves it alone. The startup steps are repeated there rather than taken from `app::run`, which is why the order in `app::run` has a test of its own.

## Checks

| Check | Result |
|---|---|
| `cargo fmt --all --check` | pass |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | pass |
| `cargo test --locked` | pass |
| `cargo audit` | pass, only the warnings that were already allowed |
| Dash scans over the tracked files and the commit messages | no hits |

## Mutation probes

Each probe broke one thing on purpose and ran the tests that should notice:

| What was broken | Test that failed |
|---|---|
| `MEDIUM` set to `FontWeight(500.0)` | `system_font_medium_is_its_own_face` (`weight 500 lands on Regular`), `medium_is_530_for_the_system_font_and_500_for_mono`, `typography_matches_tokens` |
| The startup check returning the bundled font without looking at `all_font_names()` | `mono_family_falls_back_only_when_the_font_is_missing`, `a_missing_mono_font_never_reaches_the_kit` |
| The startup check always falling back | `atkinson_loads_and_digits_are_tabular` (`the startup check fell back`) |
| Menlo before the startup check has run | `the_fallback_comes_only_from_the_check` |
| The Kit theme left with the theme set's mono font after a switch | `a_missing_mono_font_never_reaches_the_kit` |
| The Medium file of the mono font left out of `font_bytes` | `atkinson_loads_and_digits_are_tabular` (`Medium of the mono font is Regular again`) |
| `Code` without tabular figures | `typography_matches_tokens` |
| `Caption` at 10 points | `nothing_below_11_points`, `typography_matches_tokens` |
| A `"JetBrains Mono"` constant in `kit_theme.rs` | `no_mono_literal_in_new_modules`, naming the line |
| A `to_uppercase()` call in `kit_theme.rs` | `no_uppercase_in_new_modules`, naming the line |
| The Kit theme given `.SystemUIFont` instead of the fallback, only when the startup check fell back | `without_the_mono_font_the_kit_gets_menlo` (`the Kit theme's mono font, Light`, `.SystemUIFont` where `Menlo` was expected); the unit test `a_missing_mono_font_never_reaches_the_kit` stays green, see above |
| A `.font_family(gpui::SharedString::from("Helvetica"))` call in `kit_theme.rs` | `font_families_in_new_modules_come_from_typography`, naming the line |
| `Family::name` for mono naming the bundled font without reading the startup check | `text_style_sets_family_size_line_weight_and_figures` (`Secret`) |
| `ui::ds::init` moved ahead of `add_fonts` in `app::run` | `the_app_adds_its_fonts_before_the_design_system_starts` (`src/app/mod.rs:94 reaches ui::ds before src/app/mod.rs:97 adds the fonts`); the checks on the real text system stay green |

The checks on the real text system also run on the CI runner and pass there, so none of them is marked as ignored.

## Not checked yet

- How the text styles look on screen. No screen uses them before the components do.
- The startup lines on stderr in the built app. They need the app started from a terminal on a Mac. The fallback itself runs on the real text system, but nothing reads the line it writes.
- A font family set other than through `font_family`: `.font()` with a `gpui::Font`, `gpui::font()`, or a font or text style refinement built field by field. The scan reads `font_family` calls only.
- A call into `ui::ds` that `app::run` makes through a helper of its own. The order test reads the text of `app::run`, not what it calls.
