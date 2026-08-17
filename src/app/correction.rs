// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The correction-map half of pane 3: what the inner decoder did with each
//! coded bit, scrolling by time.
//!
//! The `WaterfallDisplay` idiom, wall-clock paced like its model.  X is the bit
//! index within a *single* codeword, and the codewords in a slice are overlaid
//! on those same columns rather than laid end to end — so a cell is "the worst
//! thing that happened at this bit position in this slice", and per-codeword
//! identity is what the overlay trades for a readable scroll rate.
//!
//! **The Y axis is time**, at a fixed [`ROWS_PER_SEC`], with each row the union
//! of every codeword in its slice.  It was codeword index, one row per
//! codeword, and that is a correction: at the 7/8 bandwidth fraction the
//! receiver produces ~580 codewords/s, so the pane turned over in 0.44 s and
//! nothing could be tracked.  See [`ROWS_PER_SEC`] for the measurement and
//! `severity` for why the slice is a union rather than a sample.
//!
//! **A frame that fails to decode still commits a row.**  There is no ground
//! truth for a payload that did not verify, so its map is empty — which means
//! the map goes blank exactly when the link is worst.  Rendering that as a
//! frozen picture would read as "everything is fine", the same inversion the
//! `X` panel's BER rungs avoid by showing an em-dash rather than a zero.  A
//! distinct band keeps the scroll alive and makes failure legible as a texture.

use eframe::egui;
use orion_sdr::demodulate::BitOutcome;

/// Rows kept in the ring — 4.3 s of history at [`ROWS_PER_SEC`], and the same
/// 4.3 s at every bandwidth fraction now that rows are time-paced rather than
/// one per codeword.
pub const CORR_ROWS: usize = 256;

/// Row width when the inner code has no block structure to report — the
/// convolutional arm terminates once per frame, so upstream sends
/// `codeword_bits == 0` and there is no natural column count.  The coded block
/// is then simply wrapped at this width.
pub const DEFAULT_ROW_BITS: usize = 512;

/// Rows committed per second, whatever the link is doing.
///
/// **Time-paced, not event-paced, and that is a correction.**  One row per
/// codeword sounds right and is unreadable: at the 7/8 bandwidth fraction the
/// receiver produces ~580 codewords/s, so a 256-row ring turned over in 0.44 s
/// and — measured against a screen recording at the app's ~118 fps — about five
/// rows appeared per rendered frame.  Each got one 8 ms glimpse.  That is a 5x
/// rate mismatch, not a depth problem, and no ring size fixes it.
///
/// It also fixed an inversion nobody would guess: a decoded frame committed ten
/// rows and a failed one committed a single band, so a *worse* link scrolled
/// *slower*.  At 60/s the scroll is independent of both bandwidth fraction and
/// link quality, and 256 rows is 4.3 s of history everywhere.
pub const ROWS_PER_SEC: f32 = 60.0;

/// Four flat colours, not a ramp.  These are categories: a ramp would imply an
/// ordering between "the decoder broke it" and "the decoder could not fix it"
/// that does not exist.
const CLEAN: egui::Color32 = egui::Color32::from_rgb(12, 16, 24);
const CORRECTED: egui::Color32 = egui::Color32::from_rgb(0, 190, 140);
const UNCORRECTED: egui::Color32 = egui::Color32::from_rgb(230, 60, 50);
const INTRODUCED: egui::Color32 = egui::Color32::from_rgb(210, 70, 210);
/// A frame with no ground truth — not "no errors".
const NO_TRUTH: egui::Color32 = egui::Color32::from_rgb(70, 58, 30);
/// No signal at all.  **A different thing from [`NO_TRUTH`]**, which means a
/// frame arrived and failed to verify; this means nothing arrived.  Cool and
/// desaturated against that one's warm olive, and clearly lighter than
/// [`CLEAN`] so the band reads as a band rather than as a run of good bits.
const NO_SIGNAL: egui::Color32 = egui::Color32::from_rgb(40, 46, 60);
/// Padding at the end of a short final row, so it does not read as `Clean`.
const UNUSED: egui::Color32 = egui::Color32::from_rgb(4, 4, 8);

/// The colour for one bit's outcome.
///
/// Reads through [`BitOutcome`]'s own predicates rather than matching the
/// variants, so the palette cannot drift from the definition of the two halves
/// of the map.
fn outcome_color(o: BitOutcome) -> egui::Color32 {
    match (o.arrived_wrong(), o.decoder_disagreed()) {
        (false, false) => CLEAN,
        (true, false) => CORRECTED,
        (true, true) => UNCORRECTED,
        (false, true) => INTRODUCED,
    }
}

/// How bad an outcome is, for merging several codewords into one row.
///
/// A row is the **union** of everything in its time slice, worst state winning,
/// rather than a sample of it.  Decimating to hit the row rate would drop nine
/// codewords in ten at 7/8 — and the rare `Uncorrected` / `Introduced` lines are
/// exactly what the pane is watched for, so losing them to keep the density
/// honest is the wrong trade.  The cost is that density *is* inflated at the
/// wide fractions, which is why the aggregation depth is reported.
fn severity(o: BitOutcome) -> u8 {
    match (o.arrived_wrong(), o.decoder_disagreed()) {
        (false, false) => 0, // Clean
        (true, false) => 1,  // Corrected
        (false, true) => 2,  // Introduced
        (true, true) => 3,   // Uncorrected — the outer code's problem now
    }
}

/// Counts one frame's outcomes over the positions the decoder actually decided.
///
/// `info_bits == 0` means the inner code has no systematic prefix to restrict
/// to, so every position counts — see [`FrameTally`].
fn tally(correction: &[BitOutcome], n: usize, info_bits: usize) -> FrameTally {
    let counted = |i: usize| info_bits == 0 || (i % n.max(1)) < info_bits;
    let mut t = FrameTally::default();
    for (i, o) in correction.iter().enumerate() {
        if !counted(i) {
            continue;
        }
        t.bits += 1;
        match (o.arrived_wrong(), o.decoder_disagreed()) {
            (true, false) => t.corrected += 1,
            (true, true) => t.uncorrected += 1,
            (false, true) => t.introduced += 1,
            (false, false) => {}
        }
    }
    t
}

/// Scrolling correction map, one row per time slice.
pub struct CorrectionMap {
    /// Columns per row: the inner code's `n`, or [`DEFAULT_ROW_BITS`].
    cols: usize,
    /// The inner code's `k`, so the pane can mark where the systematic prefix
    /// ends.  `0` when there is no block structure.
    info_bits: usize,
    /// Row ring, `CORR_ROWS × cols`.  `head` is the next slot to write; the
    /// newest row is at `(head + CORR_ROWS - 1) % CORR_ROWS`.
    pixels: Vec<egui::Color32>,
    head: usize,
    filled: usize,
    texture: Option<egui::TextureHandle>,
    dirty_rows: Vec<usize>,
    /// Rows committed since the last clear, so a caller can tell a stalled map
    /// from an empty one.
    committed: u64,
    /// The most recent decoded frame's tallies, and the running count of frames
    /// with no ground truth.
    ///
    /// **An all-`Clean` map is a near-black rectangle indistinguishable from a
    /// dead pane.**  That is honest — a good link really has nothing to draw —
    /// but it is not informative, and it was the first thing that read wrong
    /// when this pane was looked at.  A tally beside it turns "black" into
    /// "measured, and zero".  Per *frame* rather than over the ring: keeping a
    /// windowed count means un-counting evicted rows, and the newest frame is
    /// what the top of the scroll is showing anyway.
    last: FrameTally,
    no_truth: u64,
    /// Wall-clock seconds since the last row was committed.
    since_row: f32,
    /// The slice being accumulated: every codeword since the last commit,
    /// merged worst-state-wins.  Empty until something arrives.
    pending: Vec<BitOutcome>,
    /// Codewords merged into [`pending`](Self::pending), and frames in this
    /// slice that produced no ground truth.
    pending_cw: usize,
    pending_fail: usize,
    /// Smoothed codewords-per-row, the aggregation depth reported so a reader
    /// can discount the density.
    ///
    /// `None` until a row has aggregated something.  **Never reset to zero by a
    /// gap or a run of failures** — it is a property of the link, not of the
    /// instant, and zeroing it made the readout blink out and back, which is
    /// what a reader sees as jitter rather than as information.
    depth_ema: Option<f32>,
}

/// One frame's outcome counts, for the pane's readout.
///
/// **Counted over the inner code's *systematic* positions only**, where the
/// code has them.  The decoder decides message bits; the parity half of the map
/// is a re-encode of that decision, so a handful of wrong message bits shows up
/// there as roughly half the parity bits flipped — a syndrome, not independent
/// mistakes.  Measured on the shipped link: 11 wrong message bits produced
/// **300** `Introduced` parity bits, so a whole-block tally overstated what the
/// decoder got wrong by about 25x.
///
/// The *picture* still paints the parity half, and should: a solid magenta
/// block is an unmistakable "this codeword is wrong". It is the number that has
/// to mean what a reader assumes it means.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameTally {
    /// Positions the counts are over — the denominator.  The message-bit count
    /// for a block code, or the whole coded block for a code with no systematic
    /// prefix to restrict to (the convolutional arm), where there is no better
    /// subset and the same amplification applies untreated.
    pub bits: usize,
    pub corrected: usize,
    pub uncorrected: usize,
    pub introduced: usize,
}

impl Default for CorrectionMap {
    fn default() -> Self {
        Self::new(DEFAULT_ROW_BITS)
    }
}

impl CorrectionMap {
    pub fn new(cols: usize) -> Self {
        let cols = cols.max(1);
        Self {
            cols,
            info_bits: 0,
            pixels: vec![UNUSED; cols * CORR_ROWS],
            head: 0,
            filled: 0,
            texture: None,
            dirty_rows: Vec::new(),
            committed: 0,
            last: FrameTally::default(),
            no_truth: 0,
            since_row: 0.0,
            pending: Vec::new(),
            pending_cw: 0,
            pending_fail: 0,
            depth_ema: None,
        }
    }

    /// Drop all history.  Forces a full re-upload, so no stale rows survive a
    /// source switch or a burst boundary.
    pub fn clear(&mut self) {
        self.pixels.iter_mut().for_each(|p| *p = UNUSED);
        self.head = 0;
        self.filled = 0;
        self.committed = 0;
        self.last = FrameTally::default();
        self.no_truth = 0;
        self.since_row = 0.0;
        self.discard_pending();
        self.depth_ema = None;
        self.dirty_rows = (0..CORR_ROWS).collect();
    }

    /// Re-shape to a different codeword length, discarding history.
    ///
    /// The column count is set by the *code*, not chosen, so a change means the
    /// MCS changed underneath and the old rows are not comparable.
    fn reshape(&mut self, cols: usize) {
        let cols = cols.max(1);
        if cols == self.cols {
            return;
        }
        self.cols = cols;
        self.pixels = vec![UNUSED; cols * CORR_ROWS];
        self.texture = None;
        self.head = 0;
        self.filled = 0;
        self.committed = 0;
        self.last = FrameTally::default();
        self.discard_pending();
        self.dirty_rows.clear();
    }

    /// Commit one decoded frame's map: `ceil(len / cols)` rows, oldest bit
    /// first.
    ///
    /// `codeword_bits` / `codeword_info_bits` come straight off the probe
    /// record; zero means the inner code has no block structure (the
    /// convolutional arm), and the block is wrapped at [`DEFAULT_ROW_BITS`].
    pub fn push_frame(
        &mut self,
        correction: &[BitOutcome],
        codeword_bits: usize,
        codeword_info_bits: usize,
    ) {
        if correction.is_empty() {
            return;
        }
        self.reshape(if codeword_bits > 0 {
            codeword_bits
        } else {
            DEFAULT_ROW_BITS
        });
        self.info_bits = codeword_info_bits;
        // The per-frame tally stays *per frame* and un-aggregated: it is the
        // calibrated number, the one a reader discounts the row density
        // against.
        self.last = tally(correction, self.cols, codeword_info_bits);
        // Merge into the slice being accumulated rather than committing rows
        // here — see `ROWS_PER_SEC`.
        if self.pending.len() != self.cols {
            self.pending = vec![BitOutcome::Clean; self.cols];
        }
        for chunk in correction.chunks(self.cols) {
            for (slot, &o) in self.pending.iter_mut().zip(chunk.iter()) {
                if severity(o) > severity(*slot) {
                    *slot = o;
                }
            }
            self.pending_cw += 1;
        }
    }

    /// Note a frame that reached the demapper but produced no ground truth.
    ///
    /// Counted into the current slice rather than committing a band of its own:
    /// at the wide bandwidth fractions several frames can fail inside one row's
    /// worth of time, and one band each is what made the scroll unreadable.
    pub fn push_no_truth(&mut self) {
        self.no_truth += 1;
        self.pending_fail += 1;
    }

    /// Reset the per-burst counters without touching the rows.
    ///
    /// **Called at a gap edge, so this pane's readouts share an epoch with the
    /// constellation's.**  They did not: the cloud cleared every burst while
    /// `fail` ran cumulatively since the source started, and a capture caught
    /// the two side by side reading `off-scale of 270480` — about 105 frames —
    /// next to `fail 1463`.  Two numbers with different clocks, inviting a
    /// comparison that means nothing.
    ///
    /// The *rows* deliberately survive: they are scrollback, and how the link
    /// failed on the way down is what should still be on screen once it has.
    pub fn reset_counters(&mut self) {
        self.no_truth = 0;
        self.last = FrameTally::default();
    }

    fn advance(&mut self) {
        self.dirty_rows.push(self.head);
        self.head = (self.head + 1) % CORR_ROWS;
        self.filled = (self.filled + 1).min(CORR_ROWS);
        self.committed += 1;
    }

    /// Drop the part-accumulated slice.
    fn discard_pending(&mut self) {
        self.pending.clear();
        self.pending_cw = 0;
        self.pending_fail = 0;
    }

    /// Write one row of a single flat colour at the head.
    fn commit_flat(&mut self, col: egui::Color32) {
        let base = self.head * self.cols;
        for slot in &mut self.pixels[base..base + self.cols] {
            *slot = col;
        }
        self.advance();
    }

    /// Advance the pane's clock, committing at most one row per slice.
    ///
    /// **The scroll continues through a gap.**  A frozen map reads as a link
    /// that is still delivering, which is the same inversion the
    /// no-ground-truth band exists to prevent one rung down — and a silence is
    /// itself worth seeing in the scrollback, with a length.  Call once per
    /// frame with the frame's `dt`.
    ///
    /// A slice with nothing in it commits nothing, so the scroll runs at
    /// `min(content rate, ROWS_PER_SEC)`.  Painting an empty slice `Clean`
    /// would claim a measurement that was never taken.
    pub fn tick(&mut self, dt_secs: f32, in_gap: bool) {
        self.since_row += dt_secs.max(0.0);
        let interval = 1.0 / ROWS_PER_SEC;
        // Bounded per call, so a long stall does not spin the whole ring at
        // once — the same guard the waterfall's wall-clock pacing takes.
        let mut budget = CORR_ROWS;
        while self.since_row >= interval && budget > 0 {
            self.since_row -= interval;
            budget -= 1;
            if in_gap {
                // The depth is **not** zeroed here.  It is a property of the
                // link being watched — how many codewords a row unions — not of
                // the current instant, and during a silence it is undefined
                // rather than nought.  Zeroing it made the readout say
                // the depth blink out through every gap, which reads as a broken
                // field rather than as an absence.
                self.discard_pending();
                self.commit_flat(NO_SIGNAL);
            } else if self.pending_cw > 0 {
                let base = self.head * self.cols;
                for (slot, &o) in self.pixels[base..base + self.cols]
                    .iter_mut()
                    .zip(self.pending.iter())
                {
                    *slot = outcome_color(o);
                }
                // Smoothed, and rounded only for display.  The raw count
                // wobbles by a codeword or two with decode-chunk timing, and a
                // right-anchored label that changes width every frame reads as
                // jitter.  Seeded on the first observation rather than blended
                // up from zero.
                self.depth_ema = Some(match self.depth_ema {
                    Some(prev) => 0.2 * self.pending_cw as f32 + 0.8 * prev,
                    None => self.pending_cw as f32,
                });
                self.discard_pending();
                self.advance();
            } else if self.pending_fail > 0 {
                // Nothing decoded in this slice, but something arrived and
                // failed: the band is the honest row for it.  The depth is
                // **not** touched — see the gap branch.
                self.discard_pending();
                self.commit_flat(NO_TRUTH);
            } else {
                // Nothing at all happened.  Hold the clock rather than
                // inventing a row.
                self.since_row = 0.0;
                break;
            }
        }
    }

    /// Upload the rows committed since the last call, one strip each.
    pub fn update_texture(&mut self, ctx: &egui::Context) {
        if self.texture.is_none() {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [self.cols, CORR_ROWS],
                &vec![0u8; self.cols * CORR_ROWS * 4],
            );
            self.texture =
                Some(ctx.load_texture("correction", image, egui::TextureOptions::NEAREST));
            self.dirty_rows = (0..CORR_ROWS).collect();
        }
        let Some(tex) = &mut self.texture else {
            return;
        };
        for &row in &self.dirty_rows {
            let base = row * self.cols;
            let rgba: Vec<u8> = self.pixels[base..base + self.cols]
                .iter()
                .flat_map(|c| [c.r(), c.g(), c.b(), 255])
                .collect();
            let strip = egui::ColorImage::from_rgba_unmultiplied([self.cols, 1], &rgba);
            tex.set_partial([0, row], strip, egui::TextureOptions::NEAREST);
        }
        self.dirty_rows.clear();
    }

    /// Paint the ring into `rect`, newest row at the top.
    ///
    /// Two UV quads split at `head`, exactly as the waterfall does it: physical
    /// rows `head-1..0` fill the top band and `CORR_ROWS-1..head` the bottom,
    /// each with its texture-V flipped so newer rows sit higher.
    pub fn draw_ring(&self, painter: &egui::Painter, rect: egui::Rect) {
        let Some(tex) = &self.texture else {
            return;
        };
        let split = self.head as f32 / CORR_ROWS as f32;
        if self.head > 0 {
            let scr = egui::Rect::from_min_max(
                rect.left_top(),
                egui::pos2(rect.right(), rect.top() + split * rect.height()),
            );
            super::utils::image_quad(painter, tex.id(), scr, [0.0, 1.0], [split, 0.0]);
        }
        if self.head < CORR_ROWS {
            let scr = egui::Rect::from_min_max(
                egui::pos2(rect.left(), rect.top() + split * rect.height()),
                rect.right_bottom(),
            );
            super::utils::image_quad(painter, tex.id(), scr, [0.0, 1.0], [1.0, split]);
        }
    }

    /// Columns per row — the inner code's `n`.
    pub fn cols(&self) -> usize {
        self.cols
    }

    /// The systematic prefix length `k`, or `0` when the code has no block
    /// structure.  The pane marks the boundary: "errors cluster in the parity
    /// half" is a real and readable story.
    pub fn info_bits(&self) -> usize {
        self.info_bits
    }

    /// Rows committed so far (saturates at [`CORR_ROWS`]).
    pub fn filled(&self) -> usize {
        self.filled
    }

    /// Rows committed since the last clear, unsaturated.
    pub fn committed(&self) -> u64 {
        self.committed
    }

    /// The most recent decoded frame's outcome counts.
    pub fn last_tally(&self) -> FrameTally {
        self.last
    }

    /// Frames that reached the demapper without producing ground truth, this
    /// burst — see [`reset_counters`](Self::reset_counters).
    pub fn no_truth(&self) -> u64 {
        self.no_truth
    }

    /// Codewords merged into the most recently committed row.
    ///
    /// Reported on the pane because the union inflates apparent density in
    /// proportion to it: at 7/8 a row is ~10 codewords and the same link looks
    /// ~10x busier than at 1/8, where it is barely one.
    pub fn last_depth(&self) -> usize {
        self.depth_ema.map_or(0, |d| d.round().max(1.0) as usize)
    }

    /// The committed rows in the order [`draw_ring`](Self::draw_ring) paints
    /// them — newest first, going back in codeword index downward.
    ///
    /// Only the `filled` rows actually written are yielded; the untouched
    /// remainder of a partly-filled ring is not, or a fresh map would look like
    /// it had history.
    pub fn rows_in_display_order(&self) -> impl Iterator<Item = &[egui::Color32]> {
        let (head, cols) = (self.head, self.cols);
        (0..self.filled).map(move |i| {
            let phys = (head + CORR_ROWS - 1 - i) % CORR_ROWS;
            &self.pixels[phys * cols..(phys + 1) * cols]
        })
    }
}
