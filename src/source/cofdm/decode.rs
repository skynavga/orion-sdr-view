// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! COFDM decode — the Di info line plus the instrumentation provider.
//!
//! There is no text decode.  Spectral characterisation is delegated to
//! [`SpectralState`]; this module adds the RF-level measurements and assembles
//! a [`CofdmInstrument`] on the same cadence, so the Di bar and the `X` panel
//! advance in lockstep with the `Info` line rather than at a rate of their own.
//!
//! This is the first *provider* of the instrument.  When the viewer runs a
//! COFDM receiver, a second provider fills the same struct with measurements
//! and the render path does not change — that is what the provenance tagging
//! in [`crate::decode::instrument`] is for.

use std::sync::mpsc::SyncSender;

use orion_sdr::util::rms;

use super::source::{
    COFDM_CP_LEN, COFDM_GAIN, COFDM_N_FFT, COFDM_PAYLOAD_BYTES, cofdm_data_carriers,
    cofdm_mcs_facts,
};
use crate::decode::DecodeResult;
use crate::decode::instrument::{CofdmFacts, CofdmInstrument, ERROR_COUNT_WRAP, ErrorUnit};
use crate::decode::spectral::{SpectralState, wb_spectrum_snr_db};

#[derive(Default)]
pub struct CofdmState {
    pub spectral: SpectralState,
    /// Frame errors accumulated across emits.  Lives here rather than in the
    /// instrument because the instrument is rebuilt from scratch on every
    /// emit; a counter inside it would reset each time.
    error_count: u32,
    /// Fractional error carry, so a frame error rate far below one error per
    /// emit still advances the counter at the right average pace instead of
    /// truncating to zero forever.
    error_accum: f32,
    /// True once `error_count` has rolled through the display's range.
    error_wrapped: bool,
    /// Samples seen since the last emit, for converting the frame error *rate*
    /// into a count of frames that actually elapsed.
    samples_since_emit: usize,
}

impl CofdmState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        self.spectral.reset();
        self.reset_errors();
        self.samples_since_emit = 0;
    }

    fn reset_errors(&mut self) {
        self.error_count = 0;
        self.error_accum = 0.0;
        self.error_wrapped = false;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn process(
        &mut self,
        samples: &[f32],
        is_signal: bool,
        gap_edge: bool,
        carrier_hz: f32,
        bw_hz: f32,
        edge_guard: usize,
        include_dc: bool,
        fs: f32,
        tx: &SyncSender<DecodeResult>,
    ) {
        self.samples_since_emit += samples.len();
        let emitted = self.spectral.process(
            samples,
            is_signal,
            gap_edge,
            "COFDM",
            carrier_hz,
            fs,
            // COFDM spreads its energy over the whole occupied band, so the
            // narrowband estimator every other mode uses — one peak bin against
            // the noise floor — measures a single subcarrier and reports a
            // number tens of dB off.  Compare the mean power across the
            // occupied window instead.
            |real, fs, carrier_hz| wb_spectrum_snr_db(real, fs, carrier_hz, bw_hz),
            // The occupied bandwidth of a COFDM band is a fixed property of the
            // carrier plan (it depends on the selected bandwidth fraction), not
            // a value to measure.  `spectrum_bw_hz` is a narrowband estimator
            // (it only searches ±4 kHz around the carrier) and would report a
            // tiny sliver for this wideband band — so report the analytic
            // occupied bandwidth supplied by the caller.
            |_real, _fs, _carrier_hz, _state| bw_hz,
            tx,
        );

        if gap_edge {
            // Clear the panel rather than leaving it holding numbers from a
            // burst that has ended.  This is the one case where values visibly
            // disappear, and it is deliberate.  The error count is per-burst
            // for the same reason: it counts errors in the transmission being
            // displayed, so carrying it across a gap would attribute one
            // burst's errors to the next.
            self.reset_errors();
            self.samples_since_emit = 0;
            let _ = tx.try_send(DecodeResult::Instrument(None));
            return;
        }
        if !emitted {
            return;
        }

        let elapsed = std::mem::take(&mut self.samples_since_emit);
        let inst = self.build(samples, carrier_hz, bw_hz, edge_guard, include_dc, fs);
        self.accumulate_errors(&inst, elapsed, fs);
        let _ = tx.try_send(DecodeResult::Instrument(Some(Box::new(inst))));
    }

    /// Assemble the instrument from what this block measured plus the carrier
    /// plan's known numerology.
    fn build(
        &self,
        samples: &[f32],
        carrier_hz: f32,
        bw_hz: f32,
        edge_guard: usize,
        include_dc: bool,
        fs: f32,
    ) -> CofdmInstrument {
        let peak = samples.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        let (constellation, bits_per_symbol, inner_code_rate) = cofdm_mcs_facts();
        CofdmInstrument::from_facts(&CofdmFacts {
            center_hz: carrier_hz,
            bandwidth_hz: bw_hz,
            level_amp: rms(samples),
            peak_amp: peak,
            // The modulator's fixed gain is what 0 dBFS means for this source —
            // see `CofdmFacts::full_scale`.
            full_scale: COFDM_GAIN,
            cn_db: self.spectral.smoothed_snr_db,
            fs,
            n_fft: COFDM_N_FFT,
            cp_len: COFDM_CP_LEN,
            data_carriers: cofdm_data_carriers(edge_guard, include_dc),
            constellation,
            bits_per_symbol,
            inner_code_rate,
            error_count: self.error_count,
            error_count_wrapped: self.error_wrapped,
            // Generic COFDM has no packet concept: its unit is the frame.  A
            // DVB-T provider would set `Packet` here and the label follows.
            error_unit: ErrorUnit::Frame,
        })
    }

    /// Advance the error counter by the frame error *rate* times the number of
    /// frames that actually elapsed since the last emit.
    ///
    /// The frame count is what makes `err` correlate with `FER`.  Adding the
    /// rate once per *emit* — as if one emit carried one frame — under-counts
    /// by the frame rate, which is in the hundreds per second here: a displayed
    /// `FER 6.7E-5` would then take about an hour to tick `err` once, so the
    /// two readings looked unrelated.
    ///
    /// Frames elapsed is derived from the bit rate rather than by re-deriving
    /// the frame geometry: `frames = bitrate × seconds / payload_bits`.
    fn accumulate_errors(&mut self, inst: &CofdmInstrument, elapsed_samples: usize, fs: f32) {
        let Some(bitrate) = inst.bitrate_bps.value else {
            return;
        };
        if fs <= 0.0 {
            return;
        }
        let payload_bits = (COFDM_PAYLOAD_BYTES * 8) as f64;
        let seconds = elapsed_samples as f64 / fs as f64;
        let frames = (bitrate * seconds / payload_bits) as f32;
        self.error_accum += inst.simulated_error_rate() * frames;
        while self.error_accum >= 1.0 {
            self.error_accum -= 1.0;
            self.error_count += 1;
            if self.error_count >= ERROR_COUNT_WRAP {
                self.error_count = 0;
                self.error_wrapped = true;
            }
        }
    }
}
