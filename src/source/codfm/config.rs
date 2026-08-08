// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use serde::Deserialize;

use crate::source::codfm::{
    CODFM_DEFAULT_BW_FRACTION, CODFM_DEFAULT_MASK, CODFM_DEFAULT_SHAPING_ENABLED,
    CODFM_DEFAULT_TAPER, CodfmBwFraction, CodfmMask, CodfmTaper,
};

#[derive(Debug, Deserialize)]
pub struct CodfmConfig {
    /// Occupied bandwidth as a fraction of the full display span, one of
    /// "1/8", "1/4", "1/3", "1/2", "2/3", "3/4", "7/8".
    pub bandwidth: Option<String>,
    /// Enable out-of-band spectral shaping (default true).
    pub shaping: Option<bool>,
    /// Null carriers per band edge.  Absent means "whatever `bandwidth`
    /// implies" — the two are the same lever, so a value here overrides it.
    pub edge_guard: Option<usize>,
    /// Occupy the DC subcarrier (default false).
    pub include_dc: Option<bool>,
    /// Symbol-window roll-off as a fraction of the guard: "off", "1/8", "1/4",
    /// "3/8".
    pub taper: Option<String>,
    /// Baseband-mask stop-band depth: "off", "40", "60", "80".
    pub mask: Option<String>,
    pub sig_secs: Option<f32>,
    pub gap_secs: Option<f32>,
    pub noise_amp: Option<f32>,
}

impl crate::config::ViewConfig {
    pub fn codfm_sig_secs(&self) -> f32 {
        self.codfm()
            .and_then(|c| c.sig_secs)
            .unwrap_or(crate::source::codfm::CODFM_DEFAULT_SIG_SECS)
    }
    pub fn codfm_bw_fraction(&self) -> CodfmBwFraction {
        self.codfm()
            .and_then(|c| c.bandwidth.as_deref())
            .and_then(parse_bw_fraction)
            .unwrap_or(CODFM_DEFAULT_BW_FRACTION)
    }
    pub fn codfm_gap_secs(&self) -> f32 {
        self.codfm()
            .and_then(|c| c.gap_secs)
            .unwrap_or(crate::source::codfm::CODFM_DEFAULT_GAP_SECS)
    }
    pub fn codfm_noise_amp(&self) -> f32 {
        self.codfm()
            .and_then(|c| c.noise_amp)
            .unwrap_or(crate::source::codfm::CODFM_DEFAULT_NOISE_AMP)
    }
    pub fn codfm_shaping_enabled(&self) -> bool {
        self.codfm()
            .and_then(|c| c.shaping)
            .unwrap_or(CODFM_DEFAULT_SHAPING_ENABLED)
    }
    /// Configured edge guard, or `None` to derive it from the bandwidth
    /// fraction.
    pub fn codfm_edge_guard(&self) -> Option<usize> {
        self.codfm().and_then(|c| c.edge_guard)
    }
    pub fn codfm_include_dc(&self) -> bool {
        self.codfm().and_then(|c| c.include_dc).unwrap_or(false)
    }
    pub fn codfm_taper(&self) -> CodfmTaper {
        self.codfm()
            .and_then(|c| c.taper.as_deref())
            .and_then(parse_taper)
            .unwrap_or(CODFM_DEFAULT_TAPER)
    }
    pub fn codfm_mask(&self) -> CodfmMask {
        self.codfm()
            .and_then(|c| c.mask.as_deref())
            .and_then(parse_mask)
            .unwrap_or(CODFM_DEFAULT_MASK)
    }

    fn codfm(&self) -> Option<&CodfmConfig> {
        self.sources.as_ref().and_then(|s| s.codfm.as_ref())
    }
}

/// Parse a bandwidth-fraction label (e.g. "1/4") into a `CodfmBwFraction`.
fn parse_bw_fraction(s: &str) -> Option<CodfmBwFraction> {
    CodfmBwFraction::ALL
        .iter()
        .copied()
        .find(|f| f.label() == s.trim())
}

/// Parse a taper label (e.g. "1/4", "off") into a `CodfmTaper`.
fn parse_taper(s: &str) -> Option<CodfmTaper> {
    CodfmTaper::ALL
        .iter()
        .copied()
        .find(|t| t.label().eq_ignore_ascii_case(s.trim()))
}

/// Parse a mask label into a `CodfmMask`.  Accepts both the display form
/// ("60 dB") and the bare number a YAML author is likelier to write ("60").
fn parse_mask(s: &str) -> Option<CodfmMask> {
    let want = s.trim();
    CodfmMask::ALL.iter().copied().find(|m| {
        let label = m.label();
        label.eq_ignore_ascii_case(want)
            || label
                .strip_suffix(" dB")
                .is_some_and(|n| n.eq_ignore_ascii_case(want))
    })
}
