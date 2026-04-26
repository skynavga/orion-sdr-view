// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Test Tone decode — thin wrapper around [`SpectralState`].

use std::sync::mpsc::SyncSender;

use crate::decode::DecodeResult;
use crate::decode::spectral::{SpectralState, power_spectrum};

pub struct ToneState(pub SpectralState);

impl Default for ToneState {
    fn default() -> Self {
        Self(SpectralState::new())
    }
}

impl ToneState {
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
            "Test Tone",
            carrier_hz,
            fs,
            |real, fs, _carrier_hz, _state| {
                let (_, bin_hz) = power_spectrum(real, fs);
                bin_hz
            },
            tx,
        );
    }
}
