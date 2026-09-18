<!--
  Copyright (c) 2026 G & R Associates LLC
  SPDX-License-Identifier: MIT OR Apache-2.0
-->

# Source Code

## Layout

Source-specific code lives in per-source directories:

- **Lib (`src/source/<S>/`)** — one directory per signal source.  Files:
  `source.rs` (the `<S>Source` impl + `<S>Source::apply_params`), `decode.rs`
  (decode worker state), `config.rs` (YAML schema + defaults).  No UI
  dependencies.
- **Bin app glue (`src/app/source/<S>.rs`)** — settings-to-source mapping:
  `make()`, `sync()`, `apply_message()`, HUD formatters, and a `Factory` ZST
  implementing `SourceFactory`.  Depends on
  `crate::app::settings::<S>Settings`.
- **Bin settings (`src/app/settings/<S>.rs`)** — `<S>Rows` row container +
  `impl SourceRows for <S>Rows` + per-source typed-accessor trait
  `<S>Settings` + `impl <S>Settings for SettingsState`.

Cross-source dispatch is **trait-based, not match-based**:

- `SourceRows` (`app/settings/common.rs`) — uniform settings UI surface.
  `SettingsState::sources: Vec<Box<dyn SourceRows>>` holds the per-source row
  containers, indexed by `SourceMode as usize`.  Every settings dispatch in
  `common.rs` is a single trait call on `self.active_source()` — no
  per-source `match`.
- `SourceFactory` (`app/source/common.rs`) — uniform source orchestration.
  `app::source::FACTORIES: &[&dyn SourceFactory]` is a static dispatch table
  indexed by `SourceMode as usize`.  `app/sources.rs::make_source` and
  `sync_decode_config` dispatch through `source_mode_factory(mode).method(...)`
  — no per-source `match`.
- Per-source typed accessors (`<S>Settings` traits) provide
  `self.settings.cw_wpm()`-style typed access via
  `SettingsState::source_as::<T>(idx)` downcast.

## Module definitions

- **`mod.rs` files contain only `pub mod` / `pub use`**.  Logic lives in a
  sibling `common.rs` (or other named submodule).  Adding code to a `mod.rs`
  is a layout violation; move it.

## Source Layout

```text
src/
  lib.rs     — crate root; `pub mod` tree for `app` (gui feature), `capture`, `config`,
               `decode`, `replay` (gui feature), `source`, `utils`, `viewport`
  main.rs    — thin entry point: parse arguments, then open a window or run the headless
               replay driver
  app/       — the bin: egui viewer app (`gui` feature; `--no-default-features` gives a
               pure-DSP library)
    mod.rs            — module tree; re-exports `ViewApp`, `SourceMode`, HUD/pane
                        constants
    common.rs         — HUD colors, pane background palette, `SAMPLE_RATE`/`FFT_SIZE`,
                        `DecodeBarMode`, `Pane3Mode`
    view.rs           — `ViewApp`: construction, replay wiring, source restart/lock,
                        capture triggers, scene info
    draw.rs           — `ViewApp`'s per-frame painting: HUD, decode bar,
                        spectrum/persistence/waterfall/decoder panes, frequency markers
    sources.rs        — cross-source orchestration: per-frame sync, source construction,
                        message commits, FT8 mode cycling, dispatched through
                        `app::source::*`
    capture.rs        — the app's side of capture: issuing `ViewportCommand::Screenshot`
                        requests, receiving images, deciding what to do with each
    instrument.rs     — the OFDM instrumentation panel (`X`) placement and painting
                        (metric model lives in `crate::decode::instrument`)
    persistence.rs    — density-to-color mapping + `PersistenceMap` (pane 2 hit-density
                        accumulation and draw)
    spectrum.rs       — `RingBuffer` + `SpectrumProcessor`: windowed-FFT and linear-fill
                        spectrum processing (pane 1)
    spectrogram.rs    — dB-to-thermal-color mapping + `SpectrogramDisplay` (pane 3
                        horizontal spectrogram ring buffer)
    waterfall.rs      — dB-to-thermal-color mapping + waterfall ring buffer,
                        cropped/ordered draw (pane 3 vertical waterfall)
    freqview.rs       — display-side frequency markers; re-exports `FreqView` from
                        `crate::viewport`
    constellation.rs  — pane 3's constellation half: equalizer output plotted as a CPU
                        raster of density-colored points
    correction.rs     — pane 3's correction-map half: per-coded-bit outcome, scrolling by
                        time, `WaterfallDisplay`-style
    utils.rs          — generic egui drawing primitives shared across pane renderers
    settings/         — per-source settings rows + the trait-based `SourceRows` dispatch
                        surface
      mod.rs      — module tree; re-exports `SettingsState` + per-source `SET_KEYS` tables
                    and `<S>Settings` accessor traits
      common.rs   — `SettingsState`, `SourceRows` trait, key-handling dispatch
                    (`HandleKeysResult`, `SetOutcome`, `SetTarget`, `SetScope`)
      field.rs    — row field kinds: `NumField`, `TextField`, `ToggleField`,
                    `TimeZoneField`, `CoarseStep`, `Row`, `RowDrawCtx`
      amdsb.rs    — `AmDsbRows` + `AmDsbSettings` accessor trait
      cofdm.rs    — `CofdmRows` + `CofdmSettings` accessor trait
      cw.rs       — `CwRows` + `CwSettings` accessor trait
      display.rs  — `DisplayRows` settings rows: zoom, pan, dB range, time zone
      dvbt.rs     — `DvbTRows` + `DvbTSettings` accessor trait
      ft8.rs      — `Ft8Rows` + `Ft8Settings` accessor trait
      psk31.rs    — `Psk31Rows` + `Psk31Settings` accessor trait
      tone.rs     — `ToneRows` + `ToneSettings` accessor trait
    source/           — per-source bin glue: settings to live source construction and sync
      mod.rs     — module tree; re-exports `FACTORIES`, `SourceFactory`, burst-delimiter
                   helpers
      common.rs  — shared bin-side source helpers not specific to a single `<S>.rs`
      amdsb.rs   — AM DSB `make()`/`sync()`/`apply_message()`/HUD formatters + `Factory`
                   (`SourceFactory` impl)
      cofdm.rs   — COFDM `make()`/`sync()`/`apply_message()`/HUD formatters + `Factory`
                   (`SourceFactory` impl)
      cw.rs      — CW `make()`/`sync()`/`apply_message()`/HUD formatters + `Factory`
                   (`SourceFactory` impl)
      dvbt.rs    — DVB-T `make()`/`sync()`/`apply_message()`/HUD formatters + `Factory`
                   (`SourceFactory` impl)
      ft8.rs     — FT8/FT4 `make()`/`sync()`/`apply_message()`/HUD formatters + `Factory`
                   (`SourceFactory` impl)
      psk31.rs   — PSK31 `make()`/`sync()`/`apply_message()`/HUD formatters + `Factory`
                   (`SourceFactory` impl)
      tone.rs    — Test Tone `make()`/`sync()`/`apply_message()`/HUD formatters +
                   `Factory` (`SourceFactory` impl)
  capture/   — image and video capture, independent of the render stack (frames arrive as
               plain RGBA)
    mod.rs     — module tree; re-exports `Frame`, `CaptureTag`, `CfrResampler`,
                 encode/meta/sink/writer, `raster` (gui feature)
    common.rs  — `Frame`, `CaptureTag`, `CaptureStats`, `CfrResampler` (irregular-frame to
                 constant-frame-rate resampler)
    encode.rs  — PNG encoding for captured frames
    meta.rs    — `RecordingMeta`/`StillMeta`/`SceneInfo`: the JSON sidecar written beside
                 every capture
    raster.rs  — (gui feature) CPU rasterizer for egui's tessellated output — the
                 headless-capture render path
    sink.rs    — `FrameSink`: numbered-PNG sequence, or a pipe to `ffmpeg`
    writer.rs  — `CaptureWriter`/`Recorder`: bounded queue, writer thread, and
                 dropped-frame accounting
  config/    — `.orionsdr.yaml` schema and three-tier config loading
    mod.rs       — module tree; re-exports `ViewConfig`, `DisplayConfig`, `CaptureConfig`,
                   `Defaults`, and per-source `<S>Config` (defined under
                   `source/<S>/config.rs`)
    common.rs    — `ViewConfig`, `SourcesConfig`, `ConfigFile::load` (three-tier: built-in
                   defaults, `.orionsdr.yaml`, `--config`)
    capture.rs   — the `view.capture` block: capture directory, `CaptureFormat`,
                   `expand_tilde`
    defaults.rs  — `Defaults`: built-in constants (dB range, zoom, per-source carrier/gap
                   defaults, capture dir)
    display.rs   — `DisplayConfig`: zoom, pan, dB range, `TzMode` + time-zone parsing
  decode/    — background decode thread shared by all sources
    mod.rs         — module tree; re-exports `DecodeWorker`, `DecodeState`, `DecodeMode`,
                     instrumentation + spectral helpers, plus test-only re-exports from
                     `crate::source::*`
    common.rs      — the background decode thread and its channel protocol:
                     `DecodeWorker`, `DecodeState`, `DecodeMode`, `DecodeChunk`,
                     `DecodeResult`, `DecodeTicker`
    instrument.rs  — OFDM instrumentation: the metric model, panel layout and value
                     formatting shared by COFDM and DVB-T
    spectral.rs    — `SpectralState`: shared windowed-FFT spectral analysis (SNR,
                     bandwidth) for AM DSB, Test Tone, and future modes
  replay/    — (gui feature) headless replay: script in, measurement stream out, no
               window/renderer/GPU
    mod.rs     — module tree; re-exports `run_file`/`run_into`,
                 `RunOptions`/`RunSummary`/`RunError`, `Dump`/`Record`/`sha256_hex`
    driver.rs  — the headless replay driver: runs the app from a script and emits the
                 measurement stream as JSON Lines
    dump.rs    — `Dump`/`Record`: one JSON object per line, carrying each `Metric<T>`'s
                 value and provenance
  source/    — per-source signal generation, no UI dependencies
    mod.rs     — module tree; re-exports `SignalSource`, C/N helpers (`CnNoise`,
                 `CnReference`, `NoiseDomain`, `mean_power`), and every per-source public
                 surface
    common.rs  — `SignalSource` trait, C/N impairment model (`CnNoise`, `CnReference`,
                 `sigma_for`), continuous-signal helpers, and the shared decode-bar
                 burst-width display bound
    amdsb/
      mod.rs     — module tree; re-exports `AmDsbConfig`, `AmDsbState`, `AmDsbSource`,
                   `BuiltinAudio`, `load_builtin`
      config.rs  — `AmDsbConfig`: YAML schema + defaults
      decode.rs  — `AmDsbState`: decode worker state, thin wrapper around `SpectralState`
      source.rs  — `AmDsbSource`: the `SignalSource` impl + `apply_params`, built-in audio
                   loading
    cofdm/
      mod.rs     — module tree; re-exports `CofdmConfig`, `CofdmState`,
                   `CofdmRx`/`CofdmRxStats`/`OfdmRxFacts`, `CofdmSource` and its
                   shaping/mask/taper types
      config.rs  — `CofdmConfig`: YAML schema + defaults
      decode.rs  — `CofdmState`: the Di info line plus the instrumentation provider
      rx.rs      — `CofdmRx`: the streaming demodulator and its frame accounting, driven
                   off the source's complex baseband
      source.rs  — `CofdmSource`: the `SignalSource` impl + `apply_params`,
                   occupied-bandwidth and out-of-band shaping
    cw/
      mod.rs     — module tree; re-exports `CwConfig`, `CwState` + timing helpers,
                   `CwSource`, `CwSyncFlags`
      config.rs  — `CwConfig`: YAML schema + defaults
      decode.rs  — `CwState`: CW (Morse code) decode state and processing
      source.rs  — `CwSource`: the `SignalSource` impl + `apply_params`,
                   `MorseEncoder`-driven keying
    dvbt/
      mod.rs     — module tree; re-exports `DvbTConfig`, `DvbTState`,
                   `DvbTRx`/`DvbTRxFacts`/`DvbTRxStats`, `DvbTSource` and its
                   shaping/mask/taper types
      config.rs  — `DvbTConfig`: YAML schema + defaults
      decode.rs  — `DvbTState`: the Di info line plus the instrumentation provider
      rx.rs      — `DvbTRx`: the streaming demodulator and its frame accounting, driven
                   off the source's complex baseband
      source.rs  — `DvbTSource`: the `SignalSource` impl + `apply_params`, TX
                   frame/super-frame assembly
    ft8/
      mod.rs     — module tree; re-exports `Ft8Config`, `Ft8State`, `Ft8Source`,
                   `Ft8Mode`, `Ft8MsgType`, `Ft8ViewState`
      config.rs  — `Ft8Config`: YAML schema + defaults
      decode.rs  — `Ft8State`: FT8/FT4 decode state and processing
      source.rs  — `Ft8Source`: the `SignalSource` impl + `apply_params`, FT8/FT4 message
                   encoding
      state.rs   — `Ft8ViewState`: per-frame view state cached on the main thread (frame
                   counts, decoded-frame timestamp, signal-onset time, cached
                   sub-mode/message-type)
    psk31/
      mod.rs     — module tree; re-exports `Psk31Config`, `Psk31State` + sync constants,
                   `Psk31Source`, `Psk31Mode`
      config.rs  — `Psk31Config`: YAML schema + defaults
      decode.rs  — `Psk31State`: PSK31 (BPSK31/QPSK31) decode state and processing
      source.rs  — `Psk31Source`: the `SignalSource` impl + `apply_params`
    tone/
      mod.rs     — module tree; re-exports `TestToneConfig`, `ToneState`, `TestSignalGen`,
                   `TestToneSource`
      config.rs  — `TestToneConfig`: YAML schema + defaults
      decode.rs  — `ToneState`: Test Tone decode, thin wrapper around `SpectralState`
      source.rs  — `TestToneSource`/`TestSignalGen`: the `SignalSource` impl +
                   `apply_params`
  utils/     — cross-layer helpers
    mod.rs     — module tree (`audio`, `format`, `script` (gui feature), `term`, `time`,
                 `timer`)
    audio.rs   — `write_wav`, tone/Morse audio generation for built-in fixtures
    format.rs  — `format_time`: wall-clock formatting shared across UI layers
    script.rs  — (gui feature) the timed key script format: the shared input for driving
                 the app without a human at the keyboard
    term.rs    — styling for the messages the app writes to the terminal
    time.rs    — system time-zone helpers and the wall clock the view layer stamps
                 timestamps from
    timer.rs   — generic loop/phase timer used by the decode bar, with keying-gap holdoff
  viewport/  — the frequency viewport: which slice of `0..nyquist` the panes draw
    mod.rs     — re-exports `FreqView`, `PanLimit`, `MAX_OVERSCAN_FRAC`, `MIN_SPAN_HZ`
    common.rs  — `FreqView`: zoom/pan arithmetic, pure and UI-independent (egui-dependent
                 markers/colors stay in the bin)

tests/
  common/               — shared test support (not auto-discovered as test binaries)
    mod.rs      — shared test support module tree
    harness.rs  — headless harness: drives `ViewApp` through complete egui passes with no
                  renderer/window/wgpu device
    ticker.rs   — shared harness for dt-mode ticker simulation: drives a `SignalSource`
                  through block-level gap detection into a caller-supplied decode callback
  amdsb.rs              — integration tests for the AM DSB source and its viewer
                          simulation
  app.rs                — the app layer, driven headless (UI-layer regression coverage)
  audio.rs              — tests for `utils::audio` (silence, tone/Morse generation, WAV
                          writing)
  bandwidth.rs          — bandwidth-measurement tests (`spectrum_bw_hz`) against AM
                          DSB-modulated signals, synthetic tones, and built-in fixtures
  capture.rs            — still and video capture tests, driven without a GPU via a bare
                          `egui::Context`
  cofdm.rs              — integration tests for the COFDM source: signal generation,
                          `SignalSource` surface, occupied bandwidth, shaping, dt-driven
                          timing
  cofdm_link_budget.rs  — `#[ignore]`d link-budget measurement harness for the COFDM
                          source (FER/EVM tables, not an assertion suite)
  cofdm_rx.rs           — integration tests for the COFDM receiver: complex-baseband tap,
                          frame accounting, measured diagnostics
  config.rs             — exercises the three-tier config loading logic without launching
                          the GUI
  cw.rs                 — integration tests for the CW source and decode timing
  dvbt.rs               — integration tests for the DVB-T source: frame geometry, ×2
                          display oversampling, buffer sizing, derived display level,
                          dt-driven timing
  dvbt_rx.rs            — integration tests for the DVB-T receiver: complex-baseband tap,
                          acquisition across the mode matrix, frame accounting, measured
                          diagnostics
  ft8.rs                — integration tests for FT8/FT4 source generation and decode
  impairment.rs         — the C/N impairment model and the derived display level, pinned
                          across sources
  instrument.rs         — integration tests for the OFDM instrumentation model:
                          formatting, provenance, panel grid, Di-bar line, response to
                          measured C/N
  overscan.rs           — panning past the band edge, checked in pixels (complements
                          `viewport.rs`'s arithmetic proof)
  panes.rs              — the pane ring buffers, asserted on CPU-side pixels
  psk31.rs              — integration tests for the PSK31 source and its decode pipeline
  raster.rs             — the CPU rasterizer, pinned against a committed reference image
                          (cross-architecture check)
  raster_oracle.rs      — the CPU rasterizer checked against the real GPU pipeline
                          (anti-divergence check)
  replay.rs             — the headless replay driver: reproducibility of the script to
                          measurement-stream pipeline
  script.rs             — the timed key script format, shared by the test harness and the
                          headless replay driver
  tone.rs               — tests for the Test Tone source and signal generator
  viewport.rs           — the frequency viewport: zoom arithmetic and the overscan rule
  reference/
    rasterizer.png  — committed reference image for `raster.rs`'s cross-architecture
                      rasterizer check
```
