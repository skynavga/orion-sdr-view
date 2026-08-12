// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Shared spectral analysis state for decode modes that use windowed-FFT
//! signal characterisation (SNR, bandwidth).
//!
//! Used by AM DSB, Test Tone, and any future mode that needs rolling spectral
//! analysis for the Di info bar.

use std::sync::mpsc::SyncSender;

use num_complex::Complex32 as C32;

use super::{DecodeResult, SPECTRUM_WINDOW_SAMPLES};
use crate::source::psk31::INFO_INTERVAL;
#[allow(unused_imports)] // wb_spectrum_snr_db is re-exported for tests that compare estimators
pub use orion_sdr::util::{nb_spectrum_snr_db, power_spectrum, spectrum_bw_hz, wb_spectrum_snr_db};

/// Carrier-to-noise ratio (dB) of a wideband signal spanning `occupied_hz`
/// about `carrier_hz`, calibrated to agree with a *requested* C/N.
///
/// A recalibration of [`wb_spectrum_snr_db`], not a different idea.  Both
/// compare the occupied window against the out-of-band floor; the difference is
/// which average each side uses, and it is worth several dB:
///
/// - **In band, the mean of the powers** rather than the mean of the dB values.
///   The latter is a geometric mean, and an OFDM band measured at a resolution
///   far finer than its subcarrier spacing has real dynamic range between the
///   carriers — enough that the geometric mean sat ~5 dB under the arithmetic
///   one at the default fraction.  "Mean power across the occupied window" was
///   always the intent; averaging dB does not compute it.
/// - **Out of band, still the median**, and deliberately so.  The transmit mask
///   leaves a skirt near the band edges, so those bins are signal rather than
///   noise: at 35 dB C/N the out-of-band *mean* sat 12.7 dB above the median.
///   A median ignores the skirt as long as it stays a minority of the bins.
///
/// A [`NOISE_GUARD`]-wide exclusion zone keeps the worst of the skirt out of
/// the noise estimate, clamped so a wide signal cannot leave nothing to measure.
///
/// **Two limits worth knowing, neither of them new.**  Measured against a
/// requested C/N at the default 1/4 fraction, the readout lands within ~2 dB
/// over 10-30 dB with a slope of ~0.87 — it under-reads as the true C/N rises,
/// because the transmit skirt eventually contaminates even the guarded median.
/// And at 7/8 the occupied band fills 87.5% of the display, so there are barely
/// any out-of-band bins and they are *all* skirt: the reading compresses badly
/// (slope ~0.36) and should not be trusted at wide occupancies.  Measuring the
/// noise *inside* the band — from the receiver's EVM rather than from spectrum
/// bins — is the fix, and it is a change to the instrumentation rather than to
/// this estimator.
///
/// The caller adds any domain correction — see COFDM's
/// `REAL_PROJECTION_CN_OFFSET_DB`.
pub fn wb_cn_db(samples: &[f32], fs: f32, carrier_hz: f32, occupied_hz: f32) -> f32 {
    /// Half-span of the noise-exclusion zone, as a multiple of the occupied
    /// half-span.  1.25 buys ~0.03 of slope at the default fraction; more than
    /// that buys little and starves wide signals sooner.
    const NOISE_GUARD: f32 = 1.25;
    /// Never let the exclusion zone leave fewer than this fraction of the bins
    /// available to estimate the noise floor.
    const MIN_NOISE_BINS: f32 = 0.10;

    let (power_db, bin_hz) = power_spectrum(samples, fs);
    let n_bins = power_db.len();
    if n_bins < 3 || bin_hz <= 0.0 {
        return 0.0;
    }
    let carrier_bin = (carrier_hz / bin_hz).round() as isize;
    let half_span = ((occupied_hz / 2.0) / bin_hz).round() as isize;
    let lo = (carrier_bin - half_span).max(0) as usize;
    let hi = ((carrier_bin + half_span) as usize).min(n_bins - 1);
    if lo > hi {
        return 0.0;
    }

    let lin = |db: f32| 10f32.powf(db / 10.0);
    let occupied_mean =
        power_db[lo..=hi].iter().map(|&d| lin(d)).sum::<f32>() / ((hi - lo + 1) as f32);

    // The exclusion zone, backed off until enough bins remain to measure.
    let (mut glo, mut ghi) = (lo, hi);
    let guard = (half_span as f32 * NOISE_GUARD) as isize;
    let (wide_lo, wide_hi) = (
        (carrier_bin - guard).max(0) as usize,
        ((carrier_bin + guard) as usize).min(n_bins - 1),
    );
    let remaining = n_bins.saturating_sub(wide_hi - wide_lo + 1) as f32 / n_bins as f32;
    if remaining >= MIN_NOISE_BINS {
        glo = wide_lo;
        ghi = wide_hi;
    }

    let mut outside: Vec<f32> = power_db
        .iter()
        .enumerate()
        .filter(|&(i, _)| i > 0 && (i < glo || i > ghi))
        .map(|(_, &v)| v)
        .collect();
    if outside.is_empty() {
        return 0.0;
    }
    outside.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let noise = lin(outside[outside.len() / 2]);
    if noise <= 0.0 {
        return 0.0;
    }
    10.0 * (occupied_mean / noise).log10()
}

#[derive(Default)]
pub struct SpectralState {
    pub spec_buf: Vec<C32>,
    pub smoothed_snr_db: f32,
    pub smoothed_bw_hz: f32,
    pub info_counter: usize,
}

impl SpectralState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        self.spec_buf.clear();
        self.smoothed_snr_db = 0.0;
        self.smoothed_bw_hz = 0.0;
        self.info_counter = 0;
    }

    /// Run one block of spectral analysis.
    ///
    /// `snr_fn` computes the *raw* SNR for the current window; the EMA smoothing
    /// is applied here, so every caller gets the same response.  The estimator
    /// is caller-supplied because it is not one-size-fits-all: AM DSB, CW and
    /// Test Tone are single-tone signals and want [`nb_spectrum_snr_db`], which
    /// compares one peak bin against the noise floor, while a multi-carrier
    /// signal defeats that comparison entirely and needs
    /// [`wb_spectrum_snr_db`].  See [`SpectralState::process_nb`] for the
    /// narrowband default.
    ///
    /// `bw_fn` computes the bandwidth value for the current window.  Callers
    /// supply a mode-specific closure so that AM DSB can use EMA-smoothed
    /// `spectrum_bw_hz` while Test Tone uses raw `power_spectrum` peak, etc.
    ///
    /// Returns `true` when an `Info` was sent on this call.  COFDM uses that to
    /// emit its instrumentation on exactly the same cadence, so no field
    /// updates at a visibly different rate from its neighbours.
    ///
    /// Returns without sending if the spec buffer hasn't filled a window yet.
    #[allow(clippy::too_many_arguments)]
    pub fn process(
        &mut self,
        samples: &[f32],
        is_signal: bool,
        gap_edge: bool,
        label: &str,
        carrier_hz: f32,
        fs: f32,
        snr_fn: impl FnOnce(&[f32], f32, f32) -> f32,
        bw_fn: impl FnOnce(&[f32], f32, f32, &mut Self) -> f32,
        tx: &SyncSender<DecodeResult>,
    ) -> bool {
        if !is_signal {
            if gap_edge {
                self.spec_buf.clear();
                self.info_counter = 0;
                self.smoothed_snr_db = 0.0;
                self.smoothed_bw_hz = 0.0;
                let _ = tx.try_send(DecodeResult::Info {
                    modulation: label.to_owned(),
                    center_hz: carrier_hz,
                    bw_hz: 0.0,
                    snr_db: 0.0,
                });
            }
            return false;
        }

        self.spec_buf
            .extend(samples.iter().map(|&s| C32::new(s, 0.0)));
        if self.spec_buf.len() < SPECTRUM_WINDOW_SAMPLES {
            return false;
        }

        let decode_buf: Vec<C32> = self.spec_buf[..SPECTRUM_WINDOW_SAMPLES].to_vec();
        self.spec_buf.drain(..SPECTRUM_WINDOW_SAMPLES / 2);

        let real: Vec<f32> = decode_buf.iter().map(|c| c.re).collect();
        let raw_snr = snr_fn(&real, fs, carrier_hz);
        if self.smoothed_snr_db == 0.0 {
            self.smoothed_snr_db = raw_snr;
        } else {
            self.smoothed_snr_db = 0.2 * raw_snr + 0.8 * self.smoothed_snr_db;
        }

        let bw = bw_fn(&real, fs, carrier_hz, self);

        self.info_counter += SPECTRUM_WINDOW_SAMPLES / 2;
        if self.info_counter < INFO_INTERVAL {
            return false;
        }
        self.info_counter = 0;
        let _ = tx.try_send(DecodeResult::Info {
            modulation: label.to_owned(),
            center_hz: carrier_hz,
            bw_hz: bw,
            snr_db: self.smoothed_snr_db,
        });
        true
    }

    /// [`process`](Self::process) with the narrowband single-tone SNR
    /// estimator — the right default for every mode whose signal energy sits in
    /// one bin.  A wideband mode must call `process` and pass its own estimator
    /// rather than reaching for this.
    #[allow(clippy::too_many_arguments)]
    pub fn process_nb(
        &mut self,
        samples: &[f32],
        is_signal: bool,
        gap_edge: bool,
        label: &str,
        carrier_hz: f32,
        fs: f32,
        bw_fn: impl FnOnce(&[f32], f32, f32, &mut Self) -> f32,
        tx: &SyncSender<DecodeResult>,
    ) -> bool {
        self.process(
            samples,
            is_signal,
            gap_edge,
            label,
            carrier_hz,
            fs,
            nb_spectrum_snr_db,
            bw_fn,
            tx,
        )
    }
}
