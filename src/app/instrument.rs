// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The COFDM instrumentation panel (`X`).
//!
//! Placement and painting only — the metric model, the value formatting and the
//! grid arithmetic all live in [`crate::decode::instrument`], which is library
//! code and therefore testable.  `src/app/**` is bin-only, so anything decided
//! here is unreachable from the integration tests; keeping this file to
//! measurement and paint calls is what makes the layout assertable at all.
//!
//! Its own file rather than another block in `draw.rs`, which is already
//! ~970 lines.

use eframe::egui;

use super::view::ViewApp;
use super::{DecodeBarMode, SourceMode};
use crate::decode::instrument::{
    COLUMNS, CellStyle, MERGED_FROM, Row, SIM_BADGE, column_origins, merged_span_text,
    reference_column_widths,
};

/// Authoritative values — `Measured` or `Known`.
const VALUE_COL: egui::Color32 = egui::Color32::from_gray(220);
/// Labels and section names.
const LABEL_COL: egui::Color32 = egui::Color32::from_rgb(120, 180, 255);
/// Placeholders.  Dim enough that a simulated reading cannot be mistaken for a
/// measurement at a glance — the `SIM` badge says the same thing in words.
const SIM_COL: egui::Color32 = egui::Color32::from_gray(120);
/// The em-dash for an unavailable field.
const ABSENT_COL: egui::Color32 = egui::Color32::from_gray(90);

const TITLE_PT: f32 = 15.0;
const BODY_PT: f32 = 12.0;
const ROW_DY: f32 = 20.0;
/// Inset from the overlay rect to the grid's left edge.
const MARGIN_X: f32 = 20.0;
const MARGIN_Y: f32 = 14.0;

impl ViewApp {
    pub(super) fn draw_instrument_overlay(&self, ui: &mut egui::Ui) {
        let screen = ui.ctx().content_rect();
        let painter = ui.painter();
        let body_font = egui::FontId::new(BODY_PT, egui::FontFamily::Monospace);
        let title_font = egui::FontId::new(TITLE_PT, egui::FontFamily::Monospace);

        // Measure glyphs, never `str::len()`: the panel uses Δ, µ, — and ●,
        // which are multi-byte in UTF-8 and not reliably one advance wide.
        let measure = |s: &str| {
            painter
                .layout_no_wrap(s.to_owned(), body_font.clone(), VALUE_COL)
                .size()
                .x
        };

        let Some(inst) = self.decode_ticker.last_instrument.as_deref() else {
            self.draw_instrument_placeholder(ui, &title_font, &body_font);
            return;
        };
        if self.source_mode != SourceMode::Cofdm {
            self.draw_instrument_placeholder(ui, &title_font, &body_font);
            return;
        }

        let rows = inst.panel_rows();
        // Widths come from the fixed reference specimen, so the grid does not
        // reflow as values gain or lose a digit.
        let widths = reference_column_widths(measure);
        let origins = column_origins(&widths);
        let grid_w = widths.iter().sum::<f32>();
        // The merged span can run past the last column; size the rect to
        // whichever is wider.
        let merged_w = rows
            .iter()
            .filter(|r| !r.locks.is_empty())
            .map(|r| origins[MERGED_FROM] + measure(&merged_span_text(&r.locks)))
            .fold(0.0_f32, f32::max);
        let body_w = grid_w.max(merged_w);

        let size = egui::vec2(
            body_w + 2.0 * MARGIN_X,
            2.0 * MARGIN_Y + TITLE_PT + 12.0 + rows.len() as f32 * ROW_DY + 10.0,
        );
        let rect = egui::Rect::from_center_size(screen.center(), size);
        frame(painter, rect);

        // ── Title, with the SIM badge right-aligned ───────────────────────
        let mut y = rect.top() + MARGIN_Y;
        painter.text(
            egui::pos2(rect.left() + MARGIN_X, y),
            egui::Align2::LEFT_TOP,
            "COFDM instrumentation",
            title_font.clone(),
            egui::Color32::WHITE,
        );
        if inst.any_simulated() {
            painter.text(
                egui::pos2(rect.right() - MARGIN_X, y),
                egui::Align2::RIGHT_TOP,
                SIM_BADGE,
                title_font,
                SIM_COL,
            );
        }
        y += TITLE_PT + 12.0;

        // ── Grid ──────────────────────────────────────────────────────────
        let x0 = rect.left() + MARGIN_X;
        for row in &rows {
            self.draw_instrument_row(painter, row, &body_font, x0, y, &origins);
            y += ROW_DY;
        }
    }

    fn draw_instrument_row(
        &self,
        painter: &egui::Painter,
        row: &Row,
        font: &egui::FontId,
        x0: f32,
        y: f32,
        origins: &[f32; COLUMNS],
    ) {
        let cell_col = |style: CellStyle| match style {
            CellStyle::Normal => VALUE_COL,
            CellStyle::Simulated => SIM_COL,
            CellStyle::Absent => ABSENT_COL,
        };
        painter.text(
            egui::pos2(x0 + origins[0], y),
            egui::Align2::LEFT_TOP,
            row.section,
            font.clone(),
            LABEL_COL,
        );
        for (i, cell) in row.cells.iter().enumerate() {
            let col = 1 + i;
            if col >= COLUMNS {
                break;
            }
            // Even indices are labels, odd are values — the alternation the
            // grid depends on.
            let color = if i % 2 == 0 {
                LABEL_COL
            } else {
                cell_col(cell.style)
            };
            painter.text(
                egui::pos2(x0 + origins[col], y),
                egui::Align2::LEFT_TOP,
                &cell.text,
                font.clone(),
                color,
            );
        }
        if row.locks.is_empty() {
            return;
        }
        // The merged span: one run of label/indicator pairs from the C3 origin
        // to the right edge.  Lock states are booleans, not measurements, so
        // giving each its own value column would leave most of that width blank.
        let advance = |s: &str| {
            painter
                .layout_no_wrap(s.to_owned(), font.clone(), LABEL_COL)
                .size()
                .x
        };
        let mut x = x0 + origins[MERGED_FROM];
        for (i, l) in row.locks.iter().enumerate() {
            if i > 0 {
                x += advance("  ");
            }
            for (text, color) in [
                (l.label, LABEL_COL),
                (" ", LABEL_COL),
                (l.glyph, cell_col(l.style)),
            ] {
                painter.text(
                    egui::pos2(x, y),
                    egui::Align2::LEFT_TOP,
                    text,
                    font.clone(),
                    color,
                );
                x += advance(text);
            }
        }
    }

    /// Shown for a source with no instrumentation, and for COFDM before the
    /// first burst — a named reason rather than an empty frame.
    fn draw_instrument_placeholder(
        &self,
        ui: &mut egui::Ui,
        title_font: &egui::FontId,
        body_font: &egui::FontId,
    ) {
        let screen = ui.ctx().content_rect();
        let painter = ui.painter();
        let rect = egui::Rect::from_center_size(screen.center(), egui::vec2(460.0, 96.0));
        frame(painter, rect);
        painter.text(
            egui::pos2(rect.left() + MARGIN_X, rect.top() + MARGIN_Y),
            egui::Align2::LEFT_TOP,
            "COFDM instrumentation",
            title_font.clone(),
            egui::Color32::WHITE,
        );
        let msg = if self.source_mode == SourceMode::Cofdm {
            "waiting for signal\u{2026}".to_owned()
        } else {
            format!(
                "{} has no instrumentation \u{2014} switch to COFDM (I)",
                self.source_mode.label()
            )
        };
        painter.text(
            egui::pos2(
                rect.left() + MARGIN_X,
                rect.top() + MARGIN_Y + TITLE_PT + 14.0,
            ),
            egui::Align2::LEFT_TOP,
            msg,
            body_font.clone(),
            SIM_COL,
        );
    }

    /// True when the Di bar should render the COFDM instrumentation line rather
    /// than the shared four-field `info_str`.  Every other source keeps the
    /// existing line verbatim.
    pub(super) fn di_instrument_line(&self, budget_chars: usize) -> Option<String> {
        if self.source_mode != SourceMode::Cofdm || self.decode_bar != DecodeBarMode::Info {
            return None;
        }
        self.decode_ticker
            .last_instrument
            .as_deref()
            .map(|i| i.di_bar_str(budget_chars))
    }
}

/// The shared overlay frame: translucent fill, rounded stroke.
fn frame(painter: &egui::Painter, rect: egui::Rect) {
    painter.rect_filled(
        rect,
        8.0,
        egui::Color32::from_rgba_premultiplied(0, 0, 0, 220),
    );
    painter.rect_stroke(
        rect,
        8.0,
        egui::Stroke::new(1.0_f32, egui::Color32::GRAY),
        egui::StrokeKind::Outside,
    );
}
