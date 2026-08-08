// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! COFDM decode — thin, info-only wrapper around [`SpectralState`].
//!
//! COFDM emits only the Di info line (modulation "COFDM", center, occupied
//! bandwidth, SNR).  There is no text decode.

use std::sync::mpsc::SyncSender;

use crate::decode::DecodeResult;
use crate::decode::spectral::SpectralState;

pub struct CofdmState(pub SpectralState);

impl Default for CofdmState {
    fn default() -> Self {
        Self(SpectralState::new())
    }
}

impl CofdmState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        self.0.reset();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn process(
        &mut self,
        samples: &[f32],
        is_signal: bool,
        gap_edge: bool,
        carrier_hz: f32,
        bw_hz: f32,
        fs: f32,
        tx: &SyncSender<DecodeResult>,
    ) {
        self.0.process(
            samples,
            is_signal,
            gap_edge,
            "COFDM",
            carrier_hz,
            fs,
            // The occupied bandwidth of a COFDM band is a fixed property of the
            // carrier plan (it depends on the selected bandwidth fraction), not
            // a value to measure.  `spectrum_bw_hz` is a narrowband estimator
            // (it only searches ±4 kHz around the carrier) and would report a
            // tiny sliver for this wideband band — so report the analytic
            // occupied bandwidth supplied by the caller.
            |_real, _fs, _carrier_hz, _state| bw_hz,
            tx,
        );
    }
}
