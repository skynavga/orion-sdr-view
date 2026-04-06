use super::field::{Row, NumField};
use crate::config::ViewConfig;

// ── Row indices (local) ───────────────────────────────────────────────────
const DB_MIN: usize = 0;
const DB_MAX: usize = 1;

pub(super) struct DisplayRows {
    pub rows: Vec<Row>,
}

impl DisplayRows {
    pub fn new(db_min: f32, db_max: f32) -> Self {
        Self {
            rows: vec![
                Row::Num(NumField {
                    label: "dB min", value: db_min, default: -80.0,
                    step: 1.0, min: -160.0, max: -1.0, unit: " dB",
                }),
                Row::Num(NumField {
                    label: "dB max", value: db_max, default: -20.0,
                    step: 1.0, min: -159.0, max: 0.0, unit: " dB",
                }),
            ],
        }
    }

    pub fn patch_from_config(&mut self, cfg: &ViewConfig) {
        self.rows[DB_MIN].patch_num(cfg.db_min());
        self.rows[DB_MAX].patch_num(cfg.db_max());
    }

    pub fn visible_indices(&self) -> Vec<usize> {
        (0..self.rows.len()).collect()
    }
}

// ── SettingsState accessors ───────────────────────────────────────────────

impl super::SettingsState {
    pub fn db_min(&self) -> f32 {
        if let Row::Num(f) = &self.display.rows[DB_MIN] { f.value } else { -80.0 }
    }
    pub fn db_max(&self) -> f32 {
        if let Row::Num(f) = &self.display.rows[DB_MAX] { f.value } else { -20.0 }
    }
    pub fn set_db_min(&mut self, v: f32) {
        if let Row::Num(f) = &mut self.display.rows[DB_MIN] { f.value = v.clamp(f.min, f.max); }
    }
    pub fn set_db_max(&mut self, v: f32) {
        if let Row::Num(f) = &mut self.display.rows[DB_MAX] { f.value = v.clamp(f.min, f.max); }
    }
}
