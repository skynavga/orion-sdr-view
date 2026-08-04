// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use serde::Deserialize;

use crate::source::codfm::{CODFM_DEFAULT_BW_FRACTION, CodfmBwFraction};

#[derive(Debug, Deserialize)]
pub struct CodfmConfig {
    /// Occupied bandwidth as a fraction of the full display span, one of
    /// "1/8", "1/4", "1/3", "1/2", "2/3", "3/4", "7/8".
    pub bandwidth: Option<String>,
    pub sig_secs: Option<f32>,
    pub gap_secs: Option<f32>,
    pub noise_amp: Option<f32>,
}

impl crate::config::ViewConfig {
    pub fn codfm_sig_secs(&self) -> f32 {
        self.sources
            .as_ref()
            .and_then(|s| s.codfm.as_ref())
            .and_then(|c| c.sig_secs)
            .unwrap_or(crate::source::codfm::CODFM_DEFAULT_SIG_SECS)
    }
    pub fn codfm_bw_fraction(&self) -> CodfmBwFraction {
        self.sources
            .as_ref()
            .and_then(|s| s.codfm.as_ref())
            .and_then(|c| c.bandwidth.as_deref())
            .and_then(parse_bw_fraction)
            .unwrap_or(CODFM_DEFAULT_BW_FRACTION)
    }
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

/// Parse a bandwidth-fraction label (e.g. "1/4") into a `CodfmBwFraction`.
fn parse_bw_fraction(s: &str) -> Option<CodfmBwFraction> {
    CodfmBwFraction::ALL
        .iter()
        .copied()
        .find(|f| f.label() == s.trim())
}
