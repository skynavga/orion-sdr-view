# orion-sdr-view

A keyboard-driven SDR spectrum visualization tool built on [egui](https://github.com/emilk/egui) /
[eframe](https://github.com/emilk/eframe_template). Displays live spectrum, persistence density,
and waterfall from a configurable signal source.

## Features

- **Three display panes** — instantaneous spectrum, persistence density map, and scrolling waterfall
- **Multiple signal sources** — synthetic test tone (sine + AWGN) and AM DSB from looped audio
- **Frequency pan and zoom** — keyboard-driven viewport over the full 0–Nyquist range
- **Frequency markers** — primary center marker plus two bracket markers (A/B) with label display
- **Settings popover** — live adjustment of display range, source parameters, and signal properties
- **YAML configuration** — startup defaults via `--config <file>` or `.orionsdr.yaml` in CWD

## Requirements

- Rust (edition 2024)
- macOS or Linux (uses OpenGL via `eframe` glow backend)
- [orion-sdr](https://crates.io/crates/orion-sdr) 0.0.16 (pulled automatically from crates.io)

## Building

```sh
cargo build --release
cargo run --release
```

## Configuration

All parameters have built-in defaults. To override at startup, create `.orionsdr.yaml` in the
working directory or pass `--config <path>`:

```yaml
view:
  display:
    db_min: -100.0
    db_max: -20.0
  sources:
    test_tone:
      freq_hz:    5000.0
      noise_amp:  0.05
      amp_max:    0.65
      ramp_secs:  3.0
      pause_secs: 7.0
    am_dsb:
      carrier_hz:    10000.0
      mod_index:     1.0
      loop_gap_secs: 7.0
      noise_amp:     0.05
```

All fields are optional; missing fields fall back to built-in defaults.

## Keyboard Shortcuts

| Key | Action |
| --- | --- |
| `1` / `2` / `3` | Toggle Spectrum / Persistence / Waterfall panes |
| `I` | Cycle input source (Test Tone ↔ AM DSB) |
| `C` | Toggle amplitude cycling (Test Tone only) |
| `E` | Toggle persistence envelope overlay |
| `P` | Toggle peak hold line |
| `S` | Open/close settings popover |
| `H` or `?` | Toggle help overlay |
| `Escape` | Dismiss overlays |
| `Q` | Quit |
| `←` / `→` | Pan frequency view (coarse) |
| `Shift+←` / `Shift+→` | Pan frequency view (fine) |
| `↑` / `↓` | Zoom in / out |
| `Shift+↑` / `Shift+↓` | Shift dB reference ±5 dB |
| `R` | Reset to full view (0–Nyquist) |
| `A` / `B` (Shift) | Place marker A / B at center |
| `a` / `b` | Toggle marker A / B visibility |
| `Tab` | Cycle active marker |
| `Ctrl+←/→` | Move active marker (coarse) |
| `Ctrl+Shift+←/→` | Move active marker (fine) |
| `Alt+←/→` | Move active marker (one FFT bin) |

## License

MIT OR Apache-2.0
