// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use super::field::{NumField, Row};
use crate::config::ViewConfig;

// ── Row indices (local) ───────────────────────────────────────────────────
const GAP: usize = 0;
const NOISE: usize = 1;

pub(super) struct CodfmRows {
    pub rows: Vec<Row>,
}

impl CodfmRows {
    pub fn new() -> Self {
        Self {
            rows: vec![
                Row::Num(NumField {
                    label: "Gap",
                    value: crate::source::codfm::CODFM_DEFAULT_GAP_SECS,
                    default: crate::source::codfm::CODFM_DEFAULT_GAP_SECS,
                    step: 0.5,
                    min: 0.5,
                    max: 99.99,
                    unit: " s",
                }),
                Row::Num(NumField {
                    label: "Noise amp",
                    value: crate::source::codfm::CODFM_DEFAULT_NOISE_AMP,
                    default: crate::source::codfm::CODFM_DEFAULT_NOISE_AMP,
                    step: 0.01,
                    min: 0.0,
                    max: 0.50,
                    unit: "",
                }),
            ],
        }
    }

    /// Visible rows in the order they appear in the settings overlay.
    pub fn visible_indices(&self) -> Vec<usize> {
        vec![GAP, NOISE]
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
        self.rows[GAP].patch_num(cfg.codfm_gap_secs());
        self.rows[NOISE].patch_num(cfg.codfm_noise_amp());
    }
}

// ── SettingsState accessors ───────────────────────────────────────────────

use crate::app::SourceMode;

/// Borrow this source's rows from `SettingsState`.
fn rows(state: &super::SettingsState) -> &CodfmRows {
    state.source_as::<CodfmRows>(SourceMode::Codfm as usize)
}

/// Typed accessors for CODFM settings.  Implemented for `SettingsState`;
/// callers `use crate::app::settings::CodfmSettings` to bring these in scope.
///
/// CODFM has no user-tunable carrier (it occupies a fixed wideband sub-band),
/// so there is no `set_*_carrier_hz`.
pub(in crate::app) trait CodfmSettings {
    fn codfm_gap_secs(&self) -> f32;
    fn codfm_noise_amp(&self) -> f32;
}

impl CodfmSettings for super::SettingsState {
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
}
