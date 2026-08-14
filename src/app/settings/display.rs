// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use super::field::{NumField, Row, RowDrawCtx, TimeZoneField, ToggleField};
use crate::config::{TzMode, ViewConfig, format_offset_min};
use eframe::egui;

// ── Row indices (local) ───────────────────────────────────────────────────
const DB_MIN: usize = 0;
const DB_MAX: usize = 1;
const ZOOM: usize = 2;
const SPEC_TIME_RANGE: usize = 3;
const PAN_DIR: usize = 4;
const TIME_ZONE: usize = 5;

/// Pan-direction toggle options, in enum order.  "spectrum" = arrow scrolls the
/// spectrum (panadapter convention; a fixed signal appears to move opposite the
/// arrow).  "signal" = arrow moves the center/signal in the arrow's direction.
const PAN_OPTIONS: &[&str] = &["spectrum", "signal"];

pub(super) struct DisplayRows {
    pub rows: Vec<Row>,
}

impl DisplayRows {
    pub fn new(db_min: f32, db_max: f32, spec_time_range_secs: f32, zoom: f32) -> Self {
        Self {
            rows: vec![
                Row::Num(NumField {
                    label: "dB min",
                    value: db_min,
                    default: -80.0,
                    step: 1.0,
                    min: -160.0,
                    max: -1.0,
                    unit: " dB",
                    coarse: None,
                    max_label: None,
                }),
                Row::Num(NumField {
                    label: "dB max",
                    value: db_max,
                    default: crate::config::Defaults::DB_MAX,
                    step: 1.0,
                    min: -159.0,
                    max: 0.0,
                    unit: " dB",
                    coarse: None,
                    max_label: None,
                }),
                Row::Num(NumField {
                    label: "Zoom",
                    value: zoom,
                    default: crate::config::Defaults::ZOOM,
                    // Matches the keyboard's coarse ↑/↓ step, so the row and the
                    // arrow keys move the viewport by the same amount.
                    step: 0.5,
                    min: 1.0,
                    // Re-derived per source by `set_zoom_max` from the
                    // viewport's own bound — 24x at 48 kHz, 960x for COFDM.  A
                    // fixed bound here would either forbid a legitimate
                    // wideband zoom or let this row show a ratio the viewport
                    // had silently clamped.
                    max: crate::app::SAMPLE_RATE / 2.0 / crate::viewport::MIN_SPAN_HZ,
                    unit: "x",
                    coarse: None,
                    max_label: None,
                }),
                Row::Num(NumField {
                    label: "Spec time",
                    value: spec_time_range_secs,
                    default: crate::config::Defaults::SPEC_TIME_RANGE_SECS,
                    step: 1.0,
                    min: 1.0,
                    max: 120.0,
                    unit: " s",
                    coarse: None,
                    max_label: None,
                }),
                Row::Toggle(ToggleField {
                    label: "Pan",
                    options: PAN_OPTIONS,
                    index: 0,
                    default: 0,
                }),
                Row::TimeZone(TimeZoneField {
                    label: "Time zone",
                    mode: TzMode::Utc,
                    explicit_min: 0,
                    pending_explicit: None,
                    configured_mode: TzMode::Utc,
                    configured_explicit_min: 0,
                }),
            ],
        }
    }

    pub fn patch_from_config(&mut self, cfg: &ViewConfig) {
        self.rows[DB_MIN].patch_num(cfg.db_min());
        self.rows[DB_MAX].patch_num(cfg.db_max());
        // Clamped to the *startup* source's bound, since that is the viewport
        // the configured value opens against.  Configuring a ratio beyond what
        // 48 kHz allows (>24x) is asking for a 1 kHz window on a narrowband
        // source, so clamping it is the honest answer rather than a loss.
        self.rows[ZOOM].patch_num(cfg.zoom());
        self.rows[SPEC_TIME_RANGE].patch_num(cfg.spec_time_range_secs());

        let pan_idx = usize::from(cfg.pan_signal_follows());
        if let Row::Toggle(f) = &mut self.rows[PAN_DIR] {
            f.index = pan_idx;
            f.default = pan_idx;
        }

        let mode = cfg.time_zone_mode();
        if let Row::TimeZone(f) = &mut self.rows[TIME_ZONE] {
            f.mode = mode;
            f.configured_mode = mode;
            if let TzMode::Explicit(m) = mode {
                f.explicit_min = m;
                f.configured_explicit_min = m;
            }
        }
    }

    pub fn visible_indices(&self) -> Vec<usize> {
        (0..self.rows.len()).collect()
    }

    pub(super) const TIME_ZONE_IDX: usize = TIME_ZONE;
}

// ── Custom TZ row drawer ──────────────────────────────────────────────────

/// Draw the Time zone row value.  Renders one of:
/// - `utc`
/// - `local (±HH:MM)` — parenthesized system offset, refreshed each draw
/// - `±HH:MM` — explicit, with a block-cursor suffix while in sub-edit
pub(super) fn draw_time_zone(
    ctx: &RowDrawCtx,
    f: &TimeZoneField,
    x: f32,
    y: f32,
    row_h: f32,
    focused: bool,
) {
    let editing = f.is_editing();
    let val_str = match f.mode {
        TzMode::Utc => "utc".to_owned(),
        TzMode::Local => {
            let live = crate::utils::time::local_utc_offset_min();
            format!("local ({})", format_offset_min_signed(live))
        }
        TzMode::Explicit(_) => {
            let shown = f.pending_explicit.unwrap_or(f.explicit_min);
            if editing {
                format!("{}\u{258b}", format_offset_min_signed(shown)) // ▋ cursor
            } else {
                format_offset_min_signed(shown)
            }
        }
    };

    let text_color = if focused || editing {
        egui::Color32::WHITE
    } else {
        ctx.val_color
    };
    ctx.painter.text(
        egui::pos2(x, y + row_h / 2.0),
        egui::Align2::LEFT_CENTER,
        val_str,
        ctx.med.clone(),
        text_color,
    );

    if focused {
        let hint = if editing {
            "◀ ▶ ±15m  \u{21b5} accept  Esc cancel"
        } else if f.is_explicit() {
            "◀ ▶ mode  \u{21b5} edit"
        } else {
            "◀ ▶ mode"
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

/// Explicit offsets are always shown with an explicit sign — even `+00:00`
/// for clarity — so the sub-edit cursor has something to attach to.
fn format_offset_min_signed(min: i32) -> String {
    if min == 0 {
        return "+00:00".to_owned();
    }
    format_offset_min(min)
}

// ── TZ row key handler (Enter sub-edit) ───────────────────────────────────

pub(super) struct TzKeysResult {
    /// True when user pressed Enter to commit a sub-edit.
    pub accepted: bool,
    /// True if the key event was consumed (don't fall through to navigation).
    pub consumed: bool,
}

impl DisplayRows {
    /// Handle keyboard events when the Time zone row is focused.
    ///
    /// Behavior:
    /// - If not editing: Enter (only in Explicit mode) opens a sub-edit seeded
    ///   from the current system offset.  Falls through otherwise.
    /// - If editing: Enter commits, Escape cancels, other events are consumed
    ///   (so ←/→ reach the field via `nudge_*` in the caller).
    pub fn handle_tz_keys(&mut self, events: &[egui::Event]) -> TzKeysResult {
        let mut result = TzKeysResult {
            accepted: false,
            consumed: false,
        };

        let f = match &mut self.rows[TIME_ZONE] {
            Row::TimeZone(f) => f,
            _ => return result,
        };

        if f.pending_explicit.is_some() {
            result.consumed = true;
            for e in events {
                match e {
                    egui::Event::Key {
                        key: egui::Key::Enter,
                        pressed: true,
                        ..
                    } => {
                        if let Some(pending) = f.pending_explicit.take() {
                            f.explicit_min = pending;
                            f.mode = TzMode::Explicit(pending);
                            result.accepted = true;
                        }
                    }
                    egui::Event::Key {
                        key: egui::Key::Escape,
                        pressed: true,
                        ..
                    } => {
                        f.pending_explicit = None;
                    }
                    egui::Event::Key {
                        key: egui::Key::ArrowRight,
                        pressed: true,
                        ..
                    } => {
                        if let Some(pending) = &mut f.pending_explicit {
                            *pending = (*pending + super::field::TZ_EXPLICIT_STEP).clamp(
                                super::field::TZ_EXPLICIT_MIN,
                                super::field::TZ_EXPLICIT_MAX,
                            );
                        }
                    }
                    egui::Event::Key {
                        key: egui::Key::ArrowLeft,
                        pressed: true,
                        ..
                    } => {
                        if let Some(pending) = &mut f.pending_explicit {
                            *pending = (*pending - super::field::TZ_EXPLICIT_STEP).clamp(
                                super::field::TZ_EXPLICIT_MIN,
                                super::field::TZ_EXPLICIT_MAX,
                            );
                        }
                    }
                    _ => {}
                }
            }
            return result;
        }

        // Not editing — Enter in Explicit mode opens a sub-edit seeded from
        // the current system offset.
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
        if enter_pressed && f.is_explicit() {
            let seed = crate::utils::time::local_utc_offset_min();
            f.pending_explicit = Some(seed);
            result.consumed = true;
        }

        result
    }

    /// Discard any in-progress tz sub-edit.
    pub fn discard_tz_pending(&mut self) {
        if let Row::TimeZone(f) = &mut self.rows[TIME_ZONE] {
            f.pending_explicit = None;
        }
    }

    pub fn tz_is_editing(&self) -> bool {
        matches!(&self.rows[TIME_ZONE], Row::TimeZone(f) if f.pending_explicit.is_some())
    }
}

// ── SettingsState accessors ───────────────────────────────────────────────

impl super::SettingsState {
    pub fn db_min(&self) -> f32 {
        if let Row::Num(f) = &self.display.rows[DB_MIN] {
            f.value
        } else {
            -80.0
        }
    }
    pub fn db_max(&self) -> f32 {
        if let Row::Num(f) = &self.display.rows[DB_MAX] {
            f.value
        } else {
            crate::config::Defaults::DB_MAX
        }
    }
    pub fn set_db_min(&mut self, v: f32) {
        if let Row::Num(f) = &mut self.display.rows[DB_MIN] {
            f.value = v.clamp(f.min, f.max);
        }
    }
    pub fn set_db_max(&mut self, v: f32) {
        if let Row::Num(f) = &mut self.display.rows[DB_MAX] {
            f.value = v.clamp(f.min, f.max);
        }
    }
    pub fn spec_time_range_secs(&self) -> f32 {
        if let Row::Num(f) = &self.display.rows[SPEC_TIME_RANGE] {
            f.value
        } else {
            crate::config::Defaults::SPEC_TIME_RANGE_SECS
        }
    }
    /// Viewport zoom ratio held by the `Zoom` row.
    pub fn zoom_ratio(&self) -> f32 {
        if let Row::Num(f) = &self.display.rows[ZOOM] {
            f.value
        } else {
            crate::config::Defaults::ZOOM
        }
    }
    /// Write the viewport's current zoom into the row.  Value only — `default`
    /// stays the configured startup ratio, so an R-reset on the Display tab
    /// restores what the YAML asked for rather than wherever the user last
    /// zoomed to.
    pub fn set_zoom_ratio(&mut self, v: f32) {
        if let Row::Num(f) = &mut self.display.rows[ZOOM] {
            f.value = v.clamp(f.min, f.max);
        }
    }
    /// Re-derive the `Zoom` row's upper bound after a sample-rate change, and
    /// pull the current value into it.
    ///
    /// Kept equal to `FreqView::max_zoom_ratio` so the row and the ↑/↓ keys
    /// clamp identically; if they diverged, the row would display a ratio the
    /// viewport refused to take.
    pub fn set_zoom_max(&mut self, max: f32) {
        if let Row::Num(f) = &mut self.display.rows[ZOOM] {
            f.max = max.max(f.min);
            f.value = f.value.clamp(f.min, f.max);
            f.default = f.default.clamp(f.min, f.max);
        }
    }
    /// True when pan is in "signal" mode (arrow moves the center/signal in the
    /// arrow's direction); false for "spectrum" (panadapter) mode.
    pub fn pan_signal_follows(&self) -> bool {
        if let Row::Toggle(f) = &self.display.rows[PAN_DIR] {
            f.index == 1
        } else {
            false
        }
    }
    /// Effective UTC offset in minutes for the Time zone row, resolving
    /// `local` against the current system offset.
    pub fn time_zone_offset_min(&self) -> i32 {
        if let Row::TimeZone(f) = &self.display.rows[TIME_ZONE] {
            f.effective_min(crate::utils::time::local_utc_offset_min())
        } else {
            0
        }
    }
}
