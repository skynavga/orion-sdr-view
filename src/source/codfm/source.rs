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
/// First (lowest) active data-carrier index (positive side only, DC left null).
const CODFM_CARRIER_LO: i32 = 8;
/// Last (highest) active data-carrier index.
const CODFM_CARRIER_HI: i32 = 71;

/// Native sample rate of the CODFM waveform (Hz).  Nyquist = 960 kHz.
/// Subcarrier spacing = `CODFM_FS / CODFM_N_FFT` = 7 500 Hz.
pub const CODFM_FS: f32 = 1_920_000.0;

/// RF upconversion frequency (Hz).  The occupied band is placed here so the
/// real part (`.re`) of the complex IQ lands fully inside `0..Nyquist`.
/// Equal to the nominal band center.
pub const CODFM_NOMINAL_CENTER: f32 = 480_000.0;

/// QPSK payload from the default MCS ladder (index 1: BPSK/QPSK/QAM16/QAM64).
const CODFM_MCS_INDEX: u8 = 1;
/// Payload bytes per COFDM frame (RS(204,188)-style block minus a 4-byte CRC).
const CODFM_PAYLOAD_BYTES: usize = 184;

/// Default silence gap (seconds) between COFDM frame bursts.
pub const CODFM_DEFAULT_GAP_SECS: f32 = 10.0;
/// Default additive-noise amplitude.
pub const CODFM_DEFAULT_NOISE_AMP: f32 = 0.05;

/// Analytic occupied bandwidth (Hz) for the active carrier span at `fs`:
/// `(hi - lo + 1) * fs / n_fft`.
pub fn codfm_occupied_bw(fs: f32) -> f32 {
    let active = (CODFM_CARRIER_HI - CODFM_CARRIER_LO + 1) as f32;
    active * fs / CODFM_N_FFT as f32
}

// ── CODFM HUD helper ──────────────────────────────────────────────────────────

/// CODFM has no sub-mode toggle; the HUD submode string is empty.
pub fn hud_submode_str() -> String {
    String::new()
}

// ── CodfmSource ───────────────────────────────────────────────────────────────

/// Wideband coded-OFDM (COFDM) signal source.
///
/// Pre-renders a single COFDM frame — `[preamble+training][header][payload]`
/// via [`OfdmFrameMod`] — once at construction, taking the real part of the
/// upconverted IQ.  The frame plays once, followed by a configurable silence
/// gap, then repeats indefinitely without reallocation.
///
/// Playback is deliberately NON-realtime: the viewer feeds a fixed number of
/// samples per frame regardless of `fs`, so at `CODFM_FS` (1.92 MHz) the burst
/// plays slower than wall-clock.  That is acceptable for a looped synthetic
/// demo source and keeps the sample-pacing global.
pub struct CodfmSource {
    pub gap_secs: f32,
    pub noise_amp: f32,
    fs: f32,
    samples: Vec<f32>,
    pos: usize,
    gap_remaining: usize,
    gap_samples: usize,
    rng: u64,
}

impl CodfmSource {
    pub fn new(gap_secs: f32, noise_amp: f32, fs: f32) -> Self {
        let gap_samples = (gap_secs * fs) as usize;
        let mut src = Self {
            gap_secs,
            noise_amp,
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

    /// Build the carrier plan, config, preamble, MCS table, and frame packet,
    /// then modulate one COFDM frame and store its real part.
    fn render(&mut self) {
        let data: Vec<i32> = (CODFM_CARRIER_LO..=CODFM_CARRIER_HI).collect();
        let plan = CarrierPlan::new(CODFM_N_FFT, CODFM_CP_LEN).with_data_carriers(data);

        let cfg = OfdmConfig::new(
            plan,
            self.fs,
            CODFM_NOMINAL_CENTER,
            1.0,
            ConstellationOrder::Qpsk,
        )
        .with_payload_crc(CrcKind::Crc32)
        .with_header_crc(CrcKind::Crc16);

        let preamble = OfdmPreamble::new(4, 16)
            .with_training_symbol(cfg.carrier_plan.n_fft(), cfg.carrier_plan.cp_len());
        let table = McsTable::default_ladder();
        let modu = OfdmFrameMod::new(cfg, table, preamble);

        // Deterministic pseudo-random payload (xorshift-seeded) so the spectrum
        // is fully populated across all subcarriers.
        let payload = self.build_payload();
        let frame = FramePacket::new(FrameMetadata::new(1, CODFM_MCS_INDEX), payload);
        let iq: Vec<C32> = modu.modulate_frame(&frame, 0);

        self.samples = iq.iter().map(|c| c.re).collect();
        self.pos = 0;
        self.gap_remaining = 0;
    }

    /// Build a deterministic pseudo-random payload of `CODFM_PAYLOAD_BYTES`.
    fn build_payload(&mut self) -> Vec<u8> {
        (0..CODFM_PAYLOAD_BYTES)
            .map(|_| (self.next_u64() & 0xff) as u8)
            .collect()
    }

    /// Recompute the gap sample count after `gap_secs` changes.
    pub fn update_gap(&mut self) {
        self.gap_samples = (self.gap_secs * self.fs) as usize;
    }

    /// Apply fresh timing/noise parameters.  The waveform content is fixed
    /// (no user-tunable carrier/mode), so this never re-renders the frame.
    pub fn apply_params(&mut self, gap_secs: f32, noise_amp: f32) {
        self.gap_secs = gap_secs;
        self.noise_amp = noise_amp;
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
        let max_sig_samples = (MAX_SIG_SECS * self.fs) as usize;
        // Cap the effective playback length so the signal burst never exceeds
        // MAX_SIG_SECS (keeps the decode-bar timer within bounds).
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
