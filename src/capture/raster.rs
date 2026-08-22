// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! A CPU rasterizer for egui's tessellated output.
//!
//! **Why not render on the GPU.** A headless run's headline property is that the
//! same script produces the same bytes, and a GPU render cannot carry that: fill
//! rules and filtering vary by vendor and driver version.  A capture is only
//! useful as a test fixture if it repeats, so this rasterizes on the CPU, in
//! arithmetic that is identical everywhere.
//!
//! # Matching what the GPU does
//!
//! Checked against `egui-wgpu`'s `egui.wgsl` and its pipeline setup, because
//! guessing here would produce something plausible and wrong:
//!
//! * **The framebuffer is *not* sRGB.**  `preferred_framebuffer_format` picks
//!   `Rgba8Unorm`/`Bgra8Unorm` and egui logs a warning if it is given an sRGB
//!   target instead.  So the pipeline runs `fs_main_gamma_framebuffer`, which is
//!   the whole of `out = vertex_color * texture_sample` — **no sRGB conversion
//!   anywhere**, and blending happens in gamma space.
//!
//!   That is worth stating plainly because the opposite assumption is the
//!   natural one, and it would have put `powf` on the hot path.  `powf` is a
//!   libm transcendental and is *not* guaranteed identical across platforms, so
//!   it would have quietly cost the cross-architecture reproducibility this
//!   exists for.  There is no such hazard: every operation below is `+`, `-`,
//!   `*`, `/` or a comparison, all IEEE-correctly-rounded and identical on x86
//!   and ARM.
//!
//! * **Anti-aliasing is already in the geometry.**  epaint feathers edges by
//!   emitting extra triangles with alpha, and MSAA is off by default, so
//!   coverage is a single sample at each pixel centre.  No coverage arithmetic
//!   is needed here — only a fill rule.
//!
//! * **Texture filtering is the shader's own four-tap bilinear**, the branch
//!   `predictable_texture_filtering` selects.  egui added that flag "for more
//!   predictable kittest snapshot images", which is exactly this use, and it is
//!   what the GPU oracle should be configured with when comparing.
//!
//! # What is deliberately not implemented
//!
//! `Primitive::Callback` — a user-supplied GPU draw. This app registers none,
//! and silently skipping one would be worse than saying so, so it is counted
//! and reported.

use std::collections::HashMap;

use egui::epaint::{ClippedPrimitive, Primitive};
use egui::{Color32, TextureId};

/// A texture the rasterizer can sample: RGBA8, premultiplied, gamma-encoded —
/// the same thing the GPU holds.
#[derive(Clone)]
pub struct Texture {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<Color32>,
}

/// The texture set, maintained across frames from `FullOutput.textures_delta`.
///
/// Kept because deltas are incremental: the font atlas grows a glyph at a time,
/// and a frame that only patches a sub-region says nothing about the rest.
#[derive(Default)]
pub struct Textures {
    map: HashMap<TextureId, Texture>,
}

impl Textures {
    /// Apply a frame's texture changes.
    pub fn apply(&mut self, delta: &egui::TexturesDelta) {
        for (id, image) in &delta.set {
            let egui::epaint::ImageData::Color(src) = &image.image;
            match image.pos {
                None => {
                    self.map.insert(
                        *id,
                        Texture {
                            width: src.width(),
                            height: src.height(),
                            pixels: src.pixels.clone(),
                        },
                    );
                }
                // A patch into an already-allocated texture.  Ignored if the
                // texture is unknown, which can only happen if a `set` was
                // missed — and then the glyphs would be wrong rather than
                // absent, so it is not worth guessing at.
                Some([x0, y0]) => {
                    if let Some(dst) = self.map.get_mut(id) {
                        for (row, chunk) in src.pixels.chunks_exact(src.width()).enumerate() {
                            let y = y0 + row;
                            if y >= dst.height {
                                break;
                            }
                            let start = y * dst.width + x0;
                            let n = chunk.len().min(dst.width.saturating_sub(x0));
                            dst.pixels[start..start + n].copy_from_slice(&chunk[..n]);
                        }
                    }
                }
            }
        }
        for id in &delta.free {
            self.map.remove(id);
        }
    }

    fn get(&self, id: TextureId) -> Option<&Texture> {
        self.map.get(&id)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Every texture held, for a consumer that needs the whole set rather than
    /// one frame's delta — the GPU oracle, which must be given the font atlas
    /// even though it was uploaded long before the frame being compared.
    pub fn iter(&self) -> impl Iterator<Item = (&TextureId, &Texture)> {
        self.map.iter()
    }

    /// A copy of the whole set.
    pub fn clone_set(&self) -> Self {
        Self {
            map: self.map.clone(),
        }
    }
}

/// What a rasterized frame produced.
pub struct Raster {
    pub width: u32,
    pub height: u32,
    /// Row-major RGBA8, top row first.
    pub rgba: Vec<u8>,
    /// `Primitive::Callback`s encountered, which this cannot draw.
    pub skipped_callbacks: usize,
    /// Meshes naming a texture that was never uploaded.
    pub missing_textures: usize,
}

/// Rasterize one frame.
///
/// `size` is in physical pixels and `pixels_per_point` converts the mesh's
/// logical coordinates into them — the same two numbers `ScreenDescriptor`
/// carries to the GPU.
pub fn rasterize(
    primitives: &[ClippedPrimitive],
    textures: &Textures,
    size: (u32, u32),
    pixels_per_point: f32,
) -> Raster {
    let (w, h) = (size.0 as usize, size.1 as usize);
    // Opaque black, matching a cleared framebuffer.  Not transparent: the
    // window has no transparency and a PNG with an alpha hole would look like a
    // rendering bug.
    let mut canvas = vec![[0u8, 0, 0, 255]; w * h];
    let mut skipped_callbacks = 0;
    let mut missing_textures = 0;

    for cp in primitives {
        let Primitive::Mesh(mesh) = &cp.primitive else {
            skipped_callbacks += 1;
            continue;
        };
        let Some(tex) = textures.get(mesh.texture_id) else {
            missing_textures += 1;
            continue;
        };
        // The clip rect is in points; the canvas is in pixels.
        let clip = ClipRect::new(cp.clip_rect, pixels_per_point, w, h);
        if clip.is_empty() {
            continue;
        }
        for tri in mesh.indices.as_chunks::<3>().0 {
            let v = [
                &mesh.vertices[tri[0] as usize],
                &mesh.vertices[tri[1] as usize],
                &mesh.vertices[tri[2] as usize],
            ];
            draw_triangle(&mut canvas, w, v, tex, &clip, pixels_per_point);
        }
    }

    let mut rgba = Vec::with_capacity(w * h * 4);
    for px in &canvas {
        rgba.extend_from_slice(px);
    }
    Raster {
        width: size.0,
        height: size.1,
        rgba,
        skipped_callbacks,
        missing_textures,
    }
}

/// An integer pixel rectangle, already clamped to the canvas.
struct ClipRect {
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
}

impl ClipRect {
    fn new(rect: egui::Rect, ppp: f32, w: usize, h: usize) -> Self {
        // Rounded outward, as the GPU scissor does: a partially covered pixel
        // is inside the scissor.
        let x0 = (rect.min.x * ppp).floor().max(0.0) as usize;
        let y0 = (rect.min.y * ppp).floor().max(0.0) as usize;
        let x1 = ((rect.max.x * ppp).ceil().max(0.0) as usize).min(w);
        let y1 = ((rect.max.y * ppp).ceil().max(0.0) as usize).min(h);
        Self {
            x0: x0.min(w),
            y0: y0.min(h),
            x1,
            y1,
        }
    }

    fn is_empty(&self) -> bool {
        self.x0 >= self.x1 || self.y0 >= self.y1
    }
}

/// Twice the signed area of the triangle `(a, b, c)`.
///
/// Positive for one winding, negative for the other; egui emits both, so the
/// sign is used to orient the inside test rather than to cull.
fn edge(a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> f32 {
    (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
}

/// Whether the directed edge `a -> b` is a *top* or *left* edge of a
/// positively-wound triangle, in screen coordinates with y increasing downward.
///
/// **This is what stops a shared edge being drawn twice.** Two triangles of a
/// quad meet along its diagonal, and any pixel centre lying exactly on that
/// diagonal is inside *both* by a plain `>= 0` test. Drawing it twice is
/// invisible for opaque geometry and very visible for translucent geometry —
/// and egui's anti-aliasing is made of translucent triangles, so without this
/// every feathered edge in the frame would blend twice and seam.
///
/// A top edge is horizontal with the interior below it, which for positive
/// winding means it runs left to right. A left edge has the interior to its
/// right, which means it runs upward.
fn is_top_left(a: (f32, f32), b: (f32, f32)) -> bool {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    (dy == 0.0 && dx > 0.0) || dy < 0.0
}

fn draw_triangle(
    canvas: &mut [[u8; 4]],
    stride: usize,
    v: [&egui::epaint::Vertex; 3],
    tex: &Texture,
    clip: &ClipRect,
    ppp: f32,
) {
    let mut p: [(f32, f32); 3] = [
        (v[0].pos.x * ppp, v[0].pos.y * ppp),
        (v[1].pos.x * ppp, v[1].pos.y * ppp),
        (v[2].pos.x * ppp, v[2].pos.y * ppp),
    ];
    let mut v = v;
    let mut area = edge(p[0], p[1], p[2]);
    if area == 0.0 {
        return; // degenerate: no coverage
    }
    // egui emits both windings.  Normalizing to positive lets the fill rule
    // below be stated once, in the orientation it is derived for.
    if area < 0.0 {
        p.swap(1, 2);
        v.swap(1, 2);
        area = -area;
    }

    // Bounding box, clamped to the clip rect.
    let min_x = p[0].0.min(p[1].0).min(p[2].0).floor().max(clip.x0 as f32) as usize;
    let max_x = (p[0].0.max(p[1].0).max(p[2].0).ceil().max(0.0) as usize).min(clip.x1);
    let min_y = p[0].1.min(p[1].1).min(p[2].1).floor().max(clip.y0 as f32) as usize;
    let max_y = (p[0].1.max(p[1].1).max(p[2].1).ceil().max(0.0) as usize).min(clip.y1);
    if min_x >= max_x || min_y >= max_y {
        return;
    }

    let inv_area = 1.0 / area;
    let top_left = [
        is_top_left(p[1], p[2]),
        is_top_left(p[2], p[0]),
        is_top_left(p[0], p[1]),
    ];
    let cols: [[f32; 4]; 3] = [unpack(v[0].color), unpack(v[1].color), unpack(v[2].color)];
    let uvs: [(f32, f32); 3] = [
        (v[0].uv.x, v[0].uv.y),
        (v[1].uv.x, v[1].uv.y),
        (v[2].uv.x, v[2].uv.y),
    ];

    for y in min_y..max_y {
        for x in min_x..max_x {
            // A single sample at the pixel centre: MSAA is off, and epaint's
            // feathering already carries the anti-aliasing as geometry.
            let s = (x as f32 + 0.5, y as f32 + 0.5);
            let e0 = edge(p[1], p[2], s);
            let e1 = edge(p[2], p[0], s);
            let e2 = edge(p[0], p[1], s);
            // Top-left fill rule: a pixel exactly on an edge belongs to the
            // triangle only if that edge is a top or left one, so two triangles
            // sharing an edge cover it exactly once between them.
            if !(inside(e0, top_left[0]) && inside(e1, top_left[1]) && inside(e2, top_left[2])) {
                continue;
            }
            let (w0, w1, w2) = (e0 * inv_area, e1 * inv_area, e2 * inv_area);

            let u = w0 * uvs[0].0 + w1 * uvs[1].0 + w2 * uvs[2].0;
            let vv = w0 * uvs[0].1 + w1 * uvs[1].1 + w2 * uvs[2].1;
            let texel = sample_bilinear(tex, u, vv);

            // `fs_main_gamma_framebuffer`: the fragment is the vertex colour
            // times the texel, both premultiplied and both in gamma space.
            let mut src = [0.0f32; 4];
            for i in 0..4 {
                let c = w0 * cols[0][i] + w1 * cols[1][i] + w2 * cols[2][i];
                src[i] = c * texel[i];
            }

            let dst = &mut canvas[y * stride + x];
            *dst = blend(src, *dst);
        }
    }
}

/// Whether an edge value counts as inside, given the edge's top-left status.
fn inside(e: f32, top_left: bool) -> bool {
    if top_left { e >= 0.0 } else { e > 0.0 }
}

/// Premultiplied source-over, matching the pipeline's blend state:
/// colour `src + dst * (1 - src.a)`, alpha `src.a * (1 - dst.a) + dst.a`.
///
/// Rounded to eight bits per draw, because the target is `Unorm8` and every
/// draw call writes through it.
fn blend(src: [f32; 4], dst: [u8; 4]) -> [u8; 4] {
    let d = [
        f32::from(dst[0]) / 255.0,
        f32::from(dst[1]) / 255.0,
        f32::from(dst[2]) / 255.0,
        f32::from(dst[3]) / 255.0,
    ];
    let inv_sa = 1.0 - src[3];
    let mut out = [0u8; 4];
    for i in 0..3 {
        out[i] = to_u8(src[i] + d[i] * inv_sa);
    }
    out[3] = to_u8(src[3] * (1.0 - d[3]) + d[3]);
    out
}

/// Round a 0-1 float to eight bits, clamping out of range.
///
/// `+ 0.5` then truncate — the same round-half-away-from-zero a Unorm target
/// applies, and exact integer behaviour rather than a library rounding mode.
fn to_u8(v: f32) -> u8 {
    let s = v * 255.0 + 0.5;
    if s <= 0.0 {
        0
    } else if s >= 255.0 {
        255
    } else {
        s as u8
    }
}

/// `Color32` to premultiplied gamma 0-1, matching the shader's `unpack_color`.
fn unpack(c: Color32) -> [f32; 4] {
    let a = c.to_array();
    [
        f32::from(a[0]) / 255.0,
        f32::from(a[1]) / 255.0,
        f32::from(a[2]) / 255.0,
        f32::from(a[3]) / 255.0,
    ]
}

/// The shader's four-tap bilinear sample, with clamped addressing.
///
/// Deliberately the `predictable_texture_filtering` branch rather than the
/// hardware `textureSample` one: it is defined arithmetic, so the result does
/// not depend on a GPU's filtering precision.
fn sample_bilinear(tex: &Texture, u: f32, v: f32) -> [f32; 4] {
    if tex.width == 0 || tex.height == 0 {
        return [0.0; 4];
    }
    let (tw, th) = (tex.width as f32, tex.height as f32);
    let x = u * tw - 0.5;
    let y = v * th - 0.5;
    let (fx, fy) = (x.floor(), y.floor());
    let (dx, dy) = (x - fx, y - fy);

    let max_x = tex.width as i64 - 1;
    let max_y = tex.height as i64 - 1;
    let cx = |i: i64| i.clamp(0, max_x) as usize;
    let cy = |i: i64| i.clamp(0, max_y) as usize;
    let (ix, iy) = (fx as i64, fy as i64);

    let at = |px: usize, py: usize| unpack(tex.pixels[py * tex.width + px]);
    let tl = at(cx(ix), cy(iy));
    let tr = at(cx(ix + 1), cy(iy));
    let bl = at(cx(ix), cy(iy + 1));
    let br = at(cx(ix + 1), cy(iy + 1));

    let mut out = [0.0f32; 4];
    for i in 0..4 {
        let top = tl[i] + (tr[i] - tl[i]) * dx;
        let bot = bl[i] + (br[i] - bl[i]) * dx;
        out[i] = top + (bot - top) * dy;
    }
    out
}
