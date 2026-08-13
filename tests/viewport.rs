// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The frequency viewport, and specifically the zoom arithmetic the `Zoom`
//! settings row is wired to.
//!
//! That row and the ↑/↓ keys drive the same viewport from opposite directions:
//! the panel pushes the row's value in, and the keyboard pushes the viewport's
//! ratio back out.  They are mutually exclusive per frame — the settings overlay
//! consumes the arrow keys while it is open — but only if the round trip between
//! them is stable does the pairing hold, and only if both clamp identically does
//! the row stop displaying a ratio the viewport refused.  Those are the two
//! properties here.

use orion_sdr_view::viewport::{FreqView, MIN_SPAN_HZ};

/// The narrowband sources' Nyquist (48 kHz sample rate).
const NB_NYQUIST: f32 = 24_000.0;
/// COFDM's Nyquist at its default 1.92 MHz.
const WB_NYQUIST: f32 = 960_000.0;

#[test]
fn a_fresh_viewport_is_full_span() {
    let v = FreqView::new(NB_NYQUIST);
    assert_eq!(v.span_hz, NB_NYQUIST);
    assert_eq!(v.center_hz, NB_NYQUIST / 2.0);
    assert_eq!(v.zoom_ratio(), 1.0);
    assert!(v.is_full());
}

#[test]
fn the_zoom_round_trip_is_stable() {
    // The property the row↔viewport sync rests on.  `zoom_ratio` rounds to two
    // decimals and `set_zoom_ratio` divides by it, so a value that drifted on
    // each pass would walk the viewport a little further every frame the panel
    // is open — a slow zoom nobody asked for.
    for nyquist in [NB_NYQUIST, WB_NYQUIST] {
        let mut v = FreqView::new(nyquist);
        for &r in &[1.0_f32, 1.5, 2.0, 4.0, 7.5, 12.0, 23.99] {
            v.set_zoom_ratio(r);
            let once = v.zoom_ratio();
            v.set_zoom_ratio(once);
            let twice = v.zoom_ratio();
            assert_eq!(once, twice, "ratio {r} at {nyquist} Hz drifted");
        }
    }
}

#[test]
fn the_zoom_bound_is_per_source_and_the_two_paths_agree() {
    // The `Zoom` row mirrors `max_zoom_ratio`, so if the row and the keyboard
    // clamped differently the panel could display a ratio the viewport had
    // silently refused.  24x at 48 kHz, 960x for COFDM — the same bound both
    // ways.
    let nb = FreqView::new(NB_NYQUIST);
    let wb = FreqView::new(WB_NYQUIST);
    assert_eq!(nb.max_zoom_ratio(), NB_NYQUIST / MIN_SPAN_HZ);
    assert_eq!(wb.max_zoom_ratio(), WB_NYQUIST / MIN_SPAN_HZ);
    assert_eq!(nb.max_zoom_ratio(), 24.0);
    assert_eq!(wb.max_zoom_ratio(), 960.0);

    // Both entry points land on that bound rather than past it.
    let mut a = FreqView::new(NB_NYQUIST);
    a.set_zoom_ratio(1000.0);
    let mut b = FreqView::new(NB_NYQUIST);
    for _ in 0..200 {
        b.step_zoom(0.5);
    }
    assert_eq!(a.zoom_ratio(), b.zoom_ratio());
    assert_eq!(a.span_hz, b.span_hz);
    assert!(a.span_hz >= MIN_SPAN_HZ);
}

#[test]
fn zoom_never_produces_a_window_outside_the_band() {
    // Every span/center pair has to keep `lo >= 0` and `hi <= nyquist` exactly,
    // since the spectrum, waterfall and spectrogram all index bins off them.
    let mut v = FreqView::new(WB_NYQUIST);
    for &r in &[1.0_f32, 2.0, 9.0, 100.0, 960.0] {
        for &c in &[0.0_f32, 1.0, WB_NYQUIST / 2.0, WB_NYQUIST] {
            v.center_hz = c;
            v.set_zoom_ratio(r);
            assert!(v.lo() >= 0.0, "lo {} at ratio {r} centre {c}", v.lo());
            assert!(
                v.hi() <= WB_NYQUIST + 0.5,
                "hi {} at ratio {r} centre {c}",
                v.hi()
            );
            assert!(v.visible_span() > 0.0);
        }
    }
}

#[test]
fn a_zoom_below_full_span_is_floored_not_inverted() {
    // A ratio under 1.0 would mean a window wider than the band, which `lo`/`hi`
    // would then clamp asymmetrically — a viewport that looks panned when it is
    // not.  Configured values arrive here from YAML, so this is reachable.
    let mut v = FreqView::new(NB_NYQUIST);
    v.set_zoom_ratio(0.25);
    assert_eq!(v.zoom_ratio(), 1.0);
    assert_eq!(v.span_hz, NB_NYQUIST);
    v.set_zoom_ratio(-3.0);
    assert_eq!(v.zoom_ratio(), 1.0);
}

#[test]
fn a_non_finite_zoom_leaves_the_viewport_alone() {
    // `nyquist / NaN` is NaN, and a NaN span poisons every derived pixel
    // position for the rest of the session.  Ignoring the request is the only
    // recovery, since there is nothing sensible to clamp to.
    let mut v = FreqView::new(NB_NYQUIST);
    v.set_zoom_ratio(4.0);
    let (span, center) = (v.span_hz, v.center_hz);
    v.set_zoom_ratio(f32::NAN);
    assert_eq!(v.span_hz, span);
    assert_eq!(v.center_hz, center);
    v.set_zoom_ratio(f32::INFINITY);
    assert_eq!(v.span_hz, span);
}

#[test]
fn a_sample_rate_change_re_validates_the_window() {
    // What `apply_source_sample_rate` relies on when a source switch (or a
    // configured `fs_hz`) moves Nyquist under a zoomed-in viewport.
    let mut v = FreqView::new(WB_NYQUIST);
    v.set_zoom_ratio(500.0); // 1920 Hz span, legal at 960 kHz Nyquist
    v.set_nyquist(NB_NYQUIST); // ...and still legal at 24 kHz
    assert!(v.span_hz >= MIN_SPAN_HZ && v.span_hz <= NB_NYQUIST);
    assert!(v.lo() >= 0.0 && v.hi() <= NB_NYQUIST);
    assert!(v.zoom_ratio() <= v.max_zoom_ratio());
}

#[test]
fn reframing_a_wideband_source_yields_a_representable_ratio() {
    // Precedence step 2: a switch to COFDM reframes to its full-Nyquist
    // preference, and the `Zoom` row is then written from `zoom_ratio()`.  If a
    // reframe could produce a ratio outside the row's range, the row would
    // clamp and the next panel-open would zoom the viewport unasked.
    let mut v = FreqView::new(WB_NYQUIST);
    v.set_zoom_ratio(8.0); // as if the user had zoomed in on a narrowband source
    v.reframe(WB_NYQUIST / 2.0, WB_NYQUIST);
    let r = v.zoom_ratio();
    assert_eq!(r, 1.0, "full-span reframe should read as 1.0x");
    assert!((1.0..=v.max_zoom_ratio()).contains(&r));
    assert_eq!(v.center_hz, WB_NYQUIST / 2.0);
}

#[test]
fn panning_does_not_change_the_zoom() {
    // The row shows a ratio, and pan is a separate control; if pan moved the
    // span the row would go stale on every ←/→ press.
    let mut v = FreqView::new(NB_NYQUIST);
    v.set_zoom_ratio(6.0);
    let span = v.span_hz;
    for d in [-50_000.0, -1000.0, 250.0, 1_000_000.0] {
        v.pan(d);
        assert_eq!(v.span_hz, span, "pan by {d} changed the span");
        assert!(v.lo() >= 0.0 && v.hi() <= NB_NYQUIST);
    }
}
