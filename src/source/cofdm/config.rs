// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use serde::Deserialize;

use crate::source::cofdm::{
    COFDM_DEFAULT_BW_FRACTION, COFDM_DEFAULT_MASK, COFDM_DEFAULT_SHAPING_ENABLED,
    COFDM_DEFAULT_TAPER, CofdmBwFraction, CofdmMask, CofdmTaper,
};

#[derive(Debug, Deserialize)]
pub struct CofdmConfig {
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
    pub cn_db: Option<f32>,
    /// **Retired.**  Present only so a config written before the C/N change
    /// fails loudly instead of being silently ignored: every field here is
    /// `Option<T>` and nothing sets `deny_unknown_fields`, so serde would
    /// otherwise drop this key and quietly fall back to the `cn_db` default —
    /// a config that looks like it loaded while discarding what the user wrote.
    /// See `ViewConfig::retired_key_errors`.
    pub noise_amp: Option<f32>,
}

impl crate::config::ViewConfig {
    pub fn cofdm_sig_secs(&self) -> f32 {
        self.cofdm()
            .and_then(|c| c.sig_secs)
            .unwrap_or(crate::source::cofdm::COFDM_DEFAULT_SIG_SECS)
    }
    pub fn cofdm_bw_fraction(&self) -> CofdmBwFraction {
        self.cofdm()
            .and_then(|c| c.bandwidth.as_deref())
            .and_then(parse_bw_fraction)
            .unwrap_or(COFDM_DEFAULT_BW_FRACTION)
    }
    pub fn cofdm_gap_secs(&self) -> f32 {
        self.cofdm()
            .and_then(|c| c.gap_secs)
            .unwrap_or(crate::source::cofdm::COFDM_DEFAULT_GAP_SECS)
    }
    pub fn cofdm_cn_db(&self) -> f32 {
        self.cofdm()
            .and_then(|c| c.cn_db)
            .unwrap_or(crate::source::cofdm::COFDM_DEFAULT_CN_DB)
    }
    pub fn cofdm_shaping_enabled(&self) -> bool {
        self.cofdm()
            .and_then(|c| c.shaping)
            .unwrap_or(COFDM_DEFAULT_SHAPING_ENABLED)
    }
    /// Configured edge guard, or `None` to derive it from the bandwidth
    /// fraction.
    pub fn cofdm_edge_guard(&self) -> Option<usize> {
        self.cofdm().and_then(|c| c.edge_guard)
    }
    pub fn cofdm_include_dc(&self) -> bool {
        self.cofdm().and_then(|c| c.include_dc).unwrap_or(false)
    }
    pub fn cofdm_taper(&self) -> CofdmTaper {
        self.cofdm()
            .and_then(|c| c.taper.as_deref())
            .and_then(parse_taper)
            .unwrap_or(COFDM_DEFAULT_TAPER)
    }
    pub fn cofdm_mask(&self) -> CofdmMask {
        self.cofdm()
            .and_then(|c| c.mask.as_deref())
            .and_then(parse_mask)
            .unwrap_or(COFDM_DEFAULT_MASK)
    }

    fn cofdm(&self) -> Option<&CofdmConfig> {
        self.sources.as_ref().and_then(|s| s.cofdm.as_ref())
    }
}

/// Parse a bandwidth-fraction label (e.g. "1/4") into a `CofdmBwFraction`.
fn parse_bw_fraction(s: &str) -> Option<CofdmBwFraction> {
    CofdmBwFraction::ALL
        .iter()
        .copied()
        .find(|f| f.label() == s.trim())
}

/// Parse a taper label (e.g. "1/4", "off") into a `CofdmTaper`.
fn parse_taper(s: &str) -> Option<CofdmTaper> {
    CofdmTaper::ALL
        .iter()
        .copied()
        .find(|t| t.label().eq_ignore_ascii_case(s.trim()))
}

/// Parse a mask label into a `CofdmMask`.  Accepts both the display form
/// ("60 dB") and the bare number a YAML author is likelier to write ("60").
fn parse_mask(s: &str) -> Option<CofdmMask> {
    let want = s.trim();
    CofdmMask::ALL.iter().copied().find(|m| {
        let label = m.label();
        label.eq_ignore_ascii_case(want)
            || label
                .strip_suffix(" dB")
                .is_some_and(|n| n.eq_ignore_ascii_case(want))
    })
}
