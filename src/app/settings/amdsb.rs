// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use super::field::{NumField, Row, RowDrawCtx, TextField, ToggleField};
use crate::config::ViewConfig;
use eframe::egui;

// ── Row indices (local) ───────────────────────────────────────────────────
const AUDIO: usize = 0;
const CARRIER: usize = 1;
const MOD_IDX: usize = 2;
const GAP: usize = 3;
const NOISE: usize = 4;
const WAV_FILE: usize = 5;
const REPEAT: usize = 6;

pub(super) struct AmDsbRows {
    pub rows: Vec<Row>,
    /// In-progress edit of the WAV file path.  `Some(s)` while the user
    /// is typing; committed to the row on Enter, discarded on Escape.
    pub pending_wav: Option<String>,
}

impl AmDsbRows {
    pub fn new() -> Self {
        Self {
            rows: vec![
                Row::Toggle(ToggleField {
                    label: "Audio",
                    options: &["Morse", "Voice", "Custom"],
                    index: 0,
                    default: 0,
                }),
                Row::Num(NumField {
                    label: "Carrier Hz",
                    value: 12000.0,
                    default: 12000.0,
                    step: 100.0,
                    min: 100.0,
                    max: 23_900.0,
                    unit: " Hz",
                }),
                Row::Num(NumField {
                    label: "Mod index",
                    value: 1.0,
                    default: 1.0,
                    step: 0.1,
                    min: 0.1,
                    max: 2.0,
                    unit: "",
                }),
                Row::Num(NumField {
                    label: "Gap",
                    value: 7.0,
                    default: 7.0,
                    step: 0.5,
                    min: 0.0,
                    max: 99.99,
                    unit: " s",
                }),
                Row::Num(NumField {
                    label: "Noise amp",
                    value: 0.05,
                    default: 0.05,
                    step: 0.01,
                    min: 0.0,
                    max: 0.50,
                    unit: "",
                }),
                Row::Text(TextField {
                    label: "Audio source",
                    value: String::new(),
                    default_value: String::new(),
                    status: None,
                }),
                Row::Num(NumField {
                    label: "Repeat",
                    value: 1.0,
                    default: 1.0,
                    step: 1.0,
                    min: 1.0,
                    max: 20.0,
                    unit: "×",
                }),
            ],
            pending_wav: None,
        }
    }

    pub fn patch_from_config(&mut self, cfg: &ViewConfig) {
        self.rows[CARRIER].patch_num(cfg.carrier_hz());
        self.rows[MOD_IDX].patch_num(cfg.mod_index());
        self.rows[GAP].patch_num(cfg.am_gap_secs());
        self.rows[NOISE].patch_num(cfg.am_noise_amp());
        self.rows[REPEAT].patch_num(cfg.am_msg_repeat() as f32);
    }

    /// Visible rows in the order they appear in the settings overlay.
    pub fn visible_indices(&self) -> Vec<usize> {
        vec![AUDIO, WAV_FILE, REPEAT, CARRIER, MOD_IDX, GAP, NOISE]
    }

    pub fn audio_is_custom(&self) -> bool {
        if let Row::Toggle(f) = &self.rows[AUDIO] {
            f.value_str() == "Custom"
        } else {
            false
        }
    }

    /// True if the WAV file row should accept focus and keyboard input.
    pub fn wav_row_is_active(&self) -> bool {
        self.audio_is_custom()
    }

    /// Handle keyboard input when the WAV file text row is focused.
    ///
    /// Two-phase: focused but not editing (up/down navigate normally) →
    /// Enter starts editing → Enter commits & loads, Escape cancels.
    pub fn handle_wav_keys(&mut self, events: &[egui::Event]) -> WavKeysResult {
        let mut result = WavKeysResult {
            load_requested: false,
            defocus: false,
            consumed: false,
        };

        let editing = self.pending_wav.is_some();

        if editing {
            result.consumed = true;
            for e in events {
                match e {
                    egui::Event::Text(s) => {
                        if let Some(pending) = &mut self.pending_wav {
                            for c in s.chars() {
                                if (' '..='~').contains(&c) {
                                    pending.push(c);
                                }
                            }
                        }
                    }
                    egui::Event::Key {
                        key: egui::Key::Backspace,
                        pressed: true,
                        ..
                    } => {
                        if let Some(pending) = &mut self.pending_wav {
                            pending.pop();
                        }
                    }
                    egui::Event::Key {
                        key: egui::Key::Enter,
                        pressed: true,
                        ..
                    } => {
                        if let Some(pending) = self.pending_wav.take() {
                            if let Row::Text(f) = &mut self.rows[WAV_FILE] {
                                f.value = pending;
                            }
                            result.load_requested = true;
                            // Don't defocus yet — view.rs defocuses on success,
                            // keeps focus on failure so user can re-edit.
                        }
                    }
                    egui::Event::Key {
                        key: egui::Key::Escape,
                        pressed: true,
                        ..
                    } => {
                        self.pending_wav = None;
                        result.defocus = true;
                    }
                    _ => {}
                }
            }
            return result;
        }

        // Not editing: Enter starts an edit.
        let enter_pressed = events.iter().any(|e| {
            matches!(
                e,
                egui::Event::Key {
                    key: egui::Key::Enter,
                    pressed: true,
                    ..
                }
            )
        });
        if enter_pressed {
            let current = if let Row::Text(f) = &self.rows[WAV_FILE] {
                f.value.clone()
            } else {
                String::new()
            };
            self.pending_wav = Some(current);
            result.consumed = true;
            return result;
        }

        // Up/Down/Escape/etc. fall through (consumed = false).
        result
    }

    /// Draw the WAV file text field value.
    pub fn draw_wav_field(&self, ctx: &RowDrawCtx, val_x: f32, y: f32, row_h: f32, focused: bool) {
        if let Row::Text(f) = &self.rows[WAV_FILE] {
            let editing = self.pending_wav.is_some();
            let max_chars = 36usize;
            let is_custom = self.audio_is_custom();

            // Determine the display text and whether it's a dimmed placeholder.
            let (raw_text, is_placeholder) = if let Some(pending) = &self.pending_wav {
                (format!("{}\u{258b}", pending), false) // ▋ block cursor
            } else if !is_custom {
                // Morse / Voice: always show the built-in name, even if a
                // custom path is stored (it's preserved for when Custom
                // is re-selected).
                let builtin = match self.audio_idx() {
                    1 => "cq_voice.wav (built-in)",
                    _ => "cq_morse.wav (built-in)",
                };
                (builtin.to_owned(), true)
            } else if f.value.is_empty() {
                ("no audio".to_owned(), true)
            } else {
                (f.value.clone(), false)
            };

            let display = if raw_text.chars().count() > max_chars {
                let skip = raw_text.chars().count() - max_chars;
                format!("…{}", raw_text.chars().skip(skip).collect::<String>())
            } else {
                raw_text
            };

            let status_suffix = if editing || is_placeholder || !is_custom {
                ""
            } else {
                match f.status {
                    Some(true) => "  ✓",
                    Some(false) => "  ✗",
                    None => "",
                }
            };
            let full = format!("{}{}", display, status_suffix);
            let text_color = if f.status == Some(false) && !editing && !is_placeholder {
                egui::Color32::from_rgb(255, 80, 80)
            } else if is_placeholder {
                egui::Color32::from_gray(100)
            } else if focused || editing {
                egui::Color32::WHITE
            } else {
                ctx.val_color
            };
            ctx.painter.text(
                egui::pos2(val_x, y + row_h / 2.0),
                egui::Align2::LEFT_CENTER,
                full,
                ctx.med.clone(),
                text_color,
            );
            if focused {
                let hint = if editing {
                    "\u{21b5} load  Esc cancel"
                } else {
                    "\u{21b5} edit"
                };
                ctx.painter.text(
                    egui::pos2(ctx.rect_right - 14.0, y + row_h / 2.0),
                    egui::Align2::RIGHT_CENTER,
                    hint,
                    ctx.small.clone(),
                    egui::Color32::from_gray(140),
                );
            }
        }
    }

    fn audio_idx(&self) -> usize {
        if let Row::Toggle(f) = &self.rows[AUDIO] {
            f.index
        } else {
            0
        }
    }

    pub(super) const WAV_FILE_IDX: usize = WAV_FILE;
}

pub(super) struct WavKeysResult {
    pub load_requested: bool,
    /// True when user pressed Escape or Enter — caller should defocus.
    pub defocus: bool,
    /// True if the key event was consumed (don't fall through to navigation).
    pub consumed: bool,
}

// ── SourceRows ─────────────────────────────────────────────────────────────

impl super::common::SourceRows for AmDsbRows {
    fn rows(&self) -> &[Row] {
        &self.rows
    }
    fn rows_mut(&mut self) -> &mut [Row] {
        &mut self.rows
    }
    fn visible_indices(&self) -> Vec<usize> {
        self.visible_indices()
    }
    fn discard_pending(&mut self) {
        self.pending_wav = None;
    }

    fn focused_text_field(&self, local_idx: usize) -> Option<super::common::TextFieldKind> {
        (local_idx == AmDsbRows::WAV_FILE_IDX && self.wav_row_is_active())
            .then_some(super::common::TextFieldKind::AmDsbWavFile)
    }

    fn handle_text_keys(
        &mut self,
        events: &[egui::Event],
        _local_idx: usize,
    ) -> super::common::TextOutcome {
        let r = self.handle_wav_keys(events);
        super::common::TextOutcome {
            consumed: r.consumed,
            defocus: r.defocus,
            committed: r.load_requested,
        }
    }

    fn draw_text_row(
        &self,
        ctx: &RowDrawCtx,
        local_idx: usize,
        val_x: f32,
        y: f32,
        row_h: f32,
        focused: bool,
    ) -> bool {
        if local_idx == AmDsbRows::WAV_FILE_IDX {
            self.draw_wav_field(ctx, val_x, y, row_h, focused);
            true
        } else {
            false
        }
    }

    fn footer_hint(&self, focused_local: Option<usize>) -> Option<&'static str> {
        let local = focused_local?;
        if local == AmDsbRows::WAV_FILE_IDX && self.wav_row_is_active() {
            Some(if self.pending_wav.is_some() {
                "type path   ↵ load   Esc cancel"
            } else {
                "↵ edit path   ↑↓ navigate"
            })
        } else {
            None
        }
    }
}

// ── SettingsState accessors ───────────────────────────────────────────────

impl super::SettingsState {
    pub fn am_audio_is_custom(&self) -> bool {
        self.amdsb.audio_is_custom()
    }
    pub fn am_audio_idx(&self) -> usize {
        if let Row::Toggle(f) = &self.amdsb.rows[AUDIO] {
            f.index
        } else {
            0
        }
    }
    pub fn am_audio_str(&self) -> &str {
        if let Row::Toggle(f) = &self.amdsb.rows[AUDIO] {
            f.value_str()
        } else {
            "Morse"
        }
    }
    pub fn am_carrier_hz(&self) -> f32 {
        if let Row::Num(f) = &self.amdsb.rows[CARRIER] {
            f.value
        } else {
            5000.0
        }
    }
    pub fn set_am_carrier_hz(&mut self, v: f32) {
        if let Row::Num(f) = &mut self.amdsb.rows[CARRIER] {
            f.value = v.clamp(f.min, f.max);
        }
    }
    pub fn am_mod_index(&self) -> f32 {
        if let Row::Num(f) = &self.amdsb.rows[MOD_IDX] {
            f.value
        } else {
            1.0
        }
    }
    pub fn am_gap_secs(&self) -> f32 {
        if let Row::Num(f) = &self.amdsb.rows[GAP] {
            f.value
        } else {
            2.0
        }
    }
    pub fn am_noise_amp(&self) -> f32 {
        if let Row::Num(f) = &self.amdsb.rows[NOISE] {
            f.value
        } else {
            0.05
        }
    }
    pub fn am_msg_repeat(&self) -> usize {
        if let Row::Num(f) = &self.amdsb.rows[REPEAT] {
            f.value as usize
        } else {
            1
        }
    }
    /// Reset the repeat row default (and value) to match the newly-selected audio kind.
    /// `audio_idx` 0 = Morse (default 1), 1 = Voice (default 3), other = 1.
    pub fn reset_am_repeat_for_audio(&mut self, audio_idx: usize) {
        let default = if audio_idx == 1 { 3.0 } else { 1.0 };
        if let Row::Num(f) = &mut self.amdsb.rows[REPEAT] {
            f.default = default;
            f.value = default;
        }
    }
    pub fn wav_path(&self) -> &str {
        if let Row::Text(f) = &self.amdsb.rows[WAV_FILE] {
            &f.value
        } else {
            ""
        }
    }
    pub fn set_wav_status(&mut self, ok: bool) {
        if let Row::Text(f) = &mut self.amdsb.rows[WAV_FILE] {
            f.status = Some(ok);
        }
    }
    pub fn cycle_am_audio(&mut self) {
        if let Row::Toggle(f) = &mut self.amdsb.rows[AUDIO] {
            f.next();
        }
    }
}
