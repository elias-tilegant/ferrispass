# P1a.30: Design gallery

The design gallery is an example that shows every component of the design system in each state it offers, in the four modes, at both text sizes and in both languages, followed by the glyphs. It is the reference for how the components look before a screen uses them, and the place to compare them with the component boards of the design spec. It is not part of the app: the app has no way to open it, and nothing under `src/` changes for it.

| Commit | What |
|---|---|
| `8378af0` | The gallery with a page for each component, its tests, and the `[[example]]` entry that lets `cargo test` run them |
| `1d93212` | The glyphs, the four status shapes and the Kit glyphs the app draws again |

## How to run it

```sh
cargo run --example design_gallery
```

It opens one window, 1120 by 760. On the left is the list of pages, on the right the page, and the bar at the top switches the look:

| Bar | Keys | Choices |
|---|---|---|
| mode | 1, 2, 3, 4 | light, dark, light with more contrast, dark with more contrast |
| text size | - and + | 13 and 15 points |
| language | L | English and German |

Up and Down move through the pages. The keys are off while a text field has focus, since they type there.

A debug build prints a line on stderr for every frame of the button page (see Limits). `cargo run --release --example design_gallery` builds without that count.

## What it shows

Each page has the component's name and its source file at the top, then the states side by side, each with its name under it.

| Page | States |
|---|---|
| button | Each of the six kinds (primary, secondary, quiet, ghost, danger, large) at the four sizes (24, 28, 32 and the 44 capsule), then disabled, loading, with a shortcut, with a shortcut and disabled, and as the default button. A danger button is never the default; its cell says so instead. |
| icon button | As it is, disabled, as a switch off and on, as a menu button with the menu closed and open. |
| shortcut pair | The keys and word of every command in the command table, and one in `Headline`, as the panel footer styles its first line. |
| text field | Empty, empty with a placeholder, with text, with an error, disabled. The fields take typing. |
| switch | Off, on, disabled off and on, with a note, and one that switches on a click. |
| type tile | The five kinds on their own, and each selected in a selected row, the only place a tile is drawn selected. |
| chip | The six tones, each with text only and with a glyph. |
| vault badge | The personal vault, a team in each of the four team colors, and any other vault. Two teams that start with the same letter get two letters. |
| row | The four densities, each with the row as it is (its verb shows under the pointer), selected, selected while the list has no focus, discreet, and selected with its verb. |
| result list | Five rows in the search group with the second selected; the search group with no match and the sentence in place of the list; the list on its own with nothing selected. The result list and the search group share the page, since the list goes into the group. |
| secret field | Hidden without buttons, with the copy button, with the copy and reveal buttons; the flash right after a copy; the tick; revealed with and without buttons; without a value; and one whose buttons answer clicks. |
| one-time code ring | At 16 and 18: masked and shown with 25 seconds left, shown with 4 seconds left and the next code beside it, masked with 4 seconds left, and without a code. |
| panel footer | Each of the seven tones (normal, a production system to choose first, a decision, a decision a day old, offline, matches that look the same, an update ready), with three more shortcuts and with none; then the line about what to connect to first, the menu open, and nothing but the menu button. |
| glyphs | Every glyph in `icons::all()` at 12, 16 and 20 points, with its name under it. |
| status shapes | The four status glyphs side by side at 64 points in the text color, so the shape alone tells them apart, and at 20 points in the color of each status. |
| Kit glyphs | Each Lucide glyph of the Kit that the app draws again under `assets/icons/`: the app's drawing, which every Kit control shows, next to the Kit's own, at 16 and 32 points. |

The keycap has no page of its own. It is not public outside the design system and shows inside the shortcut pairs, the buttons and the rows.

What the components say comes from the text keys wherever the app has one, so it follows the language. The rest is made-up sample data: the entries of a customer called Nordhafen AG in the vault "Team Beratung", the footer's notes on being offline and on an update, and the line about what to connect to first, which have no text keys yet. The one value that looks like a secret is `gallery-9Qz!2026`, and the one-time codes are `123456` and `654321`. None of them opens anything.

The captions under each state are names of components and states in English, written as literals. They name code, not text a user reads, so they do not go through the text keys. The test `no_literals_in_render_code` reads only `src/ui/ds` and `src/ui/windows` and does not see them.

## How the mode, the text size and the language are switched

The gallery starts the way the app starts: it loads the bundled fonts, sets up the Kit, the text keys and the design system with `ds::init`. It skips everything else the app does at start, so it reads no settings, opens no vault and does not touch the clipboard.

The mode, the text size and the language belong to the gallery. A change goes through `ds::apply` with an `Appearance` the gallery builds itself:

```rust
ds::apply(cx, Appearance {
    choice,                       // Light or Dark
    prefs: A11yPrefs { increase_contrast, reduce_motion: false, voiceover: false },
    text_size: ds::text_size_points(text_size),
});
rust_i18n::set_locale(language.code());
```

Increased contrast comes from the mode the gallery shows, not from macOS. `ds::init` at start still reads the macOS accessibility settings once, as it does in the app, and the gallery's own appearance replaces them before the first frame. Nothing is written back, so macOS and the app's settings stay as they are.

The language goes to rust-i18n directly. The app shows English only for now (`GERMAN_ENABLED` is off in `src/i18n/mod.rs`), but `tr!` reads whatever language rust-i18n has, so the gallery shows the German texts without a change in `src/i18n`. The placeholder of the text field that shows one, which the Kit's input keeps as it was given, is set again on every switch.

After a change the window draws again in the new tokens and the new language. The Kit sets the window's rem size from the text size on the next frame, and the components lay out in rems, so everything grows to 15 points and back.

## What the tests hold

The tests are in `examples/design_gallery/tests.rs`. The `[[example]]` entry in `Cargo.toml` sets `test = true`, so `cargo test` runs them with the rest, and CI keeps the gallery building and drawing.

- `every_page_renders_in_every_mode_size_and_language`: opens the gallery headless, under the Kit's `Root` as the window does, and draws every page in each of the four modes, at 13 and 15 points, in English and German, without a panic. Each time the page is laid out, and the mode and text size of the tokens, the window's rem size and the language of the catalogues are those the gallery set. "Cancel" reads "Abbrechen" in German.
- `every_component_module_has_a_page`: every `pub mod` in `src/ui/ds/components/mod.rs` is shown by some page, and no page names a module that is gone. The copper count, a module written inline that only debug builds have, is no component.
- `the_gallery_builds_no_component_of_its_own`: no code in the gallery names a color token of a component (the aliases, the brand tokens and the component tokens in `ColorSet`), the keycap, or what the components are built on: `gpui_base`, `uniform_list`, `canvas`, and the Kit's input, switch, button and spinner. The semantic tokens are left for frames and captions. Comments and strings do not count.
- `the_gallery_reads_nothing`: no code in the gallery names `settings::load`, the app's settings or state, KeePass, `fs::`, `File::`, `env::var`, the clipboard or the pasteboard in any spelling, or the macOS accessibility settings (`a11y_prefs`, `Appearance::current`, `apply_choice`).
- `the_kit_page_holds_every_glyph_the_app_draws_again`: the Kit glyphs page lists exactly the files directly under `assets/icons/`, and the drawing next to each is the Kit's, not the app's.
- `the_keys_switch_the_page_the_mode_the_text_size_and_the_language`: Up and Down move through the pages and stop at the first, 1 to 4 pick the mode, + and - the text size, L the language.
- `the_keys_stay_off_while_a_text_field_has_focus`: each of those keys is bound in the gallery and not inside a text field.

The two scans read every `.rs` file under `examples/design_gallery/` except `tests.rs`, which names what they look for.

## How to check by eye

- Every page in the four modes, at 13 and 15 points, in English and German, next to the component boards. A state that looks different from its board goes on the list for the component's own acceptance page.
- The glyphs at 12, 16 and 20 points in each mode.
- The status shapes at 64 points: four shapes that can be told apart without their color.
- The Kit glyphs: the app's drawings as heavy as the app's own glyphs, and close to the Kit's in shape.
- The secret field that answers clicks: a copy flashes the value, shows the tick and goes back; reveal shows `gallery-9Qz!2026` with its digits and symbols in their colors.

## Limits

- The scans read the gallery's source text. A frame drawn in a semantic token that copies what a component draws, or a read hidden inside a function of the app that the gallery calls, passes them.
- In debug builds the button page draws more than one copper surface: every primary and large button that is not disabled is one. The copper count, which holds a view to one copper surface, prints a line about it on stderr for every frame the page draws, and the loading buttons draw frames for as long as their spinners turn. The count is not there in a release build.
- The test platform loads no files from the app's assets into a window, so the tests lay out the glyph pages and the components without the app's glyphs. What a glyph or a page looks like is for the eye.
- The status shapes at 64 points are the glyph files scaled up, stroke included. They show the shape, not how the glyph looks at the sizes the app uses.
- The gallery sets Reduce Motion and VoiceOver off in its appearance. It does not show the components with Reduce Motion on.
- The `assistive` switch of the secret field and the one-time code ring changes only what VoiceOver reads, not what shows. The gallery leaves it off.
- The pages show what each component draws, not what a screen does with it: selection does not move in the result list, and the footer's menu button opens nothing. The text fields, the switch and the secret field's buttons answer input; nothing else does.

## Not done

- Pages for components that do not exist yet, among them the banner, the HUD, the segment, the copper thread, the field card, the sidebar entry, the stepper, the version card and the command palette. Each component brings its page when it is built.
- The comparison with the boards happens by eye. What differs goes to the acceptance page of P1a.09b, or to P1a.17 and P1a.26, not into the gallery.

The agents spent about 3 hours on this, including review and fixes.
