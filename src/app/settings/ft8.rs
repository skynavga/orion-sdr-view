use eframe::egui;
use super::field::{Row, NumField, ToggleField, TextField};
use crate::config::ViewConfig;

// ── Row indices (local) ───────────────────────────────────────────────────
const MODE:       usize = 0;
const CARRIER:    usize = 1;
const LOOP_GAP:   usize = 2;
const NOISE:      usize = 3;
const MSG_TYPE:   usize = 4;
const CALL_TO:    usize = 5;
const CALL_DE:    usize = 6;
const GRID:       usize = 7;
const FREE_TEXT:  usize = 8;
const REPEAT:     usize = 9;

pub(super) struct Ft8Rows {
    pub rows: Vec<Row>,
    /// In-progress edit of the free text field. `Some(s)` while typing;
    /// committed on Enter, discarded on Escape.
    pub pending_text: Option<String>,
}

impl Ft8Rows {
    pub fn new() -> Self {
        Self {
            rows: vec![
                Row::Toggle(ToggleField {
                    label: "Mode",
                    options: &["FT8", "FT4"],
                    index: 0, default: 0,
                }),
                Row::Num(NumField {
                    label: "Carrier",
                    value:   crate::source::ft8::FT8_DEFAULT_CARRIER_HZ,
                    default: crate::source::ft8::FT8_DEFAULT_CARRIER_HZ,
                    step: 100.0, min: 100.0, max: 22000.0, unit: " Hz",
                }),
                Row::Num(NumField {
                    label: "Loop gap",
                    value:   crate::source::ft8::FT8_DEFAULT_LOOP_GAP_SECS,
                    default: crate::source::ft8::FT8_DEFAULT_LOOP_GAP_SECS,
                    step: 1.0, min: 15.0, max: 120.0, unit: " s",
                }),
                Row::Num(NumField {
                    label: "Noise amp",
                    value: 0.0, default: 0.0,
                    step: 0.01, min: 0.0, max: 0.50, unit: "",
                }),
                Row::Toggle(ToggleField {
                    label: "Message",
                    options: &["Standard", "Free text"],
                    index: 0, default: 0,
                }),
                Row::Text(TextField {
                    label: "Call to",
                    value:         crate::source::ft8::FT8_DEFAULT_CALL_TO.to_owned(),
                    default_value: crate::source::ft8::FT8_DEFAULT_CALL_TO.to_owned(),
                    status: None,
                }),
                Row::Text(TextField {
                    label: "Call de",
                    value:         crate::source::ft8::FT8_DEFAULT_CALL_DE.to_owned(),
                    default_value: crate::source::ft8::FT8_DEFAULT_CALL_DE.to_owned(),
                    status: None,
                }),
                Row::Text(TextField {
                    label: "Grid",
                    value:         crate::source::ft8::FT8_DEFAULT_GRID.to_owned(),
                    default_value: crate::source::ft8::FT8_DEFAULT_GRID.to_owned(),
                    status: None,
                }),
                Row::Text(TextField {
                    label: "Free text",
                    value:         crate::source::ft8::FT8_DEFAULT_FREE_TEXT.to_owned(),
                    default_value: crate::source::ft8::FT8_DEFAULT_FREE_TEXT.to_owned(),
                    status: None,
                }),
                Row::Num(NumField {
                    label: "Repeat",
                    value:   crate::source::ft8::FT8_DEFAULT_REPEAT as f32,
                    default: crate::source::ft8::FT8_DEFAULT_REPEAT as f32,
                    step: 1.0, min: 1.0, max: 20.0, unit: "×",
                }),
            ],
            pending_text: None,
        }
    }

    pub fn patch_from_config(&mut self, cfg: &ViewConfig) {
        self.rows[CARRIER].patch_num(cfg.ft8_carrier_hz());
        self.rows[LOOP_GAP].patch_num(cfg.ft8_loop_gap_secs());
        self.rows[NOISE].patch_num(cfg.ft8_noise_amp());
        self.rows[REPEAT].patch_num(cfg.ft8_msg_repeat() as f32);

        let mode_idx = match cfg.ft8_mode() { "FT4" => 1, _ => 0 };
        if let Row::Toggle(f) = &mut self.rows[MODE] {
            f.index   = mode_idx;
            f.default = mode_idx;
        }

        if let Row::Text(f) = &mut self.rows[CALL_TO] {
            let s = cfg.ft8_call_to().to_owned();
            f.value = s.clone(); f.default_value = s;
        }
        if let Row::Text(f) = &mut self.rows[CALL_DE] {
            let s = cfg.ft8_call_de().to_owned();
            f.value = s.clone(); f.default_value = s;
        }
        if let Row::Text(f) = &mut self.rows[GRID] {
            let s = cfg.ft8_grid().to_owned();
            f.value = s.clone(); f.default_value = s;
        }
        if let Row::Text(f) = &mut self.rows[FREE_TEXT] {
            let s = cfg.ft8_free_text().to_owned();
            f.value = s.clone(); f.default_value = s;
        }
    }

    pub fn msg_is_free_text(&self) -> bool {
        if let Row::Toggle(f) = &self.rows[MSG_TYPE] { f.index == 1 } else { false }
    }

    pub fn visible_indices(&self) -> Vec<usize> {
        let mut v = vec![MODE, MSG_TYPE];
        if self.msg_is_free_text() {
            v.push(FREE_TEXT);
        } else {
            v.extend([CALL_TO, CALL_DE, GRID]);
        }
        v.extend([REPEAT, CARRIER, LOOP_GAP, NOISE]);
        v
    }

    /// Handle keyboard input when the free-text row is focused.
    pub fn handle_text_keys(&mut self, events: &[egui::Event]) -> TextKeysResult {
        let mut result = TextKeysResult { accepted: false, defocus: false, consumed: false };

        if self.pending_text.is_some() {
            result.consumed = true;
            for e in events {
                match e {
                    egui::Event::Text(s) => {
                        if let Some(pending) = &mut self.pending_text {
                            for c in s.chars() {
                                if c >= ' ' && c <= '~' {
                                    pending.push(c);
                                }
                            }
                        }
                    }
                    egui::Event::Key { key: egui::Key::Backspace, pressed: true, .. } => {
                        if let Some(pending) = &mut self.pending_text {
                            pending.pop();
                        }
                    }
                    egui::Event::Key { key: egui::Key::Enter, pressed: true, .. } => {
                        if let Some(pending) = self.pending_text.take() {
                            let committed = if pending.trim().is_empty() {
                                crate::source::ft8::FT8_DEFAULT_FREE_TEXT.to_owned()
                            } else {
                                pending
                            };
                            if let Row::Text(f) = &mut self.rows[FREE_TEXT] {
                                f.value = committed;
                            }
                            result.accepted = true;
                        }
                        result.defocus = true;
                    }
                    egui::Event::Key { key: egui::Key::Escape, pressed: true, .. } => {
                        self.pending_text = None;
                        result.defocus = true;
                    }
                    _ => {}
                }
            }
            return result;
        }

        // Not editing: Enter starts an edit.
        let enter_pressed = events.iter().any(|e| {
            matches!(e, egui::Event::Key { key: egui::Key::Enter, pressed: true, .. })
        });
        if enter_pressed {
            let current = if let Row::Text(f) = &self.rows[FREE_TEXT] {
                f.value.clone()
            } else {
                String::new()
            };
            self.pending_text = Some(current);
            result.consumed = true;
        }
        result
    }

    pub fn discard_pending(&mut self) {
        self.pending_text = None;
    }

    /// Draw the free-text field (editable).
    pub fn draw_free_text(
        &self,
        painter: &egui::Painter,
        val_x: f32, y: f32, row_h: f32,
        rect_right: f32,
        med: &egui::FontId,
        small: &egui::FontId,
        val_color: egui::Color32,
        focused: bool,
    ) {
        if let Row::Text(f) = &self.rows[FREE_TEXT] {
            let max_chars = 20usize;
            let (raw_text, editing) = if let Some(pending) = &self.pending_text {
                (format!("{}\u{258b}", pending), true)
            } else {
                (f.value.clone(), false)
            };
            let display = if raw_text.chars().count() > max_chars {
                let skip = raw_text.chars().count() - max_chars;
                format!("…{}", raw_text.chars().skip(skip).collect::<String>())
            } else {
                raw_text
            };
            let text_color = if focused || editing { egui::Color32::WHITE } else { val_color };
            painter.text(
                egui::pos2(val_x, y + row_h / 2.0),
                egui::Align2::LEFT_CENTER,
                &display,
                med.clone(),
                text_color,
            );
            if focused {
                let hint = if editing { "\u{21b5} accept  Esc cancel" } else { "\u{21b5} edit" };
                painter.text(
                    egui::pos2(rect_right - 14.0, y + row_h / 2.0),
                    egui::Align2::RIGHT_CENTER,
                    hint,
                    small.clone(),
                    egui::Color32::from_gray(140),
                );
            }
        }
    }

    /// Draw a read-only text field (Call To / Call De / Grid).
    pub fn draw_readonly_text(
        &self,
        row_idx: usize,
        painter: &egui::Painter,
        val_x: f32, y: f32, row_h: f32,
        rect_right: f32,
        med: &egui::FontId,
        small: &egui::FontId,
        val_color: egui::Color32,
        focused: bool,
    ) {
        if let Row::Text(f) = &self.rows[row_idx] {
            painter.text(
                egui::pos2(val_x, y + row_h / 2.0),
                egui::Align2::LEFT_CENTER,
                &f.value,
                med.clone(),
                if focused { egui::Color32::WHITE } else { val_color },
            );
            if focused {
                painter.text(
                    egui::pos2(rect_right - 14.0, y + row_h / 2.0),
                    egui::Align2::RIGHT_CENTER,
                    "(config)",
                    small.clone(),
                    egui::Color32::from_gray(100),
                );
            }
        }
    }

    pub(super) const FREE_TEXT_IDX: usize = FREE_TEXT;
}

pub(super) struct TextKeysResult {
    pub accepted: bool,
    pub defocus:  bool,
    pub consumed: bool,
}

// ── SettingsState accessors ───────────────────────────────────────────────

impl super::SettingsState {
    pub fn ft8_mode_str(&self) -> &str {
        if let Row::Toggle(f) = &self.ft8.rows[MODE] { f.value_str() } else { "FT8" }
    }
    pub fn ft8_carrier_hz(&self) -> f32 {
        if let Row::Num(f) = &self.ft8.rows[CARRIER] { f.value } else {
            crate::source::ft8::FT8_DEFAULT_CARRIER_HZ
        }
    }
    pub fn set_ft8_carrier_hz(&mut self, v: f32) {
        if let Row::Num(f) = &mut self.ft8.rows[CARRIER] {
            f.value = v.clamp(f.min, f.max);
        }
    }
    pub fn ft8_loop_gap_secs(&self) -> f32 {
        if let Row::Num(f) = &self.ft8.rows[LOOP_GAP] { f.value } else {
            crate::source::ft8::FT8_DEFAULT_LOOP_GAP_SECS
        }
    }
    pub fn ft8_noise_amp(&self) -> f32 {
        if let Row::Num(f) = &self.ft8.rows[NOISE] { f.value } else { 0.0 }
    }
    pub fn ft8_msg_repeat(&self) -> usize {
        if let Row::Num(f) = &self.ft8.rows[REPEAT] { f.value as usize } else {
            crate::source::ft8::FT8_DEFAULT_REPEAT
        }
    }
    pub fn ft8_call_to(&self) -> &str {
        if let Row::Text(f) = &self.ft8.rows[CALL_TO] { &f.value } else {
            crate::source::ft8::FT8_DEFAULT_CALL_TO
        }
    }
    pub fn ft8_call_de(&self) -> &str {
        if let Row::Text(f) = &self.ft8.rows[CALL_DE] { &f.value } else {
            crate::source::ft8::FT8_DEFAULT_CALL_DE
        }
    }
    pub fn ft8_grid(&self) -> &str {
        if let Row::Text(f) = &self.ft8.rows[GRID] { &f.value } else {
            crate::source::ft8::FT8_DEFAULT_GRID
        }
    }
    pub fn ft8_free_text(&self) -> &str {
        if let Row::Text(f) = &self.ft8.rows[FREE_TEXT] { &f.value } else {
            crate::source::ft8::FT8_DEFAULT_FREE_TEXT
        }
    }
    pub fn ft8_msg_is_free_text(&self) -> bool {
        self.ft8.msg_is_free_text()
    }
}
