// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Generic egui drawing primitives shared across pane renderers.

use eframe::egui;

/// Draw a dashed horizontal line at `y` from `x0` to `x1`.  Matches the
/// dash geometry used for vertical frequency markers in the other panes.
pub(super) fn dashed_hline(
    painter: &egui::Painter,
    x0: f32,
    x1: f32,
    y: f32,
    color: egui::Color32,
    width: f32,
) {
    const DASH: f32 = 8.0;
    const GAP: f32 = 5.0;
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
