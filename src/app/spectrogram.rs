// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use eframe::egui;

/// Maps a dB value to a waterfall color (thermal palette).
/// Same palette as the vertical waterfall for visual consistency.
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

/// Horizontal scrolling spectrogram display.
///
/// Each column represents one time slice (newest column at x=0, older
/// columns to the right).  Each row represents a frequency bin in the
/// ±freq_delta window centered on the current primary marker.
///
/// `freq_rows` is the vertical resolution of the window; `max_cols` is
/// the number of time slices kept in history and the horizontal
/// resolution of the texture.
pub struct SpectrogramDisplay {
    pub freq_rows: usize,
    pub max_cols: usize,
    /// Pixel buffer in row-major layout: `freq_rows × max_cols`.
    /// Row 0 is the *high* frequency edge of the window; row `freq_rows-1`
    /// is the *low* edge.  Columns are a **ring buffer**: new columns are
    /// written at `head` (the oldest slot); the newest column is at
    /// `(head + max_cols - 1) % max_cols`.
    pixels: Vec<egui::Color32>,
    head: usize,
    /// Number of columns written so far (saturates at `max_cols`).
    filled: usize,
    texture: Option<egui::TextureHandle>,
    /// Physical column indices whose pixels changed and need a partial upload.
    dirty_cols: Vec<usize>,
    pub db_min: f32,
    pub db_max: f32,
    /// Accumulated wall-clock seconds since the last column was committed.
    ///
    /// Columns are emitted at the time-per-pixel rate derived from
    /// `time_range_secs / max_cols`, which lets the user tune how much
    /// real time the pane represents without changing the FFT rate.
    accum_secs: f32,
    secs_per_col: f32,
    /// Most recent FFT slice pushed via `push_spectrum`.  Cached so that
    /// we can re-extract the window when the marker frequency or span
    /// changes without waiting for a fresh FFT frame.
    last_spectrum: Vec<f32>,
}

impl SpectrogramDisplay {
    pub fn new(freq_rows: usize, max_cols: usize, db_min: f32, db_max: f32) -> Self {
        Self {
            freq_rows,
            max_cols,
            pixels: vec![egui::Color32::BLACK; freq_rows * max_cols],
            head: 0,
            filled: 0,
            texture: None,
            dirty_cols: Vec::new(),
            db_min,
            db_max,
            accum_secs: 0.0,
            secs_per_col: 1.0 / 60.0,
            last_spectrum: Vec::new(),
        }
    }

    /// Set the time range covered by the full width of the spectrogram.
    /// The per-column duration is `time_range_secs / max_cols`.
    pub fn set_time_range(&mut self, time_range_secs: f32) {
        let secs = time_range_secs.max(0.1);
        self.secs_per_col = secs / self.max_cols as f32;
    }

    /// Clear history (e.g. on source switch).
    pub fn clear(&mut self) {
        for p in &mut self.pixels {
            *p = egui::Color32::BLACK;
        }
        self.head = 0;
        self.filled = 0;
        self.accum_secs = 0.0;
        // Force a full re-upload of the cleared buffer on the next update.
        self.dirty_cols = (0..self.max_cols).collect();
    }

    /// Feed a new full FFT slice (positive-frequency dB bins) together
    /// with the current viewport parameters.  A new column is committed
    /// whenever the accumulated wall-clock time reaches `secs_per_col`.
    ///
    /// `dt_secs` is the elapsed time since the previous `push_spectrum`
    /// call.  `center_hz` and `delta_hz` define the frequency window
    /// mapped vertically onto the pane (high frequencies at the top).
    /// `nyquist` is used to convert the FFT bin index back to Hz.
    pub fn push_spectrum(
        &mut self,
        spectrum_db: &[f32],
        dt_secs: f32,
        center_hz: f32,
        delta_hz: f32,
        nyquist: f32,
    ) {
        if spectrum_db.is_empty() {
            return;
        }
        if self.last_spectrum.len() != spectrum_db.len() {
            self.last_spectrum = spectrum_db.to_vec();
        } else {
            self.last_spectrum.copy_from_slice(spectrum_db);
        }

        self.accum_secs += dt_secs;
        while self.accum_secs >= self.secs_per_col {
            self.accum_secs -= self.secs_per_col;
            self.commit_column(center_hz, delta_hz, nyquist);
        }
    }

    /// Build one column from `last_spectrum` and write it into the ring at
    /// `head` (no per-pixel shift), then advance `head`.
    fn commit_column(&mut self, center_hz: f32, delta_hz: f32, nyquist: f32) {
        if self.last_spectrum.is_empty() {
            return;
        }
        let rows = self.freq_rows;
        let cols = self.max_cols;
        let bins = self.last_spectrum.len();
        let col = self.head;

        // Map row index → frequency.  Row 0 = hi edge, row (rows-1) = lo edge.
        let lo = (center_hz - delta_hz).max(0.0);
        let hi = (center_hz + delta_hz).min(nyquist);
        let span = (hi - lo).max(1.0);
        // Fractional bin position for a frequency.  When the window spans more
        // FFT bins than the pane has rows, each row covers a *range* of bins,
        // so we take the peak (max dB) over that range — this preserves narrow
        // tones instead of subsampling them to a thin, dim line.
        let hz_to_binf = |hz: f32| (hz / nyquist) * (bins - 1) as f32;
        let max_bin = (bins - 1) as f32;

        for r in 0..rows {
            // This row spans frequencies [hz_lo_r, hz_hi_r]; take the peak bin
            // over that band.
            let t_hi = (r as f32 - 0.5).max(0.0) / (rows - 1).max(1) as f32;
            let t_lo = (r as f32 + 0.5).min((rows - 1) as f32) / (rows - 1).max(1) as f32;
            let hz_hi_r = hi - t_hi * span;
            let hz_lo_r = hi - t_lo * span;
            let b0 = hz_to_binf(hz_lo_r).clamp(0.0, max_bin);
            let b1 = hz_to_binf(hz_hi_r).clamp(0.0, max_bin);
            let (lo_b, hi_b) = (b0.floor() as usize, b1.ceil() as usize);
            let mut db = f32::MIN;
            for b in lo_b..=hi_b.min(bins - 1) {
                db = db.max(self.last_spectrum[b]);
            }
            self.pixels[r * cols + col] = db_to_color(db, self.db_min, self.db_max);
        }

        self.dirty_cols.push(col);
        self.head = (self.head + 1) % cols;
        self.filled = (self.filled + 1).min(cols);
    }

    /// Upload changed columns to the GPU texture.  Uploads only the columns
    /// committed since the last call (one `1×freq_rows` strip each), not the
    /// whole texture.
    pub fn update_texture(&mut self, ctx: &egui::Context) {
        if self.texture.is_none() {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [self.max_cols, self.freq_rows],
                &vec![0u8; self.max_cols * self.freq_rows * 4],
            );
            self.texture =
                Some(ctx.load_texture("spectrogram", image, egui::TextureOptions::NEAREST));
        }
        let Some(tex) = &mut self.texture else {
            return;
        };
        let cols = self.max_cols;
        for &col in &self.dirty_cols {
            // Gather the strided column into a contiguous 1×freq_rows strip.
            let rgba: Vec<u8> = (0..self.freq_rows)
                .flat_map(|r| {
                    let c = self.pixels[r * cols + col];
                    [c.r(), c.g(), c.b(), 255]
                })
                .collect();
            let strip = egui::ColorImage::from_rgba_unmultiplied([1, self.freq_rows], &rgba);
            tex.set_partial([col, 0], strip, egui::TextureOptions::NEAREST);
        }
        self.dirty_cols.clear();
    }

    /// Draw the spectrogram into `rect`, mapping the column ring newest-at-left.
    ///
    /// The ring is displayed as two UV quads split at `head`: physical columns
    /// `head-1..0` fill the left band and `max_cols-1..head` the right band,
    /// each with its texture-U flipped so newer columns sit further left.  The
    /// frequency (Y) axis is full-height and un-flipped (row 0 = hi freq = top).
    pub fn draw_ring(&self, painter: &egui::Painter, rect: egui::Rect) {
        if self.texture.is_none() {
            return;
        }
        let tex = self.texture.as_ref().unwrap().id();
        let c = self.max_cols as f32;
        let split = self.head as f32 / c;

        // Left band: screen x [0, split], physical cols head-1..0.
        // Texture U: left edge = split, right edge = 0 (flipped).
        if self.head > 0 {
            let scr = egui::Rect::from_min_max(
                egui::pos2(rect.left(), rect.top()),
                egui::pos2(rect.left() + split * rect.width(), rect.bottom()),
            );
            super::utils::image_quad(painter, tex, scr, [split, 0.0], [0.0, 1.0]);
        }

        // Right band: screen x [split, 1], physical cols max_cols-1..head.
        // Texture U: left edge = 1, right edge = split (flipped).
        if self.head < self.max_cols {
            let scr = egui::Rect::from_min_max(
                egui::pos2(rect.left() + split * rect.width(), rect.top()),
                egui::pos2(rect.right(), rect.bottom()),
            );
            super::utils::image_quad(painter, tex, scr, [1.0, split], [0.0, 1.0]);
        }
    }
}
