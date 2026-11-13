# App icon

The app icon is Ferris holding a key, on cream. It is three flat layers drawn as SVG; Icon Composer stacks them, adds the glass and the shadows, and draws the background.

| File | Layer | Colors |
|---|---|---|
| `key.svg` | the key in the raised claw, on top | `#278C72` |
| `face.svg` | eyes with their highlights, mouth, cheeks | eyes and mouth `#2A2622`, highlights `#FFFFFF`, cheeks `#F6A27A` |
| `body.svg` | shell with its crown, claws, arms, two legs a side | shell `#F74C00`, claws, arms and legs `#C2410C` |

Each layer covers the full 1024 × 1024 canvas and holds filled shapes only: no strokes, masks, filters, transforms or opacity. Glass and shadows are set in the icon document, not drawn.

## Where the build takes it from

The build reads `bundle/icon/AppIcon.icon`, an Icon Composer document. Its `Assets/` folder holds these three files under the same names, and `icon.json` sets them up:

- `key` and `face` are opaque, `body` is slightly glassy (translucency 0.25), and every group has a neutral shadow.
- The background is the document's fill, not a file: a vertical gradient from `#F6EEE4` to `#E9D9C6`, and for the dark appearance its own gradient from `#2A2622` to `#171513` (the `fill-specializations` entry with `"appearance": "dark"`). macOS shows the dark one when the icon style is set to Dark.

`scripts/build-mac.sh` compiles the document with `actool` into `Assets.car`; see `docs/release-process.md`.

## Changing the drawing

The geometry is set by hand: every anchor point, radius and angle is a constant in `build.py`, which only works out rounded corners, mirror images and offset curves and writes paths with integer coordinates. The output is the same byte for byte on every run:

```sh
python3 -I design/brand/icon/build.py /tmp/icon
```

That writes `key.svg`, `face.svg` and `body.svg`, plus `icon.svg` (all three on cream) and `icon-graphit.svg` (on graphite) to look at. Copy the three layers here and into `bundle/icon/AppIcon.icon/Assets/`; `tests/app_bundle.rs` fails while the two copies differ. Open the document in Icon Composer to look at all appearances before committing.

## Where it comes from

Ferris was traced by hand from a generated sketch, with paths of its own and no auto-trace. The sketch is not in the repo. Ferris the Rustacean is by Karen Rustad Tölva and in the public domain (CC0).

This drawing is an interim icon. The final one will replace the three layers under the same file names and keep the number and roles of the groups.
