# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

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
