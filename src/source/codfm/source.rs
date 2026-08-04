// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use num_complex::Complex32 as C32;
use orion_sdr::fec::{CrcKind, FrameMetadata, FramePacket};
use orion_sdr::modulate::{ConstellationOrder, McsTable, OfdmConfig, OfdmFrameMod};
use orion_sdr::multicarrier::CarrierPlan;
use orion_sdr::sync::OfdmPreamble;

use crate::source::{MAX_SIG_SECS, SignalSource};

// ── CODFM constants ───────────────────────────────────────────────────────────
//
// CODFM is a synthetic *wideband* coded-OFDM (COFDM) source.  Unlike the
// narrowband sources it does not sit near a single tunable carrier: it occupies
// a fixed sub-band and runs at its own high sample rate, which the viewer adopts
// per-source (see `ViewApp::apply_source_sample_rate`).  The signal is rendered
// natively at `CODFM_FS` — there is NO resampling, so the source mirrors the
// PSK31 single-play→gap→repeat shape rather than FT8's resample+shift path.

/// OFDM FFT size (number of subcarriers).
const CODFM_N_FFT: usize = 256;
/// Cyclic-prefix length in samples.
const CODFM_CP_LEN: usize = 32;

/// Native sample rate of the CODFM waveform (Hz).  Nyquist = 960 kHz.
/// Subcarrier spacing = `CODFM_FS / CODFM_N_FFT` = 7 500 Hz.
pub const CODFM_FS: f32 = 1_920_000.0;

/// RF upconversion frequency (Hz) = the nominal band center.  The DC-centered
/// carriers make the occupied band symmetric about this frequency, so `.re`
/// lands the band centered on the marker (at Nyquist/2, mid-display).
pub const CODFM_NOMINAL_CENTER: f32 = CODFM_FS / 4.0; // 480 kHz = Nyquist/2

/// QPSK payload from the default MCS ladder (index 1: BPSK/QPSK/QAM16/QAM64).
const CODFM_MCS_INDEX: u8 = 1;
/// Payload bytes per COFDM frame (RS(204,188)-style block minus a 4-byte CRC).
const CODFM_PAYLOAD_BYTES: usize = 184;

/// The viewer's fixed sample-consumption rate (see `app::SAMPLE_RATE`).  CODFM
/// plays back NON-realtime: the viewer consumes a fixed 48 kHz regardless of the
/// native `fs`, so both the signal burst and the silence gap are sized in
/// **wall-clock seconds** via `secs * VIEWER_CONSUME_FS` samples.  Kept local to
/// avoid a bin-crate dependency from the lib.
const VIEWER_CONSUME_FS: f32 = 48_000.0;

/// Modulator output gain.  Bare OFDM spreads its energy across the active
/// subcarriers, so per-sample RMS at unit gain sits *below* the decoder's
/// `SIGNAL_THRESHOLD` (0.1) — the Di bar would never register signal.  This
/// gain places the spectrum peaks at roughly -15 dBFS on the viewer's dB scale
/// (matched by the source's preferred -15 dB reference level) and clears the
/// detection threshold on every payload block for all bandwidth fractions.
/// The f32 spectrum pipeline has no [-1, 1] clamp, so the resulting large
/// time-domain peak is fine.
const CODFM_GAIN: f32 = 121.0;

/// Display reference level (dBFS, spectrum-scale top) preferred by CODFM, set
/// to match the ~-15 dB signal peaks produced by `CODFM_GAIN`.
pub const CODFM_PREFERRED_REF_DB: f32 = -15.0;

/// Default signal-burst duration, in **wall-clock seconds**.
pub const CODFM_DEFAULT_SIG_SECS: f32 = 10.0;
/// Default silence gap between bursts, in **wall-clock seconds**.
pub const CODFM_DEFAULT_GAP_SECS: f32 = 2.0;
/// Default additive-noise amplitude.
pub const CODFM_DEFAULT_NOISE_AMP: f32 = 0.05;

// ── Bandwidth fraction ──────────────────────────────────────────────────────

/// Occupied bandwidth as a fraction of the full display span (Nyquist).  The
/// viewport span is pinned to full Nyquist for CODFM, so this directly controls
/// how much of the display width the band fills.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CodfmBwFraction {
    OneEighth,
    OneQuarter,
    OneThird,
    OneHalf,
    TwoThirds,
    ThreeQuarters,
    SevenEighths,
}

impl CodfmBwFraction {
    /// All variants in display order (matches the settings toggle options).
    pub const ALL: &'static [CodfmBwFraction] = &[
        CodfmBwFraction::OneEighth,
        CodfmBwFraction::OneQuarter,
        CodfmBwFraction::OneThird,
        CodfmBwFraction::OneHalf,
        CodfmBwFraction::TwoThirds,
        CodfmBwFraction::ThreeQuarters,
        CodfmBwFraction::SevenEighths,
    ];

    /// The fraction value in `(0, 1)`.
    pub fn value(self) -> f32 {
        match self {
            CodfmBwFraction::OneEighth => 1.0 / 8.0,
            CodfmBwFraction::OneQuarter => 1.0 / 4.0,
            CodfmBwFraction::OneThird => 1.0 / 3.0,
            CodfmBwFraction::OneHalf => 1.0 / 2.0,
            CodfmBwFraction::TwoThirds => 2.0 / 3.0,
            CodfmBwFraction::ThreeQuarters => 3.0 / 4.0,
            CodfmBwFraction::SevenEighths => 7.0 / 8.0,
        }
    }

    /// Short label for the HUD / settings toggle (e.g. "1/4").
    pub fn label(self) -> &'static str {
        match self {
            CodfmBwFraction::OneEighth => "1/8",
            CodfmBwFraction::OneQuarter => "1/4",
            CodfmBwFraction::OneThird => "1/3",
            CodfmBwFraction::OneHalf => "1/2",
            CodfmBwFraction::TwoThirds => "2/3",
            CodfmBwFraction::ThreeQuarters => "3/4",
            CodfmBwFraction::SevenEighths => "7/8",
        }
    }

    /// Half-width (in subcarriers) of the DC-centered active carrier set for
    /// this fraction: the band spans `±half` about DC, i.e. `2*half` carriers.
    /// Clamped to the plan's usable range `±(n_fft/2 - 1)`.
    fn carrier_half(self) -> i32 {
        let spacing = CODFM_FS / CODFM_N_FFT as f32;
        let band = self.value() * (CODFM_FS / 2.0); // fraction of Nyquist
        let half = (band / 2.0 / spacing).round() as i32;
        half.clamp(1, (CODFM_N_FFT / 2) as i32 - 1)
    }
}

/// Default bandwidth fraction on startup / reset.
pub const CODFM_DEFAULT_BW_FRACTION: CodfmBwFraction = CodfmBwFraction::OneQuarter;

/// Occupied bandwidth (Hz) for a given fraction at `fs`:
/// `2 * carrier_half * fs / n_fft`.
pub fn codfm_occupied_bw(fs: f32, fraction: CodfmBwFraction) -> f32 {
    let active = (2 * fraction.carrier_half()) as f32;
    active * fs / CODFM_N_FFT as f32
}

// ── CODFM HUD helper ──────────────────────────────────────────────────────────

/// Submode line for the top HUD: the selected bandwidth fraction, e.g. "  bw 1/4".
pub fn hud_submode_str(fraction: CodfmBwFraction) -> String {
    format!("  bw {}", fraction.label())
}

// ── CodfmSource ───────────────────────────────────────────────────────────────

/// Wideband coded-OFDM (COFDM) signal source.
///
/// Pre-renders a burst of back-to-back COFDM frames — each
/// `[preamble+training][header][payload]` via [`OfdmFrameMod`] — taking the
/// real part of the upconverted IQ.  Enough frames are rendered to fill
/// `sig_secs` of wall-clock playback.  The burst plays once, followed by a
/// `gap_secs` silence gap, then repeats indefinitely without reallocation.
///
/// Playback is deliberately NON-realtime: the viewer feeds a fixed number of
/// samples per frame regardless of `fs`, so at `CODFM_FS` (1.92 MHz) the burst
/// plays slower than wall-clock.  Both `sig_secs` and `gap_secs` are therefore
/// interpreted as **wall-clock seconds** and sized in emitted samples at the
/// viewer's fixed `VIEWER_CONSUME_FS`, so a "Gap 2 s" setting yields a ~2 s
/// on-screen pause — consistent with the narrowband sources.
pub struct CodfmSource {
    pub sig_secs: f32,
    pub gap_secs: f32,
    pub noise_amp: f32,
    pub fraction: CodfmBwFraction,
    fs: f32,
    samples: Vec<f32>,
    pos: usize,
    gap_remaining: usize,
    gap_samples: usize,
    rng: u64,
}

impl CodfmSource {
    pub fn new(
        sig_secs: f32,
        gap_secs: f32,
        noise_amp: f32,
        fraction: CodfmBwFraction,
        fs: f32,
    ) -> Self {
        let gap_samples = (gap_secs * VIEWER_CONSUME_FS) as usize;
        let mut src = Self {
            sig_secs,
            gap_secs,
            noise_amp,
            fraction,
            fs,
            samples: Vec::new(),
            pos: 0,
            gap_remaining: 0,
            gap_samples,
            rng: 0x853c_49e6_748f_ea9b,
        };
        src.render();
        src
    }

    /// Build the carrier plan (sized by the bandwidth fraction), config,
    /// preamble, and MCS table, then stream enough COFDM frames to fill the
    /// target burst duration, storing the real part.
    fn render(&mut self) {
        // DC-centered data carriers ±1..=±half (DC null), so the occupied band
        // is symmetric about DC and centers at the RF frequency.
        let half = self.fraction.carrier_half();
        let data: Vec<i32> = (-half..=-1).chain(1..=half).collect();
        let plan = CarrierPlan::new(CODFM_N_FFT, CODFM_CP_LEN).with_data_carriers(data);

        let cfg = OfdmConfig::new(
            plan,
            self.fs,
            CODFM_NOMINAL_CENTER,
            CODFM_GAIN,
            ConstellationOrder::Qpsk,
        )
        .with_payload_crc(CrcKind::Crc32)
        .with_header_crc(CrcKind::Crc16);

        let preamble = OfdmPreamble::new(4, 16)
            .with_training_symbol(cfg.carrier_plan.n_fft(), cfg.carrier_plan.cp_len());
        let table = McsTable::default_ladder();
        let modu = OfdmFrameMod::new(cfg, table, preamble);

        // Stream back-to-back COFDM frames until the burst reaches the target
        // wall-clock duration (sized in emitted samples at the viewer's fixed
        // consumption rate).  Each frame carries a fresh deterministic
        // pseudo-random payload with an incrementing sequence number, so the
        // spectrum stays fully populated across all subcarriers.
        let target_samples = (self.sig_secs * VIEWER_CONSUME_FS) as usize;
        self.samples.clear();
        let mut seq = 0u32;
        while self.samples.len() < target_samples {
            let payload = self.build_payload();
            let frame = FramePacket::new(FrameMetadata::new(seq, CODFM_MCS_INDEX), payload);
            let iq: Vec<C32> = modu.modulate_frame(&frame, 0);
            self.samples.extend(iq.iter().map(|c| c.re));
            seq += 1;
        }
        self.pos = 0;
        self.gap_remaining = 0;
    }

    /// Build a deterministic pseudo-random payload of `CODFM_PAYLOAD_BYTES`.
    fn build_payload(&mut self) -> Vec<u8> {
        (0..CODFM_PAYLOAD_BYTES)
            .map(|_| (self.next_u64() & 0xff) as u8)
            .collect()
    }

    /// Recompute the gap sample count (wall-clock) after `gap_secs` changes.
    pub fn update_gap(&mut self) {
        self.gap_samples = (self.gap_secs * VIEWER_CONSUME_FS) as usize;
    }

    /// Apply fresh parameters.  Re-renders the burst if the bandwidth fraction
    /// or signal duration changed (both alter the rendered samples); gap/noise
    /// changes alone do not.
    pub fn apply_params(
        &mut self,
        sig_secs: f32,
        gap_secs: f32,
        noise_amp: f32,
        fraction: CodfmBwFraction,
    ) {
        let rerender = self.fraction != fraction || (self.sig_secs - sig_secs).abs() > f32::EPSILON;
        self.sig_secs = sig_secs;
        self.gap_secs = gap_secs;
        self.noise_amp = noise_amp;
        self.fraction = fraction;
        if rerender {
            self.render();
        }
        self.update_gap();
    }

    fn next_u64(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }

    fn xorshift(&mut self) -> f32 {
        (self.next_u64() >> 11) as f32 * (1.0 / (1u64 << 53) as f32) * 2.0 - 1.0
    }
}

impl SignalSource for CodfmSource {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn restart(&mut self) {
        self.pos = 0;
        self.gap_remaining = 0;
    }

    fn next_samples(&mut self, n: usize) -> Vec<f32> {
        // Wall-clock cap: the burst is consumed at VIEWER_CONSUME_FS, so bound
        // it by MAX_SIG_SECS of wall-clock to keep the decode-bar timer within
        // its fixed-width display.
        let max_sig_samples = (MAX_SIG_SECS * VIEWER_CONSUME_FS) as usize;
        let effective_len = self.samples.len().min(max_sig_samples);
        let mut out = Vec::with_capacity(n);
        let mut i = 0;
        while i < n {
            if self.gap_remaining > 0 {
                let gap_now = self.gap_remaining.min(n - i);
                for _ in 0..gap_now {
                    let noise = if self.noise_amp > 0.0 {
                        self.noise_amp * self.xorshift()
                    } else {
                        0.0
                    };
                    out.push(noise);
                }
                self.gap_remaining -= gap_now;
                i += gap_now;
                if self.gap_remaining == 0 {
                    self.pos = 0;
                }
            } else if self.pos < effective_len {
                let available = (effective_len - self.pos).min(n - i);
                for k in 0..available {
                    let noise = if self.noise_amp > 0.0 {
                        self.noise_amp * self.xorshift()
                    } else {
                        0.0
                    };
                    out.push(self.samples[self.pos + k] + noise);
                }
                self.pos += available;
                i += available;
                if self.pos >= effective_len {
                    self.gap_remaining = self.gap_samples;
                }
            } else {
                // samples is empty (should not happen after render())
                out.push(0.0);
                i += 1;
            }
        }
        out
    }

    fn sample_rate(&self) -> f32 {
        self.fs
    }
}
