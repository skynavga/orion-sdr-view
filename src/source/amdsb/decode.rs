// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! AM DSB decode — thin wrapper around [`SpectralState`].

use std::sync::mpsc::SyncSender;

use crate::decode::DecodeResult;
use crate::decode::spectral::{SpectralState, spectrum_bw_hz};

pub struct AmDsbState(pub SpectralState);

impl Default for AmDsbState {
    fn default() -> Self {
        Self(SpectralState::new())
    }
}

impl AmDsbState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        self.0.reset();
    }

    pub fn process(
        &mut self,
        samples: &[f32],
        is_signal: bool,
        gap_edge: bool,
        carrier_hz: f32,
        fs: f32,
        tx: &SyncSender<DecodeResult>,
    ) {
        self.0.process(
            samples,
            is_signal,
            gap_edge,
            "AM DSB",
            carrier_hz,
            fs,
            |real, fs, carrier_hz, state| {
                let raw_bw = spectrum_bw_hz(real, fs, carrier_hz, 7.0);
                if state.smoothed_bw_hz == 0.0 {
                    state.smoothed_bw_hz = raw_bw;
                } else {
                    state.smoothed_bw_hz = 0.2 * raw_bw + 0.8 * state.smoothed_bw_hz;
                }
                state.smoothed_bw_hz
            },
            tx,
        );
    }
}
