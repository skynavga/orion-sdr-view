// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use orion_sdr::fec::PunctureRate;
use orion_sdr::modulate::ConstellationOrder;
use orion_sdr::waveform::dvb_t::{DvbTLinkParams, GuardInterval};

use super::common::SetKey;
use super::field::{CoarseStep, NumField, Row, ToggleField};
use crate::config::ViewConfig;
use crate::source::dvbt::{
    DVBT_CODE_RATES, DVBT_CONSTELLATIONS, DVBT_DEFAULT_BANDWIDTH, DVBT_DEFAULT_CODE_RATE,
    DVBT_DEFAULT_CONSTELLATION, DVBT_DEFAULT_GUARD, DVBT_DEFAULT_MASK,
    DVBT_DEFAULT_SHAPING_ENABLED, DVBT_DEFAULT_TAPER, DVBT_GUARDS, DvbTBandwidth, DvbTMask,
    DvbTShaping, DvbTTaper, dvbt_center_bounds, dvbt_default_center_hz,
};

// ── Row indices (local) ───────────────────────────────────────────────────
const CENTER: usize = 0;
const BANDWIDTH: usize = 1;
const GUARD: usize = 2;
const CONSTELLATION: usize = 3;
const CODE_RATE: usize = 4;
const SHAPING: usize = 5;
const TAPER: usize = 6;
const MASK: usize = 7;
const SIGNAL: usize = 8;
const GAP: usize = 9;
const CN: usize = 10;

/// The rows a script's `set` may name, in the config file's spelling.
///
/// **Every row is here**, unlike COFDM, whose `fs_hz` is config-only because a
/// nudged rate would wipe the display on each keypress.  DVB-T's rate is its
/// `bandwidth` toggle: six discrete modes where every press is a deliberate mode
/// change, which is the same thing switching *sources* already does to
/// bin-indexed history.  So there is nothing a script cannot reach.
pub(in crate::app) const SET_KEYS: &[SetKey] = &[
    SetKey::new("center_hz", CENTER),
    SetKey::new("bandwidth", BANDWIDTH),
    SetKey::new("guard", GUARD),
    SetKey::new("constellation", CONSTELLATION),
    SetKey::new("code_rate", CODE_RATE),
    SetKey::new("shaping", SHAPING),
    SetKey::new("taper", TAPER),
    SetKey::new("mask", MASK),
    SetKey::new("sig_secs", SIGNAL),
    SetKey::new("gap_secs", GAP),
    SetKey::new("cn_db", CN),
];

/// Toggle option labels, in `DvbTBandwidth::ALL` order.
const BW_OPTIONS: &[&str] = &["333k", "1M", "2M", "6M", "7M", "8M"];
/// Toggle option labels, in `DVBT_GUARDS` order.
const GUARD_OPTIONS: &[&str] = &["1/32", "1/16", "1/8", "1/4"];
/// Toggle option labels, in `DVBT_CONSTELLATIONS` order.
const CONSTELLATION_OPTIONS: &[&str] = &["QPSK", "QAM16", "QAM64"];
/// Toggle option labels, in `DVBT_CODE_RATES` order.
const CODE_RATE_OPTIONS: &[&str] = &["1/2", "2/3", "3/4", "5/6", "7/8"];
/// Toggle option labels, in `DvbTTaper::ALL` order.
const TAPER_OPTIONS: &[&str] = &["off", "1/8", "1/4", "3/8"];
/// Toggle option labels, in `DvbTMask::ALL` order.
const MASK_OPTIONS: &[&str] = &["off", "40 dB", "60 dB", "80 dB"];
/// Boolean toggle labels (index 0 = false, 1 = true).
const OFF_ON_OPTIONS: &[&str] = &["Off", "On"];

/// Position of `value` in `all`, or 0 if absent.
fn index_of<T: PartialEq + Copy>(all: &[T], value: T) -> usize {
    all.iter().position(|&v| v == value).unwrap_or(0)
}

/// Read a toggle row's index, or `fallback` if the row is not a toggle.
fn toggle_index(rows: &[Row], idx: usize, fallback: usize) -> usize {
    match &rows[idx] {
        Row::Toggle(f) => f.index,
        _ => fallback,
    }
}

/// Read a boolean toggle row (index 1 == true).
fn toggle_bool(rows: &[Row], idx: usize, fallback: bool) -> bool {
    toggle_index(rows, idx, usize::from(fallback)) == 1
}

/// Set a toggle row's index and default together.
fn set_toggle(rows: &mut [Row], idx: usize, value: usize) {
    if let Row::Toggle(f) = &mut rows[idx] {
        f.index = value;
        f.default = value;
    }
}

/// Read one of the enum toggles, falling back to its default.
fn pick<T: PartialEq + Copy>(rows: &[Row], idx: usize, all: &[T], fallback: T) -> T {
    all.get(toggle_index(rows, idx, index_of(all, fallback)))
        .copied()
        .unwrap_or(fallback)
}

pub(super) struct DvbTRows {
    pub rows: Vec<Row>,
}

impl DvbTRows {
    pub fn new() -> Self {
        let d = DvbTShaping::default_enabled();
        let bw = DVBT_DEFAULT_BANDWIDTH;
        // Every frequency row is in **display-rate** terms, because that is what
        // the viewport shows and what the `L` key writes back.  The waveform's
        // own rate is half of it — see `DVBT_DISPLAY_OVERSAMPLE`.
        let fs_d = bw.display_fs();
        let center = dvbt_default_center_hz(fs_d);
        let (center_lo, center_hi) = dvbt_center_bounds(fs_d);
        Self {
            rows: vec![
                Row::Num(NumField {
                    label: "Center",
                    value: center,
                    default: center,
                    // 1 kHz per press.  The tunable range is narrow — the band
                    // is 83% of the waveform's rate and cannot shrink, so the
                    // centre moves only ±4.2% of the display rate — and `L`
                    // (lock source to viewport centre) is the real tuning
                    // control, writing the centre in directly at 10 Hz.
                    step: 1_000.0,
                    min: center_lo,
                    max: center_hi,
                    unit: " Hz",
                    coarse: None,
                    max_label: None,
                }),
                Row::Toggle(ToggleField {
                    label: "Bandwidth",
                    options: BW_OPTIONS,
                    index: index_of(DvbTBandwidth::ALL, bw),
                    default: index_of(DvbTBandwidth::ALL, bw),
                }),
                Row::Toggle(ToggleField {
                    label: "Guard",
                    options: GUARD_OPTIONS,
                    index: index_of(DVBT_GUARDS, DVBT_DEFAULT_GUARD),
                    default: index_of(DVBT_GUARDS, DVBT_DEFAULT_GUARD),
                }),
                Row::Toggle(ToggleField {
                    label: "Constellation",
                    options: CONSTELLATION_OPTIONS,
                    index: index_of(DVBT_CONSTELLATIONS, DVBT_DEFAULT_CONSTELLATION),
                    default: index_of(DVBT_CONSTELLATIONS, DVBT_DEFAULT_CONSTELLATION),
                }),
                Row::Toggle(ToggleField {
                    label: "Code rate",
                    options: CODE_RATE_OPTIONS,
                    index: index_of(DVBT_CODE_RATES, DVBT_DEFAULT_CODE_RATE),
                    default: index_of(DVBT_CODE_RATES, DVBT_DEFAULT_CODE_RATE),
                }),
                Row::Toggle(ToggleField {
                    label: "Shaping",
                    options: OFF_ON_OPTIONS,
                    index: usize::from(d.enabled),
                    default: usize::from(d.enabled),
                }),
                Row::Toggle(ToggleField {
                    label: "Taper",
                    options: TAPER_OPTIONS,
                    index: index_of(DvbTTaper::ALL, d.taper),
                    default: index_of(DvbTTaper::ALL, d.taper),
                }),
                Row::Toggle(ToggleField {
                    label: "Mask",
                    options: MASK_OPTIONS,
                    index: index_of(DvbTMask::ALL, d.mask),
                    default: index_of(DvbTMask::ALL, d.mask),
                }),
                Row::Num(NumField {
                    label: "Signal",
                    value: crate::source::dvbt::DVBT_DEFAULT_SIG_SECS,
                    default: crate::source::dvbt::DVBT_DEFAULT_SIG_SECS,
                    step: 0.5,
                    min: 1.0,
                    max: crate::source::CONTINUOUS_SIG_SECS,
                    unit: " s",
                    coarse: Some(CoarseStep {
                        threshold: 10.0,
                        step: 1.0,
                    }),
                    max_label: Some("cont"),
                }),
                Row::Num(NumField {
                    label: "Gap",
                    value: crate::source::dvbt::DVBT_DEFAULT_GAP_SECS,
                    default: crate::source::dvbt::DVBT_DEFAULT_GAP_SECS,
                    step: 0.5,
                    min: 0.5,
                    max: 99.99,
                    unit: " s",
                    coarse: None,
                    max_label: None,
                }),
                Row::Num(NumField {
                    label: "C/N",
                    value: crate::source::dvbt::DVBT_DEFAULT_CN_DB,
                    default: crate::source::dvbt::DVBT_DEFAULT_CN_DB,
                    step: 1.0,
                    min: MIN_CN_DB,
                    max: MAX_CN_DB,
                    unit: " dB",
                    coarse: None,
                    max_label: None,
                }),
            ],
        }
    }

    /// Visible rows in the order they appear in the settings overlay.  The
    /// taper and mask are shown only while `Shaping` is on, and `Gap` is hidden
    /// while `Signal` reads `cont` — both for the reason COFDM's are: a control
    /// that cannot do anything should not be on screen.
    pub fn visible_indices(&self) -> Vec<usize> {
        let mut v = vec![CENTER, BANDWIDTH, GUARD, CONSTELLATION, CODE_RATE, SHAPING];
        if self.shaping_enabled() {
            v.extend([TAPER, MASK]);
        }
        v.push(SIGNAL);
        if !self.sig_continuous() {
            v.push(GAP);
        }
        v.push(CN);
        v
    }

    /// True when the `Signal` row asks for a burst that never ends.
    fn sig_continuous(&self) -> bool {
        match &self.rows[SIGNAL] {
            Row::Num(f) => crate::source::is_continuous_sig(f.value),
            _ => false,
        }
    }

    fn shaping_enabled(&self) -> bool {
        toggle_bool(&self.rows, SHAPING, DVBT_DEFAULT_SHAPING_ENABLED)
    }

    fn bandwidth(&self) -> DvbTBandwidth {
        pick(
            &self.rows,
            BANDWIDTH,
            DvbTBandwidth::ALL,
            DVBT_DEFAULT_BANDWIDTH,
        )
    }

    fn center_hz(&self) -> f32 {
        match &self.rows[CENTER] {
            Row::Num(f) => f.value,
            _ => dvbt_default_center_hz(self.bandwidth().display_fs()),
        }
    }

    fn link(&self) -> DvbTLinkParams {
        DvbTLinkParams {
            guard: pick(&self.rows, GUARD, DVBT_GUARDS, DVBT_DEFAULT_GUARD),
            constellation: pick(
                &self.rows,
                CONSTELLATION,
                DVBT_CONSTELLATIONS,
                DVBT_DEFAULT_CONSTELLATION,
            ),
            code_rate: pick(
                &self.rows,
                CODE_RATE,
                DVBT_CODE_RATES,
                DVBT_DEFAULT_CODE_RATE,
            ),
        }
    }

    /// Re-derive the `Center` row's range from the current bandwidth, keeping
    /// the value inside it.
    ///
    /// **The bandwidth row is a frequency-axis change, not just a waveform one**,
    /// which is what makes this necessary where COFDM needs no counterpart: its
    /// rate is fixed and only the occupied fraction moves.  Here the display
    /// rate scales 24× across the six modes, so the legal centre range scales
    /// with it and a value from the old mode is almost always outside the new
    /// one.  Without this the row would show a centre the source cannot use —
    /// the source clamps regardless, so the two would silently disagree.
    fn reseed_center_bounds(&mut self) {
        let (lo, hi) = dvbt_center_bounds(self.bandwidth().display_fs());
        if let Row::Num(f) = &mut self.rows[CENTER] {
            f.min = lo;
            f.max = hi;
            f.value = f.value.clamp(lo, hi);
            f.default = f.default.clamp(lo, hi);
        }
    }

    /// Re-centre the band on a bandwidth change.
    ///
    /// Clamping alone would pin the band to whichever edge of the new range it
    /// fell outside, so stepping through the bandwidth toggle would walk the
    /// band to one side of the display and leave it there.  Re-seeding to
    /// mid-display makes each press land the band where a fresh source would put
    /// it, which is what a mode switch should look like.
    fn recenter(&mut self) {
        let center = dvbt_default_center_hz(self.bandwidth().display_fs());
        if let Row::Num(f) = &mut self.rows[CENTER] {
            f.value = center;
        }
    }
}

// ── SourceRows ─────────────────────────────────────────────────────────────

impl super::common::SourceRows for DvbTRows {
    fn rows(&self) -> &[Row] {
        &self.rows
    }
    fn rows_mut(&mut self) -> &mut [Row] {
        &mut self.rows
    }
    fn visible_indices(&self) -> Vec<usize> {
        self.visible_indices()
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn after_nudge(&mut self, local_idx: usize) {
        if local_idx == BANDWIDTH {
            self.reseed_center_bounds();
            self.recenter();
        }
    }
    fn patch_from_config(&mut self, cfg: &ViewConfig) {
        // Bandwidth first: the centre's range and default are derived from it.
        set_toggle(
            &mut self.rows,
            BANDWIDTH,
            index_of(DvbTBandwidth::ALL, cfg.dvbt_bandwidth()),
        );
        self.reseed_center_bounds();
        self.rows[CENTER].patch_num(cfg.dvbt_center_hz());

        set_toggle(
            &mut self.rows,
            GUARD,
            index_of(DVBT_GUARDS, cfg.dvbt_guard()),
        );
        set_toggle(
            &mut self.rows,
            CONSTELLATION,
            index_of(DVBT_CONSTELLATIONS, cfg.dvbt_constellation()),
        );
        set_toggle(
            &mut self.rows,
            CODE_RATE,
            index_of(DVBT_CODE_RATES, cfg.dvbt_code_rate()),
        );
        set_toggle(
            &mut self.rows,
            SHAPING,
            usize::from(cfg.dvbt_shaping_enabled()),
        );
        set_toggle(
            &mut self.rows,
            TAPER,
            index_of(DvbTTaper::ALL, cfg.dvbt_taper()),
        );
        set_toggle(
            &mut self.rows,
            MASK,
            index_of(DvbTMask::ALL, cfg.dvbt_mask()),
        );

        self.rows[SIGNAL].patch_num(cfg.dvbt_sig_secs());
        self.rows[GAP].patch_num(cfg.dvbt_gap_secs());
        self.rows[CN].patch_num(cfg.dvbt_cn_db());
    }
}

// ── SettingsState accessors ───────────────────────────────────────────────

use crate::app::SourceMode;
use crate::source::{MAX_CN_DB, MIN_CN_DB};

/// Borrow this source's rows from `SettingsState`.
fn rows(state: &super::SettingsState) -> &DvbTRows {
    state.source_as::<DvbTRows>(SourceMode::DvbT as usize)
}
fn rows_mut(state: &mut super::SettingsState) -> &mut DvbTRows {
    state.source_as_mut::<DvbTRows>(SourceMode::DvbT as usize)
}

/// Typed accessors for DVB-T settings.  Implemented for `SettingsState`;
/// callers `use crate::app::settings::DvbTSettings` to bring these in scope.
pub trait DvbTSettings {
    fn dvbt_sig_secs(&self) -> f32;
    fn dvbt_gap_secs(&self) -> f32;
    fn dvbt_cn_db(&self) -> f32;
    fn dvbt_bandwidth(&self) -> DvbTBandwidth;
    fn dvbt_guard(&self) -> GuardInterval;
    fn dvbt_constellation(&self) -> ConstellationOrder;
    fn dvbt_code_rate(&self) -> PunctureRate;
    /// The guard / constellation / code rate as one value — the shape the
    /// modulator, the receiver and the instrument all take.
    fn dvbt_link(&self) -> DvbTLinkParams;
    fn dvbt_shaping(&self) -> DvbTShaping;
    fn dvbt_center_hz(&self) -> f32;
    fn set_dvbt_center_hz(&mut self, hz: f32);
}

impl DvbTSettings for super::SettingsState {
    fn dvbt_sig_secs(&self) -> f32 {
        if let Row::Num(f) = &rows(self).rows[SIGNAL] {
            f.value
        } else {
            crate::source::dvbt::DVBT_DEFAULT_SIG_SECS
        }
    }
    fn dvbt_gap_secs(&self) -> f32 {
        if let Row::Num(f) = &rows(self).rows[GAP] {
            f.value
        } else {
            crate::source::dvbt::DVBT_DEFAULT_GAP_SECS
        }
    }
    fn dvbt_cn_db(&self) -> f32 {
        if let Row::Num(f) = &rows(self).rows[CN] {
            f.value
        } else {
            crate::source::dvbt::DVBT_DEFAULT_CN_DB
        }
    }
    fn dvbt_bandwidth(&self) -> DvbTBandwidth {
        rows(self).bandwidth()
    }
    fn dvbt_guard(&self) -> GuardInterval {
        rows(self).link().guard
    }
    fn dvbt_constellation(&self) -> ConstellationOrder {
        rows(self).link().constellation
    }
    fn dvbt_code_rate(&self) -> PunctureRate {
        rows(self).link().code_rate
    }
    fn dvbt_link(&self) -> DvbTLinkParams {
        rows(self).link()
    }
    fn dvbt_center_hz(&self) -> f32 {
        rows(self).center_hz()
    }
    /// Write a new band centre (the `L` key's source-lock), clamped by the row
    /// to what fits at the current bandwidth.
    fn set_dvbt_center_hz(&mut self, hz: f32) {
        let r = rows_mut(self);
        if let Row::Num(f) = &mut r.rows[CENTER] {
            f.value = hz.clamp(f.min, f.max);
        }
    }
    fn dvbt_shaping(&self) -> DvbTShaping {
        let r = rows(self);
        DvbTShaping {
            enabled: r.shaping_enabled(),
            taper: pick(&r.rows, TAPER, DvbTTaper::ALL, DVBT_DEFAULT_TAPER),
            mask: pick(&r.rows, MASK, DvbTMask::ALL, DVBT_DEFAULT_MASK),
        }
    }
}
