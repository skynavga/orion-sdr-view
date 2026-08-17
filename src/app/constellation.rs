// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The constellation half of pane 3: the equalizer's output, plotted as hollow
//! circles coloured by point density.
//!
//! **This is a CPU-side raster, not painter geometry, and that is the whole
//! design.**  `pane_raster`'s contract is that a captured file is *what the pane
//! shows* — it reads the same buffers the painter reads.  The spectrum pane is
//! excluded from capture for exactly the opposite reason, and says so: "it is a
//! line plot drawn straight to a painter, with no pixel buffer to hand over."
//! Circles stamped into a `Vec<Color32>` keep the drawn thing and the captured
//! thing identical by construction rather than by care, and let
//! `tests/panes.rs` assert the picture with no GPU.  The cost is resolution
//! when the sub-rect is larger than [`CONST_PX`]; that is the right trade.
//!
//! **The extent is a constant, deliberately.**  The equalizer divides out the
//! channel *including* any uniform scalar, so the cloud sits at unit energy
//! whatever the transmit amplitude was (asserted upstream by
//! `the_equalized_cloud_sits_at_unit_energy`).  Auto-scaling would hide the one
//! thing a constellation is for — how far the cloud has spread — by renormalising
//! it away every frame.

use eframe::egui;
use num_complex::Complex32 as C32;
use orion_sdr::core::Block;
use orion_sdr::modulate::{
    BpskMapper, ConstellationOrder, Qam16Mapper, Qam64Mapper, Qam256Mapper, QpskMapper,
};

use super::persistence::density_color;

/// Raster size, in pixels, per side.  **Odd**, so the origin lands exactly on
/// pixel `CONST_PX / 2` and the I/Q axes are not a half-pixel off centre.
pub const CONST_PX: usize = 257;

/// Half-width of the plotted region, in unit-energy constellation units.
///
/// QPSK ideal points sit at `±1/√2 ≈ 0.707`, so this is roughly 3σ of headroom
/// at the FEC cliff.  Points outside are **dropped, not clamped**: clamping
/// piles the tail onto the border and reads as a hard edge that is not there.
/// The count of drops is reported instead — a measurement rather than a
/// fabrication.
pub const CONST_EXTENT: f32 = 2.0;

/// How many recent symbols are drawn as markers.  The ring's turnover is the
/// fade; the density map underneath sees *every* symbol, so the colour is
/// statistically right even though the circles are a recent sample.
pub const CONST_MARKERS: usize = 2048;

/// Marker radius in raster pixels.  Hollow, so thousands of overlapping symbols
/// stay distinguishable and the density colour underneath stays visible.
const MARKER_R: i32 = 2;

/// Background, matching `PANE_BG[2]`'s neighbourhood so the pane reads as one.
const BG: egui::Color32 = egui::Color32::from_rgb(8, 8, 16);
const AXIS_COL: egui::Color32 = egui::Color32::from_rgb(44, 44, 60);
const UNIT_COL: egui::Color32 = egui::Color32::from_rgb(30, 40, 55);
const IDEAL_COL: egui::Color32 = egui::Color32::from_rgb(245, 245, 245);
/// Drawn one pixel longer under [`IDEAL_COL`], so the reference reads against a
/// bright cluster core as well as against the background.
const IDEAL_HALO: egui::Color32 = egui::Color32::from_rgb(10, 10, 14);

/// Density decay, on the same shape as [`PersistenceMap`](super::persistence::PersistenceMap):
/// applied once every N rebuilds rather than continuously, so accumulation can
/// outrun it.
const DECAY_FACTOR: f32 = 0.82;
const DECAY_EVERY_N: u32 = 20;

/// The equalized constellation, as a stamped-marker raster over a density map.
pub struct ConstellationDisplay {
    /// Density accumulator, `CONST_PX × CONST_PX`, row-major, row 0 = +Q edge.
    counts: Vec<u32>,
    max_count: u32,
    /// The most recent symbols, drawn as markers.  A ring rather than a
    /// `Vec`: it is a fixed-size window on an unbounded stream.
    markers: std::collections::VecDeque<C32>,
    /// The rendered image — what both the painter and `pane_raster` read.
    pixels: Vec<egui::Color32>,
    texture: Option<egui::TextureHandle>,
    /// Set when symbols arrive; drives the rebuild.  Rebuilding per *arrival*
    /// (8–51 Hz) rather than per render frame (~108 Hz) costs 2–13× less and
    /// loses nothing, because nothing changed in between.
    dirty: bool,
    /// Symbols outside [`CONST_EXTENT`] since the last clear.  Reported rather
    /// than clamped — see the constant.
    off_scale: u64,
    /// Symbols accumulated since the last clear, so the off-scale count has a
    /// denominator.
    total: u64,
    /// The constellation the symbols were demapped against, for the ideal-point
    /// overlay.  A change clears the density: the old cloud belongs to a
    /// different grid.
    order: Option<ConstellationOrder>,
    decay_counter: u32,
}

impl Default for ConstellationDisplay {
    fn default() -> Self {
        Self::new()
    }
}

impl ConstellationDisplay {
    pub fn new() -> Self {
        Self {
            counts: vec![0; CONST_PX * CONST_PX],
            max_count: 1,
            markers: std::collections::VecDeque::with_capacity(CONST_MARKERS),
            pixels: vec![BG; CONST_PX * CONST_PX],
            texture: None,
            dirty: true,
            off_scale: 0,
            total: 0,
            order: None,
            decay_counter: 0,
        }
    }

    /// Drop all history — on a source switch, a gap edge, or a constellation
    /// change.
    pub fn clear(&mut self) {
        self.counts.iter_mut().for_each(|c| *c = 0);
        self.max_count = 1;
        self.markers.clear();
        self.off_scale = 0;
        self.total = 0;
        self.decay_counter = 0;
        self.dirty = true;
    }

    /// Accumulate one frame's equalized symbols.
    ///
    /// Every symbol lands in the density map; only the most recent
    /// [`CONST_MARKERS`] are kept as drawable markers.
    pub fn push_symbols(&mut self, symbols: &[C32], order: ConstellationOrder) {
        if self.order != Some(order) {
            // A different grid: the accumulated cloud is not comparable, and
            // the ideal-point overlay is about to move.
            self.clear();
            self.order = Some(order);
        }
        if symbols.is_empty() {
            return;
        }
        for &s in symbols {
            self.total += 1;
            let Some(idx) = self.cell(s) else {
                self.off_scale += 1;
                continue;
            };
            self.counts[idx] = self.counts[idx].saturating_add(1);
            self.max_count = self.max_count.max(self.counts[idx]);
            if self.markers.len() == CONST_MARKERS {
                self.markers.pop_front();
            }
            self.markers.push_back(s);
        }
        self.decay();
        self.dirty = true;
    }

    /// The raster index for a symbol, or `None` when it falls outside the plot.
    fn cell(&self, s: C32) -> Option<usize> {
        if !s.re.is_finite() || !s.im.is_finite() {
            return None;
        }
        let half = (CONST_PX / 2) as f32;
        // I → column, +Q → up (row 0 is the +Q edge).
        let x = half + s.re / CONST_EXTENT * half;
        let y = half - s.im / CONST_EXTENT * half;
        if x < 0.0 || y < 0.0 {
            return None;
        }
        let (x, y) = (x.round() as usize, y.round() as usize);
        (x < CONST_PX && y < CONST_PX).then(|| y * CONST_PX + x)
    }

    /// Decay the density every `DECAY_EVERY_N` pushes, so a cloud that moves
    /// leaves a fading trail rather than a permanent one.
    fn decay(&mut self) {
        self.decay_counter += 1;
        if self.decay_counter < DECAY_EVERY_N {
            return;
        }
        self.decay_counter = 0;
        let mut new_max = 1u32;
        for c in &mut self.counts {
            *c = (*c as f32 * DECAY_FACTOR) as u32;
            new_max = new_max.max(*c);
        }
        self.max_count = new_max;
    }

    /// Repaint the raster from the density map and the marker ring.
    ///
    /// Only when something arrived — see [`dirty`](Self::dirty).  Draw order is
    /// background, axes, unit circle, markers, ideal points: the reference
    /// geometry sits under the data so it never hides a symbol, and the ideal
    /// crosses sit on top so they stay findable in a dense cloud.
    fn rebuild(&mut self) {
        self.pixels.iter_mut().for_each(|p| *p = BG);
        let half = CONST_PX / 2;

        // I/Q axes through the origin.
        for i in 0..CONST_PX {
            self.pixels[half * CONST_PX + i] = AXIS_COL;
            self.pixels[i * CONST_PX + half] = AXIS_COL;
        }

        // Unit-circle scale reference: |s| = 1, which for a unit-energy
        // constellation is where the outer ideal points sit.
        let r_px = (1.0 / CONST_EXTENT) * half as f32;
        let steps = (std::f32::consts::TAU * r_px).ceil() as usize * 2;
        for i in 0..steps.max(1) {
            let th = std::f32::consts::TAU * i as f32 / steps.max(1) as f32;
            let x = half as f32 + r_px * th.cos();
            let y = half as f32 - r_px * th.sin();
            if x >= 0.0 && y >= 0.0 {
                let (x, y) = (x.round() as usize, y.round() as usize);
                if x < CONST_PX && y < CONST_PX {
                    self.pixels[y * CONST_PX + x] = UNIT_COL;
                }
            }
        }

        // Markers, coloured by the density at their own cell.
        let max = self.max_count.max(1) as f32;
        for i in 0..self.markers.len() {
            let s = self.markers[i];
            let Some(idx) = self.cell(s) else { continue };
            let count = self.counts[idx];
            let col = density_color(count as f32 / max, count);
            self.stamp_ring(idx % CONST_PX, idx / CONST_PX, MARKER_R, col);
        }

        // Ideal points last, so a dense cloud cannot bury the reference — and
        // over a dark halo, because "last" is not enough on its own: a white
        // cross on a bright cluster core is nearly invisible, which is exactly
        // where the reference matters most.  Measured at C/N 10 dB the cores
        // render mid-green and an unhaloed cross disappeared into them.
        if let Some(order) = self.order {
            for p in ideal_points(order) {
                if let Some(idx) = self.cell(p) {
                    let (x, y) = (idx % CONST_PX, idx / CONST_PX);
                    self.stamp_cross(x, y, 4, IDEAL_HALO);
                    self.stamp_cross(x, y, 3, IDEAL_COL);
                }
            }
        }
        self.dirty = false;
    }

    /// A hollow circle of radius `r` centred on `(cx, cy)`.
    fn stamp_ring(&mut self, cx: usize, cy: usize, r: i32, col: egui::Color32) {
        let n = CONST_PX as i32;
        let inner = (r - 1) * (r - 1);
        let outer = r * r;
        for dy in -r..=r {
            for dx in -r..=r {
                let d2 = dx * dx + dy * dy;
                if d2 > outer || d2 < inner {
                    continue;
                }
                let (x, y) = (cx as i32 + dx, cy as i32 + dy);
                if x >= 0 && y >= 0 && x < n && y < n {
                    self.pixels[y as usize * CONST_PX + x as usize] = col;
                }
            }
        }
    }

    /// A small plus centred on `(cx, cy)`, `arm` pixels each way.
    fn stamp_cross(&mut self, cx: usize, cy: usize, arm: i32, col: egui::Color32) {
        let n = CONST_PX as i32;
        for d in -arm..=arm {
            for (x, y) in [(cx as i32 + d, cy as i32), (cx as i32, cy as i32 + d)] {
                if x >= 0 && y >= 0 && x < n && y < n {
                    self.pixels[y as usize * CONST_PX + x as usize] = col;
                }
            }
        }
    }

    /// Rebuild if needed, then upload.  The whole texture goes up: unlike the
    /// waterfall's single new row, a marker ring turning over changes pixels
    /// everywhere.  It only happens when symbols arrived.
    pub fn update_texture(&mut self, ctx: &egui::Context) {
        if !self.dirty && self.texture.is_some() {
            return;
        }
        self.rebuild();
        let rgba: Vec<u8> = self
            .pixels
            .iter()
            .flat_map(|c| [c.r(), c.g(), c.b(), 255])
            .collect();
        let image = egui::ColorImage::from_rgba_unmultiplied([CONST_PX, CONST_PX], &rgba);
        match &mut self.texture {
            Some(tex) => tex.set(image, egui::TextureOptions::LINEAR),
            None => {
                self.texture =
                    Some(ctx.load_texture("constellation", image, egui::TextureOptions::LINEAR))
            }
        }
    }

    /// Paint the raster into `rect`.
    pub fn draw(&self, painter: &egui::Painter, rect: egui::Rect) {
        let Some(tex) = &self.texture else {
            painter.rect_filled(rect, 0.0, BG);
            return;
        };
        super::utils::image_quad(painter, tex.id(), rect, [0.0, 1.0], [0.0, 1.0]);
    }

    /// The raster, row-major from the +Q edge down — the order the painter maps
    /// and `pane_raster` writes.
    pub fn pixels_in_display_order(&self) -> &[egui::Color32] {
        &self.pixels
    }

    /// Force the raster up to date without a GPU context, for the headless
    /// capture path.
    pub fn sync_raster(&mut self) {
        if self.dirty {
            self.rebuild();
        }
    }

    /// Symbols dropped for falling outside the plot, and the total seen.
    pub fn off_scale(&self) -> (u64, u64) {
        (self.off_scale, self.total)
    }

    /// The constellation currently plotted, once a frame has arrived.
    pub fn order(&self) -> Option<ConstellationOrder> {
        self.order
    }

    /// Whether anything has been plotted yet.
    pub fn is_empty(&self) -> bool {
        self.total == 0
    }
}

/// The ideal constellation points for `order`, at unit energy.
///
/// **Derived from orion-sdr's own mappers, not re-tabulated here.** The crate's
/// `ideal_symbol_mapper` is `pub(crate)`, but every mapper it dispatches to is
/// public, so feeding each bit pattern through the right one recovers exactly
/// the points the transmitter used. A second copy of the geometry in the viewer
/// would be one upstream normalisation change away from drawing a reference
/// grid the signal is not on.
pub fn ideal_points(order: ConstellationOrder) -> Vec<C32> {
    let bits = order.bits_per_symbol();
    let n = 1usize << bits;
    let mut input = Vec::with_capacity(n * bits);
    for sym in 0..n {
        for b in (0..bits).rev() {
            input.push(((sym >> b) & 1) as u8);
        }
    }
    let mut out = vec![C32::default(); n];
    let w = match order {
        ConstellationOrder::Bpsk => BpskMapper::new().process(&input, &mut out),
        ConstellationOrder::Qpsk => QpskMapper::new().process(&input, &mut out),
        ConstellationOrder::Qam16 => Qam16Mapper::new().process(&input, &mut out),
        ConstellationOrder::Qam64 => Qam64Mapper::new().process(&input, &mut out),
        ConstellationOrder::Qam256 => Qam256Mapper::new().process(&input, &mut out),
    };
    out.truncate(w.out_written);
    out
}
