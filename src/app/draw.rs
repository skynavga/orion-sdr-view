// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use eframe::egui;

use crate::decode::DecodeResult;
use super::{PANE_BG, DecodeBarMode, SourceMode, WaterfallMode};
use super::view::ViewApp;
use crate::source::ft8::{Ft8Mode, Ft8MsgType};

impl ViewApp {
    pub(super) fn draw_hud(&self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("hud").show(ctx, |ui| {
            // Build the status string first so we can centre it.
            let center  = self.freq_view.center_hz;
            let span    = self.freq_view.span_hz;
            let zoom_ratio = self.freq_view.zoom_ratio();
            let center_str = if center >= 1000.0 {
                format!("{:.2}kHz", center / 1000.0)
            } else {
                format!("{:.0}Hz", center)
            };
            let span_str = if span >= 1000.0 {
                format!("{:.1}kHz", span / 1000.0)
            } else {
                format!("{:.0}Hz", span)
            };
            let zoom_str = if (zoom_ratio - 1.0).abs() < 0.01 {
                "1×".to_owned()
            } else {
                format!("{:.1}×", zoom_ratio)
            };
            // Format a single marker frequency label, bracketed if active.
            let fmt_hz = |hz: f32| -> String {
                if hz >= 1000.0 { format!("{:.2}kHz", hz / 1000.0) }
                else            { format!("{:.0}Hz", hz) }
            };
            let m_a = &self.markers[1];
            let m_b = &self.markers[2];
            let mut marker_str = String::new();
            if m_a.enabled {
                let tag = if self.active_marker == Some(1) { "[A]" } else { "A" };
                marker_str.push_str(&format!("  {} {}", tag, fmt_hz(m_a.hz)));
            }
            if m_b.enabled {
                let tag = if self.active_marker == Some(2) { "[B]" } else { "B" };
                marker_str.push_str(&format!("  {} {}", tag, fmt_hz(m_b.hz)));
            }
            if m_a.enabled && m_b.enabled {
                let diff = m_b.hz - m_a.hz;
                marker_str.push_str(&format!("  B-A {}", fmt_hz(diff.abs())));
            }
            // Active mode flags: only show user-togglable states.
            // C = amplitude cycling (Test Tone only; AM DSB loops structurally, not flagged).
            let cycling = self.source_mode == SourceMode::TestTone && self.signal_gen.cycling;
            let modes: String = {
                let mut flags = Vec::new();
                if cycling                            { flags.push("C"); }
                if self.decode_bar == DecodeBarMode::Info  { flags.push("Di"); }
                if self.decode_bar == DecodeBarMode::Text  { flags.push("Dt"); }
                if self.envelope_visible              { flags.push("E"); }
                if self.source_locked      { flags.push("L"); }
                if self.peak_hold_visible  { flags.push("P"); }
                if flags.is_empty() { String::new() }
                else { format!(" ({})", flags.join(",")) }
            };
            let submode_str: String = match self.source_mode {
                SourceMode::AmDsb => match self.settings.am_audio_str() {
                    "Voice"  => "  aud v".to_owned(),
                    "Custom" => "  aud c".to_owned(),
                    _        => "  aud m".to_owned(),
                },
                SourceMode::Psk31 => {
                    let mode_ch = match self.settings.psk31_mode_str() {
                        "QPSK31" => "q",
                        _        => "b",
                    };
                    let msg_ch = match self.settings.psk31_msg_mode_str() {
                        "Custom" => "c",
                        _        => "n",
                    };
                    format!("  mode {mode_ch}  msg {msg_ch}")
                }
                SourceMode::Ft8 => {
                    let mode_ch = match self.ft_mode { Ft8Mode::Ft8 => "8", Ft8Mode::Ft4 => "4" };
                    let msg_ch  = match self.ft_msg_type { Ft8MsgType::Standard => "s", Ft8MsgType::FreeText => "f" };
                    format!("  mode {mode_ch}  msg {msg_ch}")
                }
                _ => String::new(),
            };
            let status = format!(
                "{}{}{}  ctr {}  span {}  zoom {}  ref {:.0}dB{}",
                self.source_mode.label(), modes, submode_str, center_str, span_str, zoom_str, self.db_max, marker_str
            );

            // Three-section bar: left (title/hints) | centre (status) | —
            // The status is painted via the raw painter at the panel's horizontal
            // midpoint so it aligns with the primary marker in the panes below.
            let panel_rect = ui.max_rect();
            let mid_x = panel_rect.center().x;
            let mid_y = panel_rect.center().y;

            // Paint all three sections via the raw painter so none can overlap.
            let right_x = panel_rect.right() - 6.0;
            ui.painter().text(
                egui::pos2(mid_x, mid_y),
                egui::Align2::CENTER_CENTER,
                &status,
                self.mono_font_id.clone(),
                egui::Color32::from_rgb(0, 200, 255),
            );
            ui.painter().text(
                egui::pos2(right_x, mid_y),
                egui::Align2::RIGHT_CENTER,
                "? help",
                self.mono_font_id.clone(),
                egui::Color32::GRAY,
            );

            ui.horizontal(|ui| {
                // ── Left: title only ──────────────────────────────────────
                ui.label(
                    egui::RichText::new("orion-sdr-view")
                        .font(self.mono_font_id.clone())
                        .strong(),
                );
            });
        });
    }

    pub(super) fn draw_panes(&self, ui: &mut egui::Ui) {
        let visible_count = self.pane_visible.iter().filter(|&&v| v).count();

        let avail = ui.available_rect_before_wrap();
        let pane_total_h = avail.height();

        if visible_count > 0 {
            let total_frac: f32 = self
                .pane_visible
                .iter()
                .zip(self.pane_frac.iter())
                .map(|(&vis, &f)| if vis { f } else { 0.0 })
                .sum();

            let mut y = avail.top();
            for (i, &bg) in PANE_BG.iter().enumerate() {
                if !self.pane_visible[i] {
                    continue;
                }
                let h = (self.pane_frac[i] / total_frac) * pane_total_h;
                let rect = egui::Rect::from_min_size(
                    egui::pos2(avail.left(), y),
                    egui::vec2(avail.width(), h),
                );
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, 0.0, bg);
                match i {
                    0 => self.draw_spectrum(&painter, rect),
                    1 => self.draw_persistence_pane(&painter, rect),
                    _ => self.draw_waterfall_pane(&painter, rect),
                }
                y += h;
            }
        }

    }

    pub(super) fn draw_decode_bar(&self, painter: egui::Painter, rect: egui::Rect) {
        const BAR_BG:    egui::Color32 = egui::Color32::from_rgb(15, 15, 30);
        const LABEL_COL: egui::Color32 = egui::Color32::from_rgb(80, 100, 140);
        const TEXT_COL:  egui::Color32 = egui::Color32::from_rgb(200, 200, 200);
        const DIM_COL:   egui::Color32 = egui::Color32::from_rgb(100, 100, 100);

        painter.rect_filled(rect, 0.0, BAR_BG);

        let font   = egui::FontId::new(12.0, egui::FontFamily::Monospace);
        let text_y = rect.center().y;

        // Dim mode label on the left: "DEC·i" = info, "DEC·t" = text.
        let dec_label = match self.decode_bar {
            DecodeBarMode::Info => "DEC\u{b7}i",
            DecodeBarMode::Text => "DEC\u{b7}t",
            DecodeBarMode::Off  => "DEC",
        };
        painter.text(
            egui::pos2(rect.left() + 6.0, text_y),
            egui::Align2::LEFT_CENTER,
            dec_label,
            font.clone(),
            LABEL_COL,
        );

        let label_w = painter.layout_no_wrap(dec_label.to_owned(), font.clone(), LABEL_COL).size().x;
        let content_x = rect.left() + 6.0 + label_w + 12.0; // 12 px right margin

        // Measure loop timer label so we know where the scrolling text must stop.
        let timer_label = self.loop_timer.label();
        let timer_w = painter.layout_no_wrap(timer_label.clone(), font.clone(), TEXT_COL).size().x;
        let em_w    = painter.layout_no_wrap("M".to_owned(),       font.clone(), TEXT_COL).size().x;

        // For FT8/FT4 sources, show "frm xxx err yyy" to the left of the loop timer.
        let ft_label: Option<String> = if self.source_mode == SourceMode::Ft8 {
            Some(format!("frm {:03} err {:03} ", self.ft_frame_count, self.ft_err_count))
        } else {
            None
        };
        let ft_label_w = ft_label.as_ref().map_or(0.0, |s| {
            painter.layout_no_wrap(s.clone(), font.clone(), TEXT_COL).size().x
        });

        // Right-aligned loop timer: "sig 12.34s loop 007" / "gap 02.00s loop 007"
        let timer_x = rect.right() - 6.0;
        let timer_left = timer_x - timer_w - ft_label_w;
        // Right boundary for the scrolling text region: one 'M'-width gap before the timer block.
        let scroll_right = timer_left - em_w;

        // Build the static (non-scrolling) text for Di mode and fallback states.
        let info_str = |modulation: &str, center_hz: f32, bw_hz: f32, snr_db: f32| -> String {
            let bw_str = if bw_hz.is_nan() || bw_hz <= 0.0 {
                "\u{2014}".to_owned()
            } else if bw_hz >= 1000.0 {
                format!("{:.1}kHz", bw_hz / 1000.0)
            } else {
                format!("{:.0}Hz", bw_hz)
            };
            format!("{modulation}  ctr {:.1}kHz  bw {bw_str}  snr {snr_db:.1}dB",
                center_hz / 1000.0)
        };

        // Determine whether Dt mode has content to render.
        let has_dt_text = self.decode_bar == DecodeBarMode::Text
            && !self.decode_ticker.visible.is_empty();

        if has_dt_text {
            // ── Smooth-scrolling ticker (Dt mode) ─────────────────────────
            // Text is right-aligned at scroll_right.  sub_px shifts the text
            // smoothly leftward as the next character slides in; when sub_px
            // crosses a character width, a new character appears at the right.
            let sub_px = self.decode_ticker.sub_px;
            let text_x = scroll_right + (em_w - sub_px);

            let clip_rect = egui::Rect::from_min_max(
                egui::pos2(content_x, rect.min.y),
                egui::pos2(scroll_right, rect.max.y),
            );
            let clipped = painter.with_clip_rect(clip_rect);
            clipped.text(
                egui::pos2(text_x, text_y),
                egui::Align2::RIGHT_CENTER,
                &self.decode_ticker.visible,
                font.clone(),
                TEXT_COL,
            );
        } else {
            // ── Static left-aligned text (Di mode or waiting) ────────────────
            let (text, color) = if self.decode_bar == DecodeBarMode::Info {
                // Di mode: show last_info if available.
                if let Some(DecodeResult::Info { modulation, center_hz, bw_hz, snr_db })
                    = &self.decode_ticker.last_info
                {
                    (info_str(modulation, *center_hz, *bw_hz, *snr_db), TEXT_COL)
                } else {
                    ("waiting for signal\u{2026}".to_owned(), DIM_COL)
                }
            } else {
                // Dt mode with no visible text yet: just show waiting.
                ("waiting for signal\u{2026}".to_owned(), DIM_COL)
            };
            painter.text(
                egui::pos2(content_x, text_y),
                egui::Align2::LEFT_CENTER,
                text,
                font.clone(),
                color,
            );
        }

        // Paint FT counter prefix (if any) then the loop timer.
        if let Some(ref ft_str) = ft_label {
            painter.text(
                egui::pos2(timer_left, text_y),
                egui::Align2::LEFT_CENTER,
                ft_str,
                font.clone(),
                TEXT_COL,
            );
        }
        painter.text(
            egui::pos2(timer_x, text_y),
            egui::Align2::RIGHT_CENTER,
            timer_label,
            font,
            TEXT_COL,
        );
    }

    pub(super) fn draw_spectrum(&self, painter: &egui::Painter, rect: egui::Rect) {
        let bins = &self.spectrum.fft_out_db;
        let n = bins.len();
        if n < 2 {
            return;
        }

        let lo = self.freq_view.lo();
        let hi = self.freq_view.hi();
        let nyquist = self.freq_view.nyquist;

        // bin index → Hz
        let bin_hz = |b: usize| b as f32 * nyquist / (n - 1) as f32;

        // Hz → X pixel (within visible window)
        let x_for_hz = |hz: f32| {
            rect.left() + self.freq_view.hz_to_x_norm(hz) * rect.width()
        };
        let y_for_db = |db: f32| {
            let t = (db - self.db_min) / (self.db_max - self.db_min);
            rect.bottom() - t.clamp(0.0, 1.0) * rect.height()
        };

        // ── Horizontal dB grid lines ──────────────────────────────────────
        let grid_stroke = egui::Stroke::new(0.5, egui::Color32::from_gray(45));
        let label_font = egui::FontId::new(10.0, egui::FontFamily::Monospace);
        let mut db = (self.db_min / 10.0).ceil() * 10.0;
        while db <= self.db_max {
            let y = y_for_db(db);
            painter.hline(rect.x_range(), y, grid_stroke);
            painter.text(
                egui::pos2(rect.left() + 4.0, y - 2.0),
                egui::Align2::LEFT_BOTTOM,
                format!("{:.0}dB", db),
                label_font.clone(),
                egui::Color32::from_gray(110),
            );
            db += 10.0;
        }

        // ── Vertical frequency grid lines + labels ────────────────────────
        // Choose a nice grid step based on visible span.
        let span = hi - lo;
        let raw_step = span / 5.0;
        let magnitude = 10f32.powf(raw_step.log10().floor());
        let norm = raw_step / magnitude;
        let nice = if norm < 1.5 { 1.0 } else if norm < 3.5 { 2.0 } else if norm < 7.5 { 5.0 } else { 10.0 };
        let grid_hz = nice * magnitude;

        let first_grid = (lo / grid_hz).ceil() * grid_hz;
        let mut hz = first_grid;
        // dB labels occupy ~50 px at the bottom-left; keep freq labels clear of them.
        let db_label_clearance = 52.0_f32;
        while hz <= hi + 0.5 {
            let x = x_for_hz(hz);
            if x >= rect.left() - 1.0 && x <= rect.right() + 1.0 {
                painter.vline(x, rect.y_range(), grid_stroke);
                let label = if hz >= 1000.0 {
                    format!("{:.1}k", hz / 1000.0)
                } else {
                    format!("{:.0}", hz)
                };
                // Skip freq label if it would overlap the bottom-left dB label area.
                let label_x = x + 3.0;
                if label_x >= rect.left() + db_label_clearance {
                    painter.text(
                        egui::pos2(label_x, rect.bottom() - 14.0),
                        egui::Align2::LEFT_BOTTOM,
                        label,
                        label_font.clone(),
                        egui::Color32::from_gray(110),
                    );
                }
            }
            hz += grid_hz;
        }

        // ── Spectrum line (visible bins only) ─────────────────────────────
        let mut points: Vec<egui::Pos2> = Vec::new();
        for (b, &db) in bins.iter().enumerate().take(n) {
            let hz = bin_hz(b);
            if hz < lo || hz > hi {
                if !points.is_empty() {
                    painter.line(
                        std::mem::take(&mut points),
                        egui::Stroke::new(1.5, egui::Color32::from_rgb(0, 220, 180)),
                    );
                }
                continue;
            }
            points.push(egui::pos2(x_for_hz(hz), y_for_db(db)));
        }
        if !points.is_empty() {
            painter.line(points, egui::Stroke::new(1.5, egui::Color32::from_rgb(0, 220, 180)));
        }

        // ── Peak hold line ────────────────────────────────────────────────
        if self.peak_hold_visible {
            let mut ph_points: Vec<egui::Pos2> = Vec::new();
            for b in 0..n {
                let hz = bin_hz(b);
                if hz < lo || hz > hi {
                    if !ph_points.is_empty() {
                        painter.line(
                            std::mem::take(&mut ph_points),
                            egui::Stroke::new(1.0, egui::Color32::from_rgba_premultiplied(255, 180, 0, 180)),
                        );
                    }
                    continue;
                }
                ph_points.push(egui::pos2(x_for_hz(hz), y_for_db(self.peak_hold[b])));
            }
            if !ph_points.is_empty() {
                painter.line(
                    ph_points,
                    egui::Stroke::new(1.0, egui::Color32::from_rgba_premultiplied(255, 180, 0, 180)),
                );
            }
        }

        // ── Frequency markers ─────────────────────────────────────────────
        self.draw_freq_markers(painter, rect, &label_font);
    }

    /// Draw vertical marker lines into any pane rect.
    pub(super) fn draw_freq_markers(&self, painter: &egui::Painter, rect: egui::Rect, label_font: &egui::FontId) {
        let lo = self.freq_view.lo();
        let hi = self.freq_view.hi();

        // Approximate pixel width of a bracket marker label ("A 12.34k" or "[A 12.34k]").
        // Used to detect overlap when assigning label rows.
        let label_width = 72.0_f32;
        // Each row tracks a list of occupied [left, right] intervals.
        let mut row_intervals: [Vec<(f32, f32)>; 3] = [vec![], vec![], vec![]];

        // Pre-reserve the primary marker's label interval on row 0.
        let primary_x = rect.center().x;
        row_intervals[0].push((primary_x + 3.0, primary_x + 3.0 + label_width));

        // Collect (idx, x, row, label, color, line_width) for each visible marker.
        struct Entry {
            x: f32,
            row: usize,
            label: String,
            color: egui::Color32,
            line_width: f32,
        }
        let mut entries: Vec<Entry> = Vec::new();

        // Primary marker first (always row 0, drawn at pane center).
        {
            let m = &self.markers[0];
            if m.enabled {
                let x = rect.center().x;
                let hz_label = if m.hz >= 1000.0 {
                    format!("{} {:.2}k", m.label(), m.hz / 1000.0)
                } else {
                    format!("{} {:.0}", m.label(), m.hz)
                };
                entries.push(Entry { x, row: 0, label: hz_label,
                    color: m.color(), line_width: 1.0 });
            }
        }

        // Bracket markers: assign rows greedily starting from 0.
        for (idx, m) in self.markers[1..].iter().enumerate() {
            let idx = idx + 1; // real index into self.markers
            if !m.enabled { continue; }
            if m.hz < lo || m.hz > hi { continue; }
            let x = rect.left() + self.freq_view.hz_to_x_norm(m.hz) * rect.width();
            let is_active = self.active_marker == Some(idx);
            let color = if is_active { egui::Color32::WHITE } else { m.color() };
            let line_width = if is_active { 1.5 } else { 1.0 };

            let hz_label = if m.hz >= 1000.0 {
                format!("{} {:.2}k", m.label(), m.hz / 1000.0)
            } else {
                format!("{} {:.0}", m.label(), m.hz)
            };
            let display_label = if is_active { format!("[{}]", hz_label) } else { hz_label };

            // Find the lowest row where this label fits without overlapping any
            // already-placed label interval on that row.
            let label_x = x + 3.0;
            let label_right = label_x + label_width;
            let row = (0..3)
                .find(|&r| row_intervals[r].iter().all(|&(lo, hi)| label_right <= lo || label_x >= hi))
                .unwrap_or(2);
            row_intervals[row].push((label_x, label_right));

            entries.push(Entry { x, row, label: display_label, color, line_width });
        }

        // Draw all markers.
        let dash_len = 8.0;
        let gap_len  = 5.0;
        for e in &entries {
            // Dashed vertical line
            let stroke = egui::Stroke::new(e.line_width, e.color);
            let mut y = rect.top();
            let mut paint = true;
            while y < rect.bottom() {
                let seg_len = if paint { dash_len } else { gap_len };
                let y_end = (y + seg_len).min(rect.bottom());
                if paint {
                    painter.line_segment(
                        [egui::pos2(e.x, y), egui::pos2(e.x, y_end)],
                        stroke,
                    );
                }
                y = y_end;
                paint = !paint;
            }
            // Label
            let label_y = rect.top() + 3.0 + e.row as f32 * 14.0;
            painter.text(
                egui::pos2(e.x + 3.0, label_y),
                egui::Align2::LEFT_TOP,
                &e.label,
                label_font.clone(),
                e.color,
            );
        }
    }

    /// Draw persistence pane with freq zoom UV and markers.
    pub(super) fn draw_persistence_pane(&self, painter: &egui::Painter, rect: egui::Rect) {
        // Draw with UV cropped to visible frequency range
        let lo_uv = self.freq_view.lo() / self.freq_view.nyquist;
        let hi_uv = self.freq_view.hi() / self.freq_view.nyquist;

        if let Some(tex) = self.persistence.texture_handle() {
            let uv = egui::Rect::from_min_max(
                egui::pos2(lo_uv, 0.0),
                egui::pos2(hi_uv, 1.0),
            );
            painter.image(tex.id(), rect, uv, egui::Color32::WHITE);
        } else {
            self.persistence.draw(painter, rect, self.envelope_visible);
            return;
        }

        if self.envelope_visible {
            self.persistence.draw_envelope_cropped(painter, rect, lo_uv, hi_uv);
        }

        let label_font = egui::FontId::new(10.0, egui::FontFamily::Monospace);
        self.draw_freq_markers(painter, rect, &label_font);
    }

    /// Draw pane 3: either the vertical waterfall or the horizontal
    /// spectrogram, depending on `waterfall_mode` (cycled by `W`).
    pub(super) fn draw_waterfall_pane(&self, painter: &egui::Painter, rect: egui::Rect) {
        match self.waterfall_mode {
            WaterfallMode::Vertical   => self.draw_vertical_waterfall(painter, rect),
            WaterfallMode::Horizontal => self.draw_horizontal_spectrogram(painter, rect),
        }
    }

    fn draw_vertical_waterfall(&self, painter: &egui::Painter, rect: egui::Rect) {
        let lo_uv = self.freq_view.lo() / self.freq_view.nyquist;
        let hi_uv = self.freq_view.hi() / self.freq_view.nyquist;

        if let Some(tex) = self.waterfall.texture_handle() {
            let uv = egui::Rect::from_min_max(
                egui::pos2(lo_uv, 0.0),
                egui::pos2(hi_uv, 1.0),
            );
            painter.image(tex.id(), rect, uv, egui::Color32::WHITE);
        } else {
            self.waterfall.draw(painter, rect);
            return;
        }

        let label_font = egui::FontId::new(10.0, egui::FontFamily::Monospace);
        self.draw_freq_markers(painter, rect, &label_font);
    }

    /// Horizontal spectrogram: frequency on the y-axis, ±spec_freq_delta
    /// centered on the primary marker, with high frequencies at the top
    /// and low at the bottom.  Time on the x-axis, newest at the left
    /// (x=0) and oldest at the right.  The full width spans
    /// `spec_time_range_secs` seconds.
    fn draw_horizontal_spectrogram(&self, painter: &egui::Painter, rect: egui::Rect) {
        // Paint the texture.  The SpectrogramDisplay stores row 0 = hi freq,
        // col 0 = newest, so a straight full-UV draw already matches our
        // desired screen orientation.
        if let Some(tex) = self.spectrogram.texture_handle() {
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            painter.image(tex.id(), rect, uv, egui::Color32::WHITE);
        }

        let label_font = egui::FontId::new(10.0, egui::FontFamily::Monospace);
        let grid_stroke = egui::Stroke::new(0.5, egui::Color32::from_gray(45));

        // ── Frequency window ─────────────────────────────────────────────
        let center_hz = self.markers[0].hz;
        let delta_hz  = self.settings.spec_freq_delta_hz();
        let nyquist   = self.freq_view.nyquist;
        let f_lo = (center_hz - delta_hz).max(0.0);
        let f_hi = (center_hz + delta_hz).min(nyquist);
        let f_span = (f_hi - f_lo).max(1.0);

        let y_for_hz = |hz: f32| -> f32 {
            // hi → top, lo → bottom
            rect.top() + (f_hi - hz) / f_span * rect.height()
        };

        // ── Horizontal frequency grid + labels ──────────────────────────
        // Three labels inside the pane at center±delta/2 and center.  The
        // pane itself still spans the full ±delta window vertically; we
        // just place the gridlines/labels away from the top and bottom
        // edges so they don't collide with pane borders or the time-axis
        // "now/-Ns" row.
        let fmt_hz = |hz: f32| -> String {
            if hz >= 1000.0 { format!("{:.2}k", hz / 1000.0) }
            else            { format!("{:.0}", hz) }
        };
        let label_col = egui::Color32::from_gray(130);
        let hi_half = center_hz + delta_hz * 0.5;
        let lo_half = center_hz - delta_hz * 0.5;
        for hz in [hi_half, center_hz, lo_half] {
            painter.hline(rect.x_range(), y_for_hz(hz), grid_stroke);
            painter.text(
                egui::pos2(rect.left() + 4.0, y_for_hz(hz) - 2.0),
                egui::Align2::LEFT_BOTTOM,
                fmt_hz(hz),
                label_font.clone(),
                label_col,
            );
        }

        // ── Time window ──────────────────────────────────────────────────
        let t_range = self.settings.spec_time_range_secs();
        let x_for_t = |t: f32| -> f32 {
            // t=0 (now) → left, t=t_range → right
            rect.left() + (t / t_range) * rect.width()
        };
        // Vertical time grid: nice step 1/2/5/10 s.
        let raw_tstep = t_range / 5.0;
        let tmag = 10f32.powf(raw_tstep.log10().floor());
        let tnorm = raw_tstep / tmag;
        let tnice = if tnorm < 1.5 { 1.0 }
                    else if tnorm < 3.5 { 2.0 }
                    else if tnorm < 7.5 { 5.0 }
                    else { 10.0 };
        let grid_t = (tnice * tmag).max(0.1);
        let mut t = 0.0_f32;
        while t <= t_range + 0.001 {
            let x = x_for_t(t);
            painter.vline(x, rect.y_range(), grid_stroke);
            let label = if t < 1.0 && t > 0.0 {
                format!("-{:.1}s", t)
            } else {
                format!("-{:.0}s", t)
            };
            let label_str = if t == 0.0 { "now".to_owned() } else { label };
            // Keep "now" pinned inside the pane; other labels to the right of the line.
            let (lx, align) = if t == 0.0 {
                (x + 3.0, egui::Align2::LEFT_BOTTOM)
            } else {
                (x + 3.0, egui::Align2::LEFT_BOTTOM)
            };
            painter.text(
                egui::pos2(lx, rect.bottom() - 2.0),
                align,
                label_str,
                label_font.clone(),
                egui::Color32::from_gray(130),
            );
            t += grid_t;
        }

        // ── Marker lines ─────────────────────────────────────────────────
        // Primary marker is always at the vertical center of the window
        // (since the window is centered on it).  Draw it as a dashed
        // horizontal line across the full pane for visual anchoring.
        {
            let m = &self.markers[0];
            if m.enabled {
                let y = y_for_hz(m.hz);
                dashed_hline(painter, rect.left(), rect.right(), y, m.color(), 1.0);
                let label = if m.hz >= 1000.0 {
                    format!("{} {:.2}k", m.label(), m.hz / 1000.0)
                } else {
                    format!("{} {:.0}", m.label(), m.hz)
                };
                painter.text(
                    egui::pos2(rect.right() - 4.0, y - 2.0),
                    egui::Align2::RIGHT_BOTTOM,
                    label,
                    label_font.clone(),
                    m.color(),
                );
            }
        }

        // Bracket markers: only draw if their frequency falls inside the
        // ±delta window.
        for (idx, m) in self.markers[1..].iter().enumerate() {
            let idx = idx + 1;
            if !m.enabled { continue; }
            if m.hz < f_lo || m.hz > f_hi { continue; }
            let y = y_for_hz(m.hz);
            let is_active = self.active_marker == Some(idx);
            let color = if is_active { egui::Color32::WHITE } else { m.color() };
            let lw = if is_active { 1.5 } else { 1.0 };
            dashed_hline(painter, rect.left(), rect.right(), y, color, lw);
            let hz_label = if m.hz >= 1000.0 {
                format!("{} {:.2}k", m.label(), m.hz / 1000.0)
            } else {
                format!("{} {:.0}", m.label(), m.hz)
            };
            let display = if is_active { format!("[{}]", hz_label) } else { hz_label };
            painter.text(
                egui::pos2(rect.right() - 4.0, y - 2.0),
                egui::Align2::RIGHT_BOTTOM,
                display,
                label_font.clone(),
                color,
            );
        }
    }

    pub(super) fn draw_help_overlay(&self, ui: &mut egui::Ui) {
        let screen = ui.ctx().content_rect();
        // Width must fit the longest description ("cycle pane 3: waterfall
        // (vertical) / spectrogram (horizontal)") starting after the key
        // column; height must fit the 24 entries + 4 section headers +
        // title + the two-line copyright footer at the bottom.
        let overlay_rect = egui::Rect::from_center_size(
            screen.center(),
            egui::vec2(660.0, 600.0),
        );
        let painter = ui.painter();
        painter.rect_filled(
            overlay_rect,
            8.0,
            egui::Color32::from_rgba_premultiplied(0, 0, 0, 220),
        );
        painter.rect_stroke(
            overlay_rect,
            8.0,
            egui::Stroke::new(1.0, egui::Color32::GRAY),
            egui::StrokeKind::Outside,
        );

        // Each entry: kind 0=title, 1=section, 2=entry(key, description)
        // Entry strings use "\t" to split key column from description column.
        let lines: &[(&str, u8)] = &[
            ("Keyboard shortcuts", 0),
            ("Panes & Sources", 1),
            ("1 / 2 / 3\ttoggle Spectrum / Persistence / Waterfall", 2),
            ("I / M / N\tselect next source / mode / audio or message", 2),
            ("C / E / P\tcycle amplitude  |  envelope  |  peak hold", 2),
            ("L\tlock source freq/carrier to display center", 2),
            ("D\tcycle decode bar: off → info → text → off", 2),
            ("W\tcycle pane 3: waterfall (vertical) / spectrogram (horizontal)", 2),
            ("Frequency Pan / Zoom", 1),
            ("← / →\tpan left / right", 2),
            ("Shift+← / →\tfine pan, snap 100 Hz", 2),
            ("Ctrl+Shift+← / →\textra-fine pan, snap 10 Hz", 2),
            ("↑ / ↓ | Shift+↑ / ↓\tzoom | fine zoom (in / out)", 2),
            ("[ / ]\tref level ±5 dB", 2),
            ("R\treset to full view (0 – Nyquist)", 2),
            ("Markers", 1),
            ("A / B (shift)\tplace marker A / B at center, select it", 2),
            ("a / b\ttoggle marker A / B; select when enabling", 2),
            ("Tab\tcycle active marker: A → B → none", 2),
            ("Ctrl+← / →\tmove active marker (coarse)", 2),
            ("Alt+← / →\tmove active marker (one bin)", 2),
            ("Display", 1),
            ("S\topen/close settings popover", 2),
            ("? or H\ttoggle this help overlay", 2),
            ("Escape\tdismiss overlays", 2),
            ("Q\tquit", 2),
        ];

        // Fixed column positions. The key column uses monospace 12pt (~7.2 px/char).
        // Longest key is "Ctrl+Shift+← / →" (17 display chars) → ~122 px + 16 gutter.
        let col_x  = overlay_rect.left() + 28.0;
        let desc_x = col_x + 148.0;

        let mut y = overlay_rect.top() + 14.0;
        for (text, kind) in lines {
            let (size, color, dy) = match kind {
                0 => (15.0, egui::Color32::WHITE, 26.0),
                1 => (11.0, egui::Color32::from_rgb(120, 180, 255), 20.0),
                _ => (12.0, egui::Color32::from_gray(220), 18.0),
            };
            let font = egui::FontId::new(size, egui::FontFamily::Monospace);
            if *kind == 2 {
                let mut parts = text.splitn(2, '\t');
                let key  = parts.next().unwrap_or("");
                let desc = parts.next().unwrap_or("");
                painter.text(egui::pos2(col_x,  y), egui::Align2::LEFT_TOP, key,  font.clone(), color);
                painter.text(egui::pos2(desc_x, y), egui::Align2::LEFT_TOP, desc, font,         color);
            } else {
                let x = overlay_rect.left() + 20.0;
                painter.text(egui::pos2(x, y), egui::Align2::LEFT_TOP, *text, font.clone(), color);
                // Right-aligned version on the title row, with the same
                // 20 px margin used for the title on the left.
                if *kind == 0 {
                    painter.text(
                        egui::pos2(overlay_rect.right() - 20.0, y),
                        egui::Align2::RIGHT_TOP,
                        concat!("Version ", env!("CARGO_PKG_VERSION")),
                        font,
                        color,
                    );
                }
            }
            y += dy;
        }

        // ── Copyright footer (centered, 20 px bottom margin) ─────────────
        let foot_font = egui::FontId::new(11.0, egui::FontFamily::Monospace);
        let foot_color = egui::Color32::from_gray(140);
        let cx = overlay_rect.center().x;
        let line2_y = overlay_rect.bottom() - 20.0;
        let line1_y = line2_y - 14.0;
        painter.text(
            egui::pos2(cx, line1_y),
            egui::Align2::CENTER_BOTTOM,
            "Copyright (c) 2026 G & R Associates LLC",
            foot_font.clone(),
            foot_color,
        );
        painter.text(
            egui::pos2(cx, line2_y),
            egui::Align2::CENTER_BOTTOM,
            "SPDX-License-Identifier: MIT OR Apache-2.0",
            foot_font,
            foot_color,
        );
    }
}

/// Draw a dashed horizontal line at `y` from `x0` to `x1`.  Matches the
/// dash geometry used for vertical frequency markers in the other panes.
fn dashed_hline(
    painter: &egui::Painter,
    x0: f32, x1: f32, y: f32,
    color: egui::Color32,
    width: f32,
) {
    const DASH: f32 = 8.0;
    const GAP:  f32 = 5.0;
    let stroke = egui::Stroke::new(width, color);
    let mut x = x0;
    let mut paint = true;
    while x < x1 {
        let seg = if paint { DASH } else { GAP };
        let xe = (x + seg).min(x1);
        if paint {
            painter.line_segment([egui::pos2(x, y), egui::pos2(xe, y)], stroke);
        }
        x = xe;
        paint = !paint;
    }
}
