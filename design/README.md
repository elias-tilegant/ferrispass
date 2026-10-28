# Design tokens

`tokens.json` is the one place where the colors, sizes and shadows of FerrisPass are defined. The files the app reads are generated from it and checked in:

| Generated file | Read by |
|---|---|
| `src/ui/ds/tokens/generated.rs` | the app, through `cx.ds()` |
| `assets/theme/ferrispass.theme.json` | GPUI Kit, one theme per mode |

There is no build script. A golden test generates each file again and compares it byte for byte with the checked-in copy, so a change to `tokens.json` that wasn't regenerated fails `cargo test`.

## Changing a value

1. Edit `tokens.json`. A color is written once, as a primitive. Tokens point at primitives or at other tokens. Outside the primitives, only colors with transparency (`scrim`, shadow colors) are written as hex, as `#RRGGBBAA`.
2. Run `FP_UPDATE_GOLDEN=1 cargo test golden` to rewrite the generated files.
3. Run `cargo test`. The contrast tests check every pair under `contrast` in all four modes. If a ratio in `documented` changes, that is a change to the design spec too, so update the number on purpose rather than to make the test pass.
4. Commit `tokens.json` and the generated files together.

## What is in `tokens.json`

- `primitives`: the raw colors. A name is the color family plus its OKLCH lightness times 1000, so `sand.974` has lightness 0.974 and the names sort from dark to light. Each one has its hex value, which is what ships, and its OKLCH coordinates, where new steps are designed: move the lightness, keep chroma and hue, lower the chroma if the color leaves sRGB, then freeze it as hex. A test checks that both describe the same color.
- `semantic`: what UI code reads, per mode. The modes are `light`, `dark`, `light-hc` and `dark-hc` (increased contrast in macOS). A token without its own `-hc` value keeps the value of its base mode.
- `aliases`, `brand`, `components`: fixed names for one use, colors for brand graphics only, and the places where a component departs from the semantic layer.
- `elevation`, `dimensions`, `typography`: shadows, sizes in points at text size 13, and the text styles. The typography is data only for now.
- `contrast`, `exempt`, `neutral_family`, `hue_separation`: what the tests in `src/ui/ds/tokens/contrast.rs` hold the colors to.
- `kit`: how the tokens map onto the color keys of GPUI Kit.
- `css_aliases`: the variable names the HTML mockups use for some tokens.

## Color family names

The color families have English names here. The design spec and the HTML mockups use other names, so this is how they match:

| Here | Design spec |
|---|---|
| `sand` | `sand` |
| `copper` | `kupfer` |
| `brand` | `marke` |
| `patina` | `patina` |
| `ochre` | `ocker` |
| `blue` | `blau` |
| `carmine` | `karmin` |
| `teal` | `petrol` |
| `plum` | `pflaume` |
| `moss` | `moos` |
| `slate` | `schiefer` |

The vault colors follow the same table: `team.teal` is `team.petrol` in the design spec. The keys of `css_aliases` keep the names the mockups use, such as `--team-petrol`.

## The GPUI Kit theme

`assets/theme/ferrispass.theme.json` holds four Kit themes, one per mode. `kit.colors` sets every one of the 140 color keys the Kit's schema knows, so nothing falls back to the Kit's stock colors. Two things look odd there and are on purpose:

- The Kit's `accent` is its hover color in menus and lists, not an accent color. Copper is `primary`.
- `switch.background` is the track of a switch that is off. The Kit paints a switch that is on in `primary` unless the control sets its own color, and a switch that is on is never copper here, so a Kit switch has to be given `switch.on.bg` where it is used.

Two settings stay out of the theme set for now:

- `font.size`. The Kit applies it on every theme change, over whatever text size the app set. The app sets the text size after each change instead.
- The mono font is still JetBrains Mono, because it is the only monospace font the app ships today. GPUI gives up on the first layout in a font family that isn't there, so the theme can only name Atkinson Hyperlegible Mono once the app loads it.

## The old screens

The screens and widgets from before the redesign read their colors from `src/ui/palette.rs` without a context. That module now returns tokens, so the old screens use the new colors until they are rebuilt. It is deprecated, and a test keeps new code from naming it.

| Old accessor | Token |
|---|---|
| `bg()` | `bg.window` |
| `panel()` | `bg.surface` |
| `sidebar()` | `bg.sidebar` |
| `border()` | `border.separator` |
| `border_strong()` | `border.control` |
| `text()` | `text.primary` |
| `text_muted()` | `text.secondary` |
| `text_faint()` | `text.tertiary` |
| `blue()` | `accent.fill` |
| `blue_hover()` | `accent.fill.hover` |
| `blue_soft()` | `bg.selected` |
| `blue_border()` | `focus.ring` |
| `orange()` | `accent.text` |
| `orange_deep()` | `accent.fill.hover` |
| `orange_soft()` | `bg.selected` |
| `orange_border()` | `focus.ring` |
| `green()` | `status.ok.text` |
| `green_soft()` | `status.ok.soft` |
| `green_border()` | `status.ok.text` |
| `red()` | `status.danger.text` |
| `yellow()` | `status.attention.text` |
| `transparent_overlay()` | `scrim` |

A test reads this table and checks each accessor against it in all four modes, so the table is the mapping.

## Still to come

Two more outputs will be generated from the same file: CSS custom properties for the HTML mockups (the names in `css_aliases` included) and the token file for the published design system page. `tokens.json` already holds what both need.
