// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::config::ViewConfig;
use eframe::egui;

use super::amdsb::{AmDsbRows, AmDsbSettings};
use super::cw::CwRows;
use super::display::{self, DisplayRows};
use super::field::{Row, RowDrawCtx, ToggleField, draw_num, draw_toggle};
use super::ft8::Ft8Rows;
use super::psk31::Psk31Rows;
use super::tone::ToneRows;

const OVERLAY_W: f32 = 560.0;
const OVERLAY_H: f32 = 446.0;
/// At 13 pt mono, 1 em ≈ 7.8 px.
const EM: f32 = 7.8;
/// Label column: 1 em left margin + max label width (12 chars) + 4 em right margin.
const VAL_X: f32 = EM + 12.0 * EM + 4.0 * EM; // ≈ 133 px
const ROW_H: f32 = 26.0;

// ── Tab index constants ────────────────────────────────────────────────────
const TAB_SOURCE: usize = 0;
const TAB_DISPLAY: usize = 1;
const N_TABS: usize = 2;
const TAB_NAMES: [&str; N_TABS] = ["Source", "Display"];

// ── SourceRows trait ───────────────────────────────────────────────────────
//
// Uniform interface implemented by each per-source `*Rows` struct.  The
// settings dispatch in this file calls trait methods directly — there is
// **no** per-source `match` for any operation that has a uniform shape
// across sources.  Adding a new source means: implement this trait, push an
// instance into `SettingsState::sources`.  No edits to `common.rs`.
//
// Methods after `discard_pending` have default impls so sources without
// text fields / hints / special draw logic don't need to override them.

pub(super) trait SourceRows: std::any::Any {
    // ── Required ─────────────────────────────────────────────────────────
    fn rows(&self) -> &[Row];
    fn rows_mut(&mut self) -> &mut [Row];
    fn visible_indices(&self) -> Vec<usize>;
    /// Boilerplate to enable downcasting via `SettingsState::source_as`.
    fn as_any(&self) -> &dyn std::any::Any;
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;

    // ── Optional ─────────────────────────────────────────────────────────

    /// Reset per-source extras (pending edits, edit-mode flags) beyond the
    /// row-level default reset that `Row::reset()` already does.
    fn reset_extras(&mut self) {}

    /// Discard any in-progress pending text edit.
    fn discard_pending(&mut self) {}

    /// Patch row values + defaults from a loaded `ViewConfig`.  Sources that
    /// are not represented in the YAML schema can rely on the default no-op.
    fn patch_from_config(&mut self, _cfg: &ViewConfig) {}

    /// Identifies the source's text-editable row, if `local_idx` points at
    /// one (and the row is currently editable).
    fn focused_text_field(&self, _local_idx: usize) -> Option<TextFieldKind> {
        None
    }

    /// Handle key events when the source's text-editable row is focused.
    fn handle_text_keys(&mut self, _events: &[egui::Event], _local_idx: usize) -> TextOutcome {
        TextOutcome::default()
    }

    /// Render a per-source-special text row.  Returns `true` if rendered;
    /// `false` to fall through to generic Text-row drawing.
    fn draw_text_row(
        &self,
        _ctx: &RowDrawCtx,
        _local_idx: usize,
        _val_x: f32,
        _y: f32,
        _row_h: f32,
        _focused: bool,
    ) -> bool {
        false
    }

    /// Footer hint when one of this source's rows is focused.
    fn footer_hint(&self, _focused_local: Option<usize>) -> Option<&'static str> {
        None
    }
}

// ── Per-source text-field plumbing ─────────────────────────────────────────

/// Identifies a source's currently-focused text-editable row, so the common
/// dispatcher knows which `HandleKeysResult` flag to set on commit and which
/// hint string to show.
#[derive(Clone, Copy)]
pub(super) enum TextFieldKind {
    CwCustomMsg,
    AmDsbWavFile,
    Psk31CustomMsg,
    Ft8FreeText,
}

/// Uniform per-source text-key outcome.
#[derive(Default)]
pub(super) struct TextOutcome {
    /// True if at least one event was consumed (text input in progress).
    pub consumed: bool,
    /// True if the focused row should be deselected (Escape).
    pub defocus: bool,
    /// True if Enter was pressed to commit the pending edit.
    pub committed: bool,
}

// ── Row routing ────────────────────────────────────────────────────────────

/// Routes a visual row position to the correct sub-struct and local index.
/// `ActiveSource(local)` means "the local-th visible row of the source that
/// `source_mode_idx()` selects" — the per-source variant is implicit, since
/// `active_rows()` only ever includes one source's rows at a time.
#[derive(Clone, Copy)]
enum RowTarget {
    Selector,
    Display(usize),
    ActiveSource(usize),
}

#[derive(Clone, Copy)]
enum NudgeDir {
    Left,
    Right,
}

// ── HandleKeysResult ──────────────────────────────────────────────────────

/// Signals back to ViewApp after a key event in the settings popover.
pub struct HandleKeysResult {
    pub source_switched: bool,
    pub am_audio_changed: bool,
    pub wav_load_requested: bool,
    /// True when the user pressed Enter to commit a new CW message.
    pub cw_msg_accepted: bool,
    /// True when the user pressed Enter to commit a new PSK31 message.
    pub psk31_msg_accepted: bool,
    /// True when the user pressed Enter to commit a new FT8 free-text message.
    pub ft8_text_accepted: bool,
    /// True when a text field is actively consuming all keyboard input.
    pub text_editing: bool,
}

impl HandleKeysResult {
    /// Set the appropriate per-source "accepted" / "load_requested" flag
    /// for the given text-field kind.
    fn set_committed(&mut self, kind: TextFieldKind) {
        match kind {
            TextFieldKind::CwCustomMsg => self.cw_msg_accepted = true,
            TextFieldKind::AmDsbWavFile => self.wav_load_requested = true,
            TextFieldKind::Psk31CustomMsg => self.psk31_msg_accepted = true,
            TextFieldKind::Ft8FreeText => self.ft8_text_accepted = true,
        }
    }
}

// ── SettingsState ──────────────────────────────────────────────────────────

pub struct SettingsState {
    pub visible: bool,
    active_tab: usize,
    focused_row: Option<usize>,

    /// Source selector toggle: "Test Tone" / "CW" / "AM DSB" / "PSK31" / "FT8".
    source_selector: Row,

    pub(super) display: DisplayRows,

    /// Per-source row containers, indexed by `SourceMode as usize`.  Adding a
    /// new source means: implement `SourceRows` for `<S>Rows`, add the variant
    /// to `SourceMode`, push `Box::new(<S>Rows::new())` here.  `common.rs`
    /// stays untouched.
    sources: Vec<Box<dyn SourceRows>>,
}

impl SettingsState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db_min: f32,
        db_max: f32,
        spec_freq_delta_hz: f32,
        spec_time_range_secs: f32,
        freq_hz: f32,
        noise_amp: f32,
        amp_max: f32,
        ramp_secs: f32,
        pause_secs: f32,
    ) -> Self {
        // Order matches the SourceMode enum: TestTone, Cw, AmDsb, Psk31, Ft8.
        let sources: Vec<Box<dyn SourceRows>> = vec![
            Box::new(ToneRows::new(
                freq_hz, noise_amp, amp_max, ramp_secs, pause_secs,
            )),
            Box::new(CwRows::new()),
            Box::new(AmDsbRows::new()),
            Box::new(Psk31Rows::new()),
            Box::new(Ft8Rows::new()),
        ];
        Self {
            visible: false,
            active_tab: TAB_SOURCE,
            focused_row: None,
            source_selector: Row::Toggle(ToggleField {
                label: "Source",
                options: &["Test Tone", "CW", "AM DSB", "PSK31", "FT8"],
                index: 0,
                default: 0,
            }),
            display: DisplayRows::new(db_min, db_max, spec_freq_delta_hz, spec_time_range_secs),
            sources,
        }
    }

    /// Build a `SettingsState` from a loaded `ViewConfig`, patching all
    /// configurable fields and updating `default` so the **R** key resets to
    /// the configured value rather than the hard-coded built-in default.
    pub fn from_config(cfg: &ViewConfig) -> Self {
        let mut s = Self::new(
            cfg.db_min(),
            cfg.db_max(),
            cfg.spec_freq_delta_hz(),
            cfg.spec_time_range_secs(),
            cfg.freq_hz(),
            cfg.noise_amp(),
            cfg.amp_max(),
            cfg.ramp_secs(),
            cfg.pause_secs(),
        );
        s.display.patch_from_config(cfg);
        for source in s.sources.iter_mut() {
            source.patch_from_config(cfg);
        }
        s
    }

    /// Borrow source `idx` as its concrete type `T`.  Panics on type mismatch
    /// — always loud, never silent.  Used by per-source typed accessors.
    #[track_caller]
    pub(super) fn source_as<T: 'static>(&self, idx: usize) -> &T {
        self.sources[idx]
            .as_any()
            .downcast_ref::<T>()
            .expect("source type mismatch")
    }

    /// Mutable counterpart of `source_as`.
    #[track_caller]
    pub(super) fn source_as_mut<T: 'static>(&mut self, idx: usize) -> &mut T {
        self.sources[idx]
            .as_any_mut()
            .downcast_mut::<T>()
            .expect("source type mismatch")
    }

    // ── Source-mode helpers ───────────────────────────────────────────────

    fn source_index(&self) -> usize {
        if let Row::Toggle(f) = &self.source_selector {
            f.index
        } else {
            0
        }
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

    /// Borrow the currently-active source's `*Rows` as `&dyn SourceRows`.
    fn active_source(&self) -> &dyn SourceRows {
        self.sources[self.source_index()].as_ref()
    }

    /// Borrow the currently-active source's `*Rows` as `&mut dyn SourceRows`.
    fn active_source_mut(&mut self) -> &mut dyn SourceRows {
        let idx = self.source_index();
        self.sources[idx].as_mut()
    }

    // ── Row routing ──────────────────────────────────────────────────────

    fn active_rows(&self) -> Vec<RowTarget> {
        match self.active_tab {
            TAB_DISPLAY => self
                .display
                .visible_indices()
                .into_iter()
                .map(RowTarget::Display)
                .collect(),
            _ => {
                let mut v = vec![RowTarget::Selector];
                v.extend(
                    self.active_source()
                        .visible_indices()
                        .into_iter()
                        .map(RowTarget::ActiveSource),
                );
                v
            }
        }
    }

    fn n_visible_rows(&self) -> usize {
        self.active_rows().len()
    }

    /// Reset all source-mode settings rows to their defaults.
    /// Called on R-key (outside settings panel) and on source cycling.
    pub fn reset_source_rows(&mut self) {
        for source in self.sources.iter_mut() {
            for row in source.rows_mut() {
                row.reset();
            }
            source.discard_pending();
            source.reset_extras();
        }
    }

    /// Get a reference to the Row for a given RowTarget.
    fn row_ref(&self, target: RowTarget) -> &Row {
        match target {
            RowTarget::Selector => &self.source_selector,
            RowTarget::Display(i) => &self.display.rows[i],
            RowTarget::ActiveSource(i) => &self.active_source().rows()[i],
        }
    }

    /// Get a mutable reference to the Row for a given RowTarget.
    fn row_mut(&mut self, target: RowTarget) -> &mut Row {
        match target {
            RowTarget::Selector => &mut self.source_selector,
            RowTarget::Display(i) => &mut self.display.rows[i],
            RowTarget::ActiveSource(i) => &mut self.active_source_mut().rows_mut()[i],
        }
    }

    // ── Key handling ──────────────────────────────────────────────────────

    pub fn handle_keys(&mut self, ctx: &egui::Context) -> HandleKeysResult {
        let mut result = HandleKeysResult {
            source_switched: false,
            am_audio_changed: false,
            wav_load_requested: false,
            cw_msg_accepted: false,
            psk31_msg_accepted: false,
            ft8_text_accepted: false,
            text_editing: false,
        };

        if !self.visible {
            return result;
        }

        let active = self.active_rows();
        let focused_target = self.focused_row.and_then(|r| active.get(r).copied());

        let focused_text_kind = self.focused_text_field_kind(focused_target);
        let tz_row_focused = matches!(
            focused_target,
            Some(RowTarget::Display(i)) if i == DisplayRows::TIME_ZONE_IDX
        );

        ctx.input(|i| {
            // Per-source text field handling (one branch covers all sources).
            if let (Some(kind), Some(RowTarget::ActiveSource(local_idx))) =
                (focused_text_kind, focused_target)
            {
                let outcome = self
                    .active_source_mut()
                    .handle_text_keys(&i.events, local_idx);
                if outcome.committed {
                    result.set_committed(kind);
                }
                if outcome.defocus {
                    self.focused_row = None;
                }
                if outcome.consumed {
                    result.text_editing = true;
                    return;
                }
                // Not consumed — fall through to navigation.
            }

            // Time zone row: intercept Enter to open/commit the explicit
            // sub-edit.  ←/→ fall through to `nudge_*` which dispatches to
            // the field-level logic (mode cycle vs. ±15 min nudge).
            if tz_row_focused {
                let tz_result = self.display.handle_tz_keys(&i.events);
                if tz_result.consumed {
                    result.text_editing = true;
                    return;
                }
            }

            // If a text row is no longer focused (user navigated away),
            // discard any in-progress pending edit on every source.
            self.discard_all_pending();
            if !tz_row_focused && self.display.tz_is_editing() {
                self.display.discard_tz_pending();
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
                    self.nudge_focused_row(row_vis, NudgeDir::Left, &mut result);
                } else {
                    self.active_tab = (self.active_tab + N_TABS - 1) % N_TABS;
                    self.focused_row = None;
                }
                return;
            }
            if i.key_pressed(egui::Key::ArrowRight) {
                if let Some(row_vis) = self.focused_row {
                    self.nudge_focused_row(row_vis, NudgeDir::Right, &mut result);
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

    /// Resolve the active source's `TextFieldKind` for the focused row, if it
    /// is a per-source text-editable row.
    fn focused_text_field_kind(&self, focused_target: Option<RowTarget>) -> Option<TextFieldKind> {
        let RowTarget::ActiveSource(local_idx) = focused_target? else {
            return None;
        };
        self.active_source().focused_text_field(local_idx)
    }

    /// Discard any in-progress pending text edit on every source.
    fn discard_all_pending(&mut self) {
        for source in self.sources.iter_mut() {
            source.discard_pending();
        }
    }

    /// Nudge the focused row left or right, handling the AM-source-switch and
    /// AM-audio-toggle side-effects and clamping focused_row to the new
    /// visible-row count if a toggle changed visibility.
    fn nudge_focused_row(&mut self, row_vis: usize, dir: NudgeDir, result: &mut HandleKeysResult) {
        let target = self.active_rows()[row_vis];
        let prev_source_idx = self.source_index();
        let prev_audio = self.am_audio_idx();
        match dir {
            NudgeDir::Left => self.row_mut(target).nudge_left(),
            NudgeDir::Right => self.row_mut(target).nudge_right(),
        }
        if matches!(target, RowTarget::Selector) && self.source_index() != prev_source_idx {
            result.source_switched = true;
        }
        // AmDsb's audio toggle is at local index 0.
        if matches!(target, RowTarget::ActiveSource(0))
            && self.source_index() == 2
            && self.am_audio_idx() != prev_audio
        {
            result.am_audio_changed = true;
        }
        // Clamp focused_row to new visible count after any toggle change.
        let new_n = self.n_visible_rows();
        if let Some(r) = self.focused_row
            && r >= new_n
        {
            self.focused_row = Some(new_n.saturating_sub(1));
        }
    }

    // ── Drawing ────────────────────────────────────────────────────────────

    pub fn draw(&self, ui: &mut egui::Ui, mono: &egui::FontId) {
        if !self.visible {
            return;
        }

        let screen = ui.ctx().content_rect();
        let rect = egui::Rect::from_center_size(screen.center(), egui::vec2(OVERLAY_W, OVERLAY_H));

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
                if selected {
                    egui::Color32::WHITE
                } else {
                    egui::Color32::from_gray(140)
                },
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
        let val_color = egui::Color32::from_rgb(100, 220, 180);
        let draw_ctx = RowDrawCtx {
            painter,
            rect_right: rect.right(),
            med: &med,
            small: &small,
            val_color,
        };
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
                if focused {
                    egui::Color32::WHITE
                } else {
                    egui::Color32::from_gray(180)
                },
            );

            // Value
            let val_x = rect.left() + VAL_X;
            match (target, row) {
                (RowTarget::Display(_), Row::TimeZone(f)) => {
                    display::draw_time_zone(&draw_ctx, f, val_x, y, ROW_H, focused);
                }
                (_, Row::Num(f)) => {
                    draw_num(&draw_ctx, f, val_x, y, ROW_H, focused);
                }
                (_, Row::Toggle(f)) => {
                    draw_toggle(&draw_ctx, f, val_x, y, ROW_H, focused);
                }
                (RowTarget::ActiveSource(local), Row::Text(_)) => {
                    self.active_source()
                        .draw_text_row(&draw_ctx, local, val_x, y, ROW_H, focused);
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
        let hint = self.compute_footer_hint(focused_target);
        painter.text(
            egui::pos2(rect.left() + 12.0, y),
            egui::Align2::LEFT_TOP,
            hint,
            small,
            egui::Color32::from_gray(110),
        );
    }

    /// Footer hint for the current focus state.  Per-source hints come from
    /// each source's `SourceRows::footer_hint`; the time-zone row hint and
    /// the generic fallbacks live here.
    fn compute_footer_hint(&self, focused_target: Option<RowTarget>) -> &'static str {
        // Per-source per-row hint, if any.
        if let Some(RowTarget::ActiveSource(local)) = focused_target
            && let Some(h) = self.active_source().footer_hint(Some(local))
        {
            return h;
        }
        // Time-zone row hint (display tab).
        let tz_focused = matches!(
            focused_target,
            Some(RowTarget::Display(i)) if i == DisplayRows::TIME_ZONE_IDX
        );
        if tz_focused {
            let tz_editing = self.display.tz_is_editing();
            let tz_explicit = matches!(
                &self.display.rows[DisplayRows::TIME_ZONE_IDX],
                Row::TimeZone(f) if f.is_explicit()
            );
            if tz_editing {
                return "◀▶ ±15 min   ↵ accept   Esc cancel";
            }
            if tz_explicit {
                return "◀▶ cycle mode   ↵ edit offset   ↑↓ navigate";
            }
        }
        // Generic fallbacks.
        if self.focused_row.is_some() {
            "↑↓ navigate   ◀▶ adjust   R reset field   Esc deselect"
        } else {
            "↑↓ select field   Tab switch tab   R reset all   Esc close"
        }
    }
}
