<!--
  Copyright (c) 2026 G & R Associates LLC
  SPDX-License-Identifier: MIT OR Apache-2.0
-->

# Keyboard Shortcuts

`H` or `?` brings the same list up in the app.

## Display

| Key | Action |
| --- | --- |
| `1` / `2` / `3` | Toggle Spectrum / Persistence / Waterfall panes |
| `E` | Toggle persistence envelope overlay |
| `P` | Toggle peak hold line |
| `W` | Cycle pane 3 between vertical waterfall and horizontal spectrogram |
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
| `←` / `→` | Pan frequency view (coarse, 1/12 of span per press; zooms in first if at full span) |
| `Shift+←` / `Shift+→` | Pan frequency view (fine, 10% of coarse) |
| `Ctrl+Shift+←` / `Ctrl+Shift+→` | Pan frequency view (extra-fine, 1% of coarse) |
| `↑` / `↓` | Zoom in / out (±0.5×) |
| `Shift+↑` / `Shift+↓` | Fine zoom in / out (±0.1×) |
| `Z` | Center view to mid-band (keeps zoom) |

## Markers

| Key | Action |
| --- | --- |
| `A` / `B` (Shift) | Place marker A / B at center |
| `a` / `b` | Toggle marker A / B visibility |
| `Tab` | Cycle active marker |
| `Ctrl+←/→` | Move active marker (coarse) |
| `Alt+←/→` | Move active marker (one FFT bin) |

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
