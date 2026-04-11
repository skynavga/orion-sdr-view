// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

/// Frequency viewport: defines which portion of [0, nyquist] is displayed.
///
/// `center_hz` is the displayed center frequency (also the primary marker position).
/// `span_hz` is the total visible bandwidth.
///
/// The displayed range is `[center_hz - span_hz/2, center_hz + span_hz/2]`,
/// clamped to `[0, nyquist]`.
pub struct FreqView {
    pub center_hz: f32,
    pub span_hz: f32,
    pub nyquist: f32,
}

impl FreqView {
    pub fn new(nyquist: f32) -> Self {
        Self {
            center_hz: nyquist / 2.0,
            span_hz: nyquist,
            nyquist,
        }
    }

    /// Low frequency edge of the visible window (clamped to 0).
    pub fn lo(&self) -> f32 {
        (self.center_hz - self.span_hz / 2.0).max(0.0)
    }

    /// High frequency edge of the visible window (clamped to nyquist).
    pub fn hi(&self) -> f32 {
        (self.center_hz + self.span_hz / 2.0).min(self.nyquist)
    }

    /// The actual displayed span (may be narrower than `span_hz` near edges).
    pub fn visible_span(&self) -> f32 {
        self.hi() - self.lo()
    }

    /// Fractional UV position [0.0, 1.0] within the full spectrum for `hz`.
    /// Used for waterfall/persistence texture UV mapping.
    #[allow(dead_code)]
    pub fn hz_to_uv(&self, hz: f32) -> f32 {
        hz / self.nyquist
    }

    /// Convert a frequency in Hz to a normalized X position [0.0, 1.0]
    /// within the visible window. Values outside `[lo, hi]` may be outside [0,1].
    pub fn hz_to_x_norm(&self, hz: f32) -> f32 {
        (hz - self.lo()) / self.visible_span()
    }

    /// Pan by `delta_hz`, keeping the window fully within [0, nyquist].
    ///
    /// Center is clamped to [span/2, nyquist - span/2] so that lo() >= 0
    /// and hi() <= nyquist always hold exactly.  At full zoom (span == nyquist)
    /// the two bounds are equal and pan is a no-op, which is correct.
    pub fn pan(&mut self, delta_hz: f32) {
        let half = self.span_hz / 2.0;
        self.center_hz = (self.center_hz + delta_hz).clamp(half, self.nyquist - half);
    }

    /// Reset to full span (show all frequencies 0..nyquist).
    pub fn reset(&mut self) {
        self.span_hz = self.nyquist;
        self.center_hz = self.nyquist / 2.0;
    }

    /// Snap `hz` to the nearest multiple of `grid` Hz.
    pub fn snap_hz(hz: f32, grid: f32) -> f32 {
        (hz / grid).round() * grid
    }

    /// Current zoom ratio (nyquist / span_hz), rounded to two decimal places.
    pub fn zoom_ratio(&self) -> f32 {
        (self.nyquist / self.span_hz * 100.0).round() / 100.0
    }

    /// Step zoom by `delta` added to the current ratio (e.g. +0.5 or +0.1),
    /// clamped to [1.0, nyquist/1000].
    /// Positive delta = zoom in (narrower span); negative = zoom out (wider span).
    ///
    /// For coarse steps (|delta| >= 0.5), the current ratio is first snapped to
    /// the nearest 0.5 boundary before applying the delta, so repeated coarse
    /// steps always land on exact 0.5 multiples.
    pub fn step_zoom(&mut self, delta: f32) {
        let max_ratio = self.nyquist / 1000.0;
        let current = self.zoom_ratio();
        let base = if delta.abs() >= 0.5 {
            (current / 0.5).round() * 0.5
        } else {
            current
        };
        let new_ratio = (base + delta).clamp(1.0, max_ratio);
        self.span_hz = self.nyquist / new_ratio;
        let half = self.span_hz / 2.0;
        self.center_hz = self.center_hz.clamp(half, self.nyquist - half);
    }

    /// Returns true if the view is showing the full spectrum (no zoom/pan).
    #[allow(dead_code)]
    pub fn is_full(&self) -> bool {
        (self.span_hz - self.nyquist).abs() < 1.0
    }
}

// ── FreqMarker ────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub enum MarkerKind {
    Primary,   // center marker — tracks FreqView.center_hz; shown in cyan
    BracketA,  // user-placed bracket A; shown in yellow
    BracketB,  // user-placed bracket B; shown in orange
}

#[derive(Clone)]
pub struct FreqMarker {
    pub kind: MarkerKind,
    pub hz: f32,
    pub enabled: bool,
}

impl FreqMarker {
    pub fn primary(hz: f32) -> Self {
        Self { kind: MarkerKind::Primary, hz, enabled: true }
    }

    pub fn bracket_a(hz: f32) -> Self {
        Self { kind: MarkerKind::BracketA, hz, enabled: false }
    }

    pub fn bracket_b(hz: f32) -> Self {
        Self { kind: MarkerKind::BracketB, hz, enabled: false }
    }

    pub fn color(&self) -> eframe::egui::Color32 {
        match self.kind {
            MarkerKind::Primary  => eframe::egui::Color32::from_rgb(0, 220, 255),
            MarkerKind::BracketA => eframe::egui::Color32::from_rgb(255, 220, 0),
            MarkerKind::BracketB => eframe::egui::Color32::from_rgb(255, 140, 0),
        }
    }

    pub fn label(&self) -> &'static str {
        match self.kind {
            MarkerKind::Primary  => "▼",
            MarkerKind::BracketA => "A",
            MarkerKind::BracketB => "B",
        }
    }
}
