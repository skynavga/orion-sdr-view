<!--
  Copyright (c) 2026 G & R Associates LLC
  SPDX-License-Identifier: MIT OR Apache-2.0
-->

# Viewport: `zoom` and panning

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

## Panning past the band

`←`/`→` may take the window past either end of `0..Nyquist`, the way most panadapters do. It stops
when the band edge reaches screen centre, so at most half the pane is ever empty and the band is
always visible on one side — `Z` recentres and `R` resets.

**This exists to separate two things that used to be one number.** While the window was held inside
the band, the distance it could travel was exactly the part of the band that was *not* on screen,
while the pan step was a fraction of the part that *was*:

```text
travel  = Nyquist - span
step    = span / 12
presses = travel / step = 12 · (ratio - 1)
```

So widening the span to make a signal look smaller was the same act as shortening the travel, and no
zoom ratio gave both. Worse, at full span the pan could not move at all — the centre had exactly one
legal value — so `←` had to zoom in before it could do anything, and how far to zoom was that same
unwinnable trade. Panning now works at every zoom, reaches every frequency in the band, and the zoom
is free to be chosen for how the signal should look.

### Reading the empty region

The part of a pane outside the band is dimmed, and the band edge itself is drawn as a line. The line
is the point: in the waterfall an absent signal is already dark, so dimming alone would not
distinguish *the band stops here* from *the band is quiet here*. Frequency grid lines continue
through the empty region — the ruler is what makes a pan legible — but the axis is not labelled out
there, since the sources are real-valued at the display tap and a negative frequency label would
assert a mirrored spectrum that is not being shown.

Nothing is drawn in the empty region. That is worth stating because the obvious implementation gets
it wrong in a way that looks right: the pane textures are sampled with `ClampToEdge`, so a texture
coordinate past the end does not come back empty — it repeats the band's edge column across the whole
region as a smooth, entirely fabricated continuation of the spectrum.

### Interaction with the source lock

With `L` engaged the pan stays inside the band. The lock writes the viewport centre into the active
source's carrier setting, and that setting clamps to the source's own range; panning into empty space
would pin it at that bound while the view kept moving, so the lock would quietly stop tracking with
nothing on screen to say why. Engaging `L` while already panned out pulls the view back into the
band for the same reason.

### A source switch drops it

Changing sample rate or auto-framing a new source re-seats the window inside the band. The empty
space was measured against the old band, so carrying a fraction of it across would land the new
source somewhere neither you nor the arithmetic chose.
