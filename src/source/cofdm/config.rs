// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use serde::Deserialize;

use crate::source::cofdm::{
    COFDM_DEFAULT_BW_FRACTION, COFDM_DEFAULT_MASK, COFDM_DEFAULT_SHAPING_ENABLED,
    COFDM_DEFAULT_TAPER, CofdmBwFraction, CofdmMask, CofdmTaper,
};

#[derive(Debug, Deserialize)]
pub struct CofdmConfig {
    /// Band centre (Hz).  Absent means Nyquist/2 (`fs_hz / 4`), which puts the
    /// band mid-display.  Clamped to the range in which the narrowest
    /// renderable band still fits inside `0..Nyquist`.
    ///
    /// Named `center_hz`, not `carrier_hz` as the five narrowband sources are:
    /// an OFDM band has no carrier — the DC subcarrier is null by default — and
    /// this block already speaks its own vocabulary.  The *trait* surface stays
    /// `decode_carrier_hz` / `set_carrier_hz`, which is the concept-independent
    /// name for "where this source sits", and is what makes the `L` key uniform.
    pub center_hz: Option<f32>,
    /// Native sample rate (Hz).  Absent means [`COFDM_DEFAULT_FS`].
    ///
    /// **No settings row, deliberately.**  Changing the rate re-derives Nyquist
    /// and clears the waterfall, persistence and spectrogram — bin-indexed
    /// history at the old scaling cannot be drawn at the new one — so an
    /// arrow-nudged row would wipe the display on every keypress.  This is the
    /// one place where a config key is right and a live knob is wrong.
    pub fs_hz: Option<f32>,
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
}

impl crate::config::ViewConfig {
    /// Configured sample rate, clamped to the supported range.
    pub fn cofdm_fs_hz(&self) -> f32 {
        crate::source::cofdm::cofdm_clamp_fs(
            self.cofdm()
                .and_then(|c| c.fs_hz)
                .unwrap_or(crate::source::cofdm::COFDM_DEFAULT_FS),
        )
    }
    /// Configured band centre, clamped to what fits at the configured rate.
    /// Defaults to Nyquist/2 — which is why it is derived from `cofdm_fs_hz`
    /// rather than from the constant: configuring only `fs_hz` still centres
    /// the band.
    pub fn cofdm_center_hz(&self) -> f32 {
        let fs = self.cofdm_fs_hz();
        let (lo, hi) = crate::source::cofdm::cofdm_center_bounds(fs);
        self.cofdm()
            .and_then(|c| c.center_hz)
            .filter(|v| v.is_finite())
            .unwrap_or_else(|| crate::source::cofdm::cofdm_default_center_hz(fs))
            .clamp(lo, hi)
    }
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
