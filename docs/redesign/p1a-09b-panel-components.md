# P1a.09b: Panel components

The second part of P1a.09b, on the branch `p1a-09b-components`, which is stacked on the text styles. It brings the first components the new screens are built from: the button, keycaps and shortcut pairs, the text field and the switch, in `src/ui/ds/components/`. Rows and the result list, type tiles, chips, the vault badge, the secret field, the one-time code ring and the panel footer follow on the same branch, and this page grows with them. Nothing in the app uses the components yet, so nothing looks different today.

| Commit | What |
|---|---|
| `fff85d9` | The button in its six kinds and four sizes, the icon button, the copper count, `gpui-base` as a dependency |
| `f088c58` | Keycaps and shortcut pairs; a button can show its shortcut |
| `56518bf` | The text field and the switch on the GPUI Kit controls |
| `0cbf6bd` | A control without a name is refused in release builds too, not only in debug builds |
| `1c606e5` | A shortcut pair without its word is refused the same way |
| `448ceee` | The tests write the English of the button's texts out, so only the button reads its keys; `a11y.copy_password` stays reserved |
| `36ac9e2` | A scan for literal names, descriptions, values and tooltips in the design system |
| `da9609c` | The VoiceOver press test also checks that the press is registered next to the click |

## What every component does

- **A name that is never a literal in the component.** Visible text, accessible names and descriptions come in as parameters, from a text key (`tr!`) or from the caller. `no_literals_in_render_code` in `src/i18n/tests.rs` scans every file under `src/ui/ds` for a literal handed to `.child(` or `aria_label(`, and `no_literal_names_descriptions_or_tooltips` does the same for `accessibility_label(`, `aria_description(`, `aria_value(`, `aria_placeholder(`, `tooltip(` and `Label::new(`. Neither finds one. The only literals a component shows are keycap glyphs and the word "Esc". The descriptions the button adds itself are text keys: `a11y.not_available` and `a11y.in_progress` in `locales/a11y.yml`. The button is the only code that reads them, its tests included, so `no_unused_keys` fails if a literal ever takes the place of either.
- **A name in every build.** A button, icon button, switch or text field with an empty name panics in its constructor, in release builds as well, because VoiceOver can only call such a control "button". So does a shortcut pair with an empty word, which would be keycaps alone.
- **A stable element id** as the first argument of every constructor, and an optional `AccessibilityId`, the identifier assistive technology and UI tests see (`fp.<window>.<component>.<index>`). It is built from names fixed in the code and a number, so an entry title, a user name or a host cannot end up in it.
- **Sizes in points at text size 13**, set with `u()`, so the larger text size (15) scales heights, spacing and text together. Radii, strokes and the focus ring stay in pixels.

## The button

`Button` draws its own look on the unstyled `gpui_base::Button`, which brings focus, Tab, Space, Return and the accessibility role. GPUI Kit does not re-export it, so `gpui-base` is now a direct dependency, pinned at the 0.7.1 that `Cargo.lock` already had. Only the dependency list of `ferrispass` changed in `Cargo.lock`.

| Kind | Surface | Text | For |
|---|---|---|---|
| `Primary` | `button.primary.bg`, copper | `button.primary.text` | the action the view is for |
| `Secondary` | `button.secondary.bg` | `button.secondary.text` | other actions |
| `Quiet` | none until hovered | `button.quiet.text` | small, calm actions; also the icon button |
| `Ghost` | none until hovered | `button.ghost.text` | a verb inside a row |
| `Danger` | `button.danger.bg` | `button.danger.text` | anything that cannot be undone |
| `Large` | copper, as a capsule | `button.primary.text` | the lock screen and onboarding |

The sizes are 24, 28, 32 and the 44 capsule, which stays fully rounded at any text size. `Large` is a capsule by default, every other kind 28. The label is `Headline`.

- **States.** Under the pointer the surface takes the kind's hover color. Held down, the button fades to 0.72 instead of shrinking, because GPUI cannot scale an element. Keyboard focus draws a ring outside the button, 2 px in `focus.ring` with 1 px of space, which takes no room; focus from a click shows none. A disabled button keeps its shape, takes `button.disabled.bg` and `button.disabled.text`, has no handler and is not focusable. A loading button keeps its label, takes no click, and shows a spinner next to the label after 400 ms, so a quick action does not flash one.
- **VoiceOver.** The role is `Button`, or `DefaultButton` for the button Return presses (`default_button()`). The name is the label, so a label says what happens and to what. A description can add to it, such as "Production system"; a disabled button says "not available" there instead, because GPUI has no disabled flag for assistive technology yet, and a loading one "in progress". The keys of a shortcut go into `aria_keyshortcuts`, never into the name.
- **VoiceOver's press.** GPUI answers a press by clicking the middle of the button with a synthetic mouse, which misses when something covers the button. The button registers its own press, which calls the handler directly, as a click from the keyboard would.
- **The default button** shows the Return keycap unless it has a shortcut of its own. A danger button can never be it: Return never triggers something that cannot be undone. Asking for it fails in a debug build and does nothing in a release build.
- **The icon button** (`IconButton`, 28 by 28) shows a glyph instead of a label. It has no constructor without a name, which is what VoiceOver reads, for example `tr!("a11y.copy_password")`. No code reads that key yet, so it stays in `locales/_reserved.txt` until the copy button of the secret field does. Until the app's own glyphs arrive, the icon button takes a GPUI Kit icon.

### One copper surface per view

`Primary` and `Large` are the copper surface, and a view has at most one. In debug builds every copper button that is not disabled reports to a count per window while it renders. After the frame the count is kept, and a frame with more than one copper surface writes a line to stderr. `one_copper_per_view` draws a view with one copper button and all other kinds and holds the count to one; `the_copper_counter_sees_both_copper_kinds` checks that both copper kinds count and a disabled one does not.

## Keycaps and shortcut pairs

A keycap shows one keystroke as a Mac prints it: 18 high, at least 18 wide, radius 4, `Caption`. The modifiers come in the order ⌃⌥⇧⌘, then the key: Return as ⏎, Escape as "Esc", Tab, Delete, the arrows and Page Up and Down as their glyphs, a letter as its capital. GPUI's own text for a keystroke writes `^` for Control, puts ⌘ before ⇧ and spells out `enter`, so the glyphs are set in `keycap.rs`. Keys come in as GPUI key strings (`cmd-shift-c`), the same form the action registry keeps; a string GPUI cannot read fails in a debug build.

A keycap never stands alone. `Keycap` is visible inside `ui::ds` only, and a scan (`keycap_cannot_be_built_outside_ds`) keeps it from being named anywhere else in `src/`, `tests/` or `examples/`, in case the visibility is ever widened. Inside the design system it is built in two places:

- A `Shortcut` pair: keycaps followed by the word for what they do, as in the panel footer ("⌘C Password"). Its constructor takes the word and panics on an empty one, in every build. The pair is plain text for VoiceOver, like the footer it sits in; `keys_text()` gives the keys for the `aria_keyshortcuts` of the element they act on.
- A `Button` with `.shortcut(keys)`: the keycaps follow the label, and the button's `aria_keyshortcuts` holds them. On the copper button a keycap has no surface of its own, only an edge in `keycap.on_accent.border`.

`aria_keyshortcuts` gets the same glyphs as the keycap, so VoiceOver reads them in the user's language.

## The text field

`TextField` keeps the GPUI Kit input, with its editing, selection, input methods and context menu, and changes three things about it:

- **Size.** Kit Medium is 26 high at text size 13 and writes 11.4 px. The field is 28 high and writes `Body`, at 13 as at 15, where it is 32.3 high. `Input::h` only sets the height of a multi-line input, so the wrapper sets it through the style, after the Kit's own.
- **Name.** Without a name of its own, the Kit names a field after its placeholder, and macOS reads the placeholder as name and value at once. The field's label stands above it in `Callout` and is always its name.
- **Errors.** With an error the edge turns `field.border.error`, and a sentence with a glyph below says what is wrong. The Kit input takes no description, so the sentence is a node of its own (a label with the sentence as its value) that VoiceOver reaches right after the field.

Password fields stay `SecretInput`.

## The switch

`Switch` keeps the GPUI Kit switch. The Kit paints a switch that is on in its primary color, which in this theme is copper; this one is `switch.on.bg`, the color of text, so copper stays with the one button that has it. The Kit writes its own label at 11.4 px; here the label stands beside the switch in `Body` and is the switch's name for VoiceOver, and a sentence below in `Callout` can say what changes. The track keeps the Kit's fixed 36 by 20 pixels at both text sizes. A disabled switch gets no handler.

## Checks

| Check | Result |
|---|---|
| `cargo fmt --all --check` | pass |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | pass |
| `cargo test --locked` | pass |
| `cargo audit` | pass, only the warnings that were already allowed |
| Dash scans over the tracked files and the commit messages | no hits |

The headless tests draw each component in a window under the Kit's `Root`, which sets the rem size from the text size as the app does. What a component tells VoiceOver is read from the accessibility node its element writes, because the test platform builds no accessibility tree.

Two things the node cannot show are checked on the source. GPUI lists the press for any element with a click handler, so the node looks the same whether the button registered its own press or not; `button_click_reaches_voiceover` reads the code in `button.rs` where the handler goes to the base button and checks that the press is registered there next to the click. And a node shows a text but not where it came from, which the two literal scans and `no_unused_keys` look at.

| Component | Tests |
|---|---|
| Button | `button_has_role_and_name`, `return_button_is_the_default_button`, `danger_is_never_the_default_button`, `a_button_needs_a_name_in_every_build`, `an_icon_button_needs_a_name_in_every_build`, `disabled_button_has_no_handler_and_says_so`, `loading_keeps_the_label_and_takes_no_click`, `icon_button_requires_a_name`, `button_click_reaches_voiceover`, `pressed_uses_opacity_not_scale`, `the_focus_ring_shows_for_the_keyboard_only`, `sizes_are_the_design_heights` |
| Copper | `one_copper_per_view`, `the_copper_counter_sees_both_copper_kinds` |
| Keycap | `keycap_order_is_control_option_shift_command`, `keys_show_as_a_mac_prints_them`, `a_key_string_gpui_cannot_read_is_loud`, `keycap_cannot_be_built_outside_ds` |
| Shortcut | `shortcut_reads_the_keystroke_text`, `a_shortcut_needs_its_word_in_every_build`, `shortcut_has_a_named_parent`, `a_button_keycap_sits_inside_its_named_button` |
| Text field | `a_text_field_needs_a_name_in_every_build`, `text_field_is_28_high_at_13_and_32_at_15`, `text_field_has_its_own_name_not_the_placeholder`, `an_error_turns_the_edge_carmine_and_is_read_after_the_field` |
| Switch | `a_switch_needs_a_name_in_every_build`, `switch_on_is_never_copper` (all four modes), `switch_has_role_name_and_state` |
| Texts | `no_literal_names_descriptions_or_tooltips`, `the_text_scan_finds_literals_in_code_only`, and in `src/i18n/tests.rs` `no_literals_in_render_code` and `no_unused_keys` |

## Mutation probes

Each probe changed one thing on a clean tree, ran the tests that should notice, and was reverted.

| What was broken | Test that failed |
|---|---|
| A second `Primary` in the view of `one_copper_per_view` | `one_copper_per_view` (`left: 2`, `right: 1`), and the stderr line `one frame drew 2 copper surfaces in a window` |
| ⌘ before ⇧ in `keycap_text` | `keycap_order_is_control_option_shift_command` (`"⌃⌥⌘⇧K"`), `shortcut_reads_the_keystroke_text` (`"⌘⇧C"`), `a_button_keycap_sits_inside_its_named_button` |
| The text field without its own `aria_label` | `text_field_has_its_own_name_not_the_placeholder` (`left: Some("name@example.com")`, `right: Some("User name")`) |
| A disabled or loading button keeping its handler | `disabled_button_has_no_handler_and_says_so` (`!node.supports_action(Click)`), `loading_keeps_the_label_and_takes_no_click` (`left: 1`, `right: 0`) |
| The switch without its own on color, so the Kit's copper | `switch_on_is_never_copper` (`Light, on: true`, with the copper fill in the list) |
| No fade while pressed | `pressed_uses_opacity_not_scale` (alpha `1`) |
| The focus ring for any focus, not only from the keyboard | `the_focus_ring_shows_for_the_keyboard_only` (`a click shows no ring`) |
| `Large` not counted as copper | `the_copper_counter_sees_both_copper_kinds` (`left: 1`, `right: 2`) |
| VoiceOver's press not calling the handler | `button_click_reaches_voiceover` (`left: 0`, `right: 1`) |
| VoiceOver's press no longer registered, only the click handler | `button_click_reaches_voiceover` (`VoiceOver's press is not registered with the click handler`) |
| `SharedString::new_static("not available")` in place of `tr!("a11y.not_available")` | `no_unused_keys` (`a11y.not_available: nothing reads it`) |
| The same for `a11y.in_progress` | `no_unused_keys` (`a11y.in_progress: nothing reads it`) |
| A literal name for the switch, `accessibility_label("Lock when idle")` | `no_literal_names_descriptions_or_tooltips` (`switch.rs:85: "Lock when idle"`); `switch_has_role_name_and_state` stays green, since the name reads the same |
| The shortcut pair without the check for its word | `a_shortcut_needs_its_word_in_every_build` (`test did not panic as expected`) |
| `a11y.copy_password` taken off `locales/_reserved.txt` | `no_unused_keys` (`a11y.copy_password: nothing reads it`) |
| A keycap built in `src/ui/mod.rs` | does not compile: `module keycap is private`, `associated function new is private` |
| The name check in debug builds only (`cargo test --release`) | `a_button_needs_a_name_in_every_build`, `an_icon_button_needs_a_name_in_every_build` (`test did not panic as expected`) |

## Limits

- The copper count sees what renders. A view drawn from GPUI's view cache does not render again and does not report, and a frame without copper leaves the count of the last frame that had some. Nothing in the app uses the view cache.
- The macOS adapter of AccessKit (0.26.3) does not pass `aria_keyshortcuts` on to VoiceOver yet. The buttons set it anyway, so it arrives with the adapter.
- GPUI has no disabled flag for assistive technology. A disabled button says "not available" in its description; the Kit text field and switch take no description, so a disabled field or switch is only greyed out and does nothing.
- The Kit switch and input take an accessibility id only where the Kit offers one: the text field passes it on, the switch has none.
- The literal scans see a literal handed straight to a call. One that reaches a call through a variable is past them; for the button's own descriptions `no_unused_keys` notices, because nothing else reads their keys.
- The test platform builds no accessibility tree and cannot deliver VoiceOver's press, so the tests read each element's node, check in the source that the press is registered next to the click, and call the press directly. The source check reads the code, not what GPUI does with it: that the press arrives on a real Mac, and how every component looks against the component board, need the gallery and a device session. They come later.
