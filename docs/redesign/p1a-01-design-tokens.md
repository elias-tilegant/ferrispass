# P1a.01: Design tokens and theme foundation

A step of the redesign on the `rebuild` branch, switched over on 2026-10-28 and finished after the shell split on 2026-11-01. It gives the design system a single source for its values and puts it into effect: the old screens now draw with the new warm palette, through a shim, until they are rebuilt.

| Commit | What |
|---|---|
| `c95bb90` | `design/tokens.json`, the generator and the generated Rust color sets |
| `8989db9` | Contrast tests and the generated GPUI Kit theme set |
| `acafb79` | `ui::ds::init` and `apply`, the accessibility settings, the palette shim, the app switched over |
| `5f275fc` | Lock as a secondary button |
| `5bab26d` | The old `src/ui/theme.rs` removed, its callers call `ui::ds` |
| `b66875c` | The palette check narrowed to the one switch in `ds::apply` |

## Where the values live

`design/tokens.json` is the only place to change a color, size or shadow. `design/README.md` describes the file. Two files are generated from it and checked in:

- `src/ui/ds/tokens/generated.rs`: the primitives, one `ColorSet` per mode (light, dark, and both with increased contrast), the dimensions and the shadows. Each color holds its hex value for the contrast math and the `Hsla` GPUI draws with.
- `assets/theme/ferrispass.theme.json`: four GPUI Kit themes, with every one of the Kit's 140 color keys set from a token, so nothing falls back to the Kit's stock colors.

The color families have English names (`copper`, `ochre`, `teal` and so on). `design/README.md` has a table that maps them to the names in the design spec and the mockups.

There is no build script. A golden test generates each file again and compares it byte for byte, so a JSON change that wasn't regenerated fails `cargo test`. To regenerate:

```
FP_UPDATE_GOLDEN=1 cargo test golden
```

## What the contrast tests guarantee

`src/ui/ds/tokens/contrast.rs` computes WCAG 2.x contrast (relative luminance with the 0.04045 threshold) on the hex values, for every pair under `contrast` in `tokens.json` and in all four modes:

- Text reaches 4.5:1 on every surface it is listed for, borders, focus rings and vault colors 3:1. With increased contrast it is 7:1 and 4.5:1. The ratio is not rounded before the comparison.
- The ratios the design spec states for light and dark still come out, to two places. Changing a color means changing those numbers on purpose.
- Three places don't meet the contract, and `exempt` lists them with a reason: the selection against the surface (about 1.2:1, so a selected row also gets semibold text, the focus ring and the selected state for VoiceOver), and the separator and keycap borders in light and dark, which are decorative. A fourth gap fails the test until it is listed, and so does a listed exemption that nothing needs.
- The background neutrals stay one warm family, and vault colors keep their distance from the hues that carry a meaning (copper, ochre, carmine, blue).

In light mode the faintest text in the old screens (`text_faint`) had 2.54:1 on white and 2.31:1 on the sidebar. It now has at least 4.83:1 on every surface.

## Why two settings stay out of the theme set

- **The font size.** A `font.size` in a Kit theme is applied on every theme change, over whatever text size the app set. So `ds::apply` changes the theme first and sets the text size (13 for now) afterwards.
- **The mono font.** The theme set names JetBrains Mono, the only monospace font the app ships today. GPUI gives up on the first layout in a font family that isn't loaded, so the theme can only name Atkinson Hyperlegible Mono once the app embeds it.

## How the app uses it

`ui::ds::init` runs at startup, after the fonts and `gpui_component::init`, before the first window. `ui::ds::apply_choice` runs on every appearance change: when macOS switches while System is chosen, when an appearance is picked in Settings, and on Toggle Theme (in the menu or with Cmd-Shift-D). The old shell calls it directly; the old `src/ui/theme.rs` is gone. Whether System is chosen comes from the settings in memory, not from the settings file: the file is written in the background, so right after a change it can still hold the previous choice, and after a failed write it keeps it. Each call:

1. swaps the Kit's light and dark configs for the normal or increased contrast ones, then changes the theme mode. In this order, because the Kit treats a new config and a new mode in one update as already installed and keeps the old colors,
2. sets the text size and the focus ring,
3. sets the tokens behind `cx.ds()` and the mode of the palette shim,
4. sets reduce motion from macOS, which gpui-base otherwise reads only once at startup.

Increase contrast, reduce motion and VoiceOver are read from `NSWorkspace` each time (`src/ui/ds/a11y_prefs.rs`). For now they are read when the appearance changes or the setting is used, not when only the accessibility setting changes.

`src/ui/palette.rs` keeps its 22 accessors, but each returns a token now. The table in `design/README.md` lists which, and a test holds the accessors to that table in all four modes. The module is deprecated, and a test fails if code outside the old shell, screens and widgets names it. The one exception is the line in `ds::apply` that switches the palette mode, and the test also fails if that line is missing or there twice. The tests for the shim are in `src/ui/palette_tests.rs`.

## The toolbar

With a vault open, the toolbar had two filled copper buttons, New entry and Lock. Lock now has the outlined style of Group and Sync, so New entry is the only filled button. Open vault, shown in the same place while the vault is locked, already had the outlined style.

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
| `bg.window` pointed at another primitive, files not regenerated | `generated_rs_is_current`, `kit_theme_json_is_current` |
| `ratio()` without the 0.05 flare term | `matches_documented_ratios`, `luminance_and_ratio_know_the_ends` |
| Luminance threshold 0.4045 instead of 0.04045 | `matches_documented_ratios`, `hue_separation`, `luminance_and_ratio_know_the_ends` |
| `text.tertiary` lightened to about 3:1, files regenerated | `contract_holds_in_all_modes`, `matches_documented_ratios`, `exemptions_are_explicit` |
| The `border.keycap` exemption removed | `contract_holds_in_all_modes`, `exemptions_are_explicit` |
| `link` misspelled in the theme set | `kit_keys_survive_roundtrip`, `kit_theme_matches_tokens`, `kit_theme_json_is_current` |
| One theme set color off by one step | `kit_theme_matches_tokens` |
| Theme mode changed before the configs were swapped | `hc_configs_switch_with_theme_change` |
| Text size not set after the theme change | `hc_configs_switch_with_theme_change` |
| `ds::apply` without the palette mode | `ds_apply_switches_the_palette`, `no_palette_outside_legacy` |
| `ds::apply` without reduce motion | `apply_sets_every_piece` |
| `palette::orange()` returning `accent.fill` | `palette_follows_the_readme_table` |
| `kit_theme.rs` importing the palette | `no_palette_outside_legacy` |
| A palette call added to `ds/mod.rs`, once in `apply` and once in its tests | `no_palette_outside_legacy`, naming both lines |
| The palette switch in `ds::apply` written twice | `no_palette_outside_legacy` |
| The appearance observer reading the theme from a settings file that says Light | `appearance_change_follows_the_choice_in_memory`, the System case |
| The appearance observer reading System, as from a missing settings file | `appearance_change_follows_the_choice_in_memory`, the Dark case |
| A hex string in `kit_theme.rs` | `no_raw_colors` |
| A primitive name that doesn't match its lightness | `primitive_names` |
| A primitive hex three steps from its OKLCH | `primitives_roundtrip` |

## Not checked yet

- How the old screens look in light and dark with the new colors, the toolbar with Lock as a secondary button, and switching the appearance while the app runs. These need the app on a Mac.
- Increased contrast on screen. The values are tested; how they look is checked later.
