# Nivra brand assets

Nivra's own application mark: a glass N on a rounded plate carrying the violet-to-blue
brand gradient. These files are Nivra design assets, not Apple, Discord or any other
vendor's branding, and no third-party font is embedded in them.

## Current mark

- `nivra.svg` — full colour mark on its plate; the master vector source.
- `nivra-flat.svg` — flat (non-gradient) variant for small or low-colour contexts.
- `nivra-mark.svg` — solid white N monogram, view box trimmed and square.
  `tools/generate-icons.py` rasterizes this one into the shared icon atlas as
  `nivra-mark`, which the interface tints at draw time. Its view box must stay square:
  the generator scales a repository mark by `viewBox[2]` into a fixed 64 pixel cell.
- `nivra-1024.png` — 1024x1024 sRGB RGBA render of the full colour mark.
- `nivra-tray.png` — 72x72 black-on-transparent render of `nivra-mark.svg`, embedded by
  `crates/platform` as the macOS menu bar template image; macOS tints it per appearance,
  so only its alpha is used.

The packaged platform icons built from the same artwork live in `packaging/`
(`macos/Nivra.icns`, `windows/Nivra.ico`, `linux/hicolor/*`).

## How the masters were produced

The supplied master artwork was a 1254x1254 RGB file with **no alpha channel** and an
opaque black surround, which cannot be published as an application icon: every edge pixel
was the plate colour multiplied by its coverage, so cutting the black away left a dark
fringe. The masters here were therefore rebuilt rather than cut:

1. the plate silhouette was recovered by flood filling the dark surround, and modelled as a
   superellipse of exponent 4.5 (mean radial error 1.75 px over the traced outline);
2. the plate colour field was estimated with a *normalised* blur, so the black surround
   never bleeds into the field near the rim;
3. the coverage of every pixel was measured against that field, and the stored colour was
   **un-premultiplied** (`colour / coverage`) before being composited, which is what
   removes the fringe;
4. the N was traced from the artwork, simplified to 201 nodes and smoothed into cubic
   segments; the vector reproduces the traced silhouette with an IoU of 0.993.

`assets/brand/test_assets.py` measures the result: four channels, transparent corners, an
opaque centre, and no more than 2% of the antialiased rim darker than a quarter of the
plate median luminance. A naive cut of the same master fails that last check at 43.8%.

## Palette

Plate gradient `#F0C4FE` → `#8B44F4` → `#3E27ED` → `#0A6BFD` → `#9BDCFE`; the glass N runs
`#FFFFFF` → `#EADCFF` → `#AE8CFF`. The flat variant uses plate `#5B2BE0` and N `#F3EAFF`.