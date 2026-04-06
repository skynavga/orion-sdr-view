use eframe::egui;
use super::field::{Row, NumField, ToggleField, TextField};
use crate::config::ViewConfig;

// ── Row indices (local) ───────────────────────────────────────────────────
const AUDIO:    usize = 0;
const CARRIER:  usize = 1;
const MOD_IDX:  usize = 2;
const LOOP_GAP: usize = 3;
const NOISE:    usize = 4;
const WAV_FILE: usize = 5;
const REPEAT:   usize = 6;

pub(super) struct AmDsbRows {
    pub rows: Vec<Row>,
}

impl AmDsbRows {
    pub fn new() -> Self {
        Self {
            rows: vec![
                Row::Toggle(ToggleField {
                    label: "Audio",
                    options: &["Morse", "Voice", "Custom"],
                    index: 0, default: 0,
                }),
                Row::Num(NumField {
                    label: "Carrier Hz", value: 12000.0, default: 12000.0,
                    step: 100.0, min: 100.0, max: 23_900.0, unit: " Hz",
                }),
                Row::Num(NumField {
                    label: "Mod index", value: 1.0, default: 1.0,
                    step: 0.1, min: 0.1, max: 2.0, unit: "",
                }),
                Row::Num(NumField {
                    label: "Loop gap", value: 7.0, default: 7.0,
                    step: 0.5, min: 0.0, max: 30.0, unit: " s",
                }),
                Row::Num(NumField {
                    label: "Noise amp", value: 0.05, default: 0.05,
                    step: 0.01, min: 0.0, max: 0.50, unit: "",
                }),
                Row::Text(TextField {
                    label: "Audio source",
                    value: String::new(),
                    default_value: String::new(),
                    status: None,
                }),
                Row::Num(NumField {
                    label: "Repeat", value: 1.0, default: 1.0,
                    step: 1.0, min: 1.0, max: 20.0, unit: "×",
                }),
            ],
        }
    }

    pub fn patch_from_config(&mut self, cfg: &ViewConfig) {
        self.rows[CARRIER].patch_num(cfg.carrier_hz());
        self.rows[MOD_IDX].patch_num(cfg.mod_index());
        self.rows[LOOP_GAP].patch_num(cfg.loop_gap_secs());
        self.rows[NOISE].patch_num(cfg.am_noise_amp());
        self.rows[REPEAT].patch_num(cfg.am_msg_repeat() as f32);
    }

    /// Visible rows in the order they appear in the settings overlay.
    pub fn visible_indices(&self) -> Vec<usize> {
        vec![AUDIO, WAV_FILE, REPEAT, CARRIER, MOD_IDX, LOOP_GAP, NOISE]
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
    /// Returns true if the user pressed Enter (load requested).
    pub fn handle_wav_keys(&mut self, events: &[egui::Event]) -> WavKeysResult {
        let mut result = WavKeysResult { load_requested: false, defocus: false };
        for e in events {
            match e {
                egui::Event::Text(s) => {
                    for c in s.chars() {
                        if let Row::Text(f) = &mut self.rows[WAV_FILE] {
                            f.push_char(c);
                        }
                    }
                }
                egui::Event::Key { key: egui::Key::Backspace, pressed: true, .. } => {
                    if let Row::Text(f) = &mut self.rows[WAV_FILE] {
                        f.pop_char();
                    }
                }
                egui::Event::Key { key: egui::Key::Enter, pressed: true, .. } => {
                    result.load_requested = true;
                }
                egui::Event::Key { key: egui::Key::Escape, pressed: true, .. } => {
                    result.defocus = true;
                }
                _ => {}
            }
        }
        result
    }

    /// Draw the WAV file text field value.
    pub fn draw_wav_field(
        &self,
        painter: &egui::Painter,
        val_x: f32, y: f32, row_h: f32,
        rect_right: f32,
        med: &egui::FontId,
        small: &egui::FontId,
        val_color: egui::Color32,
        focused: bool,
    ) {
        if let Row::Text(f) = &self.rows[WAV_FILE] {
            let builtin_placeholder = match self.audio_idx() {
                1 => "cq_voice.wav (built-in)",
                _ => "cq_morse.wav (built-in)",
            };
            let display = if f.value.is_empty() {
                builtin_placeholder.to_owned()
            } else {
                let max_chars = 36usize;
                if f.value.len() > max_chars {
                    format!("…{}", &f.value[f.value.len() - max_chars..])
                } else {
                    f.value.clone()
                }
            };
            let status_suffix = match f.status {
                Some(true)  => "  ✓",
                Some(false) => "  ✗",
                None        => "",
            };
            let full = format!("{}{}", display, status_suffix);
            let text_color = if focused { egui::Color32::WHITE } else { val_color };
            painter.text(
                egui::pos2(val_x, y + row_h / 2.0),
                egui::Align2::LEFT_CENTER,
                full,
                med.clone(),
                text_color,
            );
            if focused {
                painter.text(
                    egui::pos2(rect_right - 14.0, y + row_h / 2.0),
                    egui::Align2::RIGHT_CENTER,
                    "↵ load",
                    small.clone(),
                    egui::Color32::from_gray(140),
                );
            }
        }
    }

    fn audio_idx(&self) -> usize {
        if let Row::Toggle(f) = &self.rows[AUDIO] { f.index } else { 0 }
    }

    pub(super) const WAV_FILE_IDX: usize = WAV_FILE;
}

pub(super) struct WavKeysResult {
    pub load_requested: bool,
    /// True when user pressed Escape — caller should defocus.
    pub defocus: bool,
}

// ── SettingsState accessors ───────────────────────────────────────────────

impl super::SettingsState {
    pub fn am_audio_is_custom(&self) -> bool {
        self.amdsb.audio_is_custom()
    }
    pub fn am_audio_idx(&self) -> usize {
        if let Row::Toggle(f) = &self.amdsb.rows[AUDIO] { f.index } else { 0 }
    }
    pub fn am_audio_str(&self) -> &str {
        if let Row::Toggle(f) = &self.amdsb.rows[AUDIO] { f.value_str() } else { "Morse" }
    }
    pub fn am_carrier_hz(&self) -> f32 {
        if let Row::Num(f) = &self.amdsb.rows[CARRIER] { f.value } else { 5000.0 }
    }
    pub fn set_am_carrier_hz(&mut self, v: f32) {
        if let Row::Num(f) = &mut self.amdsb.rows[CARRIER] {
            f.value = v.clamp(f.min, f.max);
        }
    }
    pub fn am_mod_index(&self) -> f32 {
        if let Row::Num(f) = &self.amdsb.rows[MOD_IDX] { f.value } else { 1.0 }
    }
    pub fn am_loop_gap_secs(&self) -> f32 {
        if let Row::Num(f) = &self.amdsb.rows[LOOP_GAP] { f.value } else { 2.0 }
    }
    pub fn am_noise_amp(&self) -> f32 {
        if let Row::Num(f) = &self.amdsb.rows[NOISE] { f.value } else { 0.05 }
    }
    pub fn am_msg_repeat(&self) -> usize {
        if let Row::Num(f) = &self.amdsb.rows[REPEAT] { f.value as usize } else { 1 }
    }
    /// Reset the repeat row default (and value) to match the newly-selected audio kind.
    /// `audio_idx` 0 = Morse (default 1), 1 = Voice (default 3), other = 1.
    pub fn reset_am_repeat_for_audio(&mut self, audio_idx: usize) {
        let default = if audio_idx == 1 { 3.0 } else { 1.0 };
        if let Row::Num(f) = &mut self.amdsb.rows[REPEAT] {
            f.default = default;
            f.value   = default;
        }
    }
    pub fn wav_path(&self) -> &str {
        if let Row::Text(f) = &self.amdsb.rows[WAV_FILE] { &f.value } else { "" }
    }
    pub fn set_wav_status(&mut self, ok: bool) {
        if let Row::Text(f) = &mut self.amdsb.rows[WAV_FILE] {
            f.status = Some(ok);
        }
    }
    pub fn cycle_am_audio(&mut self) {
        if let Row::Toggle(f) = &mut self.amdsb.rows[AUDIO] { f.next(); }
    }
}
