<!--
  Copyright (c) 2026 G & R Associates LLC
  SPDX-License-Identifier: MIT OR Apache-2.0
-->

# Configuration

All parameters have built-in defaults. To override at startup, create `.orionsdr.yaml` in the
working directory or pass `--config <path>`:

```yaml
view:
  display:
    db_min:               -100.0
    db_max:               -15.0
    time_zone:            utc     # "utc", "local", or "+HH:MM" / "-HH:MM"
    spec_time_range_secs: 10.0    # horizontal spectrogram time span
    pan:                  spectrum # arrow pan: "spectrum" (panadapter) or "signal"
    zoom:                 1.0     # startup viewport zoom (1.0 = full 0..Nyquist)
  sources:
    test_tone:
      freq_hz:    12000.0
      amp_max:    0.65
      ramp_secs:  3.0
      pause_secs: 7.0   # dwell at both amplitude extremes (not a gap)
      cn_db:      36.0  # carrier-to-noise ratio in dB
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
      cn_db:       45.0
    am_dsb:
      msg_repeat: 1
      carrier_hz: 12000.0
      mod_index:  1.0
      gap_secs:   7.0
      cn_db:      34.0
    psk31:
      mode:        BPSK31             # or QPSK31
      canned_text: "CQ CQ CQ DE N0GNR"
      custom_text: "Custom message"
      msg_repeat:  3
      carrier_hz:  12000.0
      gap_secs:    15.0
      cn_db:       54.0
    ft8:
      mode:       FT8                    # or FT4
      call_to:    CQ
      call_de:    N0GNR
      grid:       FN31
      free_text:  "CQ DX"
      carrier_hz: 12000.0
      gap_secs:   15.0
      cn_db:      55.0
    cofdm:
      center_hz:  480000 # band centre; omit for Nyquist/2 (`fs_hz / 4`)
      fs_hz:      1920000 # native sample rate; sets Nyquist and subcarrier spacing
      bandwidth:  1/4    # occupied BW as a fraction of span: 1/8 1/4 1/3 1/2 2/3 3/4 7/8
      shaping:    true   # out-of-band spectral shaping (default true)
      edge_guard: 111    # null carriers per band edge; omit to derive from `bandwidth`
      include_dc: false  # occupy the DC subcarrier (needs `shaping: true`)
      taper:      1/4    # symbol-window roll-off, as a fraction of the guard: off 1/8 1/4 3/8
      mask:       60     # baseband-mask stop-band depth in dB: off 40 60 80
      sig_secs:   10.0   # signal-burst duration (wall-clock seconds); >= 100 means continuous
      gap_secs:   2.0    # silence gap between bursts (wall-clock seconds)
      cn_db:      35.0   # carrier-to-noise ratio in dB
  capture:
    dir:      "./capture"  # output directory, relative to CWD; `~/` expands
    overlays: true         # include help / settings / instrument overlays
    fps:      30           # video frame rate
    format:   mp4          # mp4 (ffmpeg) or png (frame sequence)
```

All fields are optional; missing fields fall back to built-in defaults. An unrecognised key is
ignored rather than refused.

## What the individual keys mean

- **`cn_db`**, on every source — [impairment.md](impairment.md)
- **`display.zoom`** — [viewport.md](viewport.md)
- **`sources.cofdm.*`** — [cofdm.md](cofdm.md), which covers band placement, burst timing and
  the three spectral-shaping levers
- **`capture.*`** — [capture.md](capture.md)
