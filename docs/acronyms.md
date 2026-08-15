<!--
  Copyright (c) 2026 G & R Associates LLC
  SPDX-License-Identifier: MIT OR Apache-2.0
-->

# Acronym Glossary

Expansions for the acronyms used across the `orion-sdr-view` source, docs and on-screen display.

This is the **viewer's** glossary: HUD field labels, config keys, and the signal concepts a user
meets on screen. For the DSP internals behind them — equalizers, carrier plans, code families,
window functions — see [`orion-sdr`'s glossary](https://github.com/skynavga/orion-sdr/blob/main/docs/acronyms.md),
which this one deliberately does not duplicate in depth.

<!-- A glossary row is one entry; wrapping it would split an entry across lines for no gain. -->
<!-- markdownlint-disable MD013 -->

| Acronym | Expansion | Notes |
| ------- | --------- | ----- |
| AM | Amplitude Modulation | The AM DSB source: double-sideband, modulated from looped audio |
| AWGN | Additive White Gaussian Noise | The impairment every source adds; scaled by `cn_db`, see [impairment.md](impairment.md) |
| BER | Bit Error Rate | Fraction of bits decoded wrongly; see `CBER`/`IBER` for which stage's output is meant |
| BPSK | Binary Phase-Shift Keying | 1 bit/symbol; PSK31's BPSK31 mode, and COFDM's header modulation |
| BW | Bandwidth | Di bar field. **Authoritative** for what COFDM actually transmits — the `Bandwidth` fraction is only a label once `Edge guard` overrides it |
| C/N | Carrier-to-Noise ratio | Every source's impairment knob (`cn_db`), in dB. A ratio rather than an absolute noise amplitude, which is what makes `fs_hz` safe to configure |
| CBER | Channel Bit Error Rate | Pre-FEC: measured at the inner decoder's *input*. First rung of the error ladder |
| CFR | Constant Frame Rate | Every frame the same duration apart. ffmpeg's rawvideo demuxer assumes it, so a recording is resampled onto fixed slots rather than carrying timestamps. See [capture.md](capture.md) |
| COFDM | Coded OFDM | The wideband source: framed, concatenated-FEC OFDM at 1.92 MHz. See [cofdm.md](cofdm.md) |
| CP | Cyclic Prefix | `cp_len` = 32 here. The guard samples a receiver discards, and the budget the taper and mask share |
| CR | Code Rate | The **inner** FEC's rate, on the instrumentation panel. The outer block code is a separate stage |
| CRC | Cyclic Redundancy Check | Frame check; a CRC failure is what makes a frame count toward `FER` and `err` |
| CW | Continuous Wave | Morse-code keyed carrier; one of the six sources |
| dBFS | Decibels relative to Full Scale | `lvl` and `pk`, measured against **the source's own full scale, not 1.0** — COFDM's modulator peaks well above unity |
| DC | Direct Current | The zero-frequency bin. `include_dc` decides whether COFDM occupies it |
| Di | Decode bar, Info line | `D` cycles off → Di → Dt → off. Modulation, carrier, BW, SNR — plus a prioritised subset of the COFDM instrument readings |
| DSB | Double-Sideband | Both sidebands transmitted; see AM |
| DSP | Digital Signal Processing | — |
| dt | Frame delta | Seconds of signal one pass advances. **Injected, not measured**, which is what makes a scripted run reproducible; 1/60 s in both the interactive app and the replay driver. Not `Dt` |
| Dt | Decode bar, Text line | The smooth-scrolling teletype ticker, for the sources that decode to text (CW, PSK31, FT8/FT4). Not `dt` |
| DVB-T | Digital Video Broadcasting – Terrestrial | Not implemented here. Named where this viewer's labels deliberately avoid DVB's, e.g. `IBER` rather than `VBER` |
| EVM | Error Vector Magnitude | Soft-vs-ideal constellation distance, in dB. `MER_dB = −EVM_dB` exactly, so one reading fills both fields |
| FEC | Forward Error Correction | COFDM concatenates an inner and an outer code; every error-ladder rung refers to the **inner** one |
| FER | Frame Error Rate | The *fraction* of frames that fail to decode. A rate, where `err` beside it is a running count |
| FFT | Fast Fourier Transform | The spectrum transform; `Alt+←/→` moves a marker by exactly one FFT bin |
| FT4 | Fast Telegraphy 4-FSK | 4-FSK weak-signal mode; 6-second transmit period |
| FT8 | Fast Telegraphy 8-FSK | 8-FSK weak-signal mode; 15-second transmit period |
| GUI | Graphical User Interface | The optional `gui` feature; `--no-default-features` builds the DSP library alone |
| HUD | Heads-Up Display | The text overlaid on the spectrum pane. Field widths are fixed throughout, so a value gaining a digit cannot reflow its neighbours |
| IBER | Inner-decoder Bit Error Rate | BER at the inner decoder's *output*, before the outer code. Second rung of the ladder |
| IQ | In-phase / Quadrature | Complex baseband. COFDM is the only complex-valued source; the other five are real |
| ISO 8601 | (date and time format) | How a capture is named. **Basic** format in filenames (`20260816T112233.456Z`), **extended** in metadata (`2026-08-16T11:22:33.456Z`); mixing the two in one representation is not conformant |
| JSONL | JSON Lines | The dump format: one JSON object per line. Chosen over CSV because a column cannot hold `null` without a sentinel. See [headless.md](headless.md) |
| LDPC | Low-Density Parity-Check | One of the inner FEC families COFDM's `CR` and `FEC` lock may refer to |
| LO | Local Oscillator | Receiver frequency reference. Its leakage lands on DC, which is why `include_dc` defaults off |
| MCS | Modulation and Coding Scheme | Per-frame index mapping to a constellation and an inner/outer FEC pair |
| MER | Modulation Error Ratio | Signal quality in dB, higher is better. The sign-flipped `EVM` |
| OFDM | Orthogonal Frequency-Division Multiplexing | The waveform under COFDM's frame layer |
| OVL | Overload | HUD indicator; raised against the source's own full scale, not against 1.0 |
| PER | Packet Error Rate | What `FER` is called under a packet-oriented profile |
| PNG | Portable Network Graphics | Still capture format, and the no-dependency alternative to mp4 for recordings |
| PSK31 | Phase-Shift Keying, 31 baud | 31.25 baud keyboard-to-keyboard mode; BPSK31 and QPSK31 variants |
| QPSK | Quadrature Phase-Shift Keying | 2 bits/symbol; PSK31's QPSK31 mode |
| RF | Radio Frequency | Upconverted (non-baseband) signal; `rf_hz` places a modulated band |
| RGBA | Red, Green, Blue, Alpha | The 8-bit-per-channel pixel layout a capture reads back and pipes to the encoder |
| RMS | Root Mean Square | Per-block level. What the loop timer compares against a threshold to call signal vs. gap |
| SDR | Software-Defined Radio | — |
| SHA-256 | Secure Hash Algorithm, 256-bit | The script digest in a dump's header record, so a dump names the script that produced it |
| SIM | Simulated | Instrumentation badge: the panel is running off a simulation rather than a receiver. Absent unless some field is simulated |
| SNR | Signal-to-Noise Ratio | In dB. The narrowband estimator compares one peak bin against the floor, which a multi-carrier signal defeats — hence `C/N` for COFDM |
| TS | Transport Stream | The MPEG-2 packet stream DVB-T carries. Permanently blank here: generic COFDM has no transport-stream layer |
| UTC | Coordinated Universal Time | `display.time_zone` default; also `local` or an explicit `+HH:MM` offset |
| VBER | Viterbi Bit Error Rate | DVB's name for `IBER`. **Deliberately not used** — it names a decoder the inner FEC need not be |
| WPM | Words Per Minute | CW keying speed (`sources.cw.wpm`) |
| YAML | YAML Ain't Markup Language | The configuration format; see [configuration.md](configuration.md) |
| Δf | Frequency error | Residual carrier offset the COFDM receiver measures, in Hz |
| Δt | Delay spread | Channel delay spread. Blank by design — a flat channel measures a large spread that depends only on the occupancy |

<!-- markdownlint-enable MD013 -->
