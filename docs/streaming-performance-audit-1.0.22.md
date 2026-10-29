# Nivra 1.0.22 — audit of live screen sharing and watching

Date: 2026-10-29. Scope: inspected sources in `main` (before this patch at
`ed717d402d9a78f7281bab3ba84bf2af73119462`). This is source-based
analysis, **not** a measured session on the owner's PC.

## Codec and negotiated protocol

Nivra Go Live currently **negotiates H.264 only** in the Discord voice/stream
session description; Opus is used for audio. It does **not** negotiate AV1,
HEVC/H.265, VP8 or VP9 based on viewer capabilities. Do not advertise a
multi-codec selector until the signaling, RTP packetization, DAVE media codec,
decoder capabilities and fallback paths are implemented and tested together.

H.264 chooses an **implementation** automatically:

| OS | Sender | Receiver |
| --- | --- | --- |
| Windows | Media Foundation hardware encoder when available; OpenH264 CPU fallback | Media Foundation H.264 (DXGI manager when available), OpenH264 fallback |
| Linux | GStreamer VA-API or NVENC first, including copy/scaling variants; OpenH264 CPU fallback | GStreamer decodebin chooses an installed decoder, or OpenH264 fallback |
| macOS | VideoToolbox hardware encoder when available; OpenH264 CPU fallback | VideoToolbox, then OpenH264 fallback |

Windows Media Foundation hardware encode uses **CPU-side BGRA-to-NV12
conversion** before the hardware MFT. Hardware encoding does not imply a
zero-copy frame path. On Windows the H.264 MFT is selected from hardware
transforms and can fail to initialize or reconfigure, in which case OpenH264
is used. Windows DXGI decoding also converts decoded data to RGBA on CPU for
the egui texture upload.

## Why the broadcaster may feel slow

Windows Graphics Capture -> D3D11 `CopyResource` into staging ->
synchronous `Map(D3D11_MAP_READ)` -> full raw BGRA CPU copy ->
CPU scaling/colorspace conversion to NV12/I420 -> hardware or CPU H.264
encoding -> paced RTP/UDP. Even with Media Foundation hardware encoding, the
readback, conversion and memory bandwidth cost remain. This is a **supported
architectural explanation**, not a proven diagnosis of the owner’s host.

Raw frame-transfer volume only, before intermediate copies, conversion or
compression (illustrative, NOT benchmark results):

| Source at capture callback | Raw bytes/frame | At source FPS |
| --- | ---: | ---: |
| 1920×1080 30 FPS | 8.29 MB | 249 MB/s |
| 2560×1440 30 FPS | 14.75 MB | 442 MB/s |
| 3840×2160 30 FPS | 33.18 MB | 995 MB/s |
| 3840×2160 60 FPS | 33.18 MB | 1.99 GB/s |

At 4K, selecting 720p output reduces encoder dimensions but does **not**
remove the original full-source GPU readback. The substantial future
improvement is GPU-side downscale and NV12 conversion with compatible
cross-device D3D11 texture interop and no blocking CPU Map. That change
requires actual hardware tests before rollout.

Existing optimizations: Windows minimum compositor update interval when
supported, paced callback, one raw-frame channel, two reusable full-frame
buffers, local preview max 10 FPS, idle 1 FPS, and no encode when the secure
transport is not ready or the encoded queue is full. The 1.0.22 audit adds:

1. **Live GPU/CPU encoder indicator** in the existing screen-sharing status,
   updated only on backend changes (not every frame). On Windows/macOS this
   distinguishes hardware H.264 from OpenH264 CPU fallback.
2. **Transport-aware capture throttling (Windows)**: when the encoded queue
   cannot accept a frame, reduce D3D11 readbacks to the idle/preview interval,
   restoring capture speed immediately after capacity returns. Keep the same
   keyframe/epoch safety model. It is bounded and covered by synthetic tests.
3. **Local low-load preset**: 720p/30 FPS in the existing quality selector,
   immediately reversible. It saves conversion/encode work, but does not
   eliminate 4K readback.
4. **Hidden remote view CPU reduction**: skip copying RGBA to egui's ColorImage
   when no view is visible. In the follow-up pass, the OpenH264 software
   decoder also continues decoding the H.264 reference pictures while hidden
   but skips the YUV→RGBA conversion until the user returns. Decoder picture
   accounting must continue to avoid false stream-stall/keyframe requests;
   the older public `watch_stream` interface remains compatible.
   Hardware decode output conversion is still platform-specific, and further
   GPU-native presentation remains a separate project.

No percentage improvement to system latency, FPS, CPU, GPU or bandwidth is
claimed without a comparable before/after measurement.

## Viewer and transport audit

- RTP H.264 packetization (RFC 6184), DAVE encryption, bounded pacing,
  retransmission history (RTX), NACK/PLI keyframe recovery.
- Loss-based sender bitrate controller with REMB ceiling; hardware encoders
  can change bitrate, software changes restart encoder only for substantial
  moves (at least 15% decrease or 25% increase).
- Decoder queue limited in bytes and aged at 150 ms, stale frames dropped,
  decoder state discarded until next keyframe; max 8 concurrent active
  decoders. Hardware decoder preferred where available and supported.
- One newest decoded RGBA per viewer and texture reuse; when the stream is
  hidden, avoid UI repaint and GPU upload. This pass also avoids unnecessary
  CPU pixel conversion in the decoder callback when invisible.
- A/V stream audio is Opus on a separate SSRC, with independently scheduled
  audio ticks. This is **not** proof of full lip-sync fidelity under network
  jitter, suspend/resume, high refresh rates or device rate drift. Requires
  measured glass-to-glass latency and skew under packet loss.
- Do not forcibly switch to AV1/HEVC to optimize CPU: unsupported Discord
  signaling could prevent all receivers from decoding the stream.

## Diagnostics on the real PC

Enable `NIVRA_VOICE_DIAGNOSTICS=1` before launching, or use
`--ultra-diagnostics` for rotating debug files. Logs already contain
aggregates for encoder backend, capture readback and encode times, packet
loss, drops, decode queue age, retransmissions and decoder counts. Export
redacted diagnostics and compare the same source/scene over 60 seconds.

Compare three runs on the **same build**: 1080p60, 1080p30, 720p30. Keep
scene, audio, fullscreen/app selection, active viewers and network constant.
Record in Windows Task Manager GPU engines (3D/Copy/Video Encode/Video
Decode), per-process CPU, working set and power usage; note whether the UI
reports CPU fallback. Have a second client report frozen frames, visible
latency, color accuracy and PLI/RTX activity. Trace frame times p50/p95/p99
and memory footprint before claiming performance gains.

## Compatibility and safeguards

The follow-up CI must pass Windows x64, Windows ARM64, macOS ARM64, Linux,
strict Clippy, unit/integration tests, fuzzing, audits and license checks.
Where possible run GPU capture and hardware decode benchmarks on a physical
device. Existing cargo-audit waiver for `RUSTSEC-2026-0387` is a documented
risk, not a vulnerability fix. Do not conflate an approved CI scan with a
fully remediated cryptography stack.

## Longer-term research proposals (not implemented)

1. GPU-based downscale and BGRA→NV12 using the capture adapter’s D3D11 device,
   with bounded GPU surfaces and nonblocking fences. Priority highest for 4K.
2. Surface encoder selection, actual rate, dropped capture frames, pacing delay,
   readback and encode p95 and receiver decode backlog in a compact diagnostics
   panel; avoid waking UI on each frame.
3. Better per-viewer feedback: track RTT/jitter/freeze-rate and protect against
   image quality oscillation; preserve negotiated H.264 fallback.
4. Hardware-aware codec negotiation only after Discord interop tests with
   H.264/AV1/HEVC/VP9 across viewers, DAVE codec integration, licensing,
   hardware encoder availability and packet loss behavior.
5. Separate sender vs viewer benchmark fixtures for 4K/1080p60/720p30,
   interactive games and static code. Include CPU/GPU battery/power impact.

Key code links:
- https://github.com/vitorhubdev/Nivra/blob/main/crates/discord-voice/src/screen/readback_windows.rs
- https://github.com/vitorhubdev/Nivra/blob/main/crates/discord-voice/src/screen/convert.rs
- https://github.com/vitorhubdev/Nivra/blob/main/crates/discord-voice/src/screen.rs
- https://github.com/vitorhubdev/Nivra/blob/main/crates/discord-voice/src/video_encode_windows.rs
- https://github.com/vitorhubdev/Nivra/blob/main/crates/discord-voice/src/transport.rs
- https://github.com/vitorhubdev/Nivra/blob/main/crates/discord-voice/src/video_receive.rs
- https://github.com/vitorhubdev/Nivra/blob/main/apps/desktop/src/watch.rs
