// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The frequency viewport: which slice of `0..nyquist` the panes draw.
//!
//! Lives in the library rather than beside the rest of the display code because
//! it is pure arithmetic with no UI dependency — and because that arithmetic is
//! worth testing.  The egui-dependent half (markers, colours) stays in the bin.

/// Narrowest viewport span (Hz) the zoom will produce.
///
/// The *maximum zoom ratio therefore depends on the source*: `nyquist /
/// MIN_SPAN_HZ` is 24x for the 48 kHz narrowband sources and 960x for COFDM at
/// 1.92 MHz.  That is why the `Zoom` settings row re-derives its upper bound
/// whenever the sample rate changes — a fixed bound would either forbid a
/// legitimate wideband zoom or let a narrowband row display a ratio the
/// viewport had silently clamped.
pub const MIN_SPAN_HZ: f32 = 1000.0;

/// Zoom the `←`/`→` handler applies when panning from full span.
///
/// At exactly full span [`FreqView::pan`] is a no-op *by construction*: the
/// centre clamp range `[span/2, nyquist - span/2]` collapses to a single point.
/// So the first arrow press has to zoom in or the key does nothing at all,
/// which reads as broken.
///
/// **How far to zoom is a single trade, because the visible span and the pan
/// range are the same quantity from opposite ends** — [`pan`](FreqView::pan)
/// keeps the window inside the band, so the distance it can travel is exactly
/// the part of the band that is *not* on screen:
///
/// ```text
/// travel  = nyquist - span
/// step    = span / 12                       (a fraction of the visible span)
/// presses = travel / step = 12 · (r - 1)    (to sweep the whole band)
/// ```
///
/// So a wide span shows a small signal and pans barely; a narrow one pans far
/// and fills the screen.  There is no ratio that gives both.
///
/// | r | span of 1.92 MHz | presses to sweep | COFDM 1/4 fills |
/// | --- | --- | --- | --- |
/// | 1.01 | 950 kHz | 1 (clamped) | 25% |
/// | **1.5** | **640 kHz** | **6** | **38%** |
/// | 2.0 | 480 kHz | 12 | 50% |
///
/// 1.5 is the middle of that: six presses to cross the band, and a COFDM band
/// at the 1/4 fraction over about a third of the width rather than half.  2.0
/// was what a coarse `step_zoom(1.0)` happened to land on, which is not a
/// reason for a value.
///
/// Having both would mean letting the window hang past the band edges into
/// empty space, as most panadapters do — a change to `pan`'s clamp, not to this
/// number, and its own piece of work.
pub const PAN_AUTO_ZOOM: f32 = 1.5;

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
    pub fn hz_to_uv(&self, hz: f32) -> f32 {
        hz / self.nyquist
    }

    /// Convert a frequency in Hz to a normalized X position [0.0, 1.0]
    /// within the visible window. Values outside `[lo, hi]` may be outside [0,1].
    pub fn hz_to_x_norm(&self, hz: f32) -> f32 {
        (hz - self.lo()) / self.visible_span()
    }

    /// Zoom to [`PAN_AUTO_ZOOM`] if the viewport is at full span, so that a
    /// subsequent [`pan`](Self::pan) can move.  No-op otherwise, so it cannot
    /// disturb a zoom the user chose.
    ///
    /// Returns whether it changed anything.
    pub fn ensure_pannable(&mut self) -> bool {
        if self.span_hz < self.nyquist {
            return false;
        }
        self.set_zoom_ratio(PAN_AUTO_ZOOM);
        true
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

    /// Change the Nyquist limit and re-validate span/center against the new
    /// range.  Used when the active source's sample rate differs from the
    /// current view (per-source sample rate).
    pub fn set_nyquist(&mut self, nyquist: f32) {
        self.nyquist = nyquist;
        self.span_hz = self.span_hz.clamp(MIN_SPAN_HZ.min(nyquist), nyquist);
        let half = self.span_hz / 2.0;
        self.center_hz = self.center_hz.clamp(half, nyquist - half);
    }

    /// Reframe to an explicit center + span, clamped to the current nyquist.
    /// Used to auto-frame a wideband source on switch.
    pub fn reframe(&mut self, center_hz: f32, span_hz: f32) {
        self.span_hz = span_hz.clamp(MIN_SPAN_HZ.min(self.nyquist), self.nyquist);
        let half = self.span_hz / 2.0;
        self.center_hz = center_hz.clamp(half, self.nyquist - half);
    }

    /// Snap `hz` to the nearest multiple of `grid` Hz.
    pub fn snap_hz(hz: f32, grid: f32) -> f32 {
        (hz / grid).round() * grid
    }

    /// Current zoom ratio (nyquist / span_hz), rounded to two decimal places.
    pub fn zoom_ratio(&self) -> f32 {
        (self.nyquist / self.span_hz * 100.0).round() / 100.0
    }

    /// Largest zoom ratio this viewport allows — the bound both [`step_zoom`]
    /// and [`set_zoom_ratio`] clamp to, and the one the `Zoom` settings row
    /// mirrors so the two cannot disagree.
    ///
    /// [`step_zoom`]: Self::step_zoom
    /// [`set_zoom_ratio`]: Self::set_zoom_ratio
    pub fn max_zoom_ratio(&self) -> f32 {
        (self.nyquist / MIN_SPAN_HZ).max(1.0)
    }

    /// Set the zoom ratio directly (1.0 = full span), keeping the center where
    /// it is.  The settings-row and startup-config path; [`step_zoom`] is the
    /// keyboard's.
    ///
    /// [`step_zoom`]: Self::step_zoom
    pub fn set_zoom_ratio(&mut self, ratio: f32) {
        if !ratio.is_finite() {
            return;
        }
        let ratio = ratio.clamp(1.0, self.max_zoom_ratio());
        self.span_hz = self.nyquist / ratio;
        let half = self.span_hz / 2.0;
        self.center_hz = self.center_hz.clamp(half, self.nyquist - half);
    }

    /// Step zoom by `delta` added to the current ratio (e.g. +0.5 or +0.1),
    /// clamped to [1.0, nyquist/MIN_SPAN_HZ].
    /// Positive delta = zoom in (narrower span); negative = zoom out (wider span).
    ///
    /// For coarse steps (|delta| >= 0.5), the current ratio is first snapped to
    /// the nearest 0.5 boundary before applying the delta, so repeated coarse
    /// steps always land on exact 0.5 multiples.
    pub fn step_zoom(&mut self, delta: f32) {
        let current = self.zoom_ratio();
        let base = if delta.abs() >= 0.5 {
            (current / 0.5).round() * 0.5
        } else {
            current
        };
        self.set_zoom_ratio(base + delta);
    }

    /// Returns true if the view is showing the full spectrum (no zoom/pan).
    pub fn is_full(&self) -> bool {
        (self.span_hz - self.nyquist).abs() < 1.0
    }
}
