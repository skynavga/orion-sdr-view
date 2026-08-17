<!--
  Copyright (c) 2026 G & R Associates LLC
  SPDX-License-Identifier: MIT OR Apache-2.0
-->

# Keyboard Shortcuts

`H` or `?` brings the same list up in the app.

## Display

| Key | Action |
| --- | --- |
| `1` / `2` / `3` | Toggle the Spectrum / Persistence / third pane |
| `E` | Toggle persistence envelope overlay |
| `P` | Toggle peak hold line |
| `W` | Cycle pane 3: vertical waterfall → horizontal spectrogram → constellation + correction map |
| `.` | Hold pane 3's decoder view — "full stop". Both halves stop together; releasing shows live data, not a backlog |
| `[` / `]` | Shift dB reference ±5 dB |

## Source

| Key | Action |
| --- | --- |
| `I` / `M` / `N` | Cycle input source / mode / audio or message |
| `C` | Toggle amplitude cycling (Test Tone only) |
| `D` | Cycle decode bar: off → info (Di) → text (Dt) → off |
| `L` | Lock source freq/carrier to display center (tracks pan/zoom/span) |
| `R` | Reset source, timers, decode state, and frequency view |

## Frequency view

| Key | Action |
| --- | --- |
| `←` / `→` | Pan frequency view (coarse, 1/12 of span per press; may pan past the band edge) |
| `Shift+←` / `Shift+→` | Pan frequency view (fine, 10% of coarse) |
| `Ctrl+Shift+←` / `Ctrl+Shift+→` | Pan frequency view (extra-fine, 1% of coarse) |
| `↑` / `↓` | Zoom in / out (±0.5×) |
| `Shift+↑` / `Shift+↓` | Fine zoom in / out (±0.1×) |
| `Z` | Center view to mid-band (keeps zoom) |

The view may be panned past either end of `0..Nyquist`, stopping when the band edge reaches screen
centre. The empty region is dimmed and the band edge drawn as a line, so it cannot be mistaken for a
quiet part of the band. `Z` recentres and `R` resets. See [Viewport](viewport.md#panning-past-the-band).

With the source lock (`L`) engaged the pan stays inside the band, because the lock writes the
viewport centre into the source's carrier and there is no source out past the edge to follow.

## Markers

| Key | Action |
| --- | --- |
| `A` / `B` (Shift) | Place marker A / B at center |
| `a` / `b` | Toggle marker A / B visibility |
| `Tab` | Cycle active marker |
| `Ctrl+←/→` | Move active marker (coarse) |
| `Alt+←/→` | Move active marker (one FFT bin) |

## Capture

| Key | Action |
| --- | --- |
| `F` | Capture a still to the capture directory |
| `V` | Start / stop recording |

Both work with an overlay up — a still of the settings or instrument panel is a first-class use, and
a `V` the settings panel swallowed would leave no way to stop recording. See
[capture.md](capture.md).

## Overlays

| Key | Action |
| --- | --- |
| `S` | Open/close settings popover |
| `X` | Toggle extended instrumentation panel |
| `H` or `?` | Toggle help overlay |
| `Escape` | Dismiss overlays |
| `Q` | Quit |

Overlays are mutually exclusive: opening one closes the others.

## Driving them from a script

Every binding here is reachable from a timed key script, which is what the headless replay driver
and the test harness both replay. Note that `a`/`b`, `A`/`B`, `?` and `[`/`]` are read from text
input rather than key events, so a script reaches them with `text` rather than `key`. See
[headless.md](headless.md).
