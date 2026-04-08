use eframe::egui;
use crate::config::ViewConfig;

mod field;
mod display;
mod tone;
mod amdsb;
mod psk31;
mod ft8;

use field::{Row, ToggleField, draw_num, draw_toggle};
use display::DisplayRows;
use tone::ToneRows;
use amdsb::AmDsbRows;
use psk31::Psk31Rows;
use ft8::Ft8Rows;

const OVERLAY_W: f32 = 560.0;
const OVERLAY_H: f32 = 446.0;
/// At 13 pt mono, 1 em ≈ 7.8 px.
const EM: f32 = 7.8;
/// Label column: 1 em left margin + max label width (12 chars) + 4 em right margin.
const VAL_X: f32 = EM + 12.0 * EM + 4.0 * EM;  // ≈ 133 px
const ROW_H: f32 = 26.0;

// ── Tab index constants ────────────────────────────────────────────────────
const TAB_SOURCE: usize = 0;
const TAB_DISPLAY: usize = 1;
const N_TABS: usize = 2;
const TAB_NAMES: [&str; N_TABS] = ["Source", "Display"];

// ── Row routing ────────────────────────────────────────────────────────────

/// Routes a visual row position to the correct sub-struct and local index.
#[derive(Clone, Copy)]
enum RowTarget {
    Selector,
    Display(usize),
    Tone(usize),
    AmDsb(usize),
    Psk31(usize),
    Ft8(usize),
}

// ── HandleKeysResult ──────────────────────────────────────────────────────

/// Signals back to ViewApp after a key event in the settings popover.
pub struct HandleKeysResult {
    pub source_switched:     bool,
    pub am_audio_changed:    bool,
    pub wav_load_requested:  bool,
    /// True when the user pressed Enter to commit a new PSK31 message.
    pub psk31_msg_accepted:  bool,
    /// True when the user pressed Enter to commit a new FT8 free-text message.
    pub ft8_text_accepted:   bool,
    /// True when a text field is actively consuming all keyboard input.
    pub text_editing:        bool,
}

// ── SettingsState ──────────────────────────────────────────────────────────

pub struct SettingsState {
    pub visible: bool,
    active_tab: usize,
    focused_row: Option<usize>,

    /// Source selector toggle: "Test Tone" / "AM DSB" / "PSK31".
    source_selector: Row,

    display: DisplayRows,
    tone: ToneRows,
    amdsb: AmDsbRows,
    psk31: Psk31Rows,
    ft8: Ft8Rows,
}

impl SettingsState {
    pub fn new(
        db_min: f32,
        db_max: f32,
        freq_hz: f32,
        noise_amp: f32,
        amp_max: f32,
        ramp_secs: f32,
        pause_secs: f32,
    ) -> Self {
        Self {
            visible: false,
            active_tab: TAB_SOURCE,
            focused_row: None,
            source_selector: Row::Toggle(ToggleField {
                label: "Source",
                options: &["Test Tone", "AM DSB", "PSK31", "FT8"],
                index: 0, default: 0,
            }),
            display: DisplayRows::new(db_min, db_max),
            tone: ToneRows::new(freq_hz, noise_amp, amp_max, ramp_secs, pause_secs),
            amdsb: AmDsbRows::new(),
            psk31: Psk31Rows::new(),
            ft8: Ft8Rows::new(),
        }
    }

    /// Build a `SettingsState` from a loaded `ViewConfig`, patching all
    /// configurable fields and updating `default` so the **R** key resets to
    /// the configured value rather than the hard-coded built-in default.
    pub fn from_config(cfg: &ViewConfig) -> Self {
        let mut s = Self::new(
            cfg.db_min(), cfg.db_max(),
            cfg.freq_hz(), cfg.noise_amp(), cfg.amp_max(),
            cfg.ramp_secs(), cfg.pause_secs(),
        );
        s.display.patch_from_config(cfg);
        s.amdsb.patch_from_config(cfg);
        s.psk31.patch_from_config(cfg);
        s.ft8.patch_from_config(cfg);
        s
    }

    // ── Source-mode helpers ───────────────────────────────────────────────

    fn source_index(&self) -> usize {
        if let Row::Toggle(f) = &self.source_selector { f.index } else { 0 }
    }

    fn source_is_am(&self) -> bool {
        self.source_index() == 1
    }

    fn source_is_psk31(&self) -> bool {
        self.source_index() == 2
    }

    fn source_is_ft8(&self) -> bool {
        self.source_index() == 3
    }

    pub fn source_mode_idx(&self) -> usize {
        self.source_index()
    }

    /// Clear the focused row so arrow keys navigate normally.
    pub fn defocus(&mut self) {
        self.focused_row = None;
    }

    pub fn set_source_mode(&mut self, idx: usize) {
        if let Row::Toggle(f) = &mut self.source_selector {
            f.index = idx.min(f.options.len() - 1);
        }
    }

    // ── Row routing ──────────────────────────────────────────────────────

    fn active_rows(&self) -> Vec<RowTarget> {
        match self.active_tab {
            TAB_DISPLAY => self.display.visible_indices().into_iter()
                .map(RowTarget::Display)
                .collect(),
            _ => {
                let mut v = vec![RowTarget::Selector];
                if self.source_is_am() {
                    v.extend(self.amdsb.visible_indices().into_iter().map(RowTarget::AmDsb));
                } else if self.source_is_psk31() {
                    v.extend(self.psk31.visible_indices().into_iter().map(RowTarget::Psk31));
                } else if self.source_is_ft8() {
                    v.extend(self.ft8.visible_indices().into_iter().map(RowTarget::Ft8));
                } else {
                    v.extend(self.tone.visible_indices().into_iter().map(RowTarget::Tone));
                }
                v
            }
        }
    }

    fn n_visible_rows(&self) -> usize {
        self.active_rows().len()
    }

    /// Get a reference to the Row for a given RowTarget.
    fn row_ref(&self, target: RowTarget) -> &Row {
        match target {
            RowTarget::Selector   => &self.source_selector,
            RowTarget::Display(i) => &self.display.rows[i],
            RowTarget::Tone(i)    => &self.tone.rows[i],
            RowTarget::AmDsb(i)   => &self.amdsb.rows[i],
            RowTarget::Psk31(i)   => &self.psk31.rows[i],
            RowTarget::Ft8(i)     => &self.ft8.rows[i],
        }
    }

    /// Get a mutable reference to the Row for a given RowTarget.
    fn row_mut(&mut self, target: RowTarget) -> &mut Row {
        match target {
            RowTarget::Selector   => &mut self.source_selector,
            RowTarget::Display(i) => &mut self.display.rows[i],
            RowTarget::Tone(i)    => &mut self.tone.rows[i],
            RowTarget::AmDsb(i)   => &mut self.amdsb.rows[i],
            RowTarget::Psk31(i)   => &mut self.psk31.rows[i],
            RowTarget::Ft8(i)     => &mut self.ft8.rows[i],
        }
    }

    // ── Key handling ──────────────────────────────────────────────────────

    pub fn handle_keys(&mut self, ctx: &egui::Context) -> HandleKeysResult {
        let mut result = HandleKeysResult {
            source_switched:    false,
            am_audio_changed:   false,
            wav_load_requested: false,
            psk31_msg_accepted: false,
            ft8_text_accepted:  false,
            text_editing:       false,
        };

        if !self.visible {
            return result;
        }

        // Determine if focused row is a special text field.
        let active = self.active_rows();
        let focused_target = self.focused_row.and_then(|r| active.get(r).copied());

        let wav_row_focused = matches!(focused_target, Some(RowTarget::AmDsb(i)) if i == AmDsbRows::WAV_FILE_IDX)
            && self.amdsb.wav_row_is_active();

        let psk31_custom_focused = matches!(focused_target, Some(RowTarget::Psk31(i)) if i == Psk31Rows::CUSTOM_MSG_IDX);

        let ft8_free_text_focused = matches!(focused_target, Some(RowTarget::Ft8(i)) if i == Ft8Rows::FREE_TEXT_IDX)
            && self.ft8.msg_is_free_text();

        ctx.input(|i| {
            // WAV text field handling
            if wav_row_focused {
                let wav_result = self.amdsb.handle_wav_keys(&i.events);
                if wav_result.load_requested {
                    result.wav_load_requested = true;
                }
                if wav_result.defocus {
                    self.focused_row = None;
                }
                if wav_result.consumed {
                    result.text_editing = true;
                    return;
                }
                // Not consumed — fall through to navigation
            }

            // PSK31 custom message field handling
            if psk31_custom_focused {
                if let Some(RowTarget::Psk31(local_idx)) = focused_target {
                    let msg_result = self.psk31.handle_msg_keys(&i.events, local_idx);
                    if msg_result.msg_accepted {
                        result.psk31_msg_accepted = true;
                    }
                    if msg_result.defocus {
                        self.focused_row = None;
                    }
                    if msg_result.consumed {
                        result.text_editing = true;
                        return;
                    }
                }
                // Not consumed — fall through to navigation
            }

            // FT8 free-text field handling
            if ft8_free_text_focused {
                let text_result = self.ft8.handle_text_keys(&i.events);
                if text_result.accepted {
                    result.ft8_text_accepted = true;
                }
                if text_result.defocus {
                    self.focused_row = None;
                }
                if text_result.consumed {
                    result.text_editing = true;
                    return;
                }
                // Not consumed — fall through to navigation
            }

            // If a text row is no longer focused (user navigated away),
            // discard any in-progress pending edit.
            if self.psk31.pending_msg.is_some() {
                self.psk31.discard_pending();
            }
            if self.amdsb.pending_wav.is_some() {
                self.amdsb.discard_pending();
            }
            if self.ft8.pending_text.is_some() {
                self.ft8.discard_pending();
            }

            // S or Escape: close
            if i.key_pressed(egui::Key::S) {
                self.visible = false;
                self.focused_row = None;
                return;
            }
            if i.key_pressed(egui::Key::Escape) {
                self.visible = false;
                self.focused_row = None;
                return;
            }

            // Tab / Shift-Tab: switch tabs
            if i.key_pressed(egui::Key::Tab) {
                if i.modifiers.shift {
                    self.active_tab = (self.active_tab + N_TABS - 1) % N_TABS;
                } else {
                    self.active_tab = (self.active_tab + 1) % N_TABS;
                }
                self.focused_row = None;
                return;
            }

            let n = self.n_visible_rows();
            let nav_max = n.saturating_sub(1);

            // Up/Down: navigate
            if i.key_pressed(egui::Key::ArrowUp) {
                self.focused_row = Some(match self.focused_row {
                    None => nav_max,
                    Some(r) => r.saturating_sub(1),
                });
                return;
            }
            if i.key_pressed(egui::Key::ArrowDown) {
                self.focused_row = Some(match self.focused_row {
                    None => 0,
                    Some(r) => (r + 1).min(nav_max),
                });
                return;
            }

            // Left/Right: nudge focused field or switch tabs
            if i.key_pressed(egui::Key::ArrowLeft) {
                if let Some(row_vis) = self.focused_row {
                    let prev_source = self.source_is_am();
                    let prev_audio = self.am_audio_idx();
                    let target = self.active_rows()[row_vis];
                    self.row_mut(target).nudge_left();
                    if matches!(target, RowTarget::Selector) && self.source_is_am() != prev_source {
                        result.source_switched = true;
                    }
                    if matches!(target, RowTarget::AmDsb(0)) && self.am_audio_idx() != prev_audio {
                        result.am_audio_changed = true;
                    }
                    // Clamp focused_row to new visible count after any toggle change
                    let new_n = self.n_visible_rows();
                    if let Some(r) = self.focused_row {
                        if r >= new_n { self.focused_row = Some(new_n.saturating_sub(1)); }
                    }
                } else {
                    self.active_tab = (self.active_tab + N_TABS - 1) % N_TABS;
                    self.focused_row = None;
                }
                return;
            }
            if i.key_pressed(egui::Key::ArrowRight) {
                if let Some(row_vis) = self.focused_row {
                    let prev_source = self.source_is_am();
                    let prev_audio = self.am_audio_idx();
                    let target = self.active_rows()[row_vis];
                    self.row_mut(target).nudge_right();
                    if matches!(target, RowTarget::Selector) && self.source_is_am() != prev_source {
                        result.source_switched = true;
                    }
                    if matches!(target, RowTarget::AmDsb(0)) && self.am_audio_idx() != prev_audio {
                        result.am_audio_changed = true;
                    }
                    // Clamp focused_row to new visible count after any toggle change
                    let new_n = self.n_visible_rows();
                    if let Some(r) = self.focused_row {
                        if r >= new_n { self.focused_row = Some(new_n.saturating_sub(1)); }
                    }
                } else {
                    self.active_tab = (self.active_tab + 1) % N_TABS;
                    self.focused_row = None;
                }
                return;
            }

            // R: reset
            if i.key_pressed(egui::Key::R) {
                if let Some(row_vis) = self.focused_row {
                    let target = self.active_rows()[row_vis];
                    self.row_mut(target).reset();
                } else {
                    let targets: Vec<_> = self.active_rows();
                    for target in targets {
                        self.row_mut(target).reset();
                    }
                }
            }
        });

        result
    }

    // ── Drawing ────────────────────────────────────────────────────────────

    pub fn draw(&self, ui: &mut egui::Ui, mono: &egui::FontId) {
        if !self.visible {
            return;
        }

        let screen = ui.ctx().content_rect();
        let rect = egui::Rect::from_center_size(
            screen.center(),
            egui::vec2(OVERLAY_W, OVERLAY_H),
        );

        let painter = ui.painter();

        // Background + border
        painter.rect_filled(
            rect,
            6.0,
            egui::Color32::from_rgba_premultiplied(15, 15, 30, 240),
        );
        painter.rect_stroke(
            rect,
            6.0,
            egui::Stroke::new(1.0, egui::Color32::from_gray(80)),
            egui::StrokeKind::Outside,
        );

        let small = egui::FontId::new(12.0, egui::FontFamily::Monospace);
        let med = egui::FontId::new(13.0, egui::FontFamily::Monospace);
        let mut y = rect.top() + 10.0;

        // ── Tab bar ────────────────────────────────────────────────────────
        let tab_w = (OVERLAY_W - 24.0) / N_TABS as f32;
        for (t, name) in TAB_NAMES.iter().enumerate() {
            let tab_rect = egui::Rect::from_min_size(
                egui::pos2(rect.left() + 12.0 + t as f32 * tab_w, y),
                egui::vec2(tab_w - 4.0, 22.0),
            );
            let active = t == self.active_tab && self.focused_row.is_none();
            let selected = t == self.active_tab;
            let bg = if selected {
                egui::Color32::from_rgb(40, 60, 100)
            } else {
                egui::Color32::from_gray(30)
            };
            painter.rect_filled(tab_rect, 4.0, bg);
            if active {
                painter.rect_stroke(
                    tab_rect,
                    4.0,
                    egui::Stroke::new(1.5, egui::Color32::from_rgb(100, 160, 255)),
                    egui::StrokeKind::Outside,
                );
            }
            painter.text(
                tab_rect.center(),
                egui::Align2::CENTER_CENTER,
                *name,
                mono.clone(),
                if selected { egui::Color32::WHITE } else { egui::Color32::from_gray(140) },
            );
        }
        y += 28.0;

        // Divider
        painter.hline(
            (rect.left() + 8.0)..=(rect.right() - 8.0),
            y,
            egui::Stroke::new(0.5, egui::Color32::from_gray(60)),
        );
        y += 8.0;

        // ── Fields ────────────────────────────────────────────────────────
        let vis_targets = self.active_rows();
        for (vis_row, &target) in vis_targets.iter().enumerate() {
            let focused = self.focused_row == Some(vis_row);

            let row = self.row_ref(target);

            let row_rect = egui::Rect::from_min_size(
                egui::pos2(rect.left() + 8.0, y),
                egui::vec2(OVERLAY_W - 16.0, ROW_H),
            );

            if focused {
                painter.rect_filled(
                    row_rect,
                    3.0,
                    egui::Color32::from_rgba_premultiplied(60, 100, 180, 80),
                );
                painter.rect_stroke(
                    row_rect,
                    3.0,
                    egui::Stroke::new(1.0, egui::Color32::from_rgb(100, 160, 255)),
                    egui::StrokeKind::Outside,
                );
            }

            // Label (left-aligned with 1 em left margin)
            painter.text(
                egui::pos2(rect.left() + EM, y + ROW_H / 2.0),
                egui::Align2::LEFT_CENTER,
                row.label(),
                med.clone(),
                if focused { egui::Color32::WHITE } else { egui::Color32::from_gray(180) },
            );

            // Value
            let val_color = egui::Color32::from_rgb(100, 220, 180);
            let val_x = rect.left() + VAL_X;
            match (target, row) {
                (_, Row::Num(f)) => {
                    draw_num(painter, f, val_x, y, ROW_H, rect.right(), &med, &small, val_color, focused);
                }
                (_, Row::Toggle(f)) => {
                    draw_toggle(painter, f, val_x, y, ROW_H, &med, val_color, focused);
                }
                (RowTarget::Psk31(idx), Row::Text(_)) if idx == Psk31Rows::MSG_IDX => {
                    self.psk31.draw_canned_msg(painter, val_x, y, ROW_H, rect.right(), &med, &small, val_color, focused);
                }
                (RowTarget::Psk31(idx), Row::Text(_)) if idx == Psk31Rows::CUSTOM_MSG_IDX => {
                    self.psk31.draw_custom_msg(painter, val_x, y, ROW_H, rect.right(), &med, &small, val_color, focused);
                }
                (RowTarget::AmDsb(idx), Row::Text(_)) if idx == AmDsbRows::WAV_FILE_IDX => {
                    self.amdsb.draw_wav_field(painter, val_x, y, ROW_H, rect.right(), &med, &small, val_color, focused);
                }
                (RowTarget::Ft8(idx), Row::Text(_)) if idx == Ft8Rows::FREE_TEXT_IDX => {
                    self.ft8.draw_free_text(painter, val_x, y, ROW_H, rect.right(), &med, &small, val_color, focused);
                }
                (RowTarget::Ft8(idx), Row::Text(_)) => {
                    self.ft8.draw_readonly_text(idx, painter, val_x, y, ROW_H, rect.right(), &med, &small, val_color, focused);
                }
                _ => {}
            }

            y += ROW_H;
        }

        // ── Footer ────────────────────────────────────────────────────────
        y = rect.bottom() - 22.0;
        painter.hline(
            (rect.left() + 8.0)..=(rect.right() - 8.0),
            y,
            egui::Stroke::new(0.5, egui::Color32::from_gray(60)),
        );
        y += 6.0;

        let focused_target = self.focused_row.and_then(|r| vis_targets.get(r).copied());

        let wav_focused = matches!(focused_target, Some(RowTarget::AmDsb(i)) if i == AmDsbRows::WAV_FILE_IDX)
            && self.amdsb.wav_row_is_active();

        let psk31_custom_focused = matches!(focused_target, Some(RowTarget::Psk31(i)) if i == Psk31Rows::CUSTOM_MSG_IDX);

        let ft8_free_text_focused = matches!(focused_target, Some(RowTarget::Ft8(i)) if i == Ft8Rows::FREE_TEXT_IDX)
            && self.ft8.msg_is_free_text();

        let hint = if wav_focused && self.amdsb.pending_wav.is_some() {
            "type path   ↵ load   Esc cancel"
        } else if wav_focused {
            "↵ edit path   ↑↓ navigate"
        } else if psk31_custom_focused && self.psk31.pending_msg.is_some() {
            "type message   ↵ accept   Esc cancel"
        } else if psk31_custom_focused {
            "↵ edit message   ↑↓ navigate"
        } else if ft8_free_text_focused && self.ft8.pending_text.is_some() {
            "type message   ↵ accept   Esc cancel"
        } else if ft8_free_text_focused {
            "↵ edit message   ↑↓ navigate"
        } else if self.focused_row.is_some() {
            "↑↓ navigate   ◀▶ adjust   R reset field   Esc deselect"
        } else {
            "↑↓ select field   Tab switch tab   R reset all   Esc close"
        };
        painter.text(
            egui::pos2(rect.left() + 12.0, y),
            egui::Align2::LEFT_TOP,
            hint,
            small,
            egui::Color32::from_gray(110),
        );
    }
}
