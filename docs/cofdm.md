<!--
  Copyright (c) 2026 G & R Associates LLC
  SPDX-License-Identifier: MIT OR Apache-2.0
-->

# COFDM

The wideband source: coded OFDM at 1.92 MHz, with a selectable occupied-bandwidth fraction, live
out-of-band spectral shaping, and a real receiver behind the instrumentation panel.

It is the only source that is not narrowband, not on a carrier, and not real-valued — so most of
what is specific to a single source in this viewer is specific to this one.

## Configuration

The keys live under `sources.cofdm` in the YAML; see [configuration.md](configuration.md) for the
full file and [impairment.md](impairment.md) for `cn_db`, which COFDM shares with every other
source.

### Band placement: `center_hz` and `fs_hz`

COFDM occupies a sub-band rather than sitting on a carrier, but it still sits *somewhere*.
`center_hz` says where; it defaults to Nyquist/2 (mid-display). `L` (lock source to viewport centre)
retunes it, the same as on the five narrowband sources — zoom in first, since panning has no room at
full span.

`fs_hz` sets the native sample rate, and with it Nyquist, the subcarrier spacing (`fs / 256`) and
every derived bandwidth. It has **no settings row**: changing the rate clears the waterfall,
persistence and spectrogram — bin-indexed history at the old scaling cannot be drawn at the new one
— so a live knob would wipe the display on every keypress.

**The centre and the edge guard are one constraint.** The occupied band must stay inside
`0..Nyquist`, so how wide it can be depends on where it sits: at the default centre the widest band
is 127 carriers per side, and at half that centre only 31 fit. Move the band out and the wider
`bandwidth` fractions are clamped down to what fits. The fraction stays a label; the Di bar's `BW`
readout is authoritative for what is actually transmitted.

Configuring `fs_hz` is safe precisely because the impairment is a ratio. While it was an absolute
amplitude, halving the rate would have silently changed the link by 3 dB with nothing on screen to
say so.

### Burst timing: `sig_secs` and `gap_secs`

The source transmits for `sig_secs`, falls silent for `gap_secs`, and repeats. A gap is not
cosmetic: it resets the receiver and restarts its frame accounting, so a measurement run that
outlasts a burst reports only the frames since the last gap.

**A `sig_secs` of 100 or more means the burst never ends.** The `Signal` settings row reaches it as
one press past the top of its finite range, where it reads `cont`; the `Gap` row hides while it is
set, since a burst with no end has no gap after it.

A sentinel rather than a larger maximum, because no usefully long burst is reachable by nudging at a
second per press. Any value at or above the threshold means the same thing, so a config asking for
`1.0e9` gets what it plainly intended.

**This used to be silently impossible.** Every source clamped its burst to 99.99 s — psk31, ft8,
amdsb and cw truncated their rendered buffer, COFDM clamped its phase timer — because the decode-bar
timer renders `sig NN.NN` in a fixed-width field and a wider number would reflow the HUD. A display
constraint was deciding how long a signal could last, and doing it without saying so: a 5-minute
audio file, or an FT8 repeat count that ran long, was simply cut off.

The timer now marks an overflow instead — `sig 99.99+s` — with the marker slot always present so the
field width never changes. That is the same convention the error counter already uses for a wrapped
count, and for the same reason: `99.99` and `99.99+` are very different readings, and clamping
without saying so under-reports silently.

### Spectral shaping: `shaping`, `edge_guard`, `taper`, `mask`, `include_dc`

Plain OFDM's out-of-band spectrum decays only as `~1/f`, so the transmitted signal carries a wide
skirt beyond its occupied band. The COFDM source composes the three shaping levers `orion-sdr`
provides, all reachable live from the settings popover (`S`) under a `Shaping` toggle that is **on by
default**:

| Lever | Setting | Where it acts |
| --- | --- | --- |
| Edge-carrier guard | `Edge guard` (null carriers per edge) | narrows the occupied band, moving the strongest `sinc` generators inward |
| Symbol-window taper | `Taper` (roll-off as a fraction of the guard) | the symbol seam — the near skirt, just outside the band edge |
| Baseband mask | `Mask` (stop-band depth) | the spectrum directly — dominates far out |

They compose, and together they beat either alone. `Edge guard` is seeded from the `Bandwidth`
fraction — the two are the same lever, so a narrower fraction already *is* a wider guard — and
nudging it overrides the fraction until `Bandwidth` moves again. Note that the top-line HUD keeps
naming the fraction; once the guard is overridden it is the decode bar's `BW` readout that is
authoritative. With `Shaping` off, all four parameters are ignored and the source emits the
fraction's plain carrier set.

The taper and the mask's group delay share one budget — `roll_off + group_delay ≤ cp_len/2` — since
both have to live in the guard samples a receiver discards. At COFDM's numerology (`n_fft` 256,
`cp_len` 32) that leaves 16 samples, so the mask is necessarily short and the payoff is tens of dB
rather than the 60+ dB a long-guard profile reaches. Tap count is derived from the current edge guard
and clamped to the remaining budget, which is why it is not a setting: no reachable combination can
overrun it, and no mask you ask for is silently dropped. `Taper` stops at `3/8` for the same reason —
`1/2` would spend the whole budget and leave the mask nothing.

See `orion-sdr`'s [modulate.md](https://docs.rs/orion-sdr) → *Out-of-band spectral shaping* for the
geometry and the transparency argument.

**`include_dc` takes effect only with `shaping: true`.** With shaping off the source renders the
bandwidth fraction's own derived plan, which never occupies DC, so the key — and the matching
settings row — are inert. This mirrors `edge_guard`, `taper` and `mask`.

Occupying DC was broken until orion-sdr 0.0.60 and the row was withdrawn for release 0.0.25: the
training symbol never transmitted bin 0, so the channel estimate there was noise and the equalizer
divided by it, taking EVM from −67 dB to **+55 dB** — error power above signal power — with about
half the frames failing on an otherwise clean link. Fixed upstream in 0.0.60; occupying DC now costs
nothing measurable.

Leaving it off remains the right default for anything resembling real hardware, since a
direct-conversion front end puts its LO leakage on bin 0.

## Instrumentation

`X` opens an instrumentation panel for the COFDM source: tuning and RF level, signal quality, the
error ladder, channel delay spread, the carrier-plan configuration, and demodulator lock states. The
decode bar's info line (`D` → Di) carries a prioritised subset of the same readings:

```text
COFDM 480.0kHz 240kHz  C/N 28.0   MER 26.5   CBER <1E-9   Δf +80 Hz   lvl -36.6 dBFS   lck ●●●●  SIM
```

As the window narrows the **lowest-priority fields are dropped** rather than the line being clipped
or scrolled — level first, then frequency error, then the lock run (`lck`, pinned to the right so it
always sits just before the `SIM` badge), leaving `C/N` last. Field widths are fixed, so a value
gaining or losing a digit cannot shift its neighbours.

### Reading the panel

The panel is laid out in the sections below, four label/value pairs to a row. An em-dash (`—`) means
the reading is **absent**, which is a statement in its own right: no provider could supply it. See
[acronyms.md](acronyms.md) for the terms that are not defined here.

<!-- Reference tables; a wrapped row would split one field across lines. -->
<!-- markdownlint-disable MD013 -->

#### Tuning

| Field | Meaning |
| --- | --- |
| `ctr` | Band centre, kHz. What `center_hz` asked for, as transmitted |
| `bw` | Occupied bandwidth, kHz. Authoritative where the `Bandwidth` fraction is only a label |
| `Δf` | Residual carrier frequency offset the receiver is correcting, signed Hz |
| `clk` | Sample-clock error, signed ppm. **Always absent** — there is no sample-clock estimator |

#### RF

All three read against the source's own full scale, not 1.0 — see [It is measured](#it-is-measured).

| Field | Meaning |
| --- | --- |
| `lvl` | RMS level of the block just processed, dBFS |
| `pk` | Largest sample in that same block, dBFS. Not a peak *hold*: it is per block, so it falls again |
| `OVL` | `YES` once `pk` has reached full scale, else `no`. Spelled out rather than dotted, because the lock dots below read "● is good" and an overload dot would have to read the opposite |

#### Quality

| Field | Meaning |
| --- | --- |
| `C/N` | Carrier-to-noise ratio, dB, from the wideband estimator |
| `MER` | Modulation error ratio, dB. Higher is better |
| `EVM` | Error vector magnitude, as a percentage. The same measurement as `MER`, in linear terms |
| `margin` | Signed dB of `MER` above the decode threshold (6.8 dB). Negative means the link is below the cliff |

#### Errors

The ladder, covered in full under [The error ladder](#the-error-ladder).

#### Channel

| Field | Meaning |
| --- | --- |
| `Δt` | Channel delay spread, µs. **Always absent**, for the reason given below |
| `echo` | `OK` if the strongest echo falls inside the guard interval, `OVER` if beyond it — beyond it, the echo causes inter-symbol interference. Absent, since it derives from `Δt` |

#### Config

What the source **declares** about the waveform — provenance `known`, not measured off the
received frames. A receiver that disagreed with these would not show it here.

| Field | Meaning |
| --- | --- |
| `mod` | Constellation carrying the payload, e.g. `QPSK`, `16-QAM` |
| `FFT` | Transform size, as `2K`/`4K`/`8K` where it is a whole multiple of 1024, else the raw count. 256 here |
| `GI` | Guard interval as a fraction of the transform, `cp_len / n_fft`. `1/8` here |
| `CR` | **Inner** code rate. The outer code's overhead is not folded in |

#### Demod

| Field | Meaning |
| --- | --- |
| `BR` | Payload bit rate after the inner decoder, matching what `CR` advertises. Declared, like the `Config` row |
| `frm` | Frames that decoded intact this burst — the denominator `err` is read against. Absent without a receiver: the simulation models a *rate*, and inventing a frame tally to go with it would be a fiction |
| `CAR` `TIM` `FEC` `TS` | Lock indicators: `●` locked, `○` not, `—` no provider. `FEC` is the **inner** decoder converging; `TS` is permanently absent, since generic COFDM has no transport-stream layer |

<!-- markdownlint-enable MD013 -->

Both counters — `frm` and `err` — are per-burst, zero-padded to a fixed width, and wrap at
1 000 000 with a trailing `+` (`000042+`). The marker slot is always present, so a rollover cannot
reflow the grid.

### It is measured

The viewer runs a real COFDM receiver: the source's complex baseband is demodulated frame by frame,
and the panel reads carrier offset, MER/EVM, the `CBER`/`IBER` error ladder, the locks and the frame
error count off the received frames. The `SIM` badge is gone — not removed by hand, but because
nothing on the panel is a placeholder any more, which is what provenance tagging was for.

Three fields stay blank (`—`) rather than being invented: `clk` (there is no sample-clock
estimator), `Δt` and the echo verdict (the inverse transform of a band-limited channel estimate is
a Dirichlet kernel, so a *flat* channel measures a large spread that depends only on the occupancy
— and calibrating that floor out still left a statistic that moved the wrong way for a small echo),
and `TS` lock (generic COFDM has no transport-stream layer). A reading worse than none is worse
than none.

Without a complex-baseband source the panel falls back to a simulation driven from the measured
C/N, rendered dim behind the `SIM` badge — a placeholder that looked measured would be the failure
mode worth designing against. `C/N` itself is a real fix rather than a relabelled SNR: the shared
narrowband estimator compares one peak bin against the noise floor, which a multi-carrier signal
defeats, so COFDM uses the wideband estimator instead.

`lvl`, `pk` and `OVL` are measured against **the source's own full scale, not 1.0**. The COFDM
modulator applies a large fixed gain — bare OFDM at unit gain sits below the decode threshold, and
the f32 spectrum pipeline has no `[-1, 1]` clamp — so raw samples routinely peak above 30. Read
against 1.0 they would report positive dBFS and a permanent overload.

### The error ladder

The error metrics form a ladder, each rung named for the stage whose *output* it measures, and all of
them refer to the **inner** FEC — the outer block code is a separate stage:

| Reading | Measured at the output of |
| --- | --- |
| `CBER` | the channel, i.e. before the inner decoder |
| `IBER` | the inner decoder, before the outer |
| `FER` | the whole chain — the *fraction* of frames that fail to decode (`PER` under a packet-oriented profile) |
| `err` | the whole chain — the *count* of frames that have failed in the current burst |

`FER` is a rate and `err` is a running total, so they correlate through the frame rate: `err` advances at
roughly `FER × frames-per-second`, which is in the hundreds here. The count is per-burst — it resets at
each signal gap, since the panel it annotates clears there too — and it wraps at 1 000 000, with a
trailing `+` once it has rolled over.

#### What counts as an error

Two things do, and the second is the one worth knowing about:

| | |
| --- | --- |
| **failed** | The demodulator ran on the frame and reported that it did not decode |
| **lost** | The frame **vanished with nothing reported** — a sequence-number gap the failure count does not already explain |

A frame that is neither is `decoded`, and the three partition what the receiver expected:

```text
expected = decoded + failed + lost
FER      = (failed + lost) / expected
err      =  failed + lost                    frm = decoded
```

**Counting only failures called a broken link a perfect one.** Before orion-sdr 0.0.59 the streaming
receiver discarded frames whenever its sync search ranked a later preamble ahead of an earlier one —
measured at 6 of 8 frames lost, with *zero* errors reported, because a frame that is never
demodulated never fails. Sequence numbers are what make a silent drop visible at all.

**Failures are subtracted from the gap, not merely distinguished from it.** A frame that fails to
decode is skipped, so the next good frame's sequence number is two ahead and the raw gap counts that
same frame a second time. Only gap-minus-failures is a loss; without that, `failed` and `lost` came
out identical at every noise level and the panel showed exactly twice the true count.

`FER` is `None` — not zero — until at least one frame has been accounted for. The distinction
matters most at the point it is easiest to get wrong: the BER rungs are measured by re-encoding a
*decoded* frame, so they go absent exactly when the link fails, and a zero there would read as
flawless.

`CR` is likewise the inner code rate, and the `FEC` lock indicator is the inner decoder converging.
The labels deliberately avoid naming a decoder or a code family — the inner FEC can be LDPC,
convolutional, or absent, so DVB's `VBER` spelling (after the Viterbi algorithm) would only ever be
right for one of them.

### Reading the numbers off a headless run

Everything the panel shows is also written by the replay driver, one JSON object per frame, each
field carrying its provenance. See [headless.md](headless.md); `scripts/cofdm-link.txt` and
`scripts/cofdm-degraded.txt` are ready-to-run recipes for a working and a broken link.

## Decoder view

`W` cycles pane 3 to a third mode: the receiver's own two views, side by side. It is COFDM-only —
the other five sources have no demodulator to look inside — and on them the pane says so rather than
going blank.

The split is the receiver's boundary. Left is the **signal domain**, continuous complex symbols;
right is the **coding domain**, discrete outcomes. Both halves show the same instant from either
side of one stage, which is why they sit together.

### Left: the constellation

The **equalizer's output** — `ŝ = r / Ĥ`, after channel correction and common-phase-error removal,
before the demapper. That is where a vector signal analyzer takes its constellation, and it is the
demapper's actual input.

Symbols are stamped as hollow circles coloured by point density, over the ideal points for whichever
constellation the frame header recovered. Hollow so thousands of overlapping symbols stay
distinguishable; the density colour underneath is the same ramp the persistence pane uses.

**The plot extent is fixed, not auto-scaled.** The equalizer divides out the channel including any
uniform gain, so the cloud sits at unit energy whatever the transmit amplitude was. Auto-scaling
would renormalise away the one thing a constellation is for — how far the cloud has spread.

Symbols outside the extent are **dropped, not clamped**, and counted underneath. Clamping piles the
tail onto the border and reads as a hard edge that is not in the signal, making the cloud look
tighter than it is.

### Right: the inner FEC outcome

**X is the bit's index within a single codeword** — 512 columns for the shipped LDPC — and the
codewords in a slice are **overlaid** on those same columns, not laid end to end. A cell is
therefore "at this bit position, the worst thing that happened to any codeword in this slice", and a
row is one slice of time rather than one codeword. The header says how many are stacked
(`10 codewords overlaid`).

That matters for reading a run of colour: it says *some* codeword failed at those positions, never
which one or how many. Per-codeword identity is what the overlay trades away for a readable scroll
rate — see the Y-axis note below.

The left half is the code's **message** bits and the right half its **parity**, split at the marked
boundary and labelled `msg 256` / `parity 256`. Four states, from comparing what the transmitter sent, what
arrived at the demapper, and what the inner decoder made of it:

| State | Colour | Meaning |
| --- | --- | --- |
| Clean | near-black navy `#0C1018` | the channel did not touch it |
| Corrected | teal-green `#00BE8C` | it arrived wrong and the inner code fixed it |
| Uncorrected | red `#E63C32` | it arrived wrong and is still wrong — the outer code's problem now |
| Introduced | magenta `#D246D2` | it arrived *right* and the decoder broke it |

Two more colours are not bit states but whole-row annotations, so they span the full width of a row
rather than appearing cell by cell:

| Band | Colour | Meaning |
| --- | --- | --- |
| No ground truth | warm olive `#463A1E` | a frame arrived and failed to verify, so there is nothing to compare against |
| No signal | cool slate `#282E3C` | nothing arrived at all — a different fact, hence a different hue |

Four flat colours for the states, not a ramp: these are categories, and a gradient would imply an
ordering between `Uncorrected` and `Introduced` that does not exist. The two bands are deliberately
opposite in temperature, because "a frame failed" and "there was no frame" are the two absences and
confusing them is the easiest mistake to make when reading the scroll.

`Introduced` is worth its own state rather than being folded into `Uncorrected`: a
belief-propagation decoder that fails to converge flips correct bits, and one doing that at high SNR
is broken rather than merely stretched.

The values above are the constants in `src/app/correction.rs`; changing one should change both.

The clean/corrected split is exactly what `CBER` is the ratio of, so this is that reading's per-bit
expansion rather than a second measurement of it.

### Why a failed codeword lights up its parity half

The decoder decides **message** bits. The parity half of the map is a re-encode of that decision, so
when the decoder gets a few message bits wrong, re-encoding scatters the error across roughly half
the parity positions — and since most of those bits *arrived* correct, they classify as
`Introduced`. Measured on this link: **11 wrong message bits produced 300 `Introduced` parity
bits**.

That is a syndrome, not three hundred independent decoder mistakes, and it is why the magenta
appears only to the right of the systematic boundary. The picture keeps it, because a solid parity
block is an unmistakable "this codeword is wrong" at a glance.

**The tally does not.** `fix` / `unc` / `intro` are counted over the systematic positions only, and
the denominator says so (`/ 2560 msg`), so the numbers mean what the decoder actually got wrong. A
whole-block tally overstated it by about 25x. A code with no systematic prefix — the convolutional
arm — has no subset to restrict to, so it tallies the whole block and the denominator reflects
that.

**The Y axis is time**, one row per 1/60 s, and each row is the **union** of every codeword that
landed in its slice — worst state winning per bit position.

It was one row per codeword, and that did not survive contact with the wide bandwidth fractions. At
7/8 the receiver produces about 580 codewords per second, so a 256-row pane turned over in 0.44 s
and roughly five rows appeared per rendered frame: each got one 8 ms glimpse. It also scrolled
*slower* on a worse link, because a decoded frame committed ten rows and a failed one committed a
single band. Time pacing makes the scroll independent of both bandwidth and link quality, and 256
rows is 4.3 s of history everywhere.

Union rather than sample, because the rare `Uncorrected` and `Introduced` lines are what the pane is
watched for — decimating to hit the row rate would drop nine codewords in ten at 7/8 and take those
with them. **The cost is that density is inflated in proportion to the aggregation**, which is
roughly ×10 at 7/8 and barely ×1 at 1/8. The depth is reported on the pane (`10 codewords overlaid`) so it can
be discounted, and the per-frame tally underneath stays un-aggregated as the calibrated figure.

### Reading it

- **A good link draws a near-black rectangle**, because a good link has nothing to correct. The
  tally underneath is what distinguishes "measured, and zero" from a dead pane.
- **A frame that fails its CRC has no ground truth**, so it has no map — the map would otherwise
  empty exactly when the link is worst. It commits an olive band instead, and the running count of
  those is the `fail` figure in the tally.
- **A silence gets a slate band**, at the same fixed row rate, so its height is how long the silence
  was. The constellation resets across a gap, since a held cloud shows a link that is not there, and
  both halves' counters restart with it so they share an epoch.
- **`.` holds the view** — "full stop" — when something goes past too fast to read. Both halves stop
  together and the header says `HELD`. The receiver keeps running underneath, so releasing shows
  live data rather than replaying the backlog.

### Capturing it

Both halves are CPU-side rasters, so `pane constellation` and `pane correction` write them headless
with no renderer — see [headless.md](headless.md). Neither writes anything until the receiver has
decoded a frame, because an empty pane has no pixels.
