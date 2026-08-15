<!--
  Copyright (c) 2026 G & R Associates LLC
  SPDX-License-Identifier: MIT OR Apache-2.0
-->

# Image and video capture

`F` captures a still. `V` starts and stops a recording. Both capture **everything the viewer
draws** — spectrum, persistence, waterfall or spectrogram, HUD, decode bar and overlays — and
nothing it does not.

```sh
orion-sdr-view --capture ~/Captures    # default: ./capture
```

## What lands in the file

Capture reads back the **surface texture**, which is the window's client area. macOS composites
the title bar and border outside it, so **window decorations are excluded by construction**: no
cropping, no scale-factor arithmetic, and no Screen Recording permission prompt — an OS-level
capture API would need one.

The readback is at **physical** pixels. A 1200 × 828 logical window on a 2× display captures at
2400 × 1656, which is 15.9 MB a frame. Stills are free; that number is what every decision about
recording below follows from.

It is asynchronous, so it does not stall the render thread: `egui-wgpu` copies to a staging buffer
and maps it, and the finished image arrives a frame or two later.

## Naming

A capture is named for the instant it depicts, in **ISO 8601 basic format** with milliseconds:

```text
20260816T112233.456Z.png          time_zone: utc
20260816T062233.456-0500.png      time_zone: "-05:00"
20260816T165233.456+0530.png      time_zone: "+05:30"
```

Basic format rather than extended, because this names a file: extended's colons are illegal in a
path on Windows and are rendered as `/` by the macOS Finder. Both are conformant, and ISO 8601
forbids *mixing* the two in one representation — which is why the offset reads `+0530` here and
`+05:30` in the metadata beside it.

**The zone follows `display.time_zone`**, so a capture's filename agrees with the timestamps
visible inside the image. UTC is written `Z`, the canonical designator.

**The timestamp is of the content, not of the callback.** The request is issued during one frame
and the image returns a frame or more later, so stamping it on arrival would smear the timeline by
the readback latency. Milliseconds are load-bearing rather than decorative: at 30 fps a frame lands
every 33 ms, and second precision would put thirty of them on one path.

## The metadata sidecar

Every still gets a `.json` beside it, and every recording a manifest:

```json
{
  "kind": "still",
  "version": "0.0.26",
  "file": "20260816T112233.456Z.png",
  "time": "2026-08-16T11:22:33.456Z",
  "seq": 7,
  "width": 2400,
  "height": 1656,
  "source": "COFDM",
  "fs_hz": 1920000.0,
  "center_hz": 480000.0,
  "span_hz": 960000.0,
  "db_min": -100.0,
  "db_max": -15.0,
  "overlays": true
}
```

**A capture outlives the session that made it.** A PNG on its own says nothing about which source
produced it, at what sample rate, over what span, or against what dB scale — and without those the
picture cannot be read. Same reasoning as the replay dump's header record: the artifact describes
itself rather than depending on someone's memory.

## Recording

`V` toggles. The frames go to a bounded queue consumed by a writer thread, which either pipes raw
RGBA to `ffmpeg` or writes a numbered PNG each:

| `capture.format` | Output | Needs |
| --- | --- | --- |
| `mp4` (default) | H.264 in MP4 | `ffmpeg` on `PATH` |
| `png` | a directory of `000000.png`, … | nothing |

Piping keeps a codec dependency tree out of a DSP tool. **ffmpeg's absence is reported when
recording starts**, not discovered later — the encoder itself cannot spawn until the first frame
arrives, since that is when the physical frame size becomes known, so without an explicit check the
failure would surface a frame after you were told recording had begun.

### Notices

Capture messages go to stderr, prefixed by severity and coloured when the stream is a terminal:

```text
• capture: wrote ./capture/20260816T112233.456Z.png
⚠ capture: ffmpeg was not found on PATH, so mp4 recording cannot start …
✗ capture: recording stopped — the window changed size mid-recording …
```

Bold yellow for a warning, bold red for a failure. The glyphs differ in *shape* as well as colour,
so severity still reads in a log, on a monochrome terminal, or to someone who cannot separate red
from green. Styling is dropped entirely when stderr is not a terminal — escape codes in a
redirected log are worse than no colour — and `NO_COLOR` is honoured by its presence.

### `REC` goes in the title bar

The indicator has to be visible to you and invisible to the capture. Decorations are excluded from
the readback by construction, so the window title is exactly the right place; anything drawn into
the window would be recorded into every frame of the recording it was announcing. Notices go to
stderr for the same reason.

### The file is constant frame rate

ffmpeg's rawvideo demuxer assumes CFR, so rather than emit a variable-rate stream plus a timestamp
sidecar to reconstruct it from, the writer resamples onto fixed slots: it duplicates when the
content clock has passed the next slot and drops when a later frame reaches the same one.

Two things fall out that are worth having anyway. The resampler's bookkeeping **is** the drop
accounting. And a video whose wall-clock duration matches the session it recorded is far easier to
reason about than one whose timing must be reconstructed.

### Drops are counted, never silent

```text
capture: 892 frames written, 61 superseded at the target rate, 4 DROPPED (4 queue-full, 0 lost)
```

Three distinct numbers, because they mean different things:

- **superseded** — a later frame reached the same slot. Recording a 60 fps display at 30 fps
  discards every other frame here. Expected, not a fault.
- **queue-full** — the writer could not keep up. The queue is deliberately shallow (four frames,
  64 MB at 2× scale); a deep one would trade a bounded, counted drop for unbounded memory and a
  stall on the render thread.
- **lost** — a gap in the sequence numbers that the queue-full count does not explain.

This repo has expensive precedent for the third: `CofdmRxStats::lost` exists because a receiver
that silently discarded frames read as a *perfect link*. A capture that quietly drops a third of
its frames and reports success is the same failure in a different costume.

### A resize stops the recording

A rawvideo stream carries no way to signal a resolution change, so feeding ffmpeg a
differently-sized frame yields a corrupt file rather than an error. The window is resizable and
moving it between displays changes `pixels_per_point`, so this is reachable rather than
theoretical — the recording stops and says why.

## What a recording of COFDM is not

**A recording of COFDM is not a recording of real time.** Sample consumption is frame-paced —
`dt × fs`, clamped to 4096 — and at 1.92 MHz that clamp binds hard, so the spectrum advances at
about an eighth of the rate the burst timer believes. Slowing the frame rate to record makes it
worse, and puts it in an artifact someone will later read as truth.

Starting a recording on a source whose budget is clamped prints a warning naming the effect. The
five narrowband sources are unaffected: their budget never reaches the clamp, so a slower frame
rate consumes proportionally more samples per frame and the recording is faithful.

## Configuration

```yaml
view:
  capture:
    dir:      "./capture"    # output directory, relative to CWD; `~/` expands
    overlays: true           # include help / settings / instrument overlays
    fps:      30             # video frame rate
    format:   mp4            # mp4 (ffmpeg) or png (frame sequence)
```

`--capture <dir>` overrides `dir`. The default is **`./capture`**, beside the project rather than
in `$HOME`: captures are usually taken *of* something being worked on, so they belong next to it
and are one line to add to a `.gitignore`. The directory is created on the first capture, not at
startup, so a session that never captures leaves no trace.

`overlays: false` has a wrinkle worth knowing: **a frame cannot be rendered twice**, so capturing
without overlays means not drawing them at all for that frame — the live window loses them too. For
a still that is one frame's flicker and imperceptible. For a recording it holds throughout, which
is usually what a clean demo wants anyway.

## Not available headless

`--capture` is refused with `--headless`. Capture reads back a rendered surface and a headless run
has no renderer — accepting the flag there would promise an artifact that never appears, which is
worse than not offering it. Scripted capture would need pixels from somewhere else; see
[headless.md](headless.md) for what a headless run *can* produce.
