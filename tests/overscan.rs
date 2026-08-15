// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Panning past the band edge, checked in pixels.
//!
//! `tests/viewport.rs` proves the arithmetic never asks for a texture
//! coordinate outside `[0, 1]`.  This file proves the panes honour it, which is
//! a separate question and the one with teeth: `TextureOptions::NEAREST` is
//! `TextureWrapMode::ClampToEdge`, so getting it wrong does not raise an error
//! or leave a gap — it repeats the band's edge column across the whole empty
//! region as a smooth continuation of the spectrum.  **The failure looks like
//! data**, which is why it is asserted here rather than left to a visual pass.
//!
//! Rasterized on the CPU (`src/capture/raster.rs`), so this runs with no GPU and
//! against the same pixels a headless `still` would write.

#![cfg(feature = "gui")]

mod common;

use common::harness::Harness;
use orion_sdr_view::app::SourceMode;
use orion_sdr_view::capture::rasterize;

/// Frame size for the rasterized checks.  Small enough to be quick, large
/// enough that each pane is tens of rows tall.
const SIZE: (f32, f32) = (640.0, 480.0);

/// A colour that could only have come from a signal: the thermal palettes and
/// the spectrum trace are saturated, while everything drawn over the empty
/// region — the wash, the grid, the band edge, axis text — is grey or near
/// black.
fn is_vivid(px: &[u8]) -> bool {
    let (r, g, b) = (px[0] as i32, px[1] as i32, px[2] as i32);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    max >= 120 && (max - min) >= 60
}

/// Drive COFDM to a steady display, then pan `presses` times to the left.
fn panned_left(presses: usize) -> Harness {
    let mut h = Harness::with_defaults();
    h.select_source(SourceMode::Cofdm);
    // Let the waterfall, persistence and spectrogram fill with real signal, so
    // there is something for a smear to smear.
    for _ in 0..240 {
        h.idle(1);
    }
    h.key_n(egui::Key::ArrowUp, 6); // zoom in, so the band edge carries signal
    h.key_n(egui::Key::ArrowLeft, presses);
    for _ in 0..120 {
        h.idle(1);
    }
    h
}

/// Rasterize one frame and return `(width, height, rgba)`.
fn frame(h: &mut Harness) -> (usize, usize, Vec<u8>) {
    let (primitives, _, textures) = h.frame_primitives(SIZE);
    let size = (SIZE.0 as u32, SIZE.1 as u32);
    let raster = rasterize(&primitives, &textures, size, 1.0);
    assert_eq!(
        raster.missing_textures, 0,
        "every texture should be present"
    );
    (
        raster.width as usize,
        raster.height as usize,
        raster.rgba.clone(),
    )
}

/// Count vivid pixels in the columns `[x0, x1)` of every row.
fn vivid_in_columns(w: usize, h: usize, rgba: &[u8], x0: usize, x1: usize) -> usize {
    let mut n = 0;
    for y in 0..h {
        for x in x0..x1 {
            if is_vivid(&rgba[(y * w + x) * 4..][..4]) {
                n += 1;
            }
        }
    }
    n
}

#[test]
fn the_empty_region_left_of_the_band_holds_no_signal() {
    // Pan hard left.  `MAX_OVERSCAN_FRAC` stops the view with the band edge at
    // screen centre, so the left half of every pane is off-band and the right
    // half still shows signal.  A `ClampToEdge` smear would fill that left half
    // with copies of the leftmost band column — vivid thermal colours across
    // roughly half the window.
    let mut h = panned_left(400);
    let view = h.app.freq_view();
    assert_eq!(
        view.center_hz, 0.0,
        "expected full deflection to the low edge"
    );
    assert_eq!(
        view.band_frac(),
        Some((0.5, 1.0)),
        "the band should occupy exactly the right half"
    );

    let (w, hgt, rgba) = frame(&mut h);
    let mid = w / 2;
    let empty = vivid_in_columns(w, hgt, &rgba, 0, mid);
    let band = vivid_in_columns(w, hgt, &rgba, mid, w);
    let half = (hgt * mid) as f64;

    println!(
        "\n  vivid off-band: {empty} ({:.2}%)   vivid in-band: {band} ({:.2}%)",
        empty as f64 / half * 100.0,
        band as f64 / half * 100.0
    );

    // The control: the right half must actually carry signal, or the left half
    // being clean would prove nothing.  Measured at 6.84%.
    assert!(
        band as f64 > half * 0.05,
        "only {band} vivid pixels in the band — the display never filled, so \
         this test cannot distinguish a smear from an empty screen"
    );
    // Markers are drawn across the whole pane and the primary sits at screen
    // centre, so a thin vivid line and its label are expected off-band.
    //
    // **Calibrated against the defect rather than guessed.**  Reverting the
    // panes to the full rect with unclamped UVs — the `ClampToEdge` smear this
    // guards against — measured 5.76%; the fix measures 0.48%.  The bound sits
    // between, with room on both sides.
    assert!(
        (empty as f64) < half * 0.02,
        "{empty} vivid pixels ({:.2}%) in the empty region — the band's edge \
         column is being repeated into it",
        empty as f64 / half * 100.0
    );
}

#[test]
fn the_spectrogram_writes_its_empty_rows_rather_than_stretching_to_fill() {
    // The spectrogram's off-band region is *inside* its texture — rows are
    // pixels it commits, not an area painted over it afterwards — so it needs
    // its own check.  Its old failure mode was the mirror of the smear: it
    // clamped its window to the band and kept mapping `0..nyquist` across every
    // row, so panning off the edge would quietly *compress* the whole spectrum
    // into the pane while the axis labels beside it said otherwise.
    let h = panned_left(400);
    assert_eq!(h.app.freq_view().center_hz, 0.0);

    let sg = h.app.spectrogram();
    let rows = sg.freq_rows();
    let col = sg
        .cols_in_display_order()
        .next()
        .expect("the spectrogram should have committed columns");
    assert_eq!(col.len(), rows);

    // Row 0 is the high edge and row `rows-1` the low edge, so with the band
    // edge at screen centre the bottom half is the empty region.
    let (top, bottom) = col.split_at(rows / 2);
    let empty = &bottom[2..]; // skip the two rows straddling 0 Hz
    let filler = empty[0];
    assert!(
        empty.iter().all(|&c| c == filler),
        "the off-band rows are not a single colour — they are still sampling bins"
    );
    assert!(
        top.iter().any(|&c| c != filler),
        "the in-band rows match the off-band filler, so this proves nothing"
    );
    // And the filler is not the palette's floor: "no band here" has to be
    // distinguishable from "no signal here".
    assert_ne!(
        filler,
        egui::Color32::BLACK,
        "the empty region reads as a silent band rather than as absent band"
    );
}

#[test]
fn the_shipped_overscan_script_still_holds() {
    // `scripts/README.md` says a script's `assert` lines make it "a regression
    // test unchanged" — the driver parses them and ignores them, the harness
    // executes them.  Nothing was actually dropping one into a test, so the
    // claim was true of the format and untrue of the tree.  This runs the
    // shipped file verbatim: every `assert` in it is now checked in CI, and the
    // frequencies quoted in `docs/viewport.md` cannot drift from the code.
    let src = std::fs::read_to_string("scripts/overscan.txt").expect("the script should be there");
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut h = Harness::with_defaults();
    // Redirect the stills, so running the suite does not write into the tree.
    h.capture_dir = tmp.path().to_path_buf();
    h.run_script(&src);

    let written: Vec<_> = std::fs::read_dir(tmp.path())
        .expect("capture dir")
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "png"))
        .collect();
    assert_eq!(
        written.len(),
        4,
        "the script's four `still` directives should have written four images"
    );
}

#[test]
fn a_locked_view_never_leaves_the_band_in_any_pane() {
    // The other half of the rule: with `L` engaged the pan is band-limited, so
    // no pane should ever have an empty region to draw.  Asserted because the
    // old failure was silent — the carrier row would pin at its bound while the
    // viewport kept going.
    let mut h = Harness::with_defaults();
    h.select_source(SourceMode::Cofdm);
    for _ in 0..120 {
        h.idle(1);
    }
    h.key_n(egui::Key::ArrowUp, 6);
    h.key(egui::Key::L);
    assert!(h.app.source_locked());

    for _ in 0..40 {
        h.key(egui::Key::ArrowLeft);
        let v = h.app.freq_view();
        assert_eq!(
            v.band_frac(),
            Some((0.0, 1.0)),
            "a locked pan left an empty region: [{}, {}]",
            v.lo(),
            v.hi()
        );
    }
}
