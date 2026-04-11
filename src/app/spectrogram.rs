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
    /// is the *low* edge.  Column 0 is the newest slice; column
    /// `max_cols-1` is the oldest.
    pixels: Vec<egui::Color32>,
    current_cols: usize,
    texture: Option<egui::TextureHandle>,
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
            current_cols: 0,
            texture: None,
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
        self.current_cols = 0;
        self.accum_secs = 0.0;
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

    /// Build one column from `last_spectrum` and prepend it at x=0,
    /// shifting older columns one pixel to the right.
    fn commit_column(&mut self, center_hz: f32, delta_hz: f32, nyquist: f32) {
        if self.last_spectrum.is_empty() {
            return;
        }
        let rows = self.freq_rows;
        let cols = self.max_cols;
        let bins = self.last_spectrum.len();

        // Shift existing columns right by one.  Iterate rows in any order;
        // each row is an independent copy of (cols-1) entries.
        for r in 0..rows {
            let row_start = r * cols;
            self.pixels.copy_within(row_start..row_start + cols - 1, row_start + 1);
        }

        // Map row index → frequency.  Row 0 = hi edge, row (rows-1) = lo edge.
        let lo = (center_hz - delta_hz).max(0.0);
        let hi = (center_hz + delta_hz).min(nyquist);
        let span = (hi - lo).max(1.0);
        let bin_to_hz = |b: usize| b as f32 * nyquist / (bins - 1).max(1) as f32;
        let hz_to_bin = |hz: f32| {
            let b = (hz / nyquist) * (bins - 1) as f32;
            b.round().clamp(0.0, (bins - 1) as f32) as usize
        };

        for r in 0..rows {
            let t = r as f32 / (rows - 1).max(1) as f32;
            // r = 0 → hi, r = rows-1 → lo
            let hz = hi - t * span;
            let bin = hz_to_bin(hz);
            let db = self.last_spectrum[bin];
            self.pixels[r * cols] = db_to_color(db, self.db_min, self.db_max);
            let _ = bin_to_hz; // silence unused warning if ever refactored
        }

        self.current_cols = (self.current_cols + 1).min(cols);
    }

    /// Upload the pixel buffer to a GPU texture. Call once per frame.
    pub fn update_texture(&mut self, ctx: &egui::Context) {
        if self.current_cols == 0 {
            return;
        }
        let rgba: Vec<u8> = self
            .pixels
            .iter()
            .flat_map(|c| [c.r(), c.g(), c.b(), 255])
            .collect();
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [self.max_cols, self.freq_rows],
            &rgba,
        );
        match &mut self.texture {
            Some(h) => h.set(image, egui::TextureOptions::NEAREST),
            None => {
                self.texture = Some(ctx.load_texture(
                    "spectrogram",
                    image,
                    egui::TextureOptions::NEAREST,
                ));
            }
        }
    }

    pub fn texture_handle(&self) -> Option<&egui::TextureHandle> {
        self.texture.as_ref()
    }
}
