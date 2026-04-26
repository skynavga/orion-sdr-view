// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use super::field::{NumField, Row, RowDrawCtx, TextField, ToggleField};
use crate::config::ViewConfig;
use eframe::egui;

// ── Row indices (local) ───────────────────────────────────────────────────
const MODE: usize = 0;
const CARRIER: usize = 1;
const GAP: usize = 2;
const NOISE: usize = 3;
const MSG_MODE: usize = 4;
const MSG: usize = 5;
const CUSTOM_MSG: usize = 6;
const REPEAT: usize = 7;

pub(super) struct Psk31Rows {
    pub rows: Vec<Row>,
    /// In-progress edit of a PSK31 message field.  `Some(s)` while the user
    /// is typing; committed to the row on Enter, discarded on Escape.
    pub pending_msg: Option<String>,
    /// Which local row index is being edited (MSG or CUSTOM_MSG).
    pub editing_msg_row: Option<usize>,
}

impl Psk31Rows {
    pub fn new() -> Self {
        Self {
            rows: vec![
                Row::Toggle(ToggleField {
                    label: "Mode",
                    options: &["BPSK31", "QPSK31"],
                    index: 0,
                    default: 0,
                }),
                Row::Num(NumField {
                    label: "Carrier",
                    value: 12000.0,
                    default: 12000.0,
                    step: 100.0,
                    min: 100.0,
                    max: 22000.0,
                    unit: " Hz",
                }),
                Row::Num(NumField {
                    label: "Gap",
                    value: crate::source::psk31::PSK31_DEFAULT_GAP_SECS,
                    default: crate::source::psk31::PSK31_DEFAULT_GAP_SECS,
                    step: 0.5,
                    min: 0.5,
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
                Row::Toggle(ToggleField {
                    label: "Message",
                    options: &["Canned", "Custom"],
                    index: 0,
                    default: 0,
                }),
                Row::Text(TextField {
                    label: "Text",
                    value: crate::source::psk31::PSK31_DEFAULT_CANNED_TEXT.to_owned(),
                    default_value: crate::source::psk31::PSK31_DEFAULT_CANNED_TEXT.to_owned(),
                    status: None,
                }),
                Row::Text(TextField {
                    label: "Text",
                    value: crate::source::psk31::PSK31_DEFAULT_CUSTOM_TEXT.to_owned(),
                    default_value: crate::source::psk31::PSK31_DEFAULT_CUSTOM_TEXT.to_owned(),
                    status: None,
                }),
                Row::Num(NumField {
                    label: "Repeat",
                    value: crate::source::psk31::PSK31_DEFAULT_REPEAT as f32,
                    default: crate::source::psk31::PSK31_DEFAULT_REPEAT as f32,
                    step: 1.0,
                    min: 1.0,
                    max: 20.0,
                    unit: "×",
                }),
            ],
            pending_msg: None,
            editing_msg_row: None,
        }
    }

    pub fn patch_from_config(&mut self, cfg: &ViewConfig) {
        self.rows[CARRIER].patch_num(cfg.psk31_carrier_hz());
        self.rows[GAP].patch_num(cfg.psk31_gap_secs());
        self.rows[NOISE].patch_num(cfg.psk31_noise_amp());
        self.rows[REPEAT].patch_num(cfg.psk31_msg_repeat() as f32);

        // Patch mode toggle
        let mode_idx = match cfg.psk31_mode() {
            "QPSK31" => 1,
            _ => 0,
        };
        if let Row::Toggle(f) = &mut self.rows[MODE] {
            f.index = mode_idx;
            f.default = mode_idx;
        }

        // Patch canned message text
        if let Row::Text(f) = &mut self.rows[MSG] {
            let msg = cfg.psk31_canned_text().to_owned();
            f.value = msg.clone();
            f.default_value = msg;
        }

        // Patch custom message text
        if let Row::Text(f) = &mut self.rows[CUSTOM_MSG] {
            let msg = cfg.psk31_custom_text().to_owned();
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
        let mut v = vec![MODE, MSG_MODE];
        if self.msg_is_custom() {
            v.push(CUSTOM_MSG);
        } else {
            v.push(MSG);
        }
        v.extend([REPEAT, CARRIER, GAP, NOISE]);
        v
    }

    /// Handle keyboard input when the custom PSK31 message row is focused.
    /// `focused_local_idx` is the local row index (CUSTOM_MSG).
    /// Returns a result indicating what happened.
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
                                crate::source::psk31::PSK31_DEFAULT_CUSTOM_TEXT
                            } else {
                                crate::source::psk31::PSK31_DEFAULT_CANNED_TEXT
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

        // Not editing: Enter starts an edit; printable text also starts editing.
        let edit_target = focused_local_idx;

        // Check for Enter key
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

        // Up/Down/Escape/etc. fall through (consumed = false).
        result
    }

    /// Discard any in-progress pending edit (called when focus moves away).
    pub fn discard_pending(&mut self) {
        self.pending_msg = None;
        self.editing_msg_row = None;
    }

    /// Draw the canned PSK31 message text field (read-only).
    pub fn draw_canned_msg(&self, ctx: &RowDrawCtx, val_x: f32, y: f32, row_h: f32, focused: bool) {
        if let Row::Text(f) = &self.rows[MSG] {
            let max_chars = 36usize;
            let display = if f.value.chars().count() > max_chars {
                let skip = f.value.chars().count() - max_chars;
                format!("…{}", f.value.chars().skip(skip).collect::<String>())
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

    /// Draw the custom PSK31 message text field (editable).
    pub fn draw_custom_msg(&self, ctx: &RowDrawCtx, val_x: f32, y: f32, row_h: f32, focused: bool) {
        if let Row::Text(f) = &self.rows[CUSTOM_MSG] {
            let max_chars = 36usize;
            let (raw_text, editing) = if let Some(pending) = &self.pending_msg {
                (format!("{}\u{258b}", pending), true) // ▋ block cursor
            } else {
                (f.value.clone(), false)
            };
            let display = if raw_text.chars().count() > max_chars {
                let skip = raw_text.chars().count() - max_chars;
                format!("…{}", raw_text.chars().skip(skip).collect::<String>())
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
    /// True when user pressed Escape or Enter — caller should defocus.
    pub defocus: bool,
    /// True if the key event was consumed (don't fall through to navigation).
    pub consumed: bool,
}

// ── SourceRows ─────────────────────────────────────────────────────────────

impl super::common::SourceRows for Psk31Rows {
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

/// Identifies a PSK31 text-editable row, if any.  The custom-message row is
/// always editable when focused; the canned-message row is read-only.
pub(super) fn focused_text_field(
    _rows: &Psk31Rows,
    local_idx: usize,
) -> Option<super::common::TextFieldKind> {
    if local_idx == Psk31Rows::CUSTOM_MSG_IDX {
        Some(super::common::TextFieldKind::Psk31CustomMsg)
    } else {
        None
    }
}

/// Handle keys when the PSK31 custom-message text row is focused.
pub(super) fn handle_text_keys(
    rows: &mut Psk31Rows,
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

/// Render a PSK31 special text row.  Returns `true` if rendered.
pub(super) fn draw_text_row(
    rows: &Psk31Rows,
    ctx: &RowDrawCtx,
    local_idx: usize,
    val_x: f32,
    y: f32,
    row_h: f32,
    focused: bool,
) -> bool {
    if local_idx == Psk31Rows::MSG_IDX {
        rows.draw_canned_msg(ctx, val_x, y, row_h, focused);
        true
    } else if local_idx == Psk31Rows::CUSTOM_MSG_IDX {
        rows.draw_custom_msg(ctx, val_x, y, row_h, focused);
        true
    } else {
        false
    }
}

/// Footer hint for PSK31-focused rows.
pub(super) fn footer_hint(rows: &Psk31Rows, focused_local: Option<usize>) -> Option<&'static str> {
    let local = focused_local?;
    if local == Psk31Rows::CUSTOM_MSG_IDX {
        Some(if rows.pending_msg.is_some() {
            "type message   ↵ accept   Esc cancel"
        } else {
            "↵ edit message   ↑↓ navigate"
        })
    } else {
        None
    }
}

// ── SettingsState accessors ───────────────────────────────────────────────

impl super::SettingsState {
    pub fn psk31_mode_str(&self) -> &str {
        if let Row::Toggle(f) = &self.psk31.rows[MODE] {
            f.value_str()
        } else {
            "BPSK31"
        }
    }
    pub fn psk31_carrier_hz(&self) -> f32 {
        if let Row::Num(f) = &self.psk31.rows[CARRIER] {
            f.value
        } else {
            10000.0
        }
    }
    pub fn set_psk31_carrier_hz(&mut self, v: f32) {
        if let Row::Num(f) = &mut self.psk31.rows[CARRIER] {
            f.value = v.clamp(f.min, f.max);
        }
    }
    pub fn psk31_gap_secs(&self) -> f32 {
        if let Row::Num(f) = &self.psk31.rows[GAP] {
            f.value
        } else {
            crate::source::psk31::PSK31_DEFAULT_GAP_SECS
        }
    }
    pub fn psk31_noise_amp(&self) -> f32 {
        if let Row::Num(f) = &self.psk31.rows[NOISE] {
            f.value
        } else {
            0.05
        }
    }
    /// Returns the active message (Canned or Custom, depending on toggle).
    pub fn psk31_message(&self) -> &str {
        if self.psk31.msg_is_custom() {
            if let Row::Text(f) = &self.psk31.rows[CUSTOM_MSG] {
                &f.value
            } else {
                ""
            }
        } else if let Row::Text(f) = &self.psk31.rows[MSG] {
            &f.value
        } else {
            ""
        }
    }
    pub fn psk31_msg_mode_str(&self) -> &str {
        if let Row::Toggle(f) = &self.psk31.rows[MSG_MODE] {
            f.value_str()
        } else {
            "Canned"
        }
    }
    pub fn cycle_psk31_mode(&mut self) {
        if let Row::Toggle(f) = &mut self.psk31.rows[MODE] {
            f.next();
        }
    }
    pub fn cycle_psk31_msg_mode(&mut self) {
        if let Row::Toggle(f) = &mut self.psk31.rows[MSG_MODE] {
            f.next();
        }
    }
    pub fn psk31_msg_repeat(&self) -> usize {
        if let Row::Num(f) = &self.psk31.rows[REPEAT] {
            f.value as usize
        } else {
            3
        }
    }
}
