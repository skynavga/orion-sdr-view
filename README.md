<!--
  Copyright (c) 2026 G & R Associates LLC
  SPDX-License-Identifier: MIT OR Apache-2.0
-->

# orion-sdr-view

A keyboard-driven SDR spectrum visualization tool built on [egui](https://github.com/emilk/egui) /
[eframe](https://github.com/emilk/eframe_template). Displays live spectrum, persistence density,
and waterfall from a configurable signal source.

## Features

- **Three display panes** — instantaneous spectrum, persistence density map, and a cycle-able waterfall
  pane (`W`) that toggles between a vertical waterfall and a horizontal spectrogram centered on the primary
  marker (±freq delta, configurable time range)
- **Multiple signal sources** — synthetic test tone (sine + AWGN), CW (Morse code), AM DSB from looped
  audio, PSK31 (BPSK31/QPSK31), FT8/FT4, and CODFM (wideband coded-OFDM at 1.92 MHz, with a selectable
  occupied-bandwidth fraction and live out-of-band [spectral shaping](#codfm-spectral-shaping) —
  edge-carrier guard, symbol-window taper, and baseband mask)
- **Decode bar** — optional bottom bar (cycled by `D`) showing signal info
  (Di: modulation, carrier, BW, SNR) or decoded text (Dt: smooth-scrolling teletype ticker)
- **Frequency pan and zoom** — keyboard-driven viewport with coarse/fine pan snap, coarse/fine zoom, and span steps
- **Source lock** — lock source frequency/carrier to the display center marker; tracks pan, zoom, and span changes
- **Frequency markers** — primary center marker plus two bracket markers (A/B) with label display
- **Settings popover** — live adjustment of display range, source parameters, and signal properties
- **YAML configuration** — startup defaults via `--config <file>` or `.orionsdr.yaml` in CWD

## Requirements

- Rust (edition 2024)
- macOS or Linux (renders via `eframe` wgpu backend — Metal/Vulkan)
- [orion-sdr](https://crates.io/crates/orion-sdr) 0.0.56 (pulled automatically from crates.io)

The GUI dependencies (`eframe`, `egui`) are behind an optional `gui` feature
(enabled by default). Use `--no-default-features` to build and test the library
without a windowing system, e.g. on headless CI runners.

## Screen Shots

### AM-DSB Image Source

<a href="./docs/images/source-am-dsb.png">
  <img alt="AM-DSB Input Source" src="./docs/images/source-am-dsb.png" width="66%">
</a>

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
    db_min:               -100.0
    db_max:               -20.0
    time_zone:            utc     # "utc", "local", or "+HH:MM" / "-HH:MM"
    spec_time_range_secs: 10.0    # horizontal spectrogram time span
    pan:                  spectrum # arrow pan: "spectrum" (panadapter) or "signal"
  sources:
    test_tone:
      freq_hz:    12000.0
      amp_max:    0.65
      ramp_secs:  3.0
      pause_secs: 7.0   # dwell at both amplitude extremes (not a gap)
      noise_amp:  0.05
    cw:
      wpm:         13.0
      jitter_pct:  5.0
      dash_weight: 3.0
      char_space:  3.0
      word_space:  7.0
      rise_ms:     5.0
      fall_ms:     5.0
      canned_text: "CQ CQ CQ DE N0GNR"
      custom_text: "Custom message"
      msg_repeat:  3
      carrier_hz:  12000.0
      gap_secs:    10.0
      noise_amp:   0.05
    am_dsb:
      msg_repeat: 1
      carrier_hz: 12000.0
      mod_index:  1.0
      gap_secs:   7.0
      noise_amp:  0.05
    psk31:
      mode:        BPSK31             # or QPSK31
      canned_text: "CQ CQ CQ DE N0GNR"
      custom_text: "Custom message"
      msg_repeat:  3
      carrier_hz:  12000.0
      gap_secs:    15.0
      noise_amp:   0.05
    ft8:
      mode:       FT8                    # or FT4
      call_to:    CQ
      call_de:    N0GNR
      grid:       FN31
      free_text:  "CQ DX"
      carrier_hz: 12000.0
      gap_secs:   15.0
      noise_amp:  0.05
    codfm:
      bandwidth:  1/4    # occupied BW as a fraction of span: 1/8 1/4 1/3 1/2 2/3 3/4 7/8
      shaping:    true   # out-of-band spectral shaping (default true)
      edge_guard: 111    # null carriers per band edge; omit to derive from `bandwidth`
      include_dc: false  # occupy the DC subcarrier
      taper:      1/4    # symbol-window roll-off, as a fraction of the guard: off 1/8 1/4 3/8
      mask:       60     # baseband-mask stop-band depth in dB: off 40 60 80
      sig_secs:   10.0   # signal-burst duration (wall-clock seconds)
      gap_secs:   2.0    # silence gap between bursts (wall-clock seconds)
      noise_amp:  0.05
```

All fields are optional; missing fields fall back to built-in defaults.

### CODFM spectral shaping

Plain OFDM's out-of-band spectrum decays only as `~1/f`, so the transmitted signal carries a wide
skirt beyond its occupied band. The CODFM source composes the three shaping levers `orion-sdr`
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
both have to live in the guard samples a receiver discards. At CODFM's numerology (`n_fft` 256,
`cp_len` 32) that leaves 16 samples, so the mask is necessarily short and the payoff is tens of dB
rather than the 60+ dB a long-guard profile reaches. Tap count is derived from the current edge guard
and clamped to the remaining budget, which is why it is not a setting: no reachable combination can
overrun it, and no mask you ask for is silently dropped. `Taper` stops at `3/8` for the same reason —
`1/2` would spend the whole budget and leave the mask nothing.

See `orion-sdr`'s [modulate.md](https://docs.rs/orion-sdr) → *Out-of-band spectral shaping* for the
geometry and the transparency argument.

## Keyboard Shortcuts

| Key | Action |
| --- | --- |
| `1` / `2` / `3` | Toggle Spectrum / Persistence / Waterfall panes |
| `I` / `M` / `N` | Cycle input source / mode / audio or message |
| `C` | Toggle amplitude cycling (Test Tone only) |
| `D` | Cycle decode bar: off → info (Di) → text (Dt) → off |
| `E` | Toggle persistence envelope overlay |
| `L` | Lock source freq/carrier to display center (tracks pan/zoom/span) |
| `P` | Toggle peak hold line |
| `W` | Cycle pane 3 between vertical waterfall and horizontal spectrogram |
| `R` | Reset source, timers, decode state, and frequency view |
| `S` | Open/close settings popover |
| `H` or `?` | Toggle help overlay |
| `Escape` | Dismiss overlays |
| `Q` | Quit |
| `←` / `→` | Pan frequency view (coarse, 1/12 of span per press; zooms in first if at full span) |
| `Shift+←` / `Shift+→` | Pan frequency view (fine, 10% of coarse) |
| `Ctrl+Shift+←` / `Ctrl+Shift+→` | Pan frequency view (extra-fine, 1% of coarse) |
| `↑` / `↓` | Zoom in / out (±0.5×) |
| `Shift+↑` / `Shift+↓` | Fine zoom in / out (±0.1×) |
| `[` / `]` | Shift dB reference ±5 dB |
| `Z` | Center view to mid-band (keeps zoom) |
| `A` / `B` (Shift) | Place marker A / B at center |
| `a` / `b` | Toggle marker A / B visibility |
| `Tab` | Cycle active marker |
| `Ctrl+←/→` | Move active marker (coarse) |
| `Alt+←/→` | Move active marker (one FFT bin) |

## License

MIT OR Apache-2.0
