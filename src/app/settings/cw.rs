// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use super::field::{NumField, Row, RowDrawCtx, TextField, ToggleField};
use crate::config::ViewConfig;
use eframe::egui;

// ── Row indices (local) ───────────────────────────────────────────────────
const WPM: usize = 0;
const JITTER: usize = 1;
const DASH_WEIGHT: usize = 2;
const CHAR_SPACE: usize = 3;
const WORD_SPACE: usize = 4;
const RISE: usize = 5;
const FALL: usize = 6;
const MSG_MODE: usize = 7;
const MSG: usize = 8;
const CUSTOM_MSG: usize = 9;
const REPEAT: usize = 10;
const CARRIER: usize = 11;
const GAP: usize = 12;
const NOISE: usize = 13;

pub(super) struct CwRows {
    pub rows: Vec<Row>,
    /// In-progress edit of a CW message field.  `Some(s)` while the user
    /// is typing; committed to the row on Enter, discarded on Escape.
    pub pending_msg: Option<String>,
    /// Which local row index is being edited (MSG or CUSTOM_MSG).
    pub editing_msg_row: Option<usize>,
}

impl CwRows {
    pub fn new() -> Self {
        Self {
            rows: vec![
                Row::Num(NumField {
                    label: "WPM",
                    value: crate::source::cw::CW_DEFAULT_WPM,
                    default: crate::source::cw::CW_DEFAULT_WPM,
                    step: 1.0,
                    min: 3.0,
                    max: 30.0,
                    unit: "",
                }),
                Row::Num(NumField {
                    label: "Jitter",
                    value: crate::source::cw::CW_DEFAULT_JITTER_PCT,
                    default: crate::source::cw::CW_DEFAULT_JITTER_PCT,
                    step: 1.0,
                    min: 0.0,
                    max: 30.0,
                    unit: " %",
                }),
                Row::Num(NumField {
                    label: "Dash weight",
                    value: crate::source::cw::CW_DEFAULT_DASH_WEIGHT,
                    default: crate::source::cw::CW_DEFAULT_DASH_WEIGHT,
                    step: 0.1,
                    min: 2.5,
                    max: 3.5,
                    unit: "",
                }),
                Row::Num(NumField {
                    label: "Char space",
                    value: crate::source::cw::CW_DEFAULT_CHAR_SPACE,
                    default: crate::source::cw::CW_DEFAULT_CHAR_SPACE,
                    step: 0.1,
                    min: 2.5,
                    max: 4.0,
                    unit: " u",
                }),
                Row::Num(NumField {
                    label: "Word space",
                    value: crate::source::cw::CW_DEFAULT_WORD_SPACE,
                    default: crate::source::cw::CW_DEFAULT_WORD_SPACE,
                    step: 0.5,
                    min: 6.0,
                    max: 9.0,
                    unit: " u",
                }),
                Row::Num(NumField {
                    label: "Rise",
                    value: crate::source::cw::CW_DEFAULT_RISE_MS,
                    default: crate::source::cw::CW_DEFAULT_RISE_MS,
                    step: 1.0,
                    min: 1.0,
                    max: 20.0,
                    unit: " ms",
                }),
                Row::Num(NumField {
                    label: "Fall",
                    value: crate::source::cw::CW_DEFAULT_FALL_MS,
                    default: crate::source::cw::CW_DEFAULT_FALL_MS,
                    step: 1.0,
                    min: 1.0,
                    max: 20.0,
                    unit: " ms",
                }),
                Row::Toggle(ToggleField {
                    label: "Message",
                    options: &["Canned", "Custom"],
                    index: 0,
                    default: 0,
                }),
                Row::Text(TextField {
                    label: "Text",
                    value: crate::source::cw::CW_DEFAULT_CANNED_TEXT.to_owned(),
                    default_value: crate::source::cw::CW_DEFAULT_CANNED_TEXT.to_owned(),
                    status: None,
                }),
                Row::Text(TextField {
                    label: "Text",
                    value: crate::source::cw::CW_DEFAULT_CUSTOM_TEXT.to_owned(),
                    default_value: crate::source::cw::CW_DEFAULT_CUSTOM_TEXT.to_owned(),
                    status: None,
                }),
                Row::Num(NumField {
                    label: "Repeat",
                    value: crate::source::cw::CW_DEFAULT_REPEAT as f32,
                    default: crate::source::cw::CW_DEFAULT_REPEAT as f32,
                    step: 1.0,
                    min: 1.0,
                    max: 20.0,
                    unit: "\u{00d7}",
                }),
                Row::Num(NumField {
                    label: "Carrier",
                    value: crate::source::cw::CW_DEFAULT_CARRIER_HZ,
                    default: crate::source::cw::CW_DEFAULT_CARRIER_HZ,
                    step: 100.0,
                    min: 100.0,
                    max: 22000.0,
                    unit: " Hz",
                }),
                Row::Num(NumField {
                    label: "Gap",
                    value: crate::source::cw::CW_DEFAULT_GAP_SECS,
                    default: crate::source::cw::CW_DEFAULT_GAP_SECS,
                    step: 0.5,
                    min: 0.5,
                    max: 99.99,
                    unit: " s",
                }),
                Row::Num(NumField {
                    label: "Noise amp",
                    value: crate::source::cw::CW_DEFAULT_NOISE_AMP,
                    default: crate::source::cw::CW_DEFAULT_NOISE_AMP,
                    step: 0.01,
                    min: 0.0,
                    max: 0.50,
                    unit: "",
                }),
            ],
            pending_msg: None,
            editing_msg_row: None,
        }
    }

    pub fn patch_from_config(&mut self, cfg: &ViewConfig) {
        self.rows[WPM].patch_num(cfg.cw_wpm());
        self.rows[JITTER].patch_num(cfg.cw_jitter_pct());
        self.rows[DASH_WEIGHT].patch_num(cfg.cw_dash_weight());
        self.rows[CHAR_SPACE].patch_num(cfg.cw_char_space());
        self.rows[WORD_SPACE].patch_num(cfg.cw_word_space());
        self.rows[RISE].patch_num(cfg.cw_rise_ms());
        self.rows[FALL].patch_num(cfg.cw_fall_ms());
        self.rows[CARRIER].patch_num(cfg.cw_carrier_hz());
        self.rows[GAP].patch_num(cfg.cw_gap_secs());
        self.rows[NOISE].patch_num(cfg.cw_noise_amp());
        self.rows[REPEAT].patch_num(cfg.cw_msg_repeat() as f32);

        // Patch canned message text
        if let Row::Text(f) = &mut self.rows[MSG] {
            let msg = cfg.cw_canned_text().to_owned();
            f.value = msg.clone();
            f.default_value = msg;
        }

        // Patch custom message text
        if let Row::Text(f) = &mut self.rows[CUSTOM_MSG] {
            let msg = cfg.cw_custom_text().to_owned();
            f.value = msg.clone();
            f.default_value = msg;
        }
    }

    pub fn msg_is_custom(&self) -> bool {
        if let Row::Toggle(f) = &self.rows[MSG_MODE] {
            f.index == 1
        } else {
            false
        }
    }

    /// Visible rows in the order they appear in the settings overlay.
    pub fn visible_indices(&self) -> Vec<usize> {
        let mut v = vec![
            WPM,
            JITTER,
            DASH_WEIGHT,
            CHAR_SPACE,
            WORD_SPACE,
            RISE,
            FALL,
            MSG_MODE,
        ];
        if self.msg_is_custom() {
            v.push(CUSTOM_MSG);
        } else {
            v.push(MSG);
        }
        v.extend([REPEAT, CARRIER, GAP, NOISE]);
        v
    }

    /// Handle keyboard input when the custom CW message row is focused.
    pub fn handle_msg_keys(
        &mut self,
        events: &[egui::Event],
        focused_local_idx: usize,
    ) -> MsgKeysResult {
        let mut result = MsgKeysResult {
            msg_accepted: false,
            defocus: false,
            consumed: false,
        };

        let editing = self.pending_msg.is_some();

        if editing {
            result.consumed = true;
            for e in events {
                match e {
                    egui::Event::Text(s) => {
                        if let Some(pending) = &mut self.pending_msg {
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
                        if let Some(pending) = &mut self.pending_msg {
                            pending.pop();
                        }
                    }
                    egui::Event::Key {
                        key: egui::Key::Enter,
                        pressed: true,
                        ..
                    } => {
                        if let Some(pending) = self.pending_msg.take() {
                            let target = self.editing_msg_row.unwrap_or(MSG);
                            let default_text = if target == CUSTOM_MSG {
                                crate::source::cw::CW_DEFAULT_CUSTOM_TEXT
                            } else {
                                crate::source::cw::CW_DEFAULT_CANNED_TEXT
                            };
                            let committed = if pending.trim().is_empty() {
                                default_text.to_owned()
                            } else {
                                pending
                            };
                            if let Row::Text(f) = &mut self.rows[target] {
                                f.value = committed;
                            }
                            self.editing_msg_row = None;
                            result.msg_accepted = true;
                        }
                        result.defocus = true;
                    }
                    egui::Event::Key {
                        key: egui::Key::Escape,
                        pressed: true,
                        ..
                    } => {
                        self.pending_msg = None;
                        self.editing_msg_row = None;
                        result.defocus = true;
                    }
                    _ => {}
                }
            }
            return result;
        }

        // Not editing: Enter starts an edit.
        let edit_target = focused_local_idx;

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
            let current = if let Row::Text(f) = &self.rows[edit_target] {
                f.value.clone()
            } else {
                String::new()
            };
            self.pending_msg = Some(current);
            self.editing_msg_row = Some(edit_target);
            result.consumed = true;
            return result;
        }

        result
    }

    /// Discard any in-progress pending edit (called when focus moves away).
    pub fn discard_pending(&mut self) {
        self.pending_msg = None;
        self.editing_msg_row = None;
    }

    /// Draw the canned CW message text field (read-only).
    pub fn draw_canned_msg(&self, ctx: &RowDrawCtx, val_x: f32, y: f32, row_h: f32, focused: bool) {
        if let Row::Text(f) = &self.rows[MSG] {
            let max_chars = 36usize;
            let display = if f.value.chars().count() > max_chars {
                let skip = f.value.chars().count() - max_chars;
                format!("\u{2026}{}", f.value.chars().skip(skip).collect::<String>())
            } else {
                f.value.clone()
            };
            let text_color = if focused {
                egui::Color32::WHITE
            } else {
                ctx.val_color
            };
            ctx.painter.text(
                egui::pos2(val_x, y + row_h / 2.0),
                egui::Align2::LEFT_CENTER,
                &display,
                ctx.med.clone(),
                text_color,
            );
            if focused {
                ctx.painter.text(
                    egui::pos2(ctx.rect_right - 14.0, y + row_h / 2.0),
                    egui::Align2::RIGHT_CENTER,
                    "(config)",
                    ctx.small.clone(),
                    egui::Color32::from_gray(100),
                );
            }
        }
    }

    /// Draw the custom CW message text field (editable).
    pub fn draw_custom_msg(&self, ctx: &RowDrawCtx, val_x: f32, y: f32, row_h: f32, focused: bool) {
        if let Row::Text(f) = &self.rows[CUSTOM_MSG] {
            let max_chars = 36usize;
            let (raw_text, editing) = if let Some(pending) = &self.pending_msg {
                (format!("{}\u{258b}", pending), true) // block cursor
            } else {
                (f.value.clone(), false)
            };
            let display = if raw_text.chars().count() > max_chars {
                let skip = raw_text.chars().count() - max_chars;
                format!(
                    "\u{2026}{}",
                    raw_text.chars().skip(skip).collect::<String>()
                )
            } else {
                raw_text
            };
            let text_color = if focused || editing {
                egui::Color32::WHITE
            } else {
                ctx.val_color
            };
            ctx.painter.text(
                egui::pos2(val_x, y + row_h / 2.0),
                egui::Align2::LEFT_CENTER,
                &display,
                ctx.med.clone(),
                text_color,
            );
            if focused {
                let hint = if editing {
                    "\u{21b5} accept  Esc cancel"
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

    pub(super) const MSG_IDX: usize = MSG;
    pub(super) const CUSTOM_MSG_IDX: usize = CUSTOM_MSG;
}

pub(super) struct MsgKeysResult {
    pub msg_accepted: bool,
    pub defocus: bool,
    pub consumed: bool,
}

// ── SourceRows ─────────────────────────────────────────────────────────────

impl super::common::SourceRows for CwRows {
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
        self.discard_pending();
    }
}

// ── Settings dispatch helpers ──────────────────────────────────────────────

/// Identifies a CW text-editable row, if any.  The custom-message row is
/// always editable when focused; the canned-message row is read-only.
pub(super) fn focused_text_field(
    _rows: &CwRows,
    local_idx: usize,
) -> Option<super::common::TextFieldKind> {
    if local_idx == CwRows::CUSTOM_MSG_IDX {
        Some(super::common::TextFieldKind::CwCustomMsg)
    } else {
        None
    }
}

/// Handle keys when the CW custom-message text row is focused.
pub(super) fn handle_text_keys(
    rows: &mut CwRows,
    events: &[egui::Event],
    local_idx: usize,
) -> super::common::TextOutcome {
    let r = rows.handle_msg_keys(events, local_idx);
    super::common::TextOutcome {
        consumed: r.consumed,
        defocus: r.defocus,
        committed: r.msg_accepted,
    }
}

/// Render a CW special text row.  Returns `true` if rendered.
pub(super) fn draw_text_row(
    rows: &CwRows,
    ctx: &RowDrawCtx,
    local_idx: usize,
    val_x: f32,
    y: f32,
    row_h: f32,
    focused: bool,
) -> bool {
    if local_idx == CwRows::MSG_IDX {
        rows.draw_canned_msg(ctx, val_x, y, row_h, focused);
        true
    } else if local_idx == CwRows::CUSTOM_MSG_IDX {
        rows.draw_custom_msg(ctx, val_x, y, row_h, focused);
        true
    } else {
        false
    }
}

/// Footer hint for CW-focused rows.
pub(super) fn footer_hint(rows: &CwRows, focused_local: Option<usize>) -> Option<&'static str> {
    let local = focused_local?;
    if local == CwRows::CUSTOM_MSG_IDX {
        Some(if rows.pending_msg.is_some() {
            "type message   \u{21b5} accept   Esc cancel"
        } else {
            "\u{21b5} edit message   \u{2191}\u{2193} navigate"
        })
    } else {
        None
    }
}

// ── SettingsState accessors ───────────────────────────────────────────────

impl super::SettingsState {
    pub fn cw_wpm(&self) -> f32 {
        if let Row::Num(f) = &self.cw.rows[WPM] {
            f.value
        } else {
            crate::source::cw::CW_DEFAULT_WPM
        }
    }
    pub fn cw_jitter_pct(&self) -> f32 {
        if let Row::Num(f) = &self.cw.rows[JITTER] {
            f.value
        } else {
            crate::source::cw::CW_DEFAULT_JITTER_PCT
        }
    }
    pub fn cw_dash_weight(&self) -> f32 {
        if let Row::Num(f) = &self.cw.rows[DASH_WEIGHT] {
            f.value
        } else {
            crate::source::cw::CW_DEFAULT_DASH_WEIGHT
        }
    }
    pub fn cw_char_space(&self) -> f32 {
        if let Row::Num(f) = &self.cw.rows[CHAR_SPACE] {
            f.value
        } else {
            crate::source::cw::CW_DEFAULT_CHAR_SPACE
        }
    }
    pub fn cw_word_space(&self) -> f32 {
        if let Row::Num(f) = &self.cw.rows[WORD_SPACE] {
            f.value
        } else {
            crate::source::cw::CW_DEFAULT_WORD_SPACE
        }
    }
    pub fn cw_rise_ms(&self) -> f32 {
        if let Row::Num(f) = &self.cw.rows[RISE] {
            f.value
        } else {
            crate::source::cw::CW_DEFAULT_RISE_MS
        }
    }
    pub fn cw_fall_ms(&self) -> f32 {
        if let Row::Num(f) = &self.cw.rows[FALL] {
            f.value
        } else {
            crate::source::cw::CW_DEFAULT_FALL_MS
        }
    }
    pub fn cw_carrier_hz(&self) -> f32 {
        if let Row::Num(f) = &self.cw.rows[CARRIER] {
            f.value
        } else {
            crate::source::cw::CW_DEFAULT_CARRIER_HZ
        }
    }
    pub fn set_cw_carrier_hz(&mut self, v: f32) {
        if let Row::Num(f) = &mut self.cw.rows[CARRIER] {
            f.value = v.clamp(f.min, f.max);
        }
    }
    pub fn cw_gap_secs(&self) -> f32 {
        if let Row::Num(f) = &self.cw.rows[GAP] {
            f.value
        } else {
            crate::source::cw::CW_DEFAULT_GAP_SECS
        }
    }
    pub fn cw_noise_amp(&self) -> f32 {
        if let Row::Num(f) = &self.cw.rows[NOISE] {
            f.value
        } else {
            crate::source::cw::CW_DEFAULT_NOISE_AMP
        }
    }
    pub fn cw_msg_repeat(&self) -> usize {
        if let Row::Num(f) = &self.cw.rows[REPEAT] {
            f.value as usize
        } else {
            crate::source::cw::CW_DEFAULT_REPEAT
        }
    }
    /// Returns the active message (Canned or Custom, depending on toggle).
    pub fn cw_message(&self) -> &str {
        if self.cw.msg_is_custom() {
            if let Row::Text(f) = &self.cw.rows[CUSTOM_MSG] {
                &f.value
            } else {
                ""
            }
        } else if let Row::Text(f) = &self.cw.rows[MSG] {
            &f.value
        } else {
            ""
        }
    }
    pub fn cw_msg_mode_str(&self) -> &str {
        if let Row::Toggle(f) = &self.cw.rows[MSG_MODE] {
            f.value_str()
        } else {
            "Canned"
        }
    }
    pub fn cycle_cw_msg_mode(&mut self) {
        if let Row::Toggle(f) = &mut self.cw.rows[MSG_MODE] {
            f.next();
        }
    }
}
