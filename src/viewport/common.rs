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

/// How far past a band edge the window may travel, as a fraction of the span.
///
/// **This is what decouples "how much of the screen the signal fills" from "how
/// far the view can pan".**  While the window was held inside the band, the
/// distance it could travel was exactly the part of the band that was *not* on
/// screen, and the step was a fraction of what *was*:
///
/// ```text
/// travel  = nyquist - span
/// step    = span / 12                       (a fraction of the visible span)
/// presses = travel / step = 12 · (r - 1)    (to sweep the whole band)
/// ```
///
/// So widening the span to shrink the signal was the same act as shortening the
/// travel, and no zoom ratio gave both — the trade a since-deleted
/// `PAN_AUTO_ZOOM` constant tried and failed to split.  Letting the window hang
/// past the edges makes the travel `nyquist` at every zoom, and the zoom is then
/// free to be chosen for how the signal should *look*.
///
/// Overscan still has to be *bounded*, or a stray key-repeat strands the user in
/// empty space with no cue which way home is.  At `0.5` the band edge can reach
/// screen centre and no further, so at most half a screen is ever empty and the
/// band is always visible on one side.  `Z` (recentre) and `R` (reset) are the
/// escapes.
///
/// **0.5 also makes the centre bound span-independent** — `[0, nyquist]`
/// whatever the zoom — which is why [`set_zoom_ratio`](FreqView::set_zoom_ratio)
/// can never move the centre.  A different fraction would reintroduce a zoom
/// that tugs a panned-out view back toward the band.
pub const MAX_OVERSCAN_FRAC: f32 = 0.5;

/// How far a pan may take the window past the band edges.
///
/// Named at the call site rather than inferred, because the choice is a policy
/// the caller owns and the two cases are one keystroke apart in the same
/// handler.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PanLimit {
    /// Keep the whole window inside `0..nyquist`.
    ///
    /// What the source lock needs: it writes the viewport centre into the active
    /// source's carrier row, and that row clamps to the source's own range.  Pan
    /// into empty space with the lock engaged and the row would pin at its bound
    /// while the view kept moving — the lock silently ceasing to track, with
    /// nothing on screen to say so.  A lock says "the source follows the view",
    /// and out there is no source to follow.
    Band,
    /// Allow up to [`MAX_OVERSCAN_FRAC`] of a span of empty beyond each edge.
    Overscan,
}

/// Frequency viewport: which slice of `0..nyquist` the panes draw.
///
/// `center_hz` is the displayed centre frequency (also the primary marker
/// position); `span_hz` is the total visible bandwidth.  The window is
/// `[center_hz - span_hz/2, center_hz + span_hz/2]` and **is not clamped to the
/// band** — see [`MAX_OVERSCAN_FRAC`].  The part of it that actually holds band
/// is [`band_window`](Self::band_window), which is `None` when there is none.
pub struct FreqView {
    pub center_hz: f32,
    pub span_hz: f32,
    pub nyquist: f32,
    /// The span that counts as zoom 1x — the width the display is *for*, which
    /// is not always the width it *has*.  See [`set_display_span`].
    ///
    /// [`set_display_span`]: Self::set_display_span
    display_span: f32,
}

impl FreqView {
    pub fn new(nyquist: f32) -> Self {
        Self {
            center_hz: nyquist / 2.0,
            span_hz: nyquist,
            nyquist,
            display_span: nyquist,
        }
    }

    /// The span zoom 1x shows: `nyquist` unless a source has narrowed it.
    pub fn display_span(&self) -> f32 {
        self.display_span
    }

    /// Narrow what zoom 1x means to `hz`, leaving the Nyquist alone.
    ///
    /// **Zoom 1x used to be Nyquist by definition**, and for five of the six
    /// sources it still is — this is a no-op at `hz >= nyquist`.  DVB-T is what
    /// separates the two, and the separation is forced by arithmetic rather than
    /// chosen: its band is a fixed 83.25% of the *waveform's* rate, and the
    /// display rate is an integer multiple of that, so at zoom 1x the band can
    /// fill at most `1.665 / oversample` of the window — 83.25% at the minimum
    /// factor of two, and less at every larger one.  Its six bandwidth modes need
    /// factors from 2 to 12 to reach a common display width, so tying 1x to
    /// Nyquist would rescale the frequency axis on every bandwidth press and
    /// leave the narrow modes as slivers.
    ///
    /// The Nyquist above `display_span` is real spectrum that is simply never
    /// framed — the same headroom a receiver has when its tuner samples wider
    /// than the window it draws.  Everything that *indexes data* still works in
    /// `0..nyquist`: bin mapping, texture UVs, and the pan bound are untouched,
    /// so panning can still reach the headroom and the out-of-band wash still
    /// knows where the band really ends.
    pub fn set_display_span(&mut self, hz: f32) {
        if !hz.is_finite() {
            return;
        }
        self.display_span = hz.clamp(MIN_SPAN_HZ.min(self.nyquist), self.nyquist);
        self.span_hz = self.span_hz.min(self.display_span);
    }

    /// Low frequency edge of the window.  **May be negative** when the view has
    /// been panned past the bottom of the band; see [`band_window`](Self::band_window).
    pub fn lo(&self) -> f32 {
        self.center_hz - self.span_hz / 2.0
    }

    /// High frequency edge of the window.  **May exceed Nyquist** when the view
    /// has been panned past the top of the band.
    pub fn hi(&self) -> f32 {
        self.center_hz + self.span_hz / 2.0
    }

    /// The displayed span, which is now always `span_hz`.
    ///
    /// It used to be `hi - lo` *after* both were clamped, so it shrank near the
    /// band edges.  Nothing shrinks any more: the window keeps its width and
    /// hangs off the end instead.  Kept as a method because it reads better at
    /// the call sites and because `hz_to_x_norm` divides by it — a divisor that
    /// can no longer reach zero, which the clamped form could in principle.
    pub fn visible_span(&self) -> f32 {
        self.span_hz
    }

    /// The part of the window that actually holds band, in Hz, or `None` when
    /// the view has been panned entirely off the end.
    ///
    /// **Every consumer that indexes real data wants this, not
    /// [`lo`](Self::lo)/[`hi`](Self::hi).**  Making the empty case a type rather
    /// than a coincidence is most of the point: the previous clamped `lo`/`hi`
    /// pair silently returned a degenerate or reversed window instead, and a
    /// texture UV derived from it samples with `ClampToEdge`, which repeats the
    /// edge column across the empty region as a smooth, entirely fabricated
    /// extension of the spectrum.  That looks like data.
    pub fn band_window(&self) -> Option<(f32, f32)> {
        let lo = self.lo().max(0.0);
        let hi = self.hi().min(self.nyquist);
        (hi > lo).then_some((lo, hi))
    }

    /// [`band_window`](Self::band_window) as a fraction of the window, for
    /// narrowing a pane rect to the sub-rect that holds band.
    ///
    /// Both values are inside `[0, 1]` by construction, so a texture drawn into
    /// the sub-rect never samples outside itself.
    pub fn band_frac(&self) -> Option<(f32, f32)> {
        let (lo, hi) = self.band_window()?;
        let (w_lo, span) = (self.lo(), self.span_hz);
        Some(((lo - w_lo) / span, (hi - w_lo) / span))
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

    /// The range `center_hz` may occupy under `limit`.
    ///
    /// Under [`PanLimit::Overscan`] and the default [`MAX_OVERSCAN_FRAC`] this is
    /// exactly `[0, nyquist]`, independent of the span — which is what lets zoom
    /// leave the centre alone.
    fn center_bounds(&self, limit: PanLimit) -> (f32, f32) {
        let half = self.span_hz / 2.0;
        let slack = match limit {
            PanLimit::Band => 0.0,
            PanLimit::Overscan => MAX_OVERSCAN_FRAC * self.span_hz,
        };
        (
            half - slack,
            (self.nyquist - half + slack).max(half - slack),
        )
    }

    /// Pan by `delta_hz` under `limit`.
    ///
    /// Unlike the old band-locked pan, this is **never inert**: at full span
    /// [`PanLimit::Overscan`] still slides the whole band sideways, which is why
    /// the `←`/`→` handler no longer needs to zoom in first before it can move.
    pub fn pan(&mut self, delta_hz: f32, limit: PanLimit) {
        let (lo, hi) = self.center_bounds(limit);
        self.center_hz = (self.center_hz + delta_hz).clamp(lo, hi);
    }

    /// Reset to full span — `0..display_span`, which is `0..nyquist` unless a
    /// source has narrowed what 1x means.
    pub fn reset(&mut self) {
        self.span_hz = self.display_span;
        self.center_hz = self.display_span / 2.0;
    }

    /// Change the Nyquist limit and re-validate span/center against the new
    /// range.  Used when the active source's sample rate differs from the
    /// current view (per-source sample rate).
    ///
    /// **Overscan does not survive a source switch.**  The empty space the user
    /// panned into was measured against the old band; carrying a fraction of it
    /// across a Nyquist change would land the new source somewhere neither the
    /// user nor the arithmetic chose.  Re-seating inside the band is the honest
    /// default, and `←` immediately puts it back.
    /// Also **restores zoom 1x to Nyquist**, so a narrowed display span belongs
    /// to the source that asked for it and never outlives it.  A caller with a
    /// preference re-states it with [`set_display_span`](Self::set_display_span)
    /// immediately after; one without gets the historical behaviour by doing
    /// nothing, which is the property that keeps five of the six sources out of
    /// this.
    pub fn set_nyquist(&mut self, nyquist: f32) {
        self.nyquist = nyquist;
        self.display_span = nyquist;
        self.span_hz = self.span_hz.clamp(MIN_SPAN_HZ.min(nyquist), nyquist);
        let (lo, hi) = self.center_bounds(PanLimit::Band);
        self.center_hz = self.center_hz.clamp(lo, hi);
    }

    /// Reframe to an explicit center + span, clamped to the display span.
    /// Used to auto-frame a wideband source on switch.
    ///
    /// [`PanLimit::Band`] for the same reason as [`set_nyquist`](Self::set_nyquist):
    /// auto-framing is the app choosing a view, and it should never choose one
    /// that starts off the end of the band.
    pub fn reframe(&mut self, center_hz: f32, span_hz: f32) {
        self.span_hz = span_hz.clamp(MIN_SPAN_HZ.min(self.display_span), self.display_span);
        let (lo, hi) = self.center_bounds(PanLimit::Band);
        self.center_hz = center_hz.clamp(lo, hi);
    }

    /// Snap `hz` to the nearest multiple of `grid` Hz.
    pub fn snap_hz(hz: f32, grid: f32) -> f32 {
        (hz / grid).round() * grid
    }

    /// Current zoom ratio (`display_span / span_hz`), rounded to two decimal
    /// places.  Against the display span rather than the Nyquist, so 1x means
    /// "the view this source is framed for" — identical for every source that
    /// has not narrowed one.
    pub fn zoom_ratio(&self) -> f32 {
        (self.display_span / self.span_hz * 100.0).round() / 100.0
    }

    /// Largest zoom ratio this viewport allows — the bound both [`step_zoom`]
    /// and [`set_zoom_ratio`] clamp to, and the one the `Zoom` settings row
    /// mirrors so the two cannot disagree.
    ///
    /// [`step_zoom`]: Self::step_zoom
    /// [`set_zoom_ratio`]: Self::set_zoom_ratio
    pub fn max_zoom_ratio(&self) -> f32 {
        (self.display_span / MIN_SPAN_HZ).max(1.0)
    }

    /// Set the zoom ratio directly (1.0 = full span), keeping the center where
    /// it is.  The settings-row and startup-config path; [`step_zoom`] is the
    /// keyboard's.
    ///
    /// [`PanLimit::Overscan`], so zooming while panned out does not yank the
    /// view home.  At the default [`MAX_OVERSCAN_FRAC`] the clamp cannot move
    /// the centre at all — the bound is `[0, nyquist]` at every span — but it is
    /// written as a clamp rather than dropped so that changing the constant
    /// stays coherent.
    ///
    /// [`step_zoom`]: Self::step_zoom
    pub fn set_zoom_ratio(&mut self, ratio: f32) {
        if !ratio.is_finite() {
            return;
        }
        let ratio = ratio.clamp(1.0, self.max_zoom_ratio());
        self.span_hz = self.display_span / ratio;
        let (lo, hi) = self.center_bounds(PanLimit::Overscan);
        self.center_hz = self.center_hz.clamp(lo, hi);
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
        (self.span_hz - self.display_span).abs() < 1.0
    }
}
