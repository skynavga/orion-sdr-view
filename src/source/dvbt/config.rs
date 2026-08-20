// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use orion_sdr::fec::PunctureRate;
use orion_sdr::modulate::ConstellationOrder;
use orion_sdr::waveform::dvb_t::GuardInterval;
use serde::Deserialize;

use crate::source::dvbt::{
    DVBT_CODE_RATES, DVBT_CONSTELLATIONS, DVBT_DEFAULT_BANDWIDTH, DVBT_DEFAULT_CODE_RATE,
    DVBT_DEFAULT_CONSTELLATION, DVBT_DEFAULT_GUARD, DVBT_DEFAULT_MASK,
    DVBT_DEFAULT_SHAPING_ENABLED, DVBT_DEFAULT_TAPER, DVBT_GUARDS, DvbTBandwidth, DvbTMask,
    DvbTTaper, code_rate_label, constellation_label, dvbt_center_bounds, dvbt_default_center_hz,
    guard_label,
};

#[derive(Debug, Deserialize)]
pub struct DvbTConfig {
    /// Band centre (Hz).  Absent means Nyquist/2 (`fs / 4`), which puts the band
    /// mid-display.  Clamped to the range in which the whole occupied band fits
    /// inside `0..Nyquist` — a tighter range than COFDM's, since DVB-T's
    /// occupancy is fixed at 1705/2048 of `fs` and has no narrower fallback.
    pub center_hz: Option<f32>,
    /// Channel bandwidth: "333k", "1M", "2M", "6M", "7M", "8M".
    ///
    /// **There is no `fs_hz` key**, unlike `sources.cofdm`.  For DVB-T the
    /// sample rate *is* the bandwidth (`fs = BW · 2048/1705`) with the 2K
    /// structure fixed above it, so a second key naming the same quantity could
    /// only contradict this one.
    pub bandwidth: Option<String>,
    /// Guard interval as a fraction of the useful symbol: "1/32", "1/16", "1/8",
    /// "1/4".
    pub guard: Option<String>,
    /// Constellation: "QPSK", "QAM16", "QAM64".
    pub constellation: Option<String>,
    /// Inner (convolutional) code rate: "1/2", "2/3", "3/4", "5/6", "7/8".
    pub code_rate: Option<String>,
    /// Enable out-of-band spectral shaping (default true).
    pub shaping: Option<bool>,
    /// Symbol-window roll-off as a fraction of the 32-sample shaping budget:
    /// "off", "1/8", "1/4", "3/8".
    pub taper: Option<String>,
    /// Baseband-mask stop-band depth: "off", "40", "60", "80".
    pub mask: Option<String>,
    pub sig_secs: Option<f32>,
    pub gap_secs: Option<f32>,
    pub cn_db: Option<f32>,
}

impl crate::config::ViewConfig {
    pub fn dvbt_bandwidth(&self) -> DvbTBandwidth {
        self.dvbt()
            .and_then(|c| c.bandwidth.as_deref())
            .and_then(parse_bandwidth)
            .unwrap_or(DVBT_DEFAULT_BANDWIDTH)
    }
    /// Configured band centre, clamped to what fits at the configured
    /// bandwidth's rate.  Derived from `dvbt_bandwidth` rather than from a
    /// constant, so configuring only the bandwidth still centres the band.
    pub fn dvbt_center_hz(&self) -> f32 {
        let fs = self.dvbt_bandwidth().fs();
        let (lo, hi) = dvbt_center_bounds(fs);
        self.dvbt()
            .and_then(|c| c.center_hz)
            .filter(|v| v.is_finite())
            .unwrap_or_else(|| dvbt_default_center_hz(fs))
            .clamp(lo, hi)
    }
    pub fn dvbt_guard(&self) -> GuardInterval {
        self.dvbt()
            .and_then(|c| c.guard.as_deref())
            .and_then(parse_guard)
            .unwrap_or(DVBT_DEFAULT_GUARD)
    }
    pub fn dvbt_constellation(&self) -> ConstellationOrder {
        self.dvbt()
            .and_then(|c| c.constellation.as_deref())
            .and_then(parse_constellation)
            .unwrap_or(DVBT_DEFAULT_CONSTELLATION)
    }
    pub fn dvbt_code_rate(&self) -> PunctureRate {
        self.dvbt()
            .and_then(|c| c.code_rate.as_deref())
            .and_then(parse_code_rate)
            .unwrap_or(DVBT_DEFAULT_CODE_RATE)
    }
    pub fn dvbt_shaping_enabled(&self) -> bool {
        self.dvbt()
            .and_then(|c| c.shaping)
            .unwrap_or(DVBT_DEFAULT_SHAPING_ENABLED)
    }
    pub fn dvbt_taper(&self) -> DvbTTaper {
        self.dvbt()
            .and_then(|c| c.taper.as_deref())
            .and_then(parse_taper)
            .unwrap_or(DVBT_DEFAULT_TAPER)
    }
    pub fn dvbt_mask(&self) -> DvbTMask {
        self.dvbt()
            .and_then(|c| c.mask.as_deref())
            .and_then(parse_mask)
            .unwrap_or(DVBT_DEFAULT_MASK)
    }
    pub fn dvbt_sig_secs(&self) -> f32 {
        self.dvbt()
            .and_then(|c| c.sig_secs)
            .unwrap_or(crate::source::dvbt::DVBT_DEFAULT_SIG_SECS)
    }
    pub fn dvbt_gap_secs(&self) -> f32 {
        self.dvbt()
            .and_then(|c| c.gap_secs)
            .unwrap_or(crate::source::dvbt::DVBT_DEFAULT_GAP_SECS)
    }
    pub fn dvbt_cn_db(&self) -> f32 {
        self.dvbt()
            .and_then(|c| c.cn_db)
            .unwrap_or(crate::source::dvbt::DVBT_DEFAULT_CN_DB)
    }

    fn dvbt(&self) -> Option<&DvbTConfig> {
        self.sources.as_ref().and_then(|s| s.dvbt.as_ref())
    }
}

/// Parse a bandwidth label (e.g. "1M").  Also accepts the spelled-out forms a
/// YAML author is likely to reach for ("1MHz", "333kHz").
fn parse_bandwidth(s: &str) -> Option<DvbTBandwidth> {
    let want = s.trim().trim_end_matches("Hz").trim_end_matches("hz");
    DvbTBandwidth::ALL
        .iter()
        .copied()
        .find(|b| b.label().eq_ignore_ascii_case(want))
}

fn parse_guard(s: &str) -> Option<GuardInterval> {
    DVBT_GUARDS
        .iter()
        .copied()
        .find(|g| guard_label(*g) == s.trim())
}

/// Parse a constellation label.  Accepts the display form ("QAM16") and the
/// hyphenated spelling from the standard ("16-QAM").
fn parse_constellation(s: &str) -> Option<ConstellationOrder> {
    let want = s.trim();
    DVBT_CONSTELLATIONS.iter().copied().find(|c| {
        let label = constellation_label(*c);
        label.eq_ignore_ascii_case(want)
            || label
                .strip_prefix("QAM")
                .is_some_and(|n| want.eq_ignore_ascii_case(&format!("{n}-QAM")))
    })
}

fn parse_code_rate(s: &str) -> Option<PunctureRate> {
    DVBT_CODE_RATES
        .iter()
        .copied()
        .find(|r| code_rate_label(*r) == s.trim())
}

fn parse_taper(s: &str) -> Option<DvbTTaper> {
    DvbTTaper::ALL
        .iter()
        .copied()
        .find(|t| t.label().eq_ignore_ascii_case(s.trim()))
}

/// Parse a mask label.  Accepts both the display form ("60 dB") and the bare
/// number a YAML author is likelier to write ("60").
fn parse_mask(s: &str) -> Option<DvbTMask> {
    let want = s.trim();
    DvbTMask::ALL.iter().copied().find(|m| {
        let label = m.label();
        label.eq_ignore_ascii_case(want)
            || label
                .strip_suffix(" dB")
                .is_some_and(|n| n.eq_ignore_ascii_case(want))
    })
}
