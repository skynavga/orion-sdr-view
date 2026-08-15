<!--
  Copyright (c) 2026 G & R Associates LLC
  SPDX-License-Identifier: MIT OR Apache-2.0
-->

# Viewport: `zoom`

`display.zoom` is the **startup** viewport zoom, as a ratio of the full `0..Nyquist` span — `1.0`
shows everything, `4.0` shows a quarter of it. A ratio rather than a span in Hz, so one value means
the same thing on a 48 kHz source and on a 1.92 MHz one.

## Precedence

In order:

1. The configured `zoom` applies at startup.
2. Switching **to** a source that states a preferred span reframes to it. COFDM does, to frame its
   band; the five narrowband sources state none and leave the viewport alone.
3. The `↑`/`↓` keys — and the `Zoom` row in the settings popover, which is the same control — apply
   until the next switch.

So `zoom` is a startup default, not a persistent override. `R` on the Display tab restores it.

## The reachable range is per source

The zoom stops at a 1 kHz window, which is 24x at 48 kHz and 960x for COFDM. The `Zoom` row's upper
bound follows the active source for that reason, so it can never display a ratio the viewport has
silently refused.
