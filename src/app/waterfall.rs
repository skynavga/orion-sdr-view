// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use eframe::egui;

/// Maps a dB value to a waterfall color (thermal palette).
/// db_min → black/deep blue, db_max → yellow/white.
fn db_to_color(db: f32, db_min: f32, db_max: f32) -> egui::Color32 {
    let t = ((db - db_min) / (db_max - db_min)).clamp(0.0, 1.0);
    let (r, g, b) = if t < 0.33 {
        let s = t / 0.33;
        (0, 0, (s * 255.0) as u8)
    } else if t < 0.66 {
        let s = (t - 0.33) / 0.33;
        (0, (s * 255.0) as u8, (255.0 * (1.0 - s)) as u8)
    } else {
        let s = (t - 0.66) / 0.34;
        ((s * 255.0) as u8, 255, 0)
    };
    egui::Color32::from_rgb(r, g, b)
}

/// Default vertical scroll rate: rows committed per wall-clock second.
/// 60 rows/s matches the legacy one-row-per-frame feel at 60 fps, but is now
/// frame-rate independent.
const DEFAULT_ROWS_PER_SEC: f32 = 60.0;

/// Scrolling 2D spectrogram display (vertical waterfall).
///
/// Implemented as a **row ring buffer**: new rows are written at a rotating
/// `head` index instead of shifting every pixel each frame, so only the single
/// new row is uploaded to the GPU (via `TextureHandle::set_partial`).  The ring
/// is rendered newest-at-top by splitting the draw into two UV-mapped quads at
/// the `head` seam.
///
/// Row commits are paced by wall-clock time (`secs_per_row`), not the frame
/// rate, so the scroll speed is stable regardless of how fast the app repaints.
pub struct WaterfallDisplay {
    pub freq_bins: usize,
    pub max_rows: usize,
    /// Pixel ring buffer: `max_rows × freq_bins`, indexed by physical row.
    /// `head` is the next slot to write (the oldest row); the newest row is at
    /// `(head + max_rows - 1) % max_rows`.
    pixels: Vec<egui::Color32>,
    head: usize,
    /// Number of rows written so far (saturates at `max_rows`).
    filled: usize,
    texture: Option<egui::TextureHandle>,
    /// Physical row indices whose pixels changed and need a partial upload.
    dirty_rows: Vec<usize>,
    pub db_min: f32,
    pub db_max: f32,
    /// Most recent spectrum, cached so wall-clock pacing can emit rows between
    /// FFT frames if needed.
    last_spectrum: Vec<f32>,
    accum_secs: f32,
    secs_per_row: f32,
}

impl WaterfallDisplay {
    pub fn new(freq_bins: usize, max_rows: usize, db_min: f32, db_max: f32) -> Self {
        Self {
            freq_bins,
            max_rows,
            pixels: vec![egui::Color32::BLACK; freq_bins * max_rows],
            head: 0,
            filled: 0,
            texture: None,
            dirty_rows: Vec::new(),
            db_min,
            db_max,
            last_spectrum: Vec::new(),
            accum_secs: 0.0,
            secs_per_row: 1.0 / DEFAULT_ROWS_PER_SEC,
        }
    }

    /// Clear history (e.g. on source switch).  Resets the ring in place and
    /// forces a full re-upload of the cleared buffer on the next update, so no
    /// stale rows from the previous source linger.
    pub fn clear(&mut self) {
        for p in &mut self.pixels {
            *p = egui::Color32::BLACK;
        }
        self.head = 0;
        self.filled = 0;
        self.accum_secs = 0.0;
        self.last_spectrum.clear();
        self.dirty_rows = (0..self.max_rows).collect();
    }

    /// Feed the latest FFT slice and advance the scroll by `dt_secs` of
    /// wall-clock time.  Commits one or more rows when enough time has elapsed.
    pub fn push_row(&mut self, spectrum_db: &[f32], dt_secs: f32) {
        if spectrum_db.is_empty() {
            return;
        }
        if self.last_spectrum.len() != spectrum_db.len() {
            self.last_spectrum = spectrum_db.to_vec();
        } else {
            self.last_spectrum.copy_from_slice(spectrum_db);
        }

        self.accum_secs += dt_secs;
        // Cap the number of rows emitted in a single call so a long stall (e.g.
        // after a pause) doesn't spin the whole ring at once.
        let mut budget = self.max_rows;
        while self.accum_secs >= self.secs_per_row && budget > 0 {
            self.accum_secs -= self.secs_per_row;
            budget -= 1;
            self.commit_row();
        }
    }

    /// Write `last_spectrum` into the ring at `head` and advance.
    fn commit_row(&mut self) {
        let n = self.last_spectrum.len().min(self.freq_bins);
        let base = self.head * self.freq_bins;
        for (slot, &db) in self.pixels[base..base + self.freq_bins]
            .iter_mut()
            .zip(self.last_spectrum.iter())
            .take(n)
        {
            *slot = db_to_color(db, self.db_min, self.db_max);
        }
        for slot in &mut self.pixels[base + n..base + self.freq_bins] {
            *slot = egui::Color32::BLACK;
        }
        self.dirty_rows.push(self.head);
        self.head = (self.head + 1) % self.max_rows;
        self.filled = (self.filled + 1).min(self.max_rows);
    }

    /// Upload changed rows to the GPU texture.  Uploads only the rows committed
    /// since the last call (one 1×freq_bins strip each), not the whole texture.
    pub fn update_texture(&mut self, ctx: &egui::Context) {
        // Ensure the texture exists (full black image on first use).
        if self.texture.is_none() {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [self.freq_bins, self.max_rows],
                &vec![0u8; self.freq_bins * self.max_rows * 4],
            );
            self.texture =
                Some(ctx.load_texture("waterfall", image, egui::TextureOptions::NEAREST));
        }
        let Some(tex) = &mut self.texture else {
            return;
        };
        for &row in &self.dirty_rows {
            let base = row * self.freq_bins;
            let rgba: Vec<u8> = self.pixels[base..base + self.freq_bins]
                .iter()
                .flat_map(|c| [c.r(), c.g(), c.b(), 255])
                .collect();
            let strip = egui::ColorImage::from_rgba_unmultiplied([self.freq_bins, 1], &rgba);
            tex.set_partial([0, row], strip, egui::TextureOptions::NEAREST);
        }
        self.dirty_rows.clear();
    }

    /// Draw the waterfall into `rect`, cropping the frequency (X) axis to the
    /// UV window `[lo_uv, hi_uv]` and mapping the row ring newest-at-top.
    ///
    /// The ring is displayed as two UV quads split at `head`: physical rows
    /// `head-1..0` fill the top band and `max_rows-1..head` the bottom band,
    /// each with its texture-V flipped so newer rows sit higher on screen.
    pub fn draw_cropped(&self, painter: &egui::Painter, rect: egui::Rect, lo_uv: f32, hi_uv: f32) {
        let Some(tex) = &self.texture else {
            return;
        };
        let r = self.max_rows as f32;
        let head = self.head as f32;
        // Screen-Y fraction where the top (newer) segment ends.  The top band
        // holds `head` rows (physical head-1..0).
        let split = head / r;

        // Top band: screen y [0, split], physical rows head-1..0.
        // Texture V: top edge = split, bottom edge = 0 (flipped).
        if self.head > 0 {
            let scr = egui::Rect::from_min_max(
                egui::pos2(rect.left(), rect.top()),
                egui::pos2(rect.right(), rect.top() + split * rect.height()),
            );
            super::utils::image_quad(painter, tex.id(), scr, [lo_uv, hi_uv], [split, 0.0]);
        }

        // Bottom band: screen y [split, 1], physical rows max_rows-1..head.
        // Texture V: top edge = 1, bottom edge = split (flipped).
        if self.head < self.max_rows {
            let scr = egui::Rect::from_min_max(
                egui::pos2(rect.left(), rect.top() + split * rect.height()),
                egui::pos2(rect.right(), rect.bottom()),
            );
            super::utils::image_quad(painter, tex.id(), scr, [lo_uv, hi_uv], [1.0, split]);
        }
    }

    /// Expose the texture handle (used to gate the fallback draw path).
    pub fn texture_handle(&self) -> Option<&egui::TextureHandle> {
        self.texture.as_ref()
    }

    /// How many rows have been committed (saturates at `max_rows`).
    pub fn filled(&self) -> usize {
        self.filled
    }

    /// The committed rows in the order [`draw_cropped`](Self::draw_cropped)
    /// paints them, top of the pane first — so **newest first**, going back in
    /// time downward, which is what makes it a waterfall.
    ///
    /// Only the `filled` rows that have actually been written are yielded; the
    /// black remainder of a partly-filled ring is not.
    ///
    /// This exists so the ring arithmetic is assertable with no renderer: the
    /// pixels are CPU-side, and resolving the two-quad seam at `head` back into
    /// one ordered sequence is precisely the logic worth a test.  Reading it
    /// through this method rather than a `pub pixels` keeps the physical layout
    /// an implementation detail.
    pub fn rows_in_display_order(&self) -> impl Iterator<Item = &[egui::Color32]> {
        let (head, rows, bins) = (self.head, self.max_rows, self.freq_bins);
        (0..self.filled).map(move |i| {
            let phys = (head + rows - 1 - i) % rows;
            &self.pixels[phys * bins..(phys + 1) * bins]
        })
    }
}
