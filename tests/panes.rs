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
