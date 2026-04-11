// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use super::field::{Row, NumField};

// ── Row indices (local) ───────────────────────────────────────────────────
const FREQ:    usize = 0;
const NOISE:   usize = 1;
const AMP_MAX: usize = 2;
const RAMP:    usize = 3;
const PAUSE:   usize = 4;

pub(super) struct ToneRows {
    pub rows: Vec<Row>,
}

impl ToneRows {
    pub fn new(freq_hz: f32, noise_amp: f32, amp_max: f32, ramp_secs: f32, pause_secs: f32) -> Self {
        Self {
            rows: vec![
                Row::Num(NumField {
                    label: "Frequency", value: freq_hz, default: 12000.0,
                    step: 100.0, min: 100.0, max: 23_900.0, unit: " Hz",
                }),
                Row::Num(NumField {
                    label: "Noise amp", value: noise_amp, default: 0.05,
                    step: 0.01, min: 0.0, max: 1.0, unit: "",
                }),
                Row::Num(NumField {
                    label: "Tone amp max", value: amp_max, default: 0.65,
                    step: 0.05, min: 0.0, max: 1.0, unit: "",
                }),
                Row::Num(NumField {
                    label: "Ramp secs", value: ramp_secs, default: 3.0,
                    step: 0.5, min: 0.5, max: 30.0, unit: " s",
                }),
                Row::Num(NumField {
                    label: "Pause secs", value: pause_secs, default: 7.0,
                    step: 0.5, min: 0.5, max: 99.99, unit: " s",
                }),
            ],
        }
    }

    /// Visible rows in the order they appear in the settings overlay.
    pub fn visible_indices(&self) -> Vec<usize> {
        // Frequency, Tone amp max, Ramp, Pause, Noise amp (bottom)
        vec![FREQ, AMP_MAX, RAMP, PAUSE, NOISE]
    }
}

// ── SettingsState accessors ───────────────────────────────────────────────

impl super::SettingsState {
    pub fn freq_hz(&self) -> f32 {
        if let Row::Num(f) = &self.tone.rows[FREQ] { f.value } else { 3000.0 }
    }
    pub fn noise_amp(&self) -> f32 {
        if let Row::Num(f) = &self.tone.rows[NOISE] { f.value } else { 0.05 }
    }
    pub fn amp_max(&self) -> f32 {
        if let Row::Num(f) = &self.tone.rows[AMP_MAX] { f.value } else { 0.65 }
    }
    pub fn ramp_secs(&self) -> f32 {
        if let Row::Num(f) = &self.tone.rows[RAMP] { f.value } else { 3.0 }
    }
    pub fn pause_secs(&self) -> f32 {
        if let Row::Num(f) = &self.tone.rows[PAUSE] { f.value } else { 7.0 }
    }
    pub fn set_freq_hz(&mut self, v: f32) {
        if let Row::Num(f) = &mut self.tone.rows[FREQ] {
            f.value = v.clamp(f.min, f.max);
        }
    }
}
