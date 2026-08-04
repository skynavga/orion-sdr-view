// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct CodfmConfig {
    pub gap_secs: Option<f32>,
    pub noise_amp: Option<f32>,
}

impl crate::config::ViewConfig {
    pub fn codfm_gap_secs(&self) -> f32 {
        self.sources
            .as_ref()
            .and_then(|s| s.codfm.as_ref())
            .and_then(|c| c.gap_secs)
            .unwrap_or(crate::source::codfm::CODFM_DEFAULT_GAP_SECS)
    }
    pub fn codfm_noise_amp(&self) -> f32 {
        self.sources
            .as_ref()
            .and_then(|s| s.codfm.as_ref())
            .and_then(|c| c.noise_amp)
            .unwrap_or(crate::source::codfm::CODFM_DEFAULT_NOISE_AMP)
    }
}
