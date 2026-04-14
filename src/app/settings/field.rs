// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::config::TzMode;
use eframe::egui;

// ── Field kinds ────────────────────────────────────────────────────────────

/// A single editable numeric field.
pub(super) struct NumField {
    pub label: &'static str,
    pub value: f32,
    pub default: f32,
    pub step: f32,
    pub min: f32,
    pub max: f32,
    pub unit: &'static str,
}

impl NumField {
    pub fn nudge(&mut self, delta: f32) {
        self.value = (self.value + delta * self.step).clamp(self.min, self.max);
    }
    pub fn reset(&mut self) {
        self.value = self.default;
    }
}

/// A discrete toggle field (cycles through a fixed list of string labels).
pub(super) struct ToggleField {
    pub label: &'static str,
    pub options: &'static [&'static str],
    pub index: usize,
    pub default: usize,
}

impl ToggleField {
    pub fn next(&mut self) {
        self.index = (self.index + 1) % self.options.len();
    }
    pub fn prev(&mut self) {
        self.index = (self.index + self.options.len() - 1) % self.options.len();
    }
    pub fn reset(&mut self) {
        self.index = self.default;
    }
    pub fn value_str(&self) -> &'static str {
        self.options[self.index]
    }
}

/// A text-edit field (e.g. file path or PSK31 message).
pub(super) struct TextField {
    pub label: &'static str,
    pub value: String,
    /// Default value restored on R-reset. Empty string for WAV path.
    pub default_value: String,
    /// None = not yet tried; Some(true) = last load ok; Some(false) = last load failed.
    pub status: Option<bool>,
}

impl TextField {
    pub fn reset(&mut self) {
        self.value = self.default_value.clone();
        self.status = None;
    }
}

/// Minimum/maximum explicit offset in minutes (matches the supported display
/// range for time-zone configuration).
pub(super) const TZ_EXPLICIT_MIN: i32 = -12 * 60;
pub(super) const TZ_EXPLICIT_MAX: i32 = 14 * 60;
/// Step size for ←/→ nudges inside the explicit sub-edit (15 minutes).
pub(super) const TZ_EXPLICIT_STEP: i32 = 15;

/// A three-mode time-zone field: cycles through Utc → Local → Explicit(±HH:MM).
///
/// In Explicit mode, Enter opens a sub-edit that re-seeds the value from the
/// current system offset and lets the user nudge it ±15 min with ←/→; Enter
/// commits, Esc cancels.
pub(super) struct TimeZoneField {
    pub label: &'static str,
    pub mode: TzMode,
    /// Committed explicit offset in minutes (used only when `mode == Explicit`).
    pub explicit_min: i32,
    /// In-progress edit of the explicit offset.  `Some(_)` while the user is
    /// in the sub-edit; committed to `explicit_min` on Enter, discarded on Esc.
    pub pending_explicit: Option<i32>,
    /// Configured mode (from YAML / defaults), restored by R-reset.
    pub configured_mode: TzMode,
    /// Configured explicit offset — the fallback value used when R-reset
    /// restores Explicit mode without a configured explicit value.
    pub configured_explicit_min: i32,
}

impl TimeZoneField {
    /// Cycle forward: Utc → Local → Explicit → Utc.
    pub fn next(&mut self) {
        self.pending_explicit = None;
        self.mode = match self.mode {
            TzMode::Utc => TzMode::Local,
            TzMode::Local => TzMode::Explicit(self.explicit_min),
            TzMode::Explicit(_) => TzMode::Utc,
        };
    }
    /// Cycle backward: Utc → Explicit → Local → Utc.
    pub fn prev(&mut self) {
        self.pending_explicit = None;
        self.mode = match self.mode {
            TzMode::Utc => TzMode::Explicit(self.explicit_min),
            TzMode::Explicit(_) => TzMode::Local,
            TzMode::Local => TzMode::Utc,
        };
    }
    pub fn reset(&mut self) {
        self.pending_explicit = None;
        self.mode = self.configured_mode;
        if let TzMode::Explicit(m) = self.configured_mode {
            self.explicit_min = m;
        } else {
            self.explicit_min = self.configured_explicit_min;
        }
    }
    /// Resolve the effective offset in minutes, using `local_now` to resolve
    /// `TzMode::Local`.  Uses the pending sub-edit value if present.
    pub fn effective_min(&self, local_now: i32) -> i32 {
        match self.mode {
            TzMode::Utc => 0,
            TzMode::Local => local_now,
            TzMode::Explicit(_) => self.pending_explicit.unwrap_or(self.explicit_min),
        }
    }
    pub fn is_explicit(&self) -> bool {
        matches!(self.mode, TzMode::Explicit(_))
    }
    pub fn is_editing(&self) -> bool {
        self.pending_explicit.is_some()
    }
}

// ── Row enum — unifies the field kinds ─────────────────────────────────────

pub(super) enum Row {
    Num(NumField),
    Toggle(ToggleField),
    Text(TextField),
    TimeZone(TimeZoneField),
}

impl Row {
    pub fn label(&self) -> &str {
        match self {
            Row::Num(f) => f.label,
            Row::Toggle(f) => f.label,
            Row::Text(f) => f.label,
            Row::TimeZone(f) => f.label,
        }
    }
    pub fn nudge_right(&mut self) {
        match self {
            Row::Num(f) => f.nudge(1.0),
            Row::Toggle(f) => f.next(),
            Row::Text(_) => {}
            Row::TimeZone(f) => {
                if let Some(pending) = &mut f.pending_explicit {
                    *pending =
                        (*pending + TZ_EXPLICIT_STEP).clamp(TZ_EXPLICIT_MIN, TZ_EXPLICIT_MAX);
                } else {
                    f.next();
                }
            }
        }
    }
    pub fn nudge_left(&mut self) {
        match self {
            Row::Num(f) => f.nudge(-1.0),
            Row::Toggle(f) => f.prev(),
            Row::Text(_) => {}
            Row::TimeZone(f) => {
                if let Some(pending) = &mut f.pending_explicit {
                    *pending =
                        (*pending - TZ_EXPLICIT_STEP).clamp(TZ_EXPLICIT_MIN, TZ_EXPLICIT_MAX);
                } else {
                    f.prev();
                }
            }
        }
    }
    pub fn reset(&mut self) {
        match self {
            Row::Num(f) => f.reset(),
            Row::Toggle(f) => f.reset(),
            Row::Text(f) => f.reset(),
            Row::TimeZone(f) => f.reset(),
        }
    }
    /// Patch a numeric row: clamp `v` to [min, max] and set both value and default.
    pub fn patch_num(&mut self, v: f32) {
        if let Row::Num(f) = self {
            let clamped = v.clamp(f.min, f.max);
            f.value = clamped;
            f.default = clamped;
        }
    }
}

// ── Drawing helpers ────────────────────────────────────────────────────────

/// Per-row drawing context.  Bundles the painter, geometry, fonts, and default
/// value color used by every settings-row draw helper.  Grouping these into
/// one parameter keeps helpers' arg lists tractable (they vary per row only in
/// `val_x`, `y`, `row_h`, `focused`).
pub(super) struct RowDrawCtx<'a> {
    pub painter: &'a egui::Painter,
    pub rect_right: f32,
    pub med: &'a egui::FontId,
    pub small: &'a egui::FontId,
    pub val_color: egui::Color32,
}

/// Draw a numeric row value.
pub(super) fn draw_num(ctx: &RowDrawCtx, f: &NumField, x: f32, y: f32, row_h: f32, focused: bool) {
    let val_str = if f.step < 0.1 {
        format!("{:.2}{}", f.value, f.unit)
    } else if f.step < 1.0 {
        format!("{:.1}{}", f.value, f.unit)
    } else {
        format!("{:.0}{}", f.value, f.unit)
    };
    ctx.painter.text(
        egui::pos2(x, y + row_h / 2.0),
        egui::Align2::LEFT_CENTER,
        val_str,
        ctx.med.clone(),
        ctx.val_color,
    );
    if focused {
        ctx.painter.text(
            egui::pos2(ctx.rect_right - 14.0, y + row_h / 2.0),
            egui::Align2::RIGHT_CENTER,
            "◀ ▶",
            ctx.small.clone(),
            egui::Color32::from_gray(140),
        );
    }
}

/// Draw a toggle row value.
pub(super) fn draw_toggle(
    ctx: &RowDrawCtx,
    f: &ToggleField,
    x: f32,
    y: f32,
    row_h: f32,
    focused: bool,
) {
    let val_str = format!("◀ {} ▶", f.value_str());
    ctx.painter.text(
        egui::pos2(x, y + row_h / 2.0),
        egui::Align2::LEFT_CENTER,
        val_str,
        ctx.med.clone(),
        if focused {
            egui::Color32::WHITE
        } else {
            ctx.val_color
        },
    );
}
