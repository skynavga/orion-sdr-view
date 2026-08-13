<!--
  Copyright (c) 2026 G & R Associates LLC
  SPDX-License-Identifier: MIT OR Apache-2.0
-->

# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- **COFDM has a band-centre knob, and `L` now retunes it.** `sources.cofdm.center_hz`
  plus a `Center` settings row, wired through the same
  `decode_carrier_hz` / `set_carrier_hz` pair the five narrowband sources use.
  `L` (lock source to viewport centre) was previously a documented no-op on
  COFDM — a key that did nothing on one source of six and said nothing about it.

  Named `center_hz` rather than `carrier_hz` because an OFDM band has no
  carrier: the DC subcarrier is null by default. The trait surface keeps the
  `*_carrier_hz` names, which are the concept-independent ones, and that is what
  makes the key uniform without a per-source special case anywhere.

  **The centre and the edge guard are one constraint, not two.**
  `COFDM_MIN_EDGE_GUARD` was a constant only because the centre was: pinned at
  `fs/4` the widest band that fits is `n_fft/4 - 1` carriers per side, giving
  64. It is now `cofdm_min_edge_guard(center, fs)`, and at the default centre it
  reproduces 64 exactly — the check that says this is a generalisation rather
  than a rewrite. Both are resolved through `CofdmShaping::effective`, which was
  already the single resolver every consumer agrees on, rather than gaining a
  second clamp at the settings row.

  The consequence, stated because it is a behaviour and not a bug: an off-centre
  band cannot be as wide as a centred one, so the wider `bandwidth` fractions
  become unreachable as the centre moves out. At half the default centre only 31
  carriers per side fit, so 1/8, 1/4 and 1/3 still do and 1/2 and up are clamped
  down. The fraction stays a label; the Di bar's `BW` readout is authoritative.

- **`sources.cofdm.fs_hz` makes the sample rate configurable.** Previously a
  constant. Needed on the critical path for a narrowband DVB-T profile, which is
  three bandwidth modes over one 2K structure — three sample rates over one
  numerology.

  **No settings row, deliberately:** changing the rate re-derives Nyquist and
  clears the waterfall, persistence and spectrogram, since bin-indexed history
  at the old scaling cannot be drawn at the new one. An arrow-nudged row would
  wipe the display on every keypress. This is the one place where a config key
  is right and a live knob is wrong.

  A configured rate is safe *because* 0.0.23 made the impairment a ratio. While
  it was an absolute amplitude, halving the rate would have silently changed the
  link by 3 dB — the same `noise_amp` spread over half the bandwidth — with
  nothing on screen to say so. The bandwidth fraction is rate-independent for a
  related reason: it is a fraction of Nyquist and the spacing is `fs / n_fft`,
  so the rate cancels and "1/4" means a quarter of the display at any rate.

- **`display.zoom` sets the startup viewport span, with a matching `Zoom` row.**
  Expressed as a ratio of full Nyquist rather than a span in Hz, so one value is
  portable across sources: "open at 4x" means the same thing at 48 kHz and at
  1.92 MHz. Previously every source opened at full span, which is 0–24 kHz to
  look at a 62.5 Hz PSK31 signal.

  Precedence: the configured value applies at startup, a source's
  `preferred_span_hz` applies on switch **to** that source (COFDM states one, to
  frame its band), and the keyboard applies until the next switch. So it is a
  startup default rather than a persistent override; `R` on the Display tab
  restores it. The row's upper bound follows the active source
  (`nyquist / 1 kHz`, i.e. 24x narrowband and 960x for COFDM) so it can never
  display a ratio the viewport has silently refused.

### Fixed

- **`M` no longer cycles the COFDM occupied bandwidth, and no longer depends on
  whether the settings popover is open.** It did both: the key cycled the
  bandwidth with the popover up and did nothing with it closed.

  The state-dependence was a straightforward bug. `handle_keys` has two key
  paths — the settings overlay consumes most input and returns early, so it
  repeats the global keys itself — and the `M`/`N` dispatch was a duplicated
  `match` in both that had drifted, with the COFDM arm reaching one copy alone.
  Both paths now call one shared method, and the matches are exhaustive over
  `SourceMode` rather than ending in `_ => {}`, so a new source has to state its
  answer instead of inheriting silence.

  The binding itself was the deeper problem. `M` cycles a *mode* — a modulation
  or protocol variant, as on PSK31 (BPSK31/QPSK31) and FT8 (FT8/FT4). COFDM's
  bandwidth is a 7-way occupancy parameter with its own settings row and its own
  HUD field, which is a different kind of thing; three of the six sources have
  no mode and the key is correctly inert on them. The name matters more than
  usual here because DVB-T already uses "mode" for something specific — the
  2K/8K FFT size — with bandwidth as a separate axis, so binding `M` to
  bandwidth would leave a narrowband DVB-T profile's real mode knob nowhere to
  go. Bandwidth remains on the `Bandwidth` settings row and in the HUD.

### Changed

- `FreqView` moved from the bin into the library (`orion_sdr_view::viewport`).
  It is UI-independent arithmetic, and the zoom round-trip the settings row and
  the `↑`/`↓` keys share is worth testing rather than asserting by inspection.
  `FreqMarker`, which depends on egui for its colours, stays in the bin.

- The stale `apply_source_sample_rate` comment naming a `Spec span` settings row
  is gone. No such row existed; the `Zoom` row is now the thing it was
  describing.

## [0.0.23] - 2026-08-12

### Changed

- **BREAKING: `noise_amp` is replaced by `cn_db` in every source's config
  block.** The impairment is now a carrier-to-noise ratio in dB rather than an
  absolute amplitude, on all six sources. There is no automatic conversion; a
  config still carrying `noise_amp` is **refused with a message naming the
  replacement** rather than silently ignored — the schema has no version field,
  no `deny_unknown_fields`, and every field is `Option<T>`, so serde would
  otherwise drop the key and fall back to a default while appearing to load.

  A ratio is the only figure comparable between sources — their amplitudes,
  occupied bandwidths and display scalings all differ — and it is the only one
  that survives a display gain that is derived rather than fixed. The defaults
  (36 / 45 / 34 / 54 / 55 / 45 dB for tone / CW / AM / PSK31 / FT8 / COFDM) each
  reproduce the noise floor the old `0.05` put on screen, so the schema change
  is not also a visual change. They differ by ~20 dB because the spreading
  factors do: 62.5 Hz of PSK31 against noise over 24 kHz is 25.8 dB, COFDM's
  240 kHz against 1.92 MHz is 9 dB.

  Note there is no longer an "off": a ratio has no infinite value. The top of
  the range (70 dB) leaves a floor far below anything the display resolves.

- **The injected noise is now Gaussian on every source.** Five of the six used a
  raw uniform `xorshift`, so the same setting meant 4.8 dB more noise power on
  them than on the test tone. Calling the knob `C/N` while the noise is uniform
  makes the resulting FER and MER incomparable to anything, because the FEC
  cliff is a tail phenomenon.

  Re-measured as-built at 150 frames per point, EVM is now flat across the
  bandwidth fractions to within **0.1 dB** (-22.2 to -22.1 dB at C/N 25 dB),
  against 0.6 dB under the old fixed-amplitude impairment — a ratio equalises
  exactly where a fixed amplitude only did approximately. FER against C/N:

  | Fraction | 25 dB | 20 dB | 17 dB | 14 dB | 11 dB |
  | --- | --- | --- | --- | --- | --- |
  | 1/8 | 0.313 | 0.487 | 0.520 | 0.753 | 0.867 |
  | 1/4 | 0.000 | 0.067 | 0.187 | 0.413 | 0.567 |
  | 1/2 | 0.000 | 0.000 | 0.013 | 0.047 | 0.220 |
  | 7/8 | 0.000 | 0.000 | 0.007 | 0.040 | 0.107 |

  The 1/8 fraction is unusable at any C/N the row offers — 31% FER where 7/8 is
  error-free at the same per-carrier SNR. That is frame duration, preamble
  correlation energy and common-phase tracking variance, none of which an
  impairment knob touches.

- **`COFDM_GAIN` is gone; the display level is derived.** `render` now
  normalises the rendered burst to a target RMS (`COFDM_DISPLAY_RMS_DBFS`,
  -15 dBFS) instead of applying a fitted 121.0. One constant could not fit —
  bare OFDM's rendered power is proportional to its occupied bandwidth, so the
  measured signal-phase RMS spanned 1.344 to 3.646 across the bandwidth
  fractions. Normalising collapses that to within 1 dB of the target at every
  fraction.

  Three constants disappear with it. `COFDM_SIGNAL_THRESHOLD` (0.6) existed
  because a fitted gain put the burst an order of magnitude above the shared
  `SIGNAL_THRESHOLD`; COFDM is unit-scale now, so the shared one applies.
  `CofdmFacts::full_scale` is 1.0 like every other source, so there is no
  per-source full-scale reference to plumb to the decode worker.
  `COFDM_PREFERRED_REF_DB` moves from -15 to -36 dB, tracking the new level so
  the on-screen picture is unchanged — it is now the derivation's *input*
  rather than a description of what the gain happened to produce.

- **`ViewApp::hud_noise_amp`'s six-arm `match` became
  `SourceFactory::cn_db`**, and `ViewApp::signal_threshold`'s `match` is gone
  outright. Uniform units are what made the first a single trait call; a
  uniform scale is what removed the second. The HUD reads `c/n nndB`, lowercase and
  spaced like its neighbours (`ctr`, `span`, `zoom`, `ref -15dB`).

- **The default display reference level is -15 dBFS**, up from -20, and
  `SourceFactory::preferred_ref_db` now *defaults* to it instead of to `None`.
  Previously only COFDM stated a preference, so switching away from it left the
  narrowband sources drawing against whatever scale COFDM had set — harmless
  when the two were 5 dB apart, but COFDM's is now -36 dB, which would have left
  every other source 21 dB down until the user noticed and corrected it by hand.
  A source that wants something else overrides; COFDM does.

- **The panel's measured `C/N` is recalibrated** (`wb_cn_db`, alongside the
  unchanged `wb_spectrum_snr_db`). Two corrections: the in-band figure now
  averages *powers* rather than dB values — the latter is a geometric mean and
  sat ~5 dB low on an OFDM band measured finer than its subcarrier spacing —
  and a complex-baseband source is corrected by exactly 3.01 dB, because taking
  the real part splits the signal into two mirror lobes while symmetric complex
  noise merely halves. Requested and measured now agree within ~2 dB over
  10-30 dB at the default fraction, against ~10 dB before.

  Two limits are now documented rather than implied: the reading under-reads as
  C/N rises (slope ~0.87) because the transmit skirt contaminates the noise
  floor, and at the 7/8 fraction there is almost no out-of-band spectrum left to
  measure, so it should not be trusted there. Estimating the noise inside the
  band, from the receiver's EVM, is the fix and is separate work.

### Added

- `tests/cofdm_link_budget.rs`: an `#[ignore]`d measurement harness that
  produces the FER and EVM tables above. Kept out of CI — it pumps ~100M samples
  through a full receiver — but reproducible, so the figures are not folklore.

- `tests/impairment.rs`: the achieved C/N measured end-to-end from the source's
  own output (within 0.5 dB of the request at every bandwidth fraction and
  level), the display level met at every bandwidth, the preamble excluded from
  the power reference, the noise verified Gaussian by kurtosis, the tone's noise
  floor pinned against following its amplitude ramp, and a stale `noise_amp`
  config refused rather than ignored.

## [0.0.22] - 2026-08-11

### Added

- **A real COFDM receiver behind the instrumentation panel.** The `X` panel and
  the Di line now read a live demodulator: carrier offset, MER/EVM, the
  `CBER`/`IBER` error ladder, frame error rate and count, and the
  carrier/timing/FEC locks all come off received frames. The `SIM` badge
  disappears on its own — nothing sets `Provenance::Simulated` any more — and no
  panel layout, formatting or rendering code changed, which is what the
  provenance tagging was for. The simulation remains the fallback for a source
  that offers no complex baseband.

  Three fields stay blank (`—`) rather than being invented: `clk` (there is no
  sample-clock estimator), `Δt` and the echo verdict (the inverse transform of a
  band-limited channel estimate is a Dirichlet kernel, so a *flat* channel
  measures a large spread set only by the occupancy — and calibrating that floor
  out still left a statistic that moved the wrong way for a small echo), and
  `TS` lock (generic COFDM has no transport-stream layer).

- **Complex baseband as a first-class sample path.** `SignalSource` gained
  `last_samples_iq`, returning the complex counterpart of the block just
  emitted, so a decoder and the display cannot drift onto different samples.
  COFDM's noise moved to baseband, impairing each sample once with the real
  output as its projection.

  The receiver does *not* decode the display's real samples. That was tried
  first and does not survive measurement: the real projection carries a
  conjugate image, so every Schmidl & Cox term shares one phase and the
  carrier-offset estimate is a constant rather than a measurement — it read the
  same −0.0134 Hz for true offsets of 0, 50, 200 and 1000 Hz. Filtering the
  image away restores observability but leaves a bias large enough to destroy
  the payload.

- **Sources report their own signal phase** (`SignalSource::signal_phase`), so
  burst detection no longer has to be inferred from block RMS. Real-valued
  sources — and anything over the air — fall back to the RMS threshold
  unchanged.

- `frm`/`err` counters on the COFDM Di line, matching FT8/FT4's field, and `frm`
  in the panel's Demod row. Injected noise amplitude shown in the top bar.

- COFDM screenshots in the README.

### Changed

- **`COFDM_GAIN` moved out of the waveform config into `CofdmSource::render`.**
  It is a display scalar, and it was the only non-unity gain at any
  `OfdmConfig::new` call site in either crate. Applied once across the whole
  concatenation so preamble, training symbol and payload scale alike — the
  invariant whose failure made this source unacquirable before orion-sdr
  0.0.57, so it is now asserted rather than assumed.

- **`Noise amp` reaches the FEC cliff**, capped at 2.0 instead of 0.50. The old
  ceiling was set by burst detection, not by the link: gap noise is
  `noise_amp / sqrt(3)`, so a louder setting climbed past the RMS discriminator
  and gap detection silently stopped, well below where frames start failing.

- Requires orion-sdr 0.0.59, which fixes a streaming receiver that silently
  discarded frames and adds residual-carrier tracking across a frame.

### Fixed

- **Frame accounting no longer invents errors.** The source rewinds its looping
  buffer at the start of each burst, so `sequence_num` restarts and a receiver
  still holding the previous burst's number read that restart as a gap — 316
  invented errors across ten burst boundaries with `Noise amp` at zero. A failed
  frame is also no longer counted twice: the receiver skips past it, so the next
  good frame's sequence lands two ahead, which double-counted every error.

- **`X` now works while the settings popover is open.** The key handler returned
  early after the global keys, so `S` swapped away from the instrument panel but
  `X` could not swap back. `H` had the same defect.

- Dropped sample blocks restart frame accounting rather than being charged to
  the link, so a render-thread hiccup no longer surfaces as frame errors.

## [0.0.21] - 2026-08-09

### Added

- **COFDM instrumentation panel, on the `X` key.** Nine left-aligned columns
  covering tuning, RF level, signal quality, the error ladder, channel delay
  spread, the carrier plan, and demodulator lock states. The decode bar's info
  line (`D` → Di) carries a prioritised subset and drops the lowest-priority
  fields as the window narrows rather than clipping or scrolling. `X` is
  mutually exclusive with the help overlay and the settings popover.

  **Most readings are simulated.** The viewer does not run a COFDM receiver, so
  only tuning, RF level, C/N and the carrier-plan facts are measured or known;
  the rest are placeholders driven from the measured C/N, rendered dim behind a
  `SIM` badge. Every metric is provenance-tagged, so an over-the-air provider
  replaces the simulated block without touching the render path.

  Error metrics are named for the stage whose output they measure — `CBER` at
  the channel, `IBER` at the inner decoder, `FER`/`err` after the whole chain —
  and all FEC-derived fields refer to the **inner** code. No label names a
  decoder or code family, since the inner FEC may be LDPC, convolutional, or
  absent.

### Changed

- **COFDM's Di bar now uses the wideband SNR estimator.** `SpectralState`
  hard-coded the narrowband single-tone estimator, which takes the strongest
  bin as signal and the median of the surrounding bins as noise — on a
  multi-carrier signal those bins are mostly signal, so it measured the band
  against itself. The error grew with occupied bandwidth; at the 7/8 fraction
  it reported a clean burst as 4 dB *below* the noise floor. AM DSB and Test
  Tone keep the narrowband estimator, which is correct for them.

### Fixed

- **Crash when pressing `R` in the settings popover with no row focused.** The
  handler iterated a snapshot of the visible-row list while resetting the
  source selector, which switches the active source mid-iteration and then
  indexed past the end of the new source's rows. Affects every source, not just
  COFDM. That path also failed to report the source switch, leaving the
  settings panel and playback disagreeing about which source was active.
- **Signal-gap detection above `Noise amp` 0.173 for COFDM.** The gap carries
  only amplitude-scaled noise, whose RMS is `noise_amp/sqrt(3)`, against a
  shared signal threshold of 0.1 — so the top two-thirds of the setting's
  0–0.50 range silently produced no gaps at all: the loop timer never flipped
  to `gap` and the decode bar never showed "waiting for signal". The threshold
  is now per-source.

## [0.0.20] - 2026-08-08

### Changed

- **Renamed the wideband source `CODFM` → `COFDM`** throughout. The acronym is
  *Coded Orthogonal Frequency-Division Multiplexing*; the transposed spelling
  had been in place since 0.0.17, even though the surrounding prose already
  used the correct one. **This is a breaking change with no back-compat shim:**
  - The module path `source::codfm` becomes `source::cofdm`.
  - Public items rename accordingly — `CodfmSource`, `CodfmConfig`,
    `CodfmBwFraction`, `CodfmShaping`, `CodfmTaper`, `CodfmMask`, the
    `CODFM_*` constants, and the `codfm_*` free functions.
  - The YAML key `sources.codfm:` becomes `sources.cofdm:`. Unknown keys are
    ignored rather than rejected, so a config still using the old spelling
    loses that block **silently** — rename it.
  - The source-selector entry and HUD label now read `COFDM`.

  Rename only: no behavior, numerics, or layout changed, and no test's numeric
  assertion moved. Historical entries below are written with the corrected
  spelling even where the old one shipped.

## [0.0.19] - 2026-08-08

### Added

- **COFDM out-of-band spectral shaping** — the three `orion-sdr` transmit
  shaping levers, composed and reachable live from the settings popover under
  a `Shaping` toggle that is on by default: an edge-carrier guard (`Edge
  guard`, seeded from the `Bandwidth` fraction — the two are the same lever),
  a symbol-window taper (`Taper`, roll-off as a fraction of the guard), and a
  baseband spectral mask (`Mask`, stop-band depth). `Include DC` occupies the
  DC subcarrier. Measured at the 1/4 fraction, the skirt drops ~1.4 dB just
  outside the band edge, ~6 dB at 700 kHz and ~15 dB from 800 kHz out, while
  in-band power holds within ±0.4 dB. The levers act in different places and
  stack. The effect is largest at the narrow fractions, which leave the mask
  unoccupied bandwidth to filter into.
- YAML `cofdm:` keys `shaping`, `edge_guard`, `include_dc`, `taper`, `mask`.

### Changed

- Bumped `orion-sdr` 0.0.53 → 0.0.56 (spectral-shaping API).
- **COFDM now modulates at baseband and upconverts in the source.**
  `OfdmConfig`'s `rf_hz` rotates per symbol inside `modulate_frame`, before the
  shaping post-passes, so a DC-centered `TxLowpass` applied there would have
  deleted the signal. Two artifacts went with it: the preamble and training
  symbol were emitted at baseband while header/payload sat at 480 kHz, and
  per-block rotators left a phase step at every header/payload and frame seam.
  A single continuous rotator now spans the whole buffer.
- The spectral mask is applied once over the 40-frame concatenation rather
  than per frame, so the interior frame seams stay continuous.
- Lengthened the Schmidl & Cox preamble repeats from 16 to 64 samples. A mask
  filters the whole burst, and the repetition a receiver correlates on only
  survives where the filter's group delay is small against the repeat length.
- Set the receiver FFT-window back-off to `cp_len/2` on the COFDM config, so
  the taper and the mask's group delay land in guard samples a receiver
  discards.
- The decode bar's `BW` readout follows the effective edge guard rather than
  the bandwidth fraction, which the guard can now be nudged away from.

## [0.0.18] - 2026-08-04

### Documentation

- Documented the COFDM source in the README: added it to the signal-sources
  list and a `cofdm:` block (bandwidth fraction, signal/gap durations, noise)
  to the configuration example. Corrected the stale `orion-sdr` version
  reference (0.0.33 → 0.0.53).

## [0.0.17] - 2026-08-04

### Added

- **COFDM signal source** — a wideband coded-OFDM source built on
  `orion-sdr`'s `OfdmFrameMod`, running at its own 1.92 MHz sample rate. The
  occupied bandwidth is a selectable fraction of the display span (1/8 … 7/8,
  cycled with `M`); the band is centered on the primary marker with an
  info-only "COFDM" decode line (modulation, center, occupied bandwidth, SNR).
- **Per-source sample rate** — the display pipeline (Nyquist, spectrum,
  waterfall/persistence/spectrogram) is re-derived from the active source's
  sample rate on switch, and the viewport reframes to a wideband source's
  nominal center and span. New `SourceFactory` hooks (`nominal_center_hz`,
  `preferred_span_hz`, `preferred_ref_db`) drive this; narrowband sources are
  unaffected.
- `Z` hotkey — recenter the frequency view to mid-band, keeping the current
  zoom.
- Display setting `pan` (`spectrum` / `signal`) to choose the pan-direction
  convention.

### Changed

- Signal/gap and other time-based playback (source gaps, Test Tone ramp/pause,
  COFDM signal/gap) now run at true wall-clock time instead of scaling with the
  frame rate. Sample consumption is paced to `dt × sample_rate`, and sources
  gain a `SignalSource::advance_time(dt)` hook (used by COFDM's dt-driven
  phases and by tests).
- Frequency pan steps are span-relative and scale across sources: coarse pan is
  1/12 of the current span per keypress, fine is 10 % of coarse, extra-fine is
  1 %. Panning uses discrete key presses (frame-rate independent) and auto-zooms
  to 2× from full span so there is room to pan.
- The horizontal spectrogram now follows the main viewport span, so up/down zoom
  scales it in step with the spectrum and waterfall panes (the separate
  "Spec span" setting was removed). It renders at full frequency resolution with
  a peak-hold mapping so narrow tones stay visible.

### Fixed

- Eliminated a progressive frame-rate decay (110 → 30 fps as buffers filled):
  the waterfall and spectrogram re-uploaded their entire textures every frame.
  They now use ring buffers and upload only the changed row/column via
  `TextureHandle::set_partial`, holding a stable high frame rate.
- Fixed frequency panning that got stuck near full span (it auto-zoomed only
  1.1× and clamped the center to a tiny window) and slewed to the band edge on a
  brief key tap at high frame rates.

## [0.0.16] - 2026-08-03

### Changed

- Upgraded `egui`/`eframe` from 0.33 to 0.35 and switched the renderer from
  the glow (OpenGL) backend to `wgpu` (Metal/Vulkan), now the eframe default.
  The renderer is pinned explicitly via `eframe::Renderer::Wgpu`.
- Migrated the frame loop to the eframe 0.34+ App API: `App::update` (removed
  upstream) is split into `logic()` (sample feed, decode drain, texture
  uploads, repaint) and `ui()` (HUD, decode bar, central panes). Panels use
  the unified `egui::Panel` API (`TopBottomPanel` removed; `exact_height` →
  `exact_size`).

### Fixed

- CI now compiles and lints the GUI (`cargo check`/`clippy --features gui`,
  with a `libwayland-dev` step), closing a gap where `--no-default-features`
  never built the egui code. Fixed a latent `manual_checked_ops` clippy lint
  in `persistence.rs` surfaced by that check.

## [0.0.15] - 2026-08-03

### Changed

- Bumped `orion-sdr` dependency from 0.0.33 to 0.0.53 and refreshed
  `Cargo.lock` to latest compatible versions. Adopted the upstream rename
  `util::spectrum_snr_db` → `util::nb_spectrum_snr_db` (identical
  signature; a new `wb_spectrum_snr_db` was added upstream for wideband).

### Fixed

- Added explicit `_f32` suffixes to bare float literals in `Stroke::new`
  calls to clear the new `float_literal_f32_fallback` future-incompatibility
  lint.
- Cleaned up rustfmt import ordering and removed redundant `&` in `println!`
  arguments (clippy `useless_borrows_in_formatting`) so CI passes under
  rust 1.97.

## [0.0.14] - 2026-04-26

### Changed

- Major reorganization of source-specific code under per-source
  directories: lib code at `src/source/<S>/{source,decode,config}.rs`,
  bin app glue at `src/app/source/<S>.rs`, bin settings at
  `src/app/settings/<S>.rs`. All `mod.rs` files now contain only
  `pub mod` / `pub use`; logic lives in sibling `common.rs`.
- Cross-source dispatch is now trait-based, not match-based, via
  three traits: `SourceRows` (settings UI), `SourceFactory`
  (orchestration), and per-source `<S>Settings` (typed accessors).
  `app/settings/common.rs` and `app/sources.rs` contain zero
  per-source matches; adding a new signal source no longer requires
  editing them.
- `<S>Source::apply_params(...)` methods consolidate field updates
  with change-detection (replaces the inline downcast-and-poke
  blocks that lived in `app/sources.rs`).
- `Ft8ViewState` extracted to `src/source/ft8/state.rs` (six fields
  on `ViewApp` collapsed to one with operations as methods).
- Generic helpers (`LoopTimer`, `format_time`, `dashed_hline`)
  moved to `src/utils/{timer,format}.rs` and `src/app/utils.rs`.
- `SettingsState` storage migrated from typed per-source fields to
  `Vec<Box<dyn SourceRows>>` indexed by `SourceMode as usize`.
  Debug asserts at startup verify `sources[]` and `FACTORIES[]`
  ordering matches the `SourceMode` enum.
- "waiting for signal..." promoted from dim grey to red `ALERT_COL`
  for visibility.
- PSK31 Dt ticker text now wrapped with `"|| HH:MM:SS.mmm | ... ||"`
  burst delimiters, matching CW and FT8 (UX consistency).

### Fixed

- C-key (TestTone amplitude cycling) now preserves its cycling
  state; `reset_playback` was undoing the toggle on every press.
- `reset_playback` split into hard (R-key, switch_source) and soft
  (cycle audio / cycle msg / message commit / FT8 M+N) variants.
  Pre-existing bug that hard-reset all source rows on every cycle,
  immediately undoing the user's change.
- AmDsb in Custom-with-no-WAV state now skips the artificial 99.99s
  burst-end gap (continuous carrier modeling PTT-keyed-mic-absent)
  and the decode bar shows "no audio" in red.
- FT8 M (cycle FT8↔FT4) and N (cycle Standard↔FreeText) keys now
  work correctly; previously the live-source field cycle was
  overwritten by the next `sync_settings` tick.
- `SignalSource` doctest un-ignored; rustdoc now compile-checks the
  example (was using a stale `am_dsb::AmDsbSource` path).

## [0.0.13] - 2026-04-19

### Changed

- Updated README with CW source documentation and orion-sdr 0.0.33

## [0.0.12] - 2026-04-19

### Added

- CW (Morse code) signal source with configurable WPM, jitter,
  dash weighting, character/word spacing, rise/fall envelope,
  message text, and repeat count
- CW decode with character-timed text output in Dt mode, using
  holdoff-based signal tracking to ride through keying gaps
- FT8-style frame delimiters (`|| HH:MM:SS.fff | text ||`) and
  word-boundary spaces in CW Dt output; truncation ellipsis (…)
  when signal exceeds the 99.99 s cap
- 30 CW integration tests: source generation, character timing,
  round-trip decode at varied WPM/noise/block sizes, and
  multi-loop 5 WPM verification
- Settings panel rows for all CW source parameters (WPM, jitter,
  dash weight, char/word spacing, rise/fall, message, repeat,
  carrier, gap, noise)

### Changed

- Split `src/decode/mod.rs` into per-mode modules (`psk31.rs`,
  `cw.rs`, `amdsb.rs`, `tone.rs`, `ft8.rs`) with shared spectral
  analysis extracted to `spectral.rs`
- `LoopTimer` now uses holdoff-aware signal tracking; CW keying
  gaps no longer cause sig/gap timer flicker or spurious loop
  count increments
- Gap injection in the main thread uses holdoff-filtered state,
  fixing Di bar "waiting for signal" flicker and Dt spurious
  space injection during CW keying gaps
- R key and source cycling now reset all source settings rows to
  defaults and reconstruct the source
- CW character schedule rebuilds mid-signal when WPM or other
  timing parameters change via the settings panel
- Bumped `orion-sdr` dependency 0.0.32 → 0.0.33

## [0.0.11] - 2026-04-13

### Added

- GitHub Actions CI workflow with check/fmt/clippy and test jobs
  (stable + beta matrix)
- `.cargo/config.toml` with macOS `target-cpu=native` build flags

### Changed

- Bumped `orion-sdr` dependency 0.0.30 → 0.0.32
- Made `eframe`/`egui` optional behind a default `gui` feature so
  the library and tests build on headless CI without windowing
  system dependencies; binary requires `gui` via `required-features`
- Removed unused `bw-sim` feature
- Applied `cargo fmt` across the codebase

### Fixed

- Clippy: dead `if`/`else` with identical branches in `draw.rs`,
  `too_many_arguments` in `settings/mod.rs`,
  `manual_is_multiple_of` in `tests/ft8.rs`

## [0.0.10] - 2026-04-10

### Added

- Horizontal spectrogram as an alternate presentation for pane 3;
  `W` cycles between the vertical waterfall and the horizontal
  spectrogram. Frequency is on the y-axis over ±`spec_freq_delta_hz`
  centered on the primary marker, time is on the x-axis with "now"
  at the left flowing right across `spec_time_range_secs`
- New `src/app/spectrogram.rs` module hosting `SpectrogramDisplay`,
  a column-major ring texture with time-dilation column commits
- Config fields `view.display.spec_freq_delta_hz` (default 2000 Hz)
  and `spec_time_range_secs` (default 10 s), exposed as Display-tab
  settings rows alongside dB min/max, plus new config tests
- Version shown right-aligned on the "Keyboard Shortcuts" title row
  of the help overlay; centered two-line copyright footer at the
  bottom of the overlay
- Project-wide copyright/SPDX headers on all tracked `.rs` files and
  HTML-comment header on `README.md`/`CHANGELOG.md`; convention
  documented in `CLAUDE.md`

### Changed

- Help overlay grew to 660×600 to fit the new `W` shortcut row and
  the copyright footer
- `reset_playback` clears spectrogram history alongside other
  playback state so switching source doesn't mix old/new windows

## [0.0.9] - 2026-04-10

### Changed

- Bump `orion-sdr` dependency from 0.0.29 to 0.0.30

## [0.0.8] - 2026-04-10

### Added

- FT8/FT4 signal source (`Ft8Source`) rendering a configurable
  standard or free-text message at a chosen carrier frequency, with
  per-cycle gap, noise, and repeat controls
- FT8/FT4 decode worker integration using `Ft8StreamDecoder` from
  orion-sdr 0.0.29, including a dedicated settings popover and
  YAML config section (`[ft8]`) for mode / message type /
  callsigns / grid / free text / carrier / gap / repeat
- Three-mode `time_zone` display setting cycling `utc` → `local` →
  explicit `±HH:MM`, with an Enter sub-edit for the explicit value
  (±15 min nudges, Esc cancels). YAML accepts `utc`, `local`, or
  `±HH:MM`
- FT8 Dt ticker wraps each decoded frame as
  `"|| HH:MM:SS.fff | … ||"` so frame boundaries are visually
  unambiguous in the scrolling decode bar
- New integration test suites: `tests/ft8.rs`, `tests/psk31.rs`,
  `tests/amdsb.rs`, `tests/bandwidth.rs`, and a shared ticker
  simulation harness in `tests/common/ticker.rs`

### Changed

- Bumped `orion-sdr` dependency 0.0.28 → 0.0.29 (registry)
- FT8/FT4 source renders at a 12 kHz default carrier by shifting
  the native 1500 Hz baseband up; the decode worker reverses the
  shift before decimating, fixing decode of off-baseband carriers
- Refactored settings row drawing around a shared `RowDrawCtx`
  struct and cleaned up all clippy warnings across lib, bin, and
  tests
- Split the former monolithic `tests/decode.rs` into per-source
  files sharing the common ticker harness

## [0.0.7] - 2026-04-07

### Changed

- Migrated `Psk31Stream`, spectral analysis functions
  (`power_spectrum`, `spectrum_snr_db`, `spectrum_bw_hz`,
  `best_sync`), and constants (`SIGNAL_THRESHOLD`, `PSK31_BW_HZ`)
  to orion-sdr 0.0.28; local definitions replaced with re-exports
- Bumped `orion-sdr` dependency 0.0.27 → 0.0.28

### Fixed

- Di bar now shows zeroed BW/SNR during signal gaps for all modes
  (previously retained the last pre-gap values)
- README: corrected orion-sdr version, added missing
  `am_dsb.msg_repeat` config field, fixed `R` key description

## [0.0.6] - 2026-04-06

### Changed

- Reorganized source tree: monolithic `main.rs` (~1490 lines) split into
  `src/app/` module with `view.rs`, `sources.rs`, and `draw.rs`; settings
  popover split into per-source modules under `settings/`; viewer modules
  (freqview, persistence, spectrum, waterfall) merged into `app/`
- Added lib target for integration tests; moved decode tests to `tests/`
- Replaced standalone `gen_audio/` mini-crate with `src/utils/audio.rs`
  (parameterized, marked for Phase 8 migration to orion-sdr)
- WAV and PSK31 text fields now use two-phase editing: focused state
  allows navigation, Enter starts editing, Enter again commits
- Custom audio starts silent ("no audio"); valid WAV path preserved
  across Morse/Voice/Custom cycling and auto-reloaded on return
- Failed WAV load shows red filename, keeps focus for re-edit, clears
  audio to carrier-only, logs descriptive error to stderr
- Global keys (Q, I, M, N) now work while settings popover is open

### Fixed

- Q key deadlock when settings popover was open (send_viewport_cmd
  called inside ctx.input() closure)
- WAV error messages: format hint only on non-OS errors

### Added

- Integration tests: tone generation (12 tests), audio utilities
  (7 tests), PSK31 and AM DSB config accessors (4 tests)

## [0.0.5] - 2026-04-05

### Added

- Decode bar (Phases 1-6): optional bottom bar cycled by `D` key
  (off -> info (Di) -> text (Dt) -> off)
- Di mode: live signal info (modulation, carrier, BW, EMA-smoothed SNR)
  with 1 Hz updates for all sources (Test Tone, AM DSB, PSK31)
- Dt mode: smooth pixel-scrolling text ticker with pending queue model;
  decoded PSK31 text enters one character at a time from the right;
  SPACE injection during signal gaps maintains visual continuity
- BPSK31 streaming decode: persistent `Psk31Stream` with incremental
  demod -> decider -> varicode pipeline; characters emerge ~0.3s after
  symbol boundaries; zero errors at high SNR across 5+ loops
- QPSK31 streaming decode: `Qpsk31Demod` (differential) ->
  `StreamingViterbi` (fixed-lag, traceback depth=32) -> varicode;
  characters emerge with ~1s Viterbi latency; zero errors at high SNR
- PSK31 message modes: Canned (read-only, from config YAML) and Custom
  (editable via settings); `N` key cycles between them
- Loop timer in decode bar: `sig/gap` phase timing and loop count
- Wall-clock dt via `std::time::Instant` for accurate timer display
- `reset_playback()` helper consolidating source restart, timer reset,
  and decode flush for all user events
- 14 regression tests including 5-loop streaming decode for both BPSK31
  and QPSK31, short message parameterized tests, and full printable
  ASCII roundtrip tests

### Changed

- Bumped `orion-sdr` dependency 0.0.26 -> 0.0.27 (crates.io)
- Settings popover: Source tab on left (default), Display on right;
  unified value column alignment (VAL_X); widened to 560px; Noise amp
  always last row; single Escape dismisses popup; all rows navigable
- PSK31 defaults: repeat=3, gap=15s
- `N` key now cycles PSK31 message mode (was AM DSB audio only)
- `R` key resets source, loop timer, and decode state (was view reset only)
- Config: added `custom_message` field for PSK31

### Fixed

- Di info persisting during gap (now clears to "waiting for signal")
- Decode thread Gap clobbering Info/Text in drain loop
- Onset alignment for cross-loop block boundary misalignment
- Settings: Audio source value color now matches other fields

## [0.0.4] - 2026-03-27

### Added

- PSK31 signal source (BPSK31 and QPSK31 modes) with pre-rendered looping playback and configurable
  carrier, loop gap, and noise amplitude; configurable via `sources.psk31` in YAML
- `M` key cycles source mode (BPSK31 ↔ QPSK31 for PSK31; no-op for other sources)
- `N` key cycles AM DSB audio input (Morse / Voice / Custom) without opening settings
- `L` key toggles source lock: source freq/carrier tracks display center continuously
- HUD now shows source sub-mode (`mode b`/`mode q` for PSK31, `aud m`/`aud v`/`aud c` for AM DSB)
  and `L` flag when source is locked
- Coarse/fine/extra-fine pan snap: `Shift+←/→` snaps to 100 Hz; `Ctrl+Shift+←/→` snaps to 10 Hz;
  all pan keys implicitly step zoom in by 0.1× when at full span
- Fine zoom: `Shift+↑/↓` steps zoom ratio by ±0.1×
- dB reference shift reassigned to `[`/`]` (±5 dB)
- `step_zoom` with ratio-based zoom steps (coarse ±0.5×, fine ±0.1×); coarse steps snap to nearest
  0.5× boundary first for consistent increments
- Zoom ratio display in HUD uses rounded value from `zoom_ratio()`

### Changed

- Bumped `orion-sdr` dependency 0.0.16 → 0.0.26
- Default freq/carrier for all sources changed to 12000 Hz (nyquist/2), aligned with initial
  primary marker position
- `↑`/`↓` zoom now steps by ±0.5× ratio instead of ×1.5 factor
- `Shift+↑/↓` reassigned from dB shift to fine zoom (±0.1×); dB shift moved to `[`/`]`
- `Ctrl+Shift+←/→` reassigned from fine marker movement to extra-fine pan (10 Hz snap);
  coarse marker movement (`Ctrl+←/→`) retained

## [0.0.3] - 2026-03-23

### Changed

- README screenshot now links to the full-size image via an anchor tag

## [0.0.2] - 2026-03-23

### Added

- Example screenshot (AM-DSB input source with markers) in `docs/images/` and embedded in README

## [0.0.1] - 2026-03-22

### Added

- Three display panes: instantaneous spectrum, persistence density map, and scrolling waterfall
- Spectrum pane: Hann-windowed FFT (1024-point), dBFS scale, 10 dB grid, teal line plot,
  peak hold overlay (`P`)
- Persistence pane: 2D density histogram with thermal color map, decay, and envelope overlay (`E`)
- Waterfall pane: scrolling spectrogram with thermal color map
- Test Tone source: sine wave with xorshift64 AWGN, 4-state amplitude FSM
  (ramp up/down, pause high/low), toggled with `C`
- AM DSB source: `AmDsbMod` block driven by looped audio; built-in morse and voice clips
  embedded at compile time; custom WAV file support via settings
- `SignalSource` trait with `as_any_mut` downcasting for live parameter updates
- Settings popover (`S`): tabbed keyboard-driven UI (Display and Source tabs) with
  numeric fields, toggle fields, and WAV file path entry; `R` resets to configured defaults
- Frequency pan and zoom: `←/→` coarse/fine pan, `↑/↓` zoom (×1.5), `R` reset to full view
- dB reference shift: `Shift+↑/↓` moves the display window ±5 dB, reflected in settings
- Frequency markers: primary center marker, bracket markers A and B with Hz label display;
  placement, toggle, Tab cycling, and coarse/fine/per-bin movement via keyboard
- UV-cropped texture rendering: all three panes zoom correctly without FFT recomputation
- YAML configuration: three-tier loader (`--config`, `.orionsdr.yaml` in CWD, built-in defaults);
  partial configs silently fall back to defaults; unknown keys silently ignored
- `--config <FILE>` CLI argument (hard-fail on error); `--help` support via clap
- Help overlay (`H` / `?`): full keyboard reference rendered in-window
- DejaVu Sans Mono font embedded at compile time for consistent cross-platform rendering
- Integration tests for all config loading scenarios (`tests/config_scenarios.rs`)
