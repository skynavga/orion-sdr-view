# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

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
