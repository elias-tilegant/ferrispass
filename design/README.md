# Design tokens

`tokens.json` is the one place where the colors, sizes and shadows of FerrisPass are defined. The files the app reads are generated from it and checked in:

| Generated file | Read by |
|---|---|
| `src/ui/ds/tokens/generated.rs` | the app, through `cx.ds()` |

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
