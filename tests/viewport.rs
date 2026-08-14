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

use orion_sdr_view::viewport::{FreqView, MIN_SPAN_HZ, PAN_AUTO_ZOOM};

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
fn the_auto_zoom_unlocks_panning_and_no_more() {
    // At full span `pan` cannot move: the centre clamp range collapses to a
    // point.  The ←/→ handler nudges off full span first, and the *size* of that
    // nudge is the whole question — it used to be a coarse `step_zoom(1.0)`,
    // landing on 2.0x, which put a COFDM band at the 1/4 fraction across half
    // the visible span before the user had asked for any magnification.
    for nyquist in [NB_NYQUIST, WB_NYQUIST] {
        let mut v = FreqView::new(nyquist);
        assert!(v.is_full());

        let before = v.center_hz;
        v.pan(nyquist / 12.0);
        assert_eq!(v.center_hz, before, "full span must not pan at all");

        assert!(v.ensure_pannable(), "should have moved off full span");
        assert_eq!(v.zoom_ratio(), PAN_AUTO_ZOOM);
        v.pan(nyquist / 12.0);
        assert!(
            v.center_hz > before,
            "the viewport should pan once off full span"
        );
        assert!(v.lo() >= 0.0 && v.hi() <= nyquist);
    }
}

#[test]
fn the_auto_zoom_leaves_a_chosen_zoom_alone() {
    // It fires on every ←/→ press, so it has to be a no-op once the user has
    // zoomed: silently re-zooming mid-pan would fight the ↑/↓ keys.
    let mut v = FreqView::new(WB_NYQUIST);
    v.set_zoom_ratio(8.0);
    assert!(!v.ensure_pannable());
    assert_eq!(v.zoom_ratio(), 8.0);

    // Including at a ratio *below* the auto-zoom's, which the `Zoom` row can
    // produce: 1.0 is the row's minimum, and anything above it already pans.
    v.set_zoom_ratio(1.005);
    let ratio = v.zoom_ratio();
    assert!(!v.ensure_pannable(), "already off full span");
    assert_eq!(v.zoom_ratio(), ratio);
}

#[test]
fn the_pan_range_is_exactly_what_is_off_screen() {
    // The identity that makes the auto-zoom a single trade rather than a free
    // choice.  `pan` keeps the window inside the band, so the distance it can
    // travel is precisely the part of the band not currently visible — and the
    // step is a fraction of the *visible* span, so:
    //
    //     presses to sweep = travel / step = (N - span) / (span/12) = 12(r - 1)
    //
    // Which is why no ratio gives both a small signal and a long pan: widening
    // the span to shrink the signal is the same act as shortening the travel.
    for &r in &[1.5_f32, 2.0, 4.0, 10.0] {
        let mut v = FreqView::new(WB_NYQUIST);
        v.set_zoom_ratio(r);
        let step = v.span_hz / 12.0;

        // Walk from one clamp edge to the other, counting presses.
        v.center_hz = 0.0;
        v.pan(0.0);
        let lo_edge = v.center_hz;
        let mut presses = 0;
        while v.center_hz < WB_NYQUIST - v.span_hz / 2.0 - 0.5 && presses < 1000 {
            v.pan(step);
            presses += 1;
        }
        assert_eq!(
            presses,
            (12.0 * (r - 1.0)).round() as i32,
            "at {r}x the band should take 12(r-1) presses to sweep"
        );
        let travel = v.center_hz - lo_edge;
        assert!(
            (travel - (WB_NYQUIST - v.span_hz)).abs() < 1.0,
            "at {r}x the travel was {travel} Hz, not the {} Hz off screen",
            WB_NYQUIST - v.span_hz
        );
    }
}

#[test]
fn the_auto_zoom_balances_the_signal_against_the_pan_range() {
    // Both halves of the trade, pinned at the chosen ratio.  COFDM's 1/4
    // bandwidth fraction is ~240 kHz of a 1.92 MHz band; the old auto-zoom was a
    // coarse `step_zoom(1.0)` landing on 2.0x, which put that band across half
    // the visible span the instant an arrow was pressed.
    const COFDM_OCCUPIED_HZ: f32 = 240_000.0;
    let mut v = FreqView::new(WB_NYQUIST);
    v.ensure_pannable();

    let fill = COFDM_OCCUPIED_HZ / v.visible_span();
    let presses = 12.0 * (PAN_AUTO_ZOOM - 1.0);
    assert!(
        fill < 0.45,
        "the band fills {:.0}% of the span, no better than the old 2.0x",
        fill * 100.0
    );
    assert!(
        presses >= 4.0,
        "only {presses:.0} presses to sweep the band — too little to be a pan"
    );

    let mut old = FreqView::new(WB_NYQUIST);
    old.step_zoom(1.0); // what the handler used to do
    assert!(
        v.visible_span() > old.visible_span(),
        "the whole point is to show more band than the old 2.0x did"
    );
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
