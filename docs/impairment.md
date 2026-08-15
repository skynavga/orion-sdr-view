<!--
  Copyright (c) 2026 G & R Associates LLC
  SPDX-License-Identifier: MIT OR Apache-2.0
-->

# Impairment: `cn_db`

Every source expresses its impairment as a **carrier-to-noise ratio in dB** rather than as an
absolute noise amplitude. The generator adds white Gaussian noise scaled so that, integrated over
the signal's occupied bandwidth, it produces the requested ratio:

```text
N0      = P_signal / (B_occupied * 10^(cn_db / 10))
P_noise = N0 * B_noise
```

`B_noise` is the bandwidth the generator's noise is white over — `fs/2` for the real-valued sources,
the full `fs` for the complex-baseband COFDM one. `B_occupied` is declared per source rather than
measured, because a measurement fed back into the impairment would make the noise floor wobble; for
the single-carrier sources (test tone, CW) it is a stated *reference* bandwidth of 500 Hz, which is
what makes a C/N meaningful for a signal that has no bandwidth of its own.

## Why the defaults differ by 20 dB

The defaults differ between sources because their spreading factors do: a 62.5 Hz PSK31
signal against noise spread over 24 kHz is 25.8 dB, where COFDM's 240 kHz against 1.92 MHz is 9 dB.
Five of the six reproduce the noise floor the pre-0.0.23 amplitude default put on screen.

**COFDM is the exception, at 35 dB.** It is set 10 dB noisier on purpose, because the guard, taper
and mask controls exist to shape the skirt *outside* the occupied band and there has to be a floor
on screen for that skirt to sit against. It is a display choice, not a link one — every bandwidth
fraction still decodes with zero frame errors there, against an FEC cliff around 11-14 dB.

Higher is cleaner. There is no "off" — a ratio has no infinite value — but the top of the range
(70 dB) leaves a floor far below anything the display resolves.

## A ratio rather than an amplitude

Expressing the impairment as a ratio is what makes the rest of the configuration safe to change.
While it was an absolute amplitude, halving `sources.cofdm.fs_hz` would have silently moved the link
by 3 dB with nothing on screen to say so, and every source's default had to be re-tuned by hand
whenever its bandwidth changed.

`noise_amp` was **removed in 0.0.23**, and through 0.0.24 a config still carrying it was refused
with a message naming the replacement. That window has now run: since 0.0.25 the key is simply
ignored, like any other unrecognised one. There is no automatic conversion — an old config needs
`noise_amp` deleted and `cn_db` set.
