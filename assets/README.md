No Discord logos or proprietary fonts are bundled. The notification sounds are Discord audio assets; see [sound provenance and redistribution limitations](sounds/README.md). The interface leads proportional text with three unmodified SIL Open Font License 1.1 Inter faces (Regular, Medium, SemiBold; the heavier two form the `medium`/`semibold` families because egui has no synthetic bold), keeps egui's default faces as fallbacks, and appends two unmodified OFL fallback faces (Arabic, Math) to every family. They are embedded once in the executable; there is no runtime font download. Japanese, Chinese and Korean text ships no face: when CJK text first appears, a worker thread reads the installed system font that covers each script (see the search order below) once, installs it into egui's fallback families and keeps it for the process. On a system with no CJK font the text falls back to replacement glyphs and the app explains it once.

| Asset | Upstream version | Bytes | Copyright and license |
|---|---|---:|---|
| `fonts/NotoSansArabic.ttf` | 2.012 | 844,676 | Copyright 2022 The Noto Project Authors; [OFL 1.1](fonts/NotoSansArabic-OFL.txt) |
| `fonts/NotoSansMath-Regular.otf` | 3.000 (unhinted) | 479,308 | Copyright 2022 The Noto Project Authors; [OFL 1.1](fonts/NotoSansMath-OFL.txt) |
| `fonts/Inter-Regular.ttf` | 3.19 (hinted) | 680,240 | Copyright (c) 2016-2020 The Inter Project Authors; [OFL 1.1](fonts/Inter-OFL.txt) |
| `fonts/Inter-Medium.ttf` | 3.19 (hinted) | 694,512 | Copyright (c) 2016-2020 The Inter Project Authors; [OFL 1.1](fonts/Inter-OFL.txt) |
| `fonts/Inter-SemiBold.ttf` | 3.19 (hinted) | 710,040 | Copyright (c) 2016-2020 The Inter Project Authors; [OFL 1.1](fonts/Inter-OFL.txt) |

Downloaded September 10, 2026 from pinned upstream sources:

- [Noto Sans Arabic font](https://github.com/google/fonts/blob/334b789e33413f3aba4264d9aa6c97f7b94c5a2f/ofl/notosansarabic/NotoSansArabic%5Bwdth%2Cwght%5D.ttf), [license](https://github.com/google/fonts/blob/334b789e33413f3aba4264d9aa6c97f7b94c5a2f/ofl/notosansarabic/OFL.txt). The upstream variable-font filename is shortened locally; font bytes are unchanged. SHA-256: `63111b5b2e074dd48cc67692e0a2726d86ee94c1c37fe8598257b7b4e87e869e`.
- [Noto Sans Math 3.000](https://github.com/notofonts/math/releases/tag/NotoSansMath-v3.000) unhinted OpenType face from the official release archive (SHA-256 `ac351837b41f8a897f020b97fb0f075ad574c1e9669fb5839ada1f92fd748356`). The font bytes are unchanged. SHA-256: `a3a3904ede36039d4ba8177ec0aa9cf90653e7c7927d74d766bf31ea26e2a7c1`.

- [Inter 3.19](https://github.com/rsms/inter/releases/tag/v3.19) static TrueType instances `Inter-Regular.ttf`, `Inter-Medium.ttf`, `Inter-SemiBold.ttf`, taken from the `Inter Hinted for Windows/Desktop/` directory of the official `Inter-3.19.zip` release archive (SHA-256 `150ab6230d1762a57bebf35dfc04d606ff91598a31d785f7f100356ecdcc0032`), re-fetched September 15, 2026. The archive's `LICENSE.txt` is byte-identical to the bundled `fonts/Inter-OFL.txt`. SHA-256: Regular `529be850e06f62f8904f22bda77e45bde4834498fdbec4ff4201fa3177447a3a`, Medium `6df88fcb83ac96582350f801355c6eff55f15710093e9627fb431caa40521151`, SemiBold `2de533bda937a063c595b07c6bd9b70c8c5087d0649a1c8330f7ac11fcc05602`.

  These are the hinted TrueType builds. Upstream also ships CFF outlines. egui paints
  grayscale coverage, not DirectWrite ClearType. Nivra leaves the TrueType interpreter
  off and keeps sub-pixel binning on. Dark mode remaps coverage with
  `FontColorTransferFunction::Gamma(0.5)`. Light mode leaves the transfer function off.
  Inter faces set `FontTweak.hinting` to `Some(false)`. The files stay the hinted
  TrueType builds. The interpreter does not run. Glyph designs and advance widths are
  unchanged, so layout is unaffected.

The five font blobs total **3,408,776 bytes (3.25 MiB)** — dropping the former 12,034,822-byte Noto Sans CJK archive cut every installer by about 12 MB. This raw size is separate from compressed distribution size, font-parser/layout memory and GPU glyph-atlas allocations. Package the license files and third-party copyright notices with the executable.

### CJK system fallback

The executable embeds no CJK face. `crates/ui/src/fonts/system.rs` documents the search order: on Windows `%SystemRoot%\Fonts` (Microsoft YaHei, Yu Gothic, Malgun Gothic, MS Gothic, SimSun, then the per-user font directory), on macOS `/System/Library/Fonts` plus `/System/Library/Fonts/Supplemental` (PingFang, Hiragino, Apple SD Gothic Neo), and on Linux fontconfig's answer for `:lang=ja`, `:lang=zh` and `:lang=ko` followed by the distribution's Noto Sans CJK paths. Each candidate is read once, off the draw thread, and a face only counts when its cmap maps the sample scalar for a script (kana, simplified Han, Hangul): fontconfig's fallback to a non-CJK face is not mistaken for coverage, a `.ttc` collection's face index is chosen by coverage, and a face is added exactly when the platform's own fallback would not cover the script. The loaded faces join egui's fallback families for the rest of the process; a stock Windows install keeps Microsoft YaHei and Malgun Gothic in memory (about 33 MB) only after CJK text first appears. Eframe's `system_fonts` provider still serves emoji and other scripts. Tests build fake font roots in a temporary directory to cover the candidate order and the missing-font path; the real search and rendering run on the four CI systems, including a `.ttc` collection on Windows.

No CJK font is embedded, downloaded or written to disk. On a minimal Linux system with none installed, CJK text shows replacement glyphs and a one-time translated notice points at installing a Japanese/Chinese/Korean font.

The focused test checks egui glyph availability for synthetic Arabic, mathematical alphanumeric symbols, Latin and combining accents in both font families, and CJK through the system provider when the machine has a font. It does not prove complete Unicode coverage, correct Arabic shaping/bidirectional editing, actual IME behavior, screen-reader behavior, or platform rendering. Regional Han forms come from whichever installed face the platform picks; extended emoji remain incomplete. Fallback glyphs in code blocks are not guaranteed to have the primary monospace font's cell width.

Test detail: egui 0.36.2's [`Font::has_glyph`](https://github.com/emilk/egui/blob/49682f8baa058bf49e011035cfbd6e825f88a5ef/crates/epaint/src/text/font.rs#L663) compares the selected face with the replacement face, and our first test observed a false negative for `H` on it. The non-CJK test therefore checks every sample scalar against the actual configured face charmaps through egui's already-resolved Skrifa parser, and the provider test measures `glyph_width` instead. It does not skip missing sample glyphs.

## Icons

`icons/atlas.png` (38,534 bytes, 512×320 RGBA) bundles 37 [Phosphor Icons](https://github.com/phosphor-icons/core) 2.1.1 glyphs under the MIT license; see [icons/README.md](icons/README.md) for the pinned sources, hashes and the `resvg` regeneration command.
