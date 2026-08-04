// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use super::field::{CoarseStep, NumField, Row, ToggleField};
use crate::config::ViewConfig;
use crate::source::codfm::{CODFM_DEFAULT_BW_FRACTION, CodfmBwFraction};

// ── Row indices (local) ───────────────────────────────────────────────────
const BANDWIDTH: usize = 0;
const SIGNAL: usize = 1;
const GAP: usize = 2;
const NOISE: usize = 3;

/// Toggle option labels, in `CodfmBwFraction::ALL` order.
const BW_OPTIONS: &[&str] = &["1/8", "1/4", "1/3", "1/2", "2/3", "3/4", "7/8"];

/// Index into `CodfmBwFraction::ALL` for the default fraction.
fn default_bw_index() -> usize {
    CodfmBwFraction::ALL
        .iter()
        .position(|&f| f == CODFM_DEFAULT_BW_FRACTION)
        .unwrap_or(0)
}

pub(super) struct CodfmRows {
    pub rows: Vec<Row>,
}

impl CodfmRows {
    pub fn new() -> Self {
        let bw_idx = default_bw_index();
        Self {
            rows: vec![
                Row::Toggle(ToggleField {
                    label: "Bandwidth",
                    options: BW_OPTIONS,
                    index: bw_idx,
                    default: bw_idx,
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

    /// Visible rows in the order they appear in the settings overlay.
    pub fn visible_indices(&self) -> Vec<usize> {
        vec![BANDWIDTH, SIGNAL, GAP, NOISE]
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
    fn patch_from_config(&mut self, cfg: &ViewConfig) {
        self.rows[SIGNAL].patch_num(cfg.codfm_sig_secs());
        self.rows[GAP].patch_num(cfg.codfm_gap_secs());
        self.rows[NOISE].patch_num(cfg.codfm_noise_amp());
        let idx = CodfmBwFraction::ALL
            .iter()
            .position(|&f| f == cfg.codfm_bw_fraction())
            .unwrap_or_else(default_bw_index);
        if let Row::Toggle(f) = &mut self.rows[BANDWIDTH] {
            f.index = idx;
            f.default = idx;
        }
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
        if let Row::Toggle(f) = &rows(self).rows[BANDWIDTH] {
            CodfmBwFraction::ALL
                .get(f.index)
                .copied()
                .unwrap_or(CODFM_DEFAULT_BW_FRACTION)
        } else {
            CODFM_DEFAULT_BW_FRACTION
        }
    }
    fn cycle_codfm_bw(&mut self) {
        if let Row::Toggle(f) = &mut rows_mut(self).rows[BANDWIDTH] {
            f.next();
        }
    }
}
