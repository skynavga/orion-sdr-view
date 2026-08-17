// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The pane ring buffers, asserted on CPU-side pixels.
//!
//! Both pane renderers keep `Vec<Color32>` in main memory and upload only the
//! rows/columns that changed, so the ring arithmetic — which is real pixel
//! logic, and which the 0.0.17 pan fixes suggest has bitten before — is testable
//! with no renderer at all.  What is *not* tested here is the composed window:
//! golden images over a spectrum display are brittle against font hinting,
//! driver antialiasing and continuously varying content.
//!
//! The property in both cases is the seam.  `draw_cropped` and `draw_ring` each
//! paint the ring as two UV quads split at `head`; the display-order iterators
//! resolve that same split back into one sequence, so if they agree with the
//! pushes they agree with the draw.

#![cfg(feature = "gui")]

use num_complex::Complex32 as C32;
use orion_sdr::demodulate::BitOutcome;
use orion_sdr::modulate::ConstellationOrder;
use orion_sdr_view::app::constellation::{ConstellationDisplay, ideal_points};
use orion_sdr_view::app::correction::{CORR_ROWS, CorrectionMap, DEFAULT_ROW_BITS};
use orion_sdr_view::app::spectrogram::SpectrogramDisplay;
use orion_sdr_view::app::waterfall::WaterfallDisplay;

const DB_MIN: f32 = -100.0;
const DB_MAX: f32 = 0.0;
/// One commit per push: both renderers pace themselves at 60 rows (or columns)
/// per second by default.
const DT: f32 = 1.0 / 60.0;

/// The colour a flat spectrum at `db` produces, read back through the same
/// public surface under test rather than by duplicating the palette here.
fn waterfall_color(db: f32) -> egui::Color32 {
    let mut w = WaterfallDisplay::new(1, 1, DB_MIN, DB_MAX);
    w.push_row(&[db], DT);
    w.rows_in_display_order().next().expect("one row")[0]
}

/// A ramp of dB values whose colours are all distinct, so an ordering test can
/// actually fail.
fn distinct_ramp(n: usize) -> Vec<f32> {
    let ramp: Vec<f32> = (0..n).map(|i| DB_MIN + i as f32 * 5.0).collect();
    let mut seen: Vec<egui::Color32> = ramp.iter().map(|&db| waterfall_color(db)).collect();
    let before = seen.len();
    seen.sort_by_key(|c| c.to_array());
    seen.dedup();
    assert_eq!(before, seen.len(), "ramp colours must be distinguishable");
    ramp
}

// ── Waterfall: rows ─────────────────────────────────────────────────────────

#[test]
fn a_partly_filled_waterfall_yields_only_what_was_written() {
    // `filled` saturates at `max_rows`, and the untouched remainder of the ring
    // is black.  Yielding it would make a fresh display look like it had
    // history.
    let mut w = WaterfallDisplay::new(4, 8, DB_MIN, DB_MAX);
    assert_eq!(w.filled(), 0);
    assert_eq!(w.rows_in_display_order().count(), 0);

    let ramp = distinct_ramp(3);
    for &db in &ramp {
        w.push_row(&[db; 4], DT);
    }
    assert_eq!(w.filled(), 3);
    assert_eq!(w.rows_in_display_order().count(), 3);
}

#[test]
fn the_waterfall_reads_back_newest_first_across_the_wrap() {
    // The seam.  `head` is the next slot to write, so the newest row is at
    // `head - 1` and the sequence runs backwards from there, wrapping once.
    // Twelve pushes into an eight-row ring puts `head` at 4 — mid-buffer, so a
    // reader that ignored the wrap would return the four *oldest* rows first.
    let (bins, rows) = (4, 8);
    let ramp = distinct_ramp(12);
    let mut w = WaterfallDisplay::new(bins, rows, DB_MIN, DB_MAX);
    for &db in &ramp {
        w.push_row(&vec![db; bins], DT);
    }
    assert_eq!(w.filled(), rows, "the ring should be saturated");

    let got: Vec<egui::Color32> = w.rows_in_display_order().map(|r| r[0]).collect();
    let want: Vec<egui::Color32> = ramp
        .iter()
        .rev()
        .take(rows)
        .map(|&db| waterfall_color(db))
        .collect();
    assert_eq!(got, want, "rows should read newest-first, oldest dropped");

    // Every row is the full width, and uniform for a flat spectrum.
    for row in w.rows_in_display_order() {
        assert_eq!(row.len(), bins);
        assert!(row.iter().all(|&c| c == row[0]));
    }
}

#[test]
fn clearing_the_waterfall_empties_the_display_order() {
    // `clear` runs on every source switch and on a sample-rate change: history
    // indexed by bin at the old scaling cannot be drawn at the new one.
    let mut w = WaterfallDisplay::new(4, 8, DB_MIN, DB_MAX);
    for &db in &distinct_ramp(5) {
        w.push_row(&[db; 4], DT);
    }
    w.clear();
    assert_eq!(w.filled(), 0);
    assert_eq!(w.rows_in_display_order().count(), 0);
}

#[test]
fn the_waterfall_commits_on_wall_clock_not_per_call() {
    // Row commits are paced by `dt`, which is what makes the scroll rate stable
    // across frame rates — and, with the injected `dt`, reproducible.
    let mut w = WaterfallDisplay::new(4, 64, DB_MIN, DB_MAX);
    w.push_row(&[-50.0; 4], DT / 4.0);
    assert_eq!(w.filled(), 0, "a quarter-frame should commit nothing");
    w.push_row(&[-50.0; 4], DT * 3.0 / 4.0);
    assert_eq!(
        w.filled(),
        1,
        "the balance of a frame should commit one row"
    );
    w.push_row(&[-50.0; 4], DT * 5.0);
    assert_eq!(
        w.filled(),
        6,
        "a long frame should commit its whole backlog"
    );
}

// ── Spectrogram: columns ────────────────────────────────────────────────────

/// The spectrogram's counterpart to [`waterfall_color`].  Its columns are
/// extracted from a frequency *window*, so the reference has to go through the
/// same mapping.
fn spectrogram_column(db: f32, rows: usize) -> Vec<egui::Color32> {
    let mut s = SpectrogramDisplay::new(rows, 1, DB_MIN, DB_MAX);
    s.push_spectrum(&[db; 8], DT, 12_000.0, 12_000.0, 24_000.0);
    s.cols_in_display_order().next().expect("one column")
}

#[test]
fn the_spectrogram_reads_back_newest_first_across_the_wrap() {
    // Same seam, transposed: `draw_ring` paints newest-at-left, so the iterator
    // runs backwards from `head - 1` along the column axis.
    let (rows, cols) = (3, 5);
    let ramp = distinct_ramp(8);
    let mut s = SpectrogramDisplay::new(rows, cols, DB_MIN, DB_MAX);
    for &db in &ramp {
        s.push_spectrum(&[db; 8], DT, 12_000.0, 12_000.0, 24_000.0);
    }
    assert_eq!(s.filled(), cols, "the ring should be saturated");

    let got: Vec<Vec<egui::Color32>> = s.cols_in_display_order().collect();
    let want: Vec<Vec<egui::Color32>> = ramp
        .iter()
        .rev()
        .take(cols)
        .map(|&db| spectrogram_column(db, rows))
        .collect();
    assert_eq!(
        got, want,
        "columns should read newest-first, oldest dropped"
    );
    assert!(got.iter().all(|c| c.len() == rows));
}

#[test]
fn a_partly_filled_spectrogram_yields_only_what_was_written() {
    let mut s = SpectrogramDisplay::new(3, 5, DB_MIN, DB_MAX);
    assert_eq!(s.cols_in_display_order().count(), 0);
    for &db in &distinct_ramp(2) {
        s.push_spectrum(&[db; 8], DT, 12_000.0, 12_000.0, 24_000.0);
    }
    assert_eq!(s.filled(), 2);
    assert_eq!(s.cols_in_display_order().count(), 2);

    s.clear();
    assert_eq!(s.filled(), 0);
    assert_eq!(s.cols_in_display_order().count(), 0);
}

#[test]
fn the_spectrogram_column_rate_follows_its_time_range() {
    // `set_time_range` is how the `Spec time` row tunes how much real time the
    // pane represents: per-column duration is `time_range / max_cols`, so a
    // wider range commits fewer columns for the same elapsed time.
    let mut fast = SpectrogramDisplay::new(3, 60, DB_MIN, DB_MAX);
    fast.set_time_range(1.0); // 1/60 s per column
    let mut slow = SpectrogramDisplay::new(3, 60, DB_MIN, DB_MAX);
    slow.set_time_range(4.0); // 4/60 s per column

    for _ in 0..12 {
        fast.push_spectrum(&[-50.0; 8], DT, 12_000.0, 12_000.0, 24_000.0);
        slow.push_spectrum(&[-50.0; 8], DT, 12_000.0, 12_000.0, 24_000.0);
    }
    assert_eq!(fast.filled(), 12);
    assert_eq!(slow.filled(), 3);
}

// ── Constellation: the raster is what gets captured ─────────────────────────

/// The constellation raster after `symbols`, forced up to date.
fn const_raster(symbols: &[C32]) -> Vec<egui::Color32> {
    let mut c = ConstellationDisplay::new();
    c.push_symbols(symbols, ConstellationOrder::Qpsk);
    c.sync_raster();
    c.pixels_in_display_order().to_vec()
}

#[test]
fn off_scale_symbols_are_dropped_not_clamped() {
    // **Clamping would look correct and be wrong.** Piling the tail onto the
    // border reads as a hard edge that is not in the signal, and makes the cloud
    // look tighter than it is. So an out-of-extent point contributes *nothing*
    // to the picture, and is counted instead.
    let mut c = ConstellationDisplay::new();
    c.push_symbols(&[C32::new(50.0, -50.0)], ConstellationOrder::Qpsk);
    c.sync_raster();
    assert_eq!(c.off_scale(), (1, 1), "counted, not drawn");

    // Identical to a raster that saw no symbols at all: the reference geometry
    // is there, the point is not.
    assert_eq!(
        c.pixels_in_display_order().to_vec(),
        const_raster(&[]),
        "an off-scale symbol must leave no mark anywhere, border included"
    );
    // The positive control, or the assertion above would pass against a
    // constellation that drew nothing ever.
    assert_ne!(
        const_raster(&[C32::new(0.707, 0.707)]),
        const_raster(&[]),
        "an in-range symbol must change the raster"
    );
}

#[test]
fn the_constellation_marks_its_ideal_points() {
    // The reference grid comes from orion-sdr's own mappers rather than a table
    // copied into the viewer, so it cannot drift from what the transmitter used.
    let pts = ideal_points(ConstellationOrder::Qpsk);
    assert_eq!(pts.len(), 4, "QPSK has four points");
    let a = 1.0 / 2.0_f32.sqrt();
    for p in &pts {
        assert!(
            (p.norm() - 1.0).abs() < 1e-5,
            "unit energy, got |{p}| = {}",
            p.norm()
        );
        assert!((p.re.abs() - a).abs() < 1e-5 && (p.im.abs() - a).abs() < 1e-5);
    }
    assert_eq!(ideal_points(ConstellationOrder::Qam16).len(), 16);
}

#[test]
fn clearing_the_constellation_empties_it() {
    let mut c = ConstellationDisplay::new();
    assert!(c.is_empty());
    c.push_symbols(&[C32::new(0.707, 0.707); 8], ConstellationOrder::Qpsk);
    assert!(!c.is_empty());
    assert_eq!(c.order(), Some(ConstellationOrder::Qpsk));

    c.clear();
    assert!(c.is_empty(), "cleared");
    assert_eq!(c.off_scale(), (0, 0));
}

#[test]
fn a_constellation_change_discards_the_old_cloud() {
    // A different grid is not comparable: the ideal-point overlay is about to
    // move, and an accumulated density belonging to the old one would read as
    // part of the new.
    let mut c = ConstellationDisplay::new();
    c.push_symbols(&[C32::new(0.707, 0.707); 16], ConstellationOrder::Qpsk);
    assert_eq!(c.off_scale().1, 16);

    c.push_symbols(&[C32::new(0.3, 0.3)], ConstellationOrder::Qam16);
    assert_eq!(c.order(), Some(ConstellationOrder::Qam16));
    assert_eq!(
        c.off_scale().1,
        1,
        "only the new grid's symbols are counted"
    );
}

// ── Correction map: rows are time slices, not codewords ─────────────────────

/// One slice at [`ROWS_PER_SEC`].
const SLICE: f32 = 1.0 / 60.0;

/// A codeword's worth of outcomes, all the same state, so a row is
/// identifiable by its colour alone.
fn uniform(n: usize, o: BitOutcome) -> Vec<BitOutcome> {
    vec![o; n]
}

/// Push one frame of `n` identical codewords and let one slice elapse.
fn push_slice(m: &mut CorrectionMap, cols: usize, n: usize, o: BitOutcome) {
    m.push_frame(&uniform(cols * n, o), cols, cols / 2);
    m.tick(SLICE, false);
}

#[test]
fn rows_are_time_paced_not_codeword_paced() {
    // **The correction this whole design turns on.** One row per codeword is
    // unreadable at the wide bandwidth fractions — ~580 codewords/s against a
    // ~118 fps render is five rows per frame — so a row is a *time slice* and
    // however many codewords land in it are merged.
    let cols = 8;
    let mut m = CorrectionMap::new(cols);

    // Twenty codewords inside one slice is one row, not twenty.
    m.push_frame(&uniform(cols * 20, BitOutcome::Clean), cols, 4);
    assert_eq!(m.committed(), 0, "nothing commits until the slice elapses");
    m.tick(SLICE, false);
    assert_eq!(m.committed(), 1, "twenty codewords, one row");
    assert_eq!(m.last_depth(), 20, "and the depth says how many");
    assert_eq!(m.last_depth(), 20, "seeded, not blended up from nothing");

    // An empty slice commits nothing: painting it `Clean` would claim a
    // measurement that was never taken.
    m.tick(SLICE * 4.0, false);
    assert_eq!(m.committed(), 1, "an empty slice is not a clean one");
}

#[test]
fn a_row_is_the_union_of_its_slice_worst_state_winning() {
    // Union rather than sample, because the rare `Uncorrected` / `Introduced`
    // lines are what the pane is watched for — decimating to hit the row rate
    // would drop nine codewords in ten at 7/8 and take those with them.
    let cols = 4;
    let mut m = CorrectionMap::new(cols);

    // Three codewords in one slice: one clean, one corrected, one uncorrected
    // at a single bit position.
    let mut a = uniform(cols, BitOutcome::Clean);
    let mut b = uniform(cols, BitOutcome::Clean);
    b[1] = BitOutcome::Corrected;
    let mut c = uniform(cols, BitOutcome::Clean);
    c[1] = BitOutcome::Uncorrected;
    c[2] = BitOutcome::Corrected;
    a.extend(b);
    a.extend(c);
    m.push_frame(&a, cols, 2);
    m.tick(SLICE, false);

    let row = m.rows_in_display_order().next().expect("one row").to_vec();
    let colour = |o| {
        let mut probe = CorrectionMap::new(cols);
        probe.push_frame(&uniform(cols, o), cols, 2);
        probe.tick(SLICE, false);
        probe.rows_in_display_order().next().unwrap()[0]
    };
    assert_eq!(
        row[0],
        colour(BitOutcome::Clean),
        "bit 0 was clean throughout"
    );
    assert_eq!(
        row[1],
        colour(BitOutcome::Uncorrected),
        "bit 1 was Corrected in one codeword and Uncorrected in another — the \
         worse state has to win, or the pane hides the bit the outer code now \
         has to deal with"
    );
    assert_eq!(
        row[2],
        colour(BitOutcome::Corrected),
        "bit 2 was corrected once"
    );
}

#[test]
fn the_correction_map_reads_back_newest_first_across_the_wrap() {
    let cols = 8;
    let mut m = CorrectionMap::new(cols);
    let states = [
        BitOutcome::Clean,
        BitOutcome::Corrected,
        BitOutcome::Uncorrected,
        BitOutcome::Introduced,
    ];
    let total = CORR_ROWS + 3;
    for i in 0..total {
        push_slice(&mut m, cols, 1, states[i % states.len()]);
    }
    assert_eq!(m.filled(), CORR_ROWS, "the ring should be saturated");
    assert_eq!(m.committed(), total as u64, "committed does not saturate");

    let rows: Vec<Vec<egui::Color32>> = m.rows_in_display_order().map(|r| r.to_vec()).collect();
    for (i, row) in rows.iter().enumerate() {
        let want = states[(total - 1 - i) % states.len()];
        let mut probe = CorrectionMap::new(cols);
        push_slice(&mut probe, cols, 1, want);
        let expect = probe.rows_in_display_order().next().expect("one row")[0];
        assert_eq!(row[0], expect, "row {i} should be the {want:?} colour");
    }
}

#[test]
fn a_partly_filled_correction_map_yields_only_what_was_written() {
    let mut m = CorrectionMap::new(16);
    assert_eq!(m.filled(), 0);
    assert_eq!(m.rows_in_display_order().count(), 0);

    for _ in 0..3 {
        push_slice(&mut m, 16, 1, BitOutcome::Clean);
    }
    assert_eq!(m.filled(), 3);
    assert_eq!(m.rows_in_display_order().count(), 3);

    m.clear();
    assert_eq!(m.filled(), 0);
    assert_eq!(m.committed(), 0);
    assert_eq!(m.rows_in_display_order().count(), 0);
}

#[test]
fn a_failed_frame_bands_the_map_instead_of_freezing_it() {
    // A frame with no ground truth has no map, so the map would otherwise empty
    // exactly when the link is worst — which reads as "everything is fine".
    let cols = 12;
    let mut m = CorrectionMap::new(cols);
    push_slice(&mut m, cols, 1, BitOutcome::Clean);
    m.push_no_truth();
    m.tick(SLICE, false);
    assert_eq!(m.filled(), 2, "the failed frame still committed a row");

    let rows: Vec<Vec<egui::Color32>> = m.rows_in_display_order().map(|r| r.to_vec()).collect();
    assert_ne!(
        rows[0][0], rows[1][0],
        "a no-ground-truth band must not be the Clean colour — that is exactly \
         the confusion it exists to prevent"
    );
}

#[test]
fn a_decoded_frame_outranks_a_failure_in_the_same_slice() {
    // Both can land in one slice at the wide fractions.  The decoded frame's
    // map is real data and the band is an absence marker, so the data wins the
    // row — an absence painted over a measurement would be a lie, where the
    // reverse merely loses a marker whose count is reported anyway.
    let cols = 8;
    let mut m = CorrectionMap::new(cols);
    m.push_no_truth();
    m.push_frame(&uniform(cols, BitOutcome::Corrected), cols, 4);
    m.tick(SLICE, false);

    let mut probe = CorrectionMap::new(cols);
    push_slice(&mut probe, cols, 1, BitOutcome::Corrected);
    assert_eq!(
        m.rows_in_display_order().next().unwrap()[0],
        probe.rows_in_display_order().next().unwrap()[0]
    );
    assert_eq!(m.no_truth(), 1, "the failure is still counted");
}

#[test]
fn the_correction_map_reshapes_on_a_codeword_change() {
    let mut m = CorrectionMap::new(8);
    push_slice(&mut m, 8, 1, BitOutcome::Clean);
    assert_eq!((m.cols(), m.filled(), m.info_bits()), (8, 1, 4));

    push_slice(&mut m, 16, 1, BitOutcome::Corrected);
    assert_eq!(m.cols(), 16, "the row width follows the codeword length");
    assert_eq!(m.filled(), 1, "history from the old geometry is discarded");
    assert_eq!(m.info_bits(), 8);
}

#[test]
fn a_codeless_inner_code_wraps_at_the_default_width() {
    // The convolutional arm terminates once per frame and reports no block
    // structure, so there is no natural column count and the block is wrapped.
    let mut m = CorrectionMap::new(DEFAULT_ROW_BITS);
    m.push_frame(&uniform(DEFAULT_ROW_BITS * 2, BitOutcome::Clean), 0, 0);
    m.tick(SLICE, false);
    assert_eq!(m.cols(), DEFAULT_ROW_BITS);
    assert_eq!(m.last_depth(), 2, "two wraps merged into the slice");
    // Depth is smoothed for display but seeded exactly, so a first reading is
    // the true count rather than a fraction of it.
    assert!(m.last_depth() >= 1, "and never rounds down to nothing");
    assert_eq!(m.info_bits(), 0, "no systematic boundary to mark");
}

#[test]
fn the_tally_counts_only_what_the_decoder_decided() {
    // An all-`Clean` map is a near-black rectangle, and nothing about that
    // distinguishes "measured, and zero" from a dead pane.  The tally is what
    // makes the difference legible — so it has to mean what a reader assumes.
    //
    // **It counts the systematic half only.**  The decoder decides message
    // bits; the parity half of the map is a re-encode of that decision, so a
    // few wrong message bits light up roughly half the parity as `Introduced`.
    // Measured on the shipped link, 11 wrong message bits produced 300 parity
    // ones — a whole-block tally overstated the decoder's damage ~25x.  The
    // picture still paints them, because a solid magenta block is an
    // unmistakable "this codeword is wrong"; the *number* must not.
    let cols = 8;
    let mut m = CorrectionMap::new(cols);
    let map = [
        // Systematic half — counted.
        BitOutcome::Clean,
        BitOutcome::Corrected,
        BitOutcome::Corrected,
        BitOutcome::Uncorrected,
        // Parity half — painted, not counted.
        BitOutcome::Introduced,
        BitOutcome::Introduced,
        BitOutcome::Uncorrected,
        BitOutcome::Clean,
    ];
    m.push_frame(&map, cols, cols / 2);
    m.tick(SLICE, false);

    let t = m.last_tally();
    assert_eq!(
        (t.bits, t.corrected, t.uncorrected, t.introduced),
        (4, 2, 1, 0),
        "the denominator is the message bits, and the parity half's three \
         disagreements are a syndrome of them rather than three more errors"
    );
    assert_eq!(m.no_truth(), 0, "nothing failed yet");
}

#[test]
fn a_code_with_no_systematic_prefix_tallies_the_whole_block() {
    // The convolutional arm reports no block structure, so there is no
    // systematic subset to restrict to.  The same re-encode amplification
    // applies and goes untreated — better a denominator that says which basis
    // it is on than a count silently over nothing.
    let cols = 8;
    let mut m = CorrectionMap::new(cols);
    let mut map = uniform(cols, BitOutcome::Clean);
    map[6] = BitOutcome::Introduced;
    m.push_frame(&map, 0, 0);
    m.tick(SLICE, false);

    let t = m.last_tally();
    assert_eq!(t.bits, cols, "the whole block is the denominator");
    assert_eq!(t.introduced, 1, "and a late-position error still counts");
}

#[test]
fn a_gap_keeps_the_map_scrolling_in_its_own_colour() {
    // A frozen map reads as a link that is still delivering.  So a silence
    // scrolls too, at the same fixed rate as everything else, and its height is
    // how long the silence was.
    let cols = 8;
    let mut m = CorrectionMap::new(cols);
    push_slice(&mut m, cols, 1, BitOutcome::Clean);
    let before = m.committed();

    m.tick(SLICE * 5.0, true);
    let gap_rows = m.committed() - before;
    assert_eq!(gap_rows, 5, "five slices of silence is five rows");

    // Its own colour — not Clean (which would say "good bits"), and not the
    // no-ground-truth band (which says "a frame arrived and failed").
    let newest = m.rows_in_display_order().next().expect("a row").to_vec();
    let mut clean = CorrectionMap::new(cols);
    push_slice(&mut clean, cols, 1, BitOutcome::Clean);
    let mut failed = CorrectionMap::new(cols);
    failed.push_no_truth();
    failed.tick(SLICE, false);
    assert_ne!(
        newest[0],
        clean.rows_in_display_order().next().unwrap()[0],
        "a silence must not look like good bits"
    );
    assert_ne!(
        newest[0],
        failed.rows_in_display_order().next().unwrap()[0],
        "nothing arriving is not the same as a frame that failed to verify"
    );
}

#[test]
fn the_aggregation_depth_survives_a_gap() {
    // The depth is a property of the *link* — how many codewords a row unions —
    // not of the current instant.  Zeroing it through a silence made the pane
    // read "x0 cw/row", which looks like a broken field rather than an absence.
    let cols = 8;
    let mut m = CorrectionMap::new(cols);
    m.push_frame(&uniform(cols * 7, BitOutcome::Clean), cols, 4);
    m.tick(SLICE, false);
    assert_eq!(m.last_depth(), 7);

    m.tick(SLICE * 10.0, true);
    assert_eq!(m.last_depth(), 7, "a silence does not make the depth zero");

    // Nor does a run of frames that all failed — the other way the readout
    // used to blink out.  Both are absences of *data*, not of the link's
    // aggregation factor.
    for _ in 0..5 {
        m.push_no_truth();
        m.tick(SLICE, false);
    }
    assert_eq!(m.last_depth(), 7, "nor does a run of failures");
}

#[test]
fn resetting_the_counters_leaves_the_rows_alone() {
    // The epoch fix.  A capture caught `off-scale of 270480` — about 105 frames
    // — beside `fail 1463`, because the constellation cleared every burst while
    // this pane's counters ran since the source started.  Two numbers on
    // different clocks, side by side, inviting a comparison that means nothing.
    //
    // The rows deliberately survive: they are scrollback, and how the link
    // failed on the way down is what should still be on screen once it has.
    let cols = 8;
    let mut m = CorrectionMap::new(cols);
    push_slice(&mut m, cols, 1, BitOutcome::Clean);
    m.push_no_truth();
    m.tick(SLICE, false);
    assert_eq!(m.no_truth(), 1);
    let rows = m.filled();

    m.reset_counters();
    assert_eq!(m.no_truth(), 0, "counters restart with the burst");
    assert_eq!(m.last_tally(), Default::default());
    assert_eq!(m.filled(), rows, "the scrollback does not");
}
