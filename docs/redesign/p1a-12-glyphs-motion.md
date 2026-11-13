# P1a.12: Glyphs, motion and text size

The design system gets its own glyphs, a single place where motion takes its time, and the text size as a setting. Nothing on screen uses the glyphs or the motion tokens yet apart from what the Kit draws; the quick access panel and the design gallery will be the first to use them. The text size can be set today and reaches the Kit theme and the tokens.

| Commit | What |
|---|---|
| `e201f6b` | The glyph set under `assets/icons/fp/`, `ui::ds::icons` with `FpIcon` and `glyph` |
| `083bbde` | Twelve of the Lucide glyphs the Kit's components draw, redrawn at the same weight |
| `a3cb542` | `ui::ds::motion`, the input kind noted by the root handlers, the Kit's motion values |
| `ed9ed85` | The text size setting and its row in Settings, General |

## Where the glyphs live and how they are drawn

Each glyph is one SVG file under `assets/icons/fp/`, named in kebab case with a prefix for its group (`type-`, `vault-`, `lock-`, `status-`, `print-`). All of them follow one grid:

- `viewBox="0 0 16 16"`, a stroke of 1.5 (the `stroke.icon` token) in `currentColor`, round ends and corners, no fill.
- Dots are `h.01` segments in a path of their own with a stroke of 2.
- Glyphs for a state are filled: `star-fill` for a set favourite, and the five `print-*` shapes that make up the print of a version.
- No transforms, no CSS, no text, at most 2 KB per file.

There are 52 glyphs in six groups:

| Group | Glyphs |
|---|---|
| Kinds of entry | `type-sap`, `type-remote`, `type-web`, `type-server`, `type-note` |
| Vaults and the lock | `vault-personal`, `vault-team`, `vault-other`, `lock-closed`, `lock-open`, `shell` |
| Status | `status-ok`, `status-attention`, `status-decision`, `status-danger` |
| Navigation | `all-entries`, `star`, `star-fill`, `clock`, `customer`, `security`, `trash`, `folder`, `tag`, `sidebar` |
| Actions | `search`, `copy`, `eye`, `eye-off`, `open-external`, `plus`, `check`, `close`, `more`, `chevron-left`, `chevron-right`, `chevron-down`, `dice`, `keyboard`, `clipboard`, `fingerprint`, `sliders`, `person`, `file`, `cloud`, `key`, `link` |
| Print of a version | `print-triangle`, `print-circle`, `print-square`, `print-diamond`, `print-star` |

The drawings are the ones in the glyph library and the boards of the design spec, copied as they are, with two changes for the grid: dots that sat in a path with lines moved into a path of their own at a stroke of 2, and the turn of the tipped lid in `lock-open` worked into its coordinates instead of a group transform. Where the spec names a glyph and its form without a drawing, it is built from the parts it names: `status-ok` is the ring with the check from `security`, `status-danger` a near-regular octagon with an exclamation mark, `eye-off` the eye with a slash, `vault-personal` the shell without its keyhole and `vault-other` the same shell dashed. The print shapes are the board's filled shapes, moved onto the grid and given the same stroke so their corners are round. The four status glyphs have four shapes, so a status never rests on color alone.

`lock-closed` and `shell` are the same drawing: the spec uses the shell with a keyhole both as the lock and as the small mark. They are two names so that the mark can change on its own later.

In code, `ui::ds::icons::FpIcon` has one variant per file. The variants, the list behind `icons::all()` and the asset paths come out of one macro, so a variant cannot miss its path. `FpIcon` implements the Kit's `IconNamed`, so it goes wherever a Kit component takes an icon: `Tile::new(FpIcon::TypeSap)` and `IconButton::new(id, FpIcon::Copy, name)` work unchanged. `icons::glyph(icon, size)` draws one with `gpui::svg()` at `IconSize::Chip12`, `Standard16` or `Large20`, as rems, so a glyph grows with the text size. GPUI paints an SVG as a mask filled with the text color, so the caller sets `text_color` from a token; without one, nothing is painted.

The old screens keep their own glyphs under `assets/icons/app/` and `src/ui/icons.rs` until they are rebuilt.

## The Kit's glyphs

Kit components draw Lucide glyphs on a 24 grid with a stroke of 2, which comes out at 1.33 at 16 points, lighter than the app's own. `AppAssets` asks the app's files before the Kit's, so a file at the path the Kit loads replaces the Kit's glyph wherever a Kit component draws it. Twelve files directly under `assets/icons/` do that, for twelve of the glyphs the Kit's components use:

- `eye`, `eye-off`: the password field
- `close`: dialogs, sheets, the clear button of a field
- `chevron-down`, `chevron-right`: selects and submenus
- `check`, `inbox`: checkboxes and menus, the empty list of a select
- `loader`: spinners and buttons that load
- `search`, `external-link`: lists and menus
- `minus`, `plus`: number fields

Each is Lucide's own shape moved onto the 16 grid, 2 to 22 on the 24 grid onto the live area 1.75 to 14.25, with a stroke of 1.5. They are derived works of Lucide, under the licence in `assets/licenses/ISC-Lucide.txt`. The Kit draws many more Lucide glyphs, among them `asterisk`, `chevron-left`, `chevron-up`, `calendar`, `ellipsis` and `info`; they stay as the Kit draws them until a screen needs them.

## Motion

`ui::ds::motion::duration(token, input, cx)` is where the app's own animations take their time. The tokens and what they are for:

| Token | Time | For |
|---|---|---|
| `Instant` | 0 ms | selection, hover, filtering, the quick access panel |
| `Micro` | 120 ms | a copy by click, the dice |
| `Short` | 180 ms | popovers, the HUD, a one-time code changing |
| `Medium` | 240 ms | unlocking, a version moving into the history, the shell closing |
| `Long` | 320 ms | onboarding and Ferris's moments |

Two rules sit on top of the table:

- **After a key press every duration is zero.** `Input::Keyboard` always gets 0 ms. Someone at the keyboard waits for nothing.
- **With Reduce Motion on, a transition is at most a 120 ms fade, and nothing moves.** `duration` caps every token at `REDUCED_FADE`, and `springs_allowed(cx)` is false, so a caller shows the end state or fades to it instead of sliding or springing. Rings and threads that count time keep running: they show information, and their timers are not durations.

Reduce Motion is read from `cx.reduce_motion()`. gpui-base reads the macOS setting only once at startup, so `ds::apply` already sets it from `A11yPrefs` every time it runs. `Input::Pointer` and `Input::System` (a timer, the app starting, a Touch ID prompt that succeeded) get the token's time.

The two springs are `spring_soft()`, 350 ms with a damping ratio of 1.0, for the sidebar, columns and the strength meter, and `spring_firm()`, 300 ms with 0.9, for things that snap into place. In gpui-base the duration is the period of the undamped spring. Next to them are the hold times (the HUD's success 1.5 s, the copy button's check 1.2 s), the delay before progress shows (400 ms) and the update rates of the copper thread (12 Hz), the one-time code ring (10 Hz) and the menu bar fill (1 Hz).

The kind of the last input comes from the old main window's root handlers in `src/ui/legacy/shell/render.rs`. They already see every key press, click, scroll and pointer move to count it for the auto-lock; next to each count they now call `motion::note_input`, and `motion::last_input(cx)` reads it back. The value is a global that is only written when the kind changes.

`Moment` is the gate for Ferris's moments. `may_run(occasion, input, cx)` answers once per occasion: the first start and the first success once at all, waking up once per unlock, after `begin_unlock`. The caller passes the input that finished the step: a Touch ID success is `Input::System` even when Return opened the prompt, so Ferris wakes up; Return in the password field is `Input::Keyboard`, so he does not, and shows himself awake at once. Asking uses the occasion up whether the moment runs or not. Nothing calls `Moment` yet; it comes in with Ferris.

The Kit's components animate on their own and never ask `duration`. Some of them, such as the switch, the checkbox and the border of an input group, read the motion values in the Kit theme, and after every theme switch `kit_theme::switch` puts those on the same steps: 0, 120, 180 and 240 ms instead of the Kit's 0, 120, 180 and 280, the firm spring for controls and the soft one for moves (keeping the Kit's settling tolerance for pixels), and 8 points as the medium distance, in rems. The Kit's easing curves stay. Others, among them dialogs, sheets, popovers and tooltips, keep fixed times of their own. The keyboard rule reaches no Kit animation: a Kit control animates after a key press as well.

No code outside `src/ui/ds/` may call `with_animation` or `with_spring`, so the app's own animations outside the design system go through this module. Today no code outside it calls either, the old screens included, so the scan has no exceptions. The scan does not look inside `src/ui/ds/`.

## The text size setting

Settings, General has a row "Text size" under the theme with Standard and Large. Large sets the base text at 15 points instead of 13. The two numbers are the `TEXT_SIZE_STANDARD` and `TEXT_SIZE_LARGE` tokens; `ds::text_size_points` maps the setting onto them, and nothing in `src/ui/ds` or `src/ui/windows` sets 13 or 15 pixels itself.

The size reaches the Kit theme (`font_size`) and the design tokens (`cx.ds().text_size`) through `ds::apply_choice` and `ds::init`, which now take it next to the theme choice. The Kit sets each window's rem size from the theme on the next frame, and the new screens lay out text, heights and spacing in rems (`typography::u`), so they grow with it and the change applies without a restart. The theme row, the theme shortcut and a change of the macOS appearance all pass the size that is set along. At startup the size comes from `settings.json`.

The setting is stored as `"text_size": "standard"` or `"large"`. A file without the field reads as standard, and older builds ignore it.

## What the tests show

Glyphs, in `src/ui/ds/icons.rs`:

- `glyph_grid`: every file under `assets/icons/fp/` keeps to the grid above. A small reader goes through the file's elements and fails on anything but shapes, on text, comments or declarations, on `style`, `class` or `transform`, on a color other than `currentColor` or `none`, on a stroke other than 1.5 (2 only on a path of nothing but dots, and every dot in such a path), and on more than 2 KB.
- `glyph_enum_complete`: files and variants match both ways, each variant is named after its file, and the app loads the file at the variant's path.
- `status_shapes_differ`: the four status glyphs differ pairwise in their elements and path commands, numbers left out.
- `kit_overrides_match_the_kit_names`: the files directly under `assets/icons/` are exactly the paths of the twelve `gpui_component::IconName` variants above. The Kit ships each in the bundle the app falls back to, and `AppAssets` hands out the app's file, not the Kit's.
- `kit_overrides_are_on_the_sixteen_grid`: the same grid for those twelve.
- `a_glyph_renders_as_a_mask_in_the_text_color`: every glyph renders through GPUI's own SVG renderer and covers part of its square; `glyph` passes the color on and lays out at 12, 16 and 20 points.
- `fp_icons_fit_the_tile_and_the_icon_button`: a tile and an icon button take a glyph and draw.

Motion, in `src/ui/ds/motion.rs`, `src/ui/ds/kit_theme.rs` and `src/ui/legacy/shell/render.rs`:

- `duration_is_zero_after_keyboard_input`, `duration_is_a_short_fade_with_reduce_motion`, `duration_follows_the_token_for_pointer_input_without_reduce_motion`, `springs_are_off_with_reduce_motion`.
- `a_moment_runs_once_per_occasion`, `the_wake_moment_runs_once_per_unlock`, `the_wake_moment_runs_after_touch_id_but_not_after_return_in_the_password_field`.
- `the_kit_theme_takes_the_motion_values`: after a switch from the Kit's defaults, in light, dark with increased contrast and system.
- `the_root_handlers_note_the_input_kind`: the real shell in a window; a key press, a click and a shortcut each leave their kind.
- `no_animation_outside_the_design_system`: the scan over `src/`.

Text size, in `src/ui/ds/mod.rs` and `tests/settings_compat.rs`:

- `large_text_sets_fifteen_points_in_the_kit_theme_and_the_tokens`, for each theme choice, and back to 13.
- `switching_the_text_size_applies_without_a_restart`: an open window's rem size follows a change both ways.
- `no_fixed_text_size_outside_tokens`: no code in `src/ui/ds` or `src/ui/windows` sets 13 or 15 pixels.
- `a_settings_file_without_the_text_size_reads_it_as_standard`: a file written with Large, read and written back by 0.9.2, reads as standard. `new_settings_load_into_the_092_struct` writes the field too, set to Large, and 0.9.2 still loads every other choice.

The existing `kit_theme` tests, the appearance test of the shell and the checks on the real text system pass with the new signatures.

## Mutation probes

Each probe changed one thing on a clean tree, ran the module's tests and was reverted.

| Test | What the probe changes | Red output |
|---|---|---|
| `glyph_grid` | `copy.svg` with `stroke-width="2"` on its root | `copy.svg: stroke-width is Some("2"), not 1.5` |
| `glyph_enum_complete` | a copy of `clock.svg` as `history.svg` | `files without a variant: ["history"], variants without a file: []` |
| `status_shapes_differ` | `status-ok.svg` as a copy of `status-danger.svg` | `StatusOk and StatusDanger have the same shape` |
| `kit_overrides_match_the_kit_names`, `kit_overrides_are_on_the_sixteen_grid` | a copy of `close.svg` as `assets/icons/x.svg` | `files under assets/icons/ and the Kit's names`, `icons/x.svg` on the left only |
| `duration_is_zero_after_keyboard_input` | `duration` without the early return for keyboard input | `Micro, reduce motion false`, `left: 120ms`, `right: 0ns` |
| `duration_is_a_short_fade_with_reduce_motion` | `duration` without the Reduce Motion branch | `Short after Pointer: 180ms` |
| `springs_are_off_with_reduce_motion` | `springs_allowed` returns `true` | `assertion failed: !springs_allowed(cx)` |
| `no_animation_outside_the_design_system` | an untracked `src/ui/windows/panel/view.rs` with `div().with_animation(..)` | `src/ui/windows/panel/view.rs:2: div().with_animation(...)` |
| `the_wake_moment_runs_after_touch_id_but_not_after_return_in_the_password_field` | waking up allowed after keyboard input | `left: Some(320ms)`, `right: None` |
| `the_wake_moment_runs_once_per_unlock` | `begin_unlock` leaves waking up used | `unlock 2`, `left: None`, `right: Some(320ms)` |
| `the_wake_moment_runs_once_per_unlock` | `begin_unlock` clears every occasion | `the success again after unlock 1`, `left: Some(320ms)` |
| `the_root_handlers_note_the_input_kind` | the key interceptor without its `note_input` line | `left: System`, `right: Keyboard` |
| `no_fixed_text_size_outside_tokens` | `.text_size(px(15.))` in the tile | `src/ui/ds/components/tile.rs:83: .text_size(px(15.))` |
| `large_text_sets_fifteen_points_in_the_kit_theme_and_the_tokens`, `switching_the_text_size_applies_without_a_restart` | `text_size_points` maps Large to the standard token | `left: 13.0`, `right: 15.0` |
| `a_settings_file_without_the_text_size_reads_it_as_standard` | `text_size` without `#[serde(default)]` | ``this build reads what 0.9.2 wrote: Error("missing field `text_size`", ...)`` |
| `a_moment_runs_once_per_occasion`, `the_wake_moment_runs_after_touch_id_but_not_after_return_in_the_password_field` | `may_run` no longer asks `cx.reduce_motion()` | `left: Some(120ms)`, `right: None` |
| `the_kit_theme_takes_the_motion_values` | `kit_theme::switch` without the call that sets the motion values | `left: [0, 120, 180, 280]`, `right: [0, 120, 180, 240]` |
| `glyph_grid` | a fill of `#C0392B` on the rect in `copy.svg` and a rotated group around `clock.svg` | `clock.svg: <g> has transform`, `copy.svg: <rect> fill="#C0392B"` |
| `no_animation_outside_the_design_system` | a function named `with_spring_probe` in the old Settings screen | `src/ui/legacy/screens/settings.rs:487: fn with_spring_probe() {}` |

## How to check by hand

Not run yet; it comes with the preview build and the design gallery.

- In the gallery, every glyph at 12, 16 and 20 points in light, dark and both contrast modes, at text size 13 and 15. The four status glyphs side by side.
- In the password field of a Kit input, the eye and the crossed eye; in a select, the chevron. They should look as heavy as the app's own glyphs next to them.
- Settings, General, Text size: Large. The Kit's controls and the new screens grow at once, without a restart; note which parts of the old screens keep their size. Back to Standard, they shrink again. Quit and start: the size is kept. With Large set, change the theme in the theme row and with Cmd+Shift+D, and switch macOS between light and dark: the size stays at Large each time.
- macOS Reduce Motion on and off while the app runs, then a theme change: nothing that moved before slides or springs any more.

## Limits

- The shapes are drawn from the spec's glyph library and boards, not by a designer. Whether they read well at 12 and 16 points, and whether the redrawn Kit glyphs sit well beside the Kit's controls, is for the gallery and the device. A glyph that does not hold up goes back to a Kit icon until it is redrawn.
- The test platform loads no assets into a window, so the tests stop at GPUI's renderer and at the layout. What a glyph looks like on screen is not tested.
- `glyph_grid` does not hold coordinates to quarter points or a drawing to the live area. The spec's drawings use finer steps in places (the shield, the keyhole), and the shell's base line runs from 1.25 to 14.75; the files keep them as drawn.
- `status_shapes_differ` compares commands, not rendered masks. Two shapes drawn with the same commands in different places would pass.
- No test checks that `glyph` points at the glyph's own file. `a_glyph_renders_as_a_mask_in_the_text_color` renders each file through the renderer directly and checks the element `glyph` returns only for its color and size, so a `glyph` with a wrong path passes it. `glyph` has no callers yet.
- `the_root_handlers_note_the_input_kind` presses a key, clicks and presses a shortcut. It does not move the pointer or scroll, so the `note_input` lines in those two handlers can go without a test failing.
- No test holds startup, the theme row, the theme shortcut or the observer for the macOS appearance to the size that is set. Each passes it today, but if one passed Standard instead, every test would stay green. The check by hand covers them.
- `duration` gives a time, not the kind of change. That a transition with Reduce Motion only fades, and that a caller asks `springs_allowed` before it moves anything, rests on the callers, and there are none yet.
- Only the old main window notes the input kind. A window that does not go through its root handlers, such as the quick access panel, has to call `note_input` itself.
- `no_animation_outside_the_design_system` reads source text. A call through a macro, a re-export under another name, or gpui-base's lower level `transition` and `spring` functions would pass it.
- `no_animation_outside_the_design_system` does not look inside `src/ui/ds/`. A component there that animates with a fixed time, or after a key press, passes it.
- The Kit's animations do not go through `duration`. Those that read the Kit theme's motion values follow the same steps, but none of them knows the input kind, so the rule that a key press gets 0 ms does not reach them. Dialogs, sheets, popovers and tooltips keep the Kit's own fixed times.
- `no_fixed_text_size_outside_tokens` looks for 13 and 15 pixels only. A size in rems, or a pixel size computed from a number elsewhere, would pass it. The old screens are not scanned; much of their layout is in pixels and does not follow the text size.
- The Text size row is built from the same segments as the other rows of the old Settings screen, and those only take a click. Tab does not reach Standard or Large, no key switches them, and VoiceOver hears neither a group of choices nor which one is selected. The theme can still be changed with Cmd+Shift+D; the text size has no shortcut. The old screen is left as it is here, and its rows get keyboard and VoiceOver access when Settings is rebuilt.
- A trip back to 0.9.2 loses the setting the first time 0.9.2 writes `settings.json`, as with every new field. The size is standard again afterwards.

## Not done

- Six glyphs the spec lists have no drawing anywhere in it: `archive`, `history`, `message`, `pin`, `way` and `import`. They are not in `FpIcon`. A screen that needs one uses a Kit icon until they are drawn.
- `Moment` and the springs have no callers yet. Ferris's moments and the first moving parts come with later packages.

The agents spent about 2 hours on this, including review and fixes.
