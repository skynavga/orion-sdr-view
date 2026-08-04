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

/// Emit one textured quad covering `scr`.  `u = [left, right]` are the U
/// coordinates at the left/right screen edges; `v = [top, bottom]` are the V
/// coordinates at the top/bottom edges.  Passing `u[0] > u[1]` flips
/// horizontally and `v[0] > v[1]` flips vertically — used to render ring-buffer
/// textures whose newest row/column is not at physical index 0.
pub(super) fn image_quad(
    painter: &egui::Painter,
    tex: egui::TextureId,
    scr: egui::Rect,
    u: [f32; 2],
    v: [f32; 2],
) {
    let mut mesh = egui::Mesh::with_texture(tex);
    let color = egui::Color32::WHITE;
    // Vertices in TL, TR, BR, BL order.
    mesh.vertices.push(egui::epaint::Vertex {
        pos: scr.left_top(),
        uv: egui::pos2(u[0], v[0]),
        color,
    });
    mesh.vertices.push(egui::epaint::Vertex {
        pos: scr.right_top(),
        uv: egui::pos2(u[1], v[0]),
        color,
    });
    mesh.vertices.push(egui::epaint::Vertex {
        pos: scr.right_bottom(),
        uv: egui::pos2(u[1], v[1]),
        color,
    });
    mesh.vertices.push(egui::epaint::Vertex {
        pos: scr.left_bottom(),
        uv: egui::pos2(u[0], v[1]),
        color,
    });
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    painter.add(egui::Shape::mesh(mesh));
}
