# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

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
