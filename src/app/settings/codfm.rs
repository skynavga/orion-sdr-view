// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use super::field::{CoarseStep, NumField, Row, ToggleField};
use crate::config::ViewConfig;
use crate::source::codfm::{
    CODFM_DEFAULT_BW_FRACTION, CODFM_DEFAULT_MASK, CODFM_DEFAULT_SHAPING_ENABLED,
    CODFM_DEFAULT_TAPER, CODFM_MAX_EDGE_GUARD, CODFM_MIN_EDGE_GUARD, CodfmBwFraction, CodfmMask,
    CodfmShaping, CodfmTaper, codfm_edge_guard_for,
};

// ── Row indices (local) ───────────────────────────────────────────────────
const BANDWIDTH: usize = 0;
const SHAPING: usize = 1;
const EDGE_GUARD: usize = 2;
const INCLUDE_DC: usize = 3;
const TAPER: usize = 4;
const MASK: usize = 5;
const SIGNAL: usize = 6;
const GAP: usize = 7;
const NOISE: usize = 8;

/// Toggle option labels, in `CodfmBwFraction::ALL` order.
const BW_OPTIONS: &[&str] = &["1/8", "1/4", "1/3", "1/2", "2/3", "3/4", "7/8"];
/// Toggle option labels, in `CodfmTaper::ALL` order.
const TAPER_OPTIONS: &[&str] = &["off", "1/8", "1/4", "3/8"];
/// Toggle option labels, in `CodfmMask::ALL` order.
const MASK_OPTIONS: &[&str] = &["off", "40 dB", "60 dB", "80 dB"];
/// Boolean toggle labels (index 0 = false, 1 = true).
const OFF_ON_OPTIONS: &[&str] = &["Off", "On"];
const NO_YES_OPTIONS: &[&str] = &["No", "Yes"];

/// Index into `CodfmBwFraction::ALL` for the default fraction.
fn default_bw_index() -> usize {
    index_of(CodfmBwFraction::ALL, CODFM_DEFAULT_BW_FRACTION)
}

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

pub(super) struct CodfmRows {
    pub rows: Vec<Row>,
}

impl CodfmRows {
    pub fn new() -> Self {
        let bw_idx = default_bw_index();
        // One source of truth for the shaping defaults: the same struct the
        // source renders from.
        let d = CodfmShaping::default_for(CODFM_DEFAULT_BW_FRACTION);
        let (shaping_idx, dc_idx) = (usize::from(d.enabled), usize::from(d.include_dc));
        let (taper_idx, mask_idx) = (
            index_of(CodfmTaper::ALL, d.taper),
            index_of(CodfmMask::ALL, d.mask),
        );
        Self {
            rows: vec![
                Row::Toggle(ToggleField {
                    label: "Bandwidth",
                    options: BW_OPTIONS,
                    index: bw_idx,
                    default: bw_idx,
                }),
                Row::Toggle(ToggleField {
                    label: "Shaping",
                    options: OFF_ON_OPTIONS,
                    index: shaping_idx,
                    default: shaping_idx,
                }),
                Row::Num(NumField {
                    label: "Edge guard",
                    value: d.edge_guard as f32,
                    default: d.edge_guard as f32,
                    step: 1.0,
                    min: CODFM_MIN_EDGE_GUARD as f32,
                    max: CODFM_MAX_EDGE_GUARD as f32,
                    unit: "",
                    coarse: None,
                }),
                Row::Toggle(ToggleField {
                    label: "Include DC",
                    options: NO_YES_OPTIONS,
                    index: dc_idx,
                    default: dc_idx,
                }),
                Row::Toggle(ToggleField {
                    label: "Taper",
                    options: TAPER_OPTIONS,
                    index: taper_idx,
                    default: taper_idx,
                }),
                Row::Toggle(ToggleField {
                    label: "Mask",
                    options: MASK_OPTIONS,
                    index: mask_idx,
                    default: mask_idx,
                }),
                Row::Num(NumField {
                    label: "Signal",
                    value: crate::source::codfm::CODFM_DEFAULT_SIG_SECS,
                    default: crate::source::codfm::CODFM_DEFAULT_SIG_SECS,
                    // 0.5 s steps below 10 s, 1 s steps at/above.
                    step: 0.5,
                    min: 1.0,
                    max: 99.99,
                    unit: " s",
                    coarse: Some(CoarseStep {
                        threshold: 10.0,
                        step: 1.0,
                    }),
                }),
                Row::Num(NumField {
                    label: "Gap",
                    value: crate::source::codfm::CODFM_DEFAULT_GAP_SECS,
                    default: crate::source::codfm::CODFM_DEFAULT_GAP_SECS,
                    step: 0.5,
                    min: 0.5,
                    max: 99.99,
                    unit: " s",
                    coarse: None,
                }),
                Row::Num(NumField {
                    label: "Noise amp",
                    value: crate::source::codfm::CODFM_DEFAULT_NOISE_AMP,
                    default: crate::source::codfm::CODFM_DEFAULT_NOISE_AMP,
                    step: 0.01,
                    min: 0.0,
                    max: 0.50,
                    unit: "",
                    coarse: None,
                }),
            ],
        }
    }

    /// Visible rows in the order they appear in the settings overlay.  The four
    /// shaping parameters are shown only while `Shaping` is on; with it off the
    /// source renders the fraction's own edge guard and no taper or mask.
    pub fn visible_indices(&self) -> Vec<usize> {
        let mut v = vec![BANDWIDTH, SHAPING];
        if self.shaping_enabled() {
            v.extend([EDGE_GUARD, INCLUDE_DC, TAPER, MASK]);
        }
        v.extend([SIGNAL, GAP, NOISE]);
        v
    }

    fn shaping_enabled(&self) -> bool {
        toggle_bool(&self.rows, SHAPING, CODFM_DEFAULT_SHAPING_ENABLED)
    }

    fn bw_fraction(&self) -> CodfmBwFraction {
        CodfmBwFraction::ALL
            .get(toggle_index(&self.rows, BANDWIDTH, default_bw_index()))
            .copied()
            .unwrap_or(CODFM_DEFAULT_BW_FRACTION)
    }

    /// Re-seed the `Edge guard` row from the current bandwidth fraction.  The
    /// fraction is what implies a guard; nudging the guard directly then
    /// overrides it until the fraction moves again.
    ///
    /// Sets the row's **value only**.  Its `default` is the startup pairing —
    /// the configured fraction's guard, or a YAML `edge_guard` that pinned it —
    /// and an R-reset restores bandwidth and guard to that pair together, so
    /// overwriting the default here would lose a pinned value.
    fn reseed_edge_guard(&mut self) {
        let guard = codfm_edge_guard_for(self.bw_fraction()) as f32;
        if let Row::Num(f) = &mut self.rows[EDGE_GUARD] {
            f.value = guard.clamp(f.min, f.max);
        }
    }
}

// ── SourceRows ─────────────────────────────────────────────────────────────

impl super::common::SourceRows for CodfmRows {
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
            self.reseed_edge_guard();
        }
    }
    fn patch_from_config(&mut self, cfg: &ViewConfig) {
        self.rows[SIGNAL].patch_num(cfg.codfm_sig_secs());
        self.rows[GAP].patch_num(cfg.codfm_gap_secs());
        self.rows[NOISE].patch_num(cfg.codfm_noise_amp());

        let fraction = cfg.codfm_bw_fraction();
        set_toggle(
            &mut self.rows,
            BANDWIDTH,
            index_of(CodfmBwFraction::ALL, fraction),
        );
        set_toggle(
            &mut self.rows,
            SHAPING,
            usize::from(cfg.codfm_shaping_enabled()),
        );
        set_toggle(
            &mut self.rows,
            INCLUDE_DC,
            usize::from(cfg.codfm_include_dc()),
        );
        set_toggle(
            &mut self.rows,
            TAPER,
            index_of(CodfmTaper::ALL, cfg.codfm_taper()),
        );
        set_toggle(
            &mut self.rows,
            MASK,
            index_of(CodfmMask::ALL, cfg.codfm_mask()),
        );
        // An absent `edge_guard` key means "whatever the fraction implies".
        // `patch_num` sets value *and* default, so this is also the pair an
        // R-reset restores alongside the bandwidth toggle above.
        self.rows[EDGE_GUARD].patch_num(
            cfg.codfm_edge_guard()
                .unwrap_or_else(|| codfm_edge_guard_for(fraction)) as f32,
        );
    }
}

// ── SettingsState accessors ───────────────────────────────────────────────

use crate::app::SourceMode;

/// Borrow this source's rows from `SettingsState`.
fn rows(state: &super::SettingsState) -> &CodfmRows {
    state.source_as::<CodfmRows>(SourceMode::Codfm as usize)
}
fn rows_mut(state: &mut super::SettingsState) -> &mut CodfmRows {
    state.source_as_mut::<CodfmRows>(SourceMode::Codfm as usize)
}

/// Typed accessors for CODFM settings.  Implemented for `SettingsState`;
/// callers `use crate::app::settings::CodfmSettings` to bring these in scope.
///
/// CODFM has no user-tunable carrier (it occupies a fixed wideband sub-band),
/// so there is no `set_*_carrier_hz`.
pub(in crate::app) trait CodfmSettings {
    fn codfm_sig_secs(&self) -> f32;
    fn codfm_gap_secs(&self) -> f32;
    fn codfm_noise_amp(&self) -> f32;
    fn codfm_bw_fraction(&self) -> CodfmBwFraction;
    fn codfm_shaping(&self) -> CodfmShaping;
    fn cycle_codfm_bw(&mut self);
}

impl CodfmSettings for super::SettingsState {
    fn codfm_sig_secs(&self) -> f32 {
        if let Row::Num(f) = &rows(self).rows[SIGNAL] {
            f.value
        } else {
            crate::source::codfm::CODFM_DEFAULT_SIG_SECS
        }
    }
    fn codfm_gap_secs(&self) -> f32 {
        if let Row::Num(f) = &rows(self).rows[GAP] {
            f.value
        } else {
            crate::source::codfm::CODFM_DEFAULT_GAP_SECS
        }
    }
    fn codfm_noise_amp(&self) -> f32 {
        if let Row::Num(f) = &rows(self).rows[NOISE] {
            f.value
        } else {
            crate::source::codfm::CODFM_DEFAULT_NOISE_AMP
        }
    }
    fn codfm_bw_fraction(&self) -> CodfmBwFraction {
        rows(self).bw_fraction()
    }
    fn codfm_shaping(&self) -> CodfmShaping {
        let r = rows(self);
        let fraction = r.bw_fraction();
        let edge_guard = match &r.rows[EDGE_GUARD] {
            Row::Num(f) => f.value.round().max(0.0) as usize,
            _ => codfm_edge_guard_for(fraction),
        };
        CodfmShaping {
            enabled: r.shaping_enabled(),
            edge_guard,
            include_dc: toggle_bool(&r.rows, INCLUDE_DC, false),
            taper: CodfmTaper::ALL
                .get(toggle_index(&r.rows, TAPER, 0))
                .copied()
                .unwrap_or(CODFM_DEFAULT_TAPER),
            mask: CodfmMask::ALL
                .get(toggle_index(&r.rows, MASK, 0))
                .copied()
                .unwrap_or(CODFM_DEFAULT_MASK),
        }
    }
    fn cycle_codfm_bw(&mut self) {
        let r = rows_mut(self);
        if let Row::Toggle(f) = &mut r.rows[BANDWIDTH] {
            f.next();
        }
        r.reseed_edge_guard();
    }
}
