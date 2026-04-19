// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Shared spectral analysis state for decode modes that use windowed-FFT
//! signal characterisation (SNR, bandwidth).
//!
//! Used by AM DSB, Test Tone, and any future mode that needs rolling spectral
//! analysis for the Di info bar.

use std::sync::mpsc::SyncSender;

use num_complex::Complex32 as C32;

use super::psk31::INFO_INTERVAL;
use super::{DecodeResult, SPECTRUM_WINDOW_SAMPLES};
pub use orion_sdr::util::{power_spectrum, spectrum_bw_hz, spectrum_snr_db};

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
    /// `bw_fn` computes the bandwidth value for the current window.  Callers
    /// supply a mode-specific closure so that AM DSB can use EMA-smoothed
    /// `spectrum_bw_hz` while Test Tone uses raw `power_spectrum` peak, etc.
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
        bw_fn: impl FnOnce(&[f32], f32, f32, &mut Self) -> f32,
        tx: &SyncSender<DecodeResult>,
    ) {
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
            return;
        }

        self.spec_buf
            .extend(samples.iter().map(|&s| C32::new(s, 0.0)));
        if self.spec_buf.len() < SPECTRUM_WINDOW_SAMPLES {
            return;
        }

        let decode_buf: Vec<C32> = self.spec_buf[..SPECTRUM_WINDOW_SAMPLES].to_vec();
        self.spec_buf.drain(..SPECTRUM_WINDOW_SAMPLES / 2);

        let real: Vec<f32> = decode_buf.iter().map(|c| c.re).collect();
        let raw_snr = spectrum_snr_db(&real, fs, carrier_hz);
        if self.smoothed_snr_db == 0.0 {
            self.smoothed_snr_db = raw_snr;
        } else {
            self.smoothed_snr_db = 0.2 * raw_snr + 0.8 * self.smoothed_snr_db;
        }

        let bw = bw_fn(&real, fs, carrier_hz, self);

        self.info_counter += SPECTRUM_WINDOW_SAMPLES / 2;
        if self.info_counter >= INFO_INTERVAL {
            self.info_counter = 0;
            let _ = tx.try_send(DecodeResult::Info {
                modulation: label.to_owned(),
                center_hz: carrier_hz,
                bw_hz: bw,
                snr_db: self.smoothed_snr_db,
            });
        }
    }
}
