// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The frequency viewport: the zoom arithmetic the `Zoom` settings row is wired
//! to, and the overscan rule that lets the window hang past the band edges.
//!
//! Two properties carry the zoom half.  That row and the ↑/↓ keys drive the same
//! viewport from opposite directions — the panel pushes the row's value in, the
//! keyboard pushes the viewport's ratio back out — so the pairing holds only if
//! the round trip between them is stable, and the row stops displaying a ratio
//! the viewport refused only if both clamp identically.
//!
//! The pan half is newer.  The window used to be held inside `0..nyquist`, which
//! tied two unrelated things together: the distance the view could travel was
//! exactly the part of the band that was *not* on screen, so widening the span
//! to shrink the signal was the same act as shortening the travel.  A
//! `PAN_AUTO_ZOOM` constant tried to split that trade and could not, because
//! there was no ratio that gave both.  Overscan unties them, and most of what is
//! asserted below is the untying: the travel is now the whole band at every
//! zoom, and nothing outside the band is ever fabricated to fill the gap.

use orion_sdr_view::viewport::{FreqView, MAX_OVERSCAN_FRAC, MIN_SPAN_HZ, PanLimit};

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
fn zoom_never_moves_the_centre() {
    // Replaces `zoom_never_produces_a_window_outside_the_band`, which asserted
    // the clamp this change removes.  What is left is stronger and is what a
    // user actually feels: ↑/↓ magnify about the centre and never slide the
    // view sideways, *including* when the view is panned out past a band edge
    // where the old clamp would have yanked it home mid-zoom.
    //
    // It holds because at `MAX_OVERSCAN_FRAC == 0.5` the centre bound is
    // `[0, nyquist]` regardless of span, so shrinking or growing the span cannot
    // put the centre out of range.
    for &c in &[0.0_f32, 1.0, WB_NYQUIST / 2.0, WB_NYQUIST] {
        let mut v = FreqView::new(WB_NYQUIST);
        v.center_hz = c;
        for &r in &[1.0_f32, 2.0, 9.0, 100.0, 960.0] {
            v.set_zoom_ratio(r);
            assert_eq!(v.center_hz, c, "zoom to {r}x moved the centre from {c}");
            assert_eq!(
                v.visible_span(),
                v.span_hz,
                "the window is always a span wide"
            );
        }
    }
}

#[test]
fn a_zoom_below_full_span_is_floored_not_inverted() {
    // A ratio under 1.0 would mean a window wider than the whole band — empty
    // space on *both* sides at once, which is not a view of anything and which
    // no pan could correct.  Configured values arrive here from YAML, so this is
    // reachable.  (It used to be floored because `lo`/`hi` would clamp such a
    // window asymmetrically and it would read as a pan nobody asked for; those
    // clamps are gone, but the floor is still right.)
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
    assert!(v.zoom_ratio() <= v.max_zoom_ratio());
    assert!(v.lo() >= 0.0 && v.hi() <= NB_NYQUIST);
}

#[test]
fn overscan_does_not_survive_a_source_switch() {
    // The empty space was measured against the old band, so carrying a fraction
    // of it across a Nyquist change would land the new source somewhere neither
    // the user nor the arithmetic chose.  Both re-seating paths — a rate change
    // and an auto-frame — put the window back inside the band.
    let mut v = FreqView::new(WB_NYQUIST);
    v.set_zoom_ratio(4.0);
    v.pan(WB_NYQUIST, PanLimit::Overscan);
    assert!(v.hi() > WB_NYQUIST, "should be panned off the top edge");

    v.set_nyquist(NB_NYQUIST);
    assert!(
        v.lo() >= 0.0 && v.hi() <= NB_NYQUIST,
        "a rate change kept overscan: [{}, {}]",
        v.lo(),
        v.hi()
    );

    let mut v = FreqView::new(WB_NYQUIST);
    v.set_zoom_ratio(4.0);
    v.pan(-WB_NYQUIST, PanLimit::Overscan);
    assert!(v.lo() < 0.0, "should be panned off the bottom edge");
    v.reframe(WB_NYQUIST / 2.0, WB_NYQUIST / 4.0);
    assert!(
        v.lo() >= 0.0 && v.hi() <= WB_NYQUIST,
        "a reframe kept overscan: [{}, {}]",
        v.lo(),
        v.hi()
    );
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
fn full_span_pans() {
    // The branch this change deletes.  At exactly full span the old `pan` was
    // inert *by construction* — the centre clamp range `[span/2, nyquist -
    // span/2]` collapsed to a single point — so the ←/→ handler had to zoom in
    // before it could move, and how far to zoom was an unwinnable trade.  Under
    // overscan the whole band simply slides sideways and the special case is
    // gone from the handler.
    for nyquist in [NB_NYQUIST, WB_NYQUIST] {
        let mut v = FreqView::new(nyquist);
        assert!(v.is_full());
        let before = v.center_hz;
        let ratio = v.zoom_ratio();

        v.pan(nyquist / 12.0, PanLimit::Overscan);
        assert!(
            v.center_hz > before,
            "full span should pan at {nyquist} Hz, stayed at {before}"
        );
        assert_eq!(v.zoom_ratio(), ratio, "and must not have zoomed to do it");
    }
}

#[test]
fn the_pan_range_no_longer_depends_on_the_zoom() {
    // The point of the change.  While the window was held inside the band the
    // reachable centres were `[span/2, nyquist - span/2]`, so the travel was
    // `nyquist - span` and shrank to nothing as the user zoomed out.  Now every
    // zoom reaches every frequency in the band.
    for &r in &[1.0_f32, 1.5, 2.0, 8.0, 100.0] {
        let mut v = FreqView::new(WB_NYQUIST);
        v.set_zoom_ratio(r);
        let step = v.span_hz / 12.0;

        // Walk to each bound the way the keyboard does, in whole steps.
        for _ in 0..2000 {
            v.pan(-step, PanLimit::Overscan);
        }
        let lo_edge = v.center_hz;
        for _ in 0..4000 {
            v.pan(step, PanLimit::Overscan);
        }
        let hi_edge = v.center_hz;

        assert_eq!(lo_edge, 0.0, "at {r}x the low bound was {lo_edge}, not 0");
        assert_eq!(
            hi_edge, WB_NYQUIST,
            "at {r}x the high bound was {hi_edge}, not Nyquist"
        );
        assert_eq!(
            hi_edge - lo_edge,
            WB_NYQUIST,
            "at {r}x the travel was not the whole band"
        );
    }
}

#[test]
fn the_overscan_bound_is_half_a_screen() {
    // Overscan has to stop somewhere, or a stray key-repeat strands the user in
    // empty space with no cue which way home is.  At full deflection the band
    // edge sits exactly at screen centre, so half the pane is empty and half
    // still shows band — which is also why `band_window` is never `None` for any
    // centre `pan` can produce.
    for &r in &[1.0_f32, 4.0, 64.0] {
        let mut v = FreqView::new(WB_NYQUIST);
        v.set_zoom_ratio(r);
        v.pan(-2.0 * WB_NYQUIST, PanLimit::Overscan);

        assert_eq!(v.center_hz, 0.0);
        assert_eq!(v.lo(), -v.span_hz / 2.0);
        assert_eq!(
            -v.lo(),
            MAX_OVERSCAN_FRAC * v.span_hz,
            "at {r}x the empty run past the edge is not the documented fraction"
        );
        let (b_lo, b_hi) = v.band_window().expect("half the pane still holds band");
        assert_eq!((b_lo, b_hi), (0.0, v.span_hz / 2.0));
        let (f_lo, f_hi) = v.band_frac().expect("...and so does half the rect");
        assert_eq!((f_lo, f_hi), (0.5, 1.0));
    }
}

#[test]
fn nothing_outside_the_band_is_ever_fabricated() {
    // The trap this change has to avoid.  `TextureOptions::NEAREST` is
    // `TextureWrapMode::ClampToEdge`, so a UV outside `[0, 1]` does not render
    // as empty — it repeats the edge column across the whole off-band region,
    // producing a smooth extension of the spectrum that is entirely invented.
    // That is worse than a visible artifact because it looks like data, and a
    // screenshot will not show it.  So assert the arithmetic directly: at every
    // centre a pan can reach, the band's sub-rect and the UVs drawn into it stay
    // inside their own bounds.
    for &r in &[1.0_f32, 3.0, 40.0] {
        let mut v = FreqView::new(WB_NYQUIST);
        v.set_zoom_ratio(r);
        for i in 0..=200 {
            v.center_hz = WB_NYQUIST * i as f32 / 200.0;

            let (b_lo, b_hi) = v.band_window().expect("some band is always visible");
            assert!(
                b_lo >= 0.0 && b_hi <= WB_NYQUIST && b_lo < b_hi,
                "band window [{b_lo}, {b_hi}] leaves the band at {r}x centre {}",
                v.center_hz
            );

            let (u_lo, u_hi) = (v.hz_to_uv(b_lo), v.hz_to_uv(b_hi));
            assert!(
                (0.0..=1.0).contains(&u_lo) && (0.0..=1.0).contains(&u_hi) && u_lo < u_hi,
                "UV [{u_lo}, {u_hi}] leaves the texture at {r}x centre {}",
                v.center_hz
            );

            let (f_lo, f_hi) = v.band_frac().expect("...and so does the sub-rect");
            assert!(
                (0.0..=1.0).contains(&f_lo) && (0.0..=1.0).contains(&f_hi) && f_lo < f_hi,
                "sub-rect [{f_lo}, {f_hi}] leaves the pane at {r}x centre {}",
                v.center_hz
            );

            // The sub-rect and the window agree: the band's fractional position
            // is where `hz_to_x_norm` puts its endpoints.
            assert!((f_lo - v.hz_to_x_norm(b_lo)).abs() < 1e-5);
            assert!((f_hi - v.hz_to_x_norm(b_hi)).abs() < 1e-5);
        }
    }
}

#[test]
fn a_view_entirely_off_the_band_has_no_band_window() {
    // Unreachable through `pan`, which stops at half a screen — but `center_hz`
    // is a public field, and the `None` arm is what makes the panes correct if
    // `MAX_OVERSCAN_FRAC` is ever raised past 0.5.  Empty has to be a value the
    // callers must handle, not a degenerate window they might not notice.
    let mut v = FreqView::new(NB_NYQUIST);
    v.set_zoom_ratio(4.0);

    v.center_hz = -NB_NYQUIST;
    assert_eq!(v.band_window(), None, "below the band");
    assert_eq!(v.band_frac(), None);

    v.center_hz = 2.0 * NB_NYQUIST;
    assert_eq!(v.band_window(), None, "above the band");
    assert_eq!(v.band_frac(), None);

    // Exactly touching the edge is still empty: a zero-width window has nothing
    // to draw and would divide by zero on the way to a UV.
    v.center_hz = -v.span_hz / 2.0;
    assert_eq!(v.band_window(), None, "touching 0 from below");
}

#[test]
fn panning_out_and_back_does_not_walk_the_view() {
    // Pan is applied as a delta, so any asymmetry accumulates: a hundred presses
    // out and a hundred back would leave the view somewhere it was never told to
    // go, and the drift would only ever grow.
    let mut v = FreqView::new(WB_NYQUIST);
    v.set_zoom_ratio(3.0);
    let step = v.span_hz / 12.0;
    let start = v.center_hz;

    for _ in 0..100 {
        v.pan(step, PanLimit::Overscan);
        v.pan(-step, PanLimit::Overscan);
    }
    assert!(
        (v.center_hz - start).abs() < 1.0,
        "100 out-and-back pans walked the centre from {start} to {}",
        v.center_hz
    );

    // And the bounds themselves are exact, so parking against one and coming
    // back is repeatable rather than approximately repeatable.
    v.pan(WB_NYQUIST * 10.0, PanLimit::Overscan);
    assert_eq!(v.center_hz, WB_NYQUIST);
    v.pan(-WB_NYQUIST * 10.0, PanLimit::Overscan);
    assert_eq!(v.center_hz, 0.0);
}

#[test]
fn panning_does_not_change_the_zoom() {
    // The row shows a ratio, and pan is a separate control; if pan moved the
    // span the row would go stale on every ←/→ press.
    let mut v = FreqView::new(NB_NYQUIST);
    v.set_zoom_ratio(6.0);
    let span = v.span_hz;
    for d in [-50_000.0, -1000.0, 250.0, 1_000_000.0] {
        v.pan(d, PanLimit::Overscan);
        assert_eq!(v.span_hz, span, "pan by {d} changed the span");
        assert_eq!(v.visible_span(), span, "...or the width of the window");
    }
}

#[test]
fn a_locked_pan_stays_inside_the_band() {
    // The lock writes the viewport centre into the active source's carrier row,
    // and that row clamps to the source's own range.  Panning into empty space
    // with the lock engaged would pin the row while the view kept moving — the
    // lock silently ceasing to track, with nothing on screen to say why.  So
    // `PanLimit::Band` keeps the old bound, and the invariant that makes the `X`
    // panel's `ctr` readable holds: a locked centre is always a real frequency.
    for &r in &[1.0_f32, 2.0, 16.0] {
        let mut v = FreqView::new(WB_NYQUIST);
        v.set_zoom_ratio(r);

        v.pan(-WB_NYQUIST * 2.0, PanLimit::Band);
        assert_eq!(v.center_hz, v.span_hz / 2.0);
        assert_eq!(v.lo(), 0.0, "at {r}x a locked pan left the band");

        v.pan(WB_NYQUIST * 2.0, PanLimit::Band);
        assert_eq!(v.center_hz, WB_NYQUIST - v.span_hz / 2.0);
        assert_eq!(v.hi(), WB_NYQUIST, "at {r}x a locked pan left the band");
    }
}

#[test]
fn a_band_limited_pan_is_still_inert_at_full_span() {
    // The old behaviour, now scoped to the lock: with the whole band on screen
    // there is nowhere inside it to go.  Worth pinning because it is the one
    // case where ←/→ does nothing, and knowing that is deliberate is the
    // difference between a rule and a bug.
    let mut v = FreqView::new(NB_NYQUIST);
    let before = v.center_hz;
    v.pan(NB_NYQUIST / 12.0, PanLimit::Band);
    assert_eq!(v.center_hz, before);
}
