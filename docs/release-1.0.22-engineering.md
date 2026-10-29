# Nivra 1.0.22 — compatibility and media/reader audit

This is a development-cycle record, **not a release announcement**. Starting point:
`main` at `5c767382f005f506629177f0fa0dbb37fb448046`, after
`v1.0.21-rc.1`. The screenshot supplied by the owner depicts the ArtCraft
App Suite, not a functioning Nivra window. No screenshots or benchmarks were
manufactured from it.

## Reader: confirmed behavior and the first 1.0.22 changes

The previous reader already fetched at most 256 KiB off the UI thread, decoded
bounded UTF-8/UTF-16/Windows-1252 text, and rendered Markdown or sanitized
static HTML through the existing egui Markdown implementation. It did not
expose Markdown's literal source view, although HTML had the switch.

- Add **Rendered / Source** for Markdown (including `.markdown` and `.mdown`).
- Add **Edit a copy**, a memory-bounded draft for complete plain text or
  Markdown attachments of at most 64 KiB. The existing native picker saves a
  **local copy** and never modifies or uploads the original attachment.
  Display cancellation separately from a write error. Warn before discarding
  unsaved changes. No HTML script execution.
- Recognize text `.csv`, `.tsv`, `.srt`, `.vtt`, `.rst`, `.adoc`, and
  common developer source formats; add text-like JSON/XML MIME types.
- Do **not** feed binary/structured `.doc`, `.docx`, `.odt`, `.rtf`, or
  `.epub` to the plaintext preview. Those remain downloadable/openable in
  dedicated local applications until a separately tested parser is integrated.
- PDF remains a download card. This cycle does **not** pretend to have integrated
  a PDF renderer, Office interpreter, document editor or full syntax engine.

## Open-source research: distinct scopes and compatibility

Source inspection performed on 2026-10-29:

| Project | Relevant capability reported by its README | Nivra integration decision |
| --- | --- | --- |
| [WordCraft](https://github.com/storytold/wordcraft) | DOCX, ODT, RTF, HTML, Markdown, TXT import/export | Study isolated format converters and licensing; do not link the full editor or copy branded UI/assets |
| [PdfCraft](https://github.com/storytold/pdfcraft) | Native PDF rendering and editing | Consider sandboxed read-only PDF page renderer and thumbnails, with resource limits and its dependency tree audited |
| [FilmCraft](https://github.com/storytold/filmcraft) | Multi-container/codec video decode and editing | Study error handling, subtitles, audio sync and seek; Nivra already has native platform decoders |
| [Ferrite](https://github.com/OlaProeis/Ferrite) | Large-file text editor and rope-based viewport | Reassess only when Nivra accepts large editing; a 64 KiB local draft does not justify adding a second editor framework |

ArtCraft's app-suite screenshot is a **visual reference** for compact grouping,
legible labels and uncomplicated actions, not an SDK or runtime dependency.
Brand names and trademarks are not being copied into Nivra.

## Screen sharing, motion and bandwidth

`crates/discord-voice/src/screen.rs` and `transport.rs` already apply
network feedback to dynamic encoder bitrate, drop frames before encoding when
the transport is full, and keep a bounded screen-video queue. Do not describe
these as newly implemented adaptive bitrate.

The quality picker already supports 480/720/1080p and 15/30/60 fps. Its cap
originally gave 15 fps the same bits/s as 30 fps. The 1.0.22 adjustment scales
the **initial maximum** by selected FPS (15/30/60), preserving the 30/60
defaults; this saves uplink budget at 15 fps but is not a measured improvement
to subjective video quality. Show the maximum and clarify that actual bitrate
may be lowered by transport feedback. Sender-clock A/V sync and network
jitter still need live measurements before further changes.

UI motion already uses `crates/ui/src/anim.rs` tokens 120/220/340 ms,
stops repainting when settled, and respects reduced-motion preferences. New
modal content inherits those animations rather than introducing endless
spinners or a new graphics framework.

## Windows screen-share audio resilience

In the pre-1.0.22 Windows implementation, the WASAPI audio worker shared the
screen capture's stop flag. A non-fatal system-audio error thus stopped the
whole screen video. The corrected design uses a separate audio-only cancellation
flag: a startup or later loopback failure is diagnosed but **video capture can
continue**. It does not promise recovery of system audio without explicitly
restarting that capture, and requires a hardware regression test with WASAPI.

The text-reader dialog also reuses the shared animated modal and adds a
low-cost file-type/size/status subtitle. This is presentation metadata, not a
new render loop or background animation.

## Dependency/size decision log

- The desktop pins egui/eframe and wgpu integration to an immutable Git
  revision. A migration to egui 0.37 or another major renderer upgrade requires
  adapting layout/widget interfaces and rerunning Windows x64/ARM64, Linux,
  macOS, accessibility, screenshots, voice and media tests. It is **not**
  authorized merely by a version number; update a compatible subset, measure
  then move the whole family atomically.
- The dependency graph includes a documented temporary cargo-audit exception
  for `crypto-bigint 0.5.5`, RUSTSEC-2026-0387. This is **not** a security
  fix. Upgrading DAVE / P-256 / OpenMLS requires protocol interoperability
  verification. Do not silently remove encryption or mute warnings.
- Measuring package size requires actual identical-mode release builds and
  compressed artifacts; compilation, memory and frame-rate gains have not been
  measured for this change. Report results from CI only after completion.
- Do not add the full WordCraft/PdfCraft/FilmCraft dependency trees to the chat
  client merely to preview a file; isolate format parsers and compare binary
  size, startup, CPU, memory and security on representative fixtures.

## Verification gates

Run formatting, strict clippy, offline UI tests (including source/view/edit
dialogs and cancellation), native platform package builds and the repository
license/security jobs. Failure must be reported and repaired on `main`.
No live Discord call, real screen capture, hardware GPU performance benchmark,
or arbitrary untrusted document fuzz corpus was executed for this change.
