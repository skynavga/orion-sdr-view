// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The CPU rasterizer, pinned against a committed reference image.
//!
//! **This is the cross-architecture check.** The reference is generated on one
//! machine and compared on another — development here is `arm64`, CI is
//! `x86-64` — so it verifies on every push that the rasterizer produces
//! identical bytes on both. Nothing else has to be built to get that.
//!
//! # Why the scene is synthetic
//!
//! It is built from hand-written meshes rather than captured from the app, and
//! that is deliberate rather than lazy.
//!
//! **A real capture is not cross-architecture reproducible, and the rasterizer
//! is not why.** `rustfft` dispatches to AVX on x86-64 and to Neon on AArch64 —
//! different algorithms, in a different operation order — so the spectrum
//! differs in its last bits between the two, and every pixel downstream of it
//! inherits that. Committing a captured frame would therefore fail in CI while
//! passing locally, and the failure would look like a rasterizer bug when it is
//! nothing of the sort.
//!
//! So this pins the part that *can* hold: blending, sampling, the fill rule and
//! the clip. Within one machine a real capture is byte-identical too, which
//! `tests/capture.rs` covers.

#![cfg(feature = "gui")]

use std::path::{Path, PathBuf};

use egui::epaint::{ClippedPrimitive, Mesh, Primitive, Vertex};
use egui::{Color32, Pos2, Rect, TextureId, pos2};
use orion_sdr_view::capture::{Textures, encode_png, rasterize};

const REFERENCE: &str = "tests/reference/rasterizer.png";
const SIZE: (u32, u32) = (160, 100);

/// The texture every mesh below samples: a 4x4 with an opaque white texel at
/// the origin, so a "solid" triangle can point at it exactly as egui's own
/// meshes point at the white texel in the font atlas.
fn texture() -> Textures {
    const W: Color32 = Color32::WHITE;
    let r = Color32::from_rgba_premultiplied(200, 40, 40, 255);
    let g = Color32::from_rgba_premultiplied(40, 200, 40, 255);
    let b = Color32::from_rgba_premultiplied(40, 40, 200, 255);
    let pixels = vec![
        W, r, g, b, //
        r, g, b, W, //
        g, b, W, r, //
        b, W, r, g,
    ];
    let mut textures = Textures::default();
    let mut delta = egui::TexturesDelta::default();
    delta.set.push((
        TextureId::Managed(0),
        egui::epaint::ImageDelta::full(
            egui::epaint::ImageData::Color(std::sync::Arc::new(egui::ColorImage {
                size: [4, 4],
                pixels,
                source_size: egui::vec2(4.0, 4.0),
            })),
            egui::TextureOptions::LINEAR,
        ),
    ));
    textures.apply(&delta);
    textures
}

/// The white texel's centre, for meshes that want no texture modulation.
const SOLID_UV: Pos2 = pos2(0.125, 0.125);

fn quad(a: Pos2, b: Pos2, colors: [Color32; 4], uvs: [Pos2; 4]) -> Mesh {
    let corners = [
        pos2(a.x, a.y),
        pos2(b.x, a.y),
        pos2(b.x, b.y),
        pos2(a.x, b.y),
    ];
    let mut mesh = Mesh::with_texture(TextureId::Managed(0));
    for i in 0..4 {
        mesh.vertices.push(Vertex {
            pos: corners[i],
            uv: uvs[i],
            color: colors[i],
        });
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    mesh
}

fn solid(a: Pos2, b: Pos2, color: Color32) -> Mesh {
    quad(a, b, [color; 4], [SOLID_UV; 4])
}

/// A scene exercising each thing the rasterizer has to get right.
fn scene() -> Vec<ClippedPrimitive> {
    let full = Rect::from_min_max(pos2(0.0, 0.0), pos2(160.0, 100.0));
    let mut out = Vec::new();

    // 1. An opaque background, so nothing below is composited against black.
    out.push(ClippedPrimitive {
        clip_rect: full,
        primitive: Primitive::Mesh(solid(
            pos2(0.0, 0.0),
            pos2(160.0, 100.0),
            Color32::from_rgba_premultiplied(20, 24, 32, 255),
        )),
    });

    // 2. A textured quad at non-integer UVs — the four-tap bilinear path.
    out.push(ClippedPrimitive {
        clip_rect: full,
        primitive: Primitive::Mesh(quad(
            pos2(8.0, 8.0),
            pos2(72.0, 56.0),
            [Color32::WHITE; 4],
            [
                pos2(0.07, 0.11),
                pos2(0.93, 0.11),
                pos2(0.93, 0.89),
                pos2(0.07, 0.89),
            ],
        )),
    });

    // 3. Per-vertex colour interpolation across a triangle pair.
    out.push(ClippedPrimitive {
        clip_rect: full,
        primitive: Primitive::Mesh(quad(
            pos2(84.0, 8.0),
            pos2(152.0, 56.0),
            [
                Color32::from_rgba_premultiplied(255, 0, 0, 255),
                Color32::from_rgba_premultiplied(0, 255, 0, 255),
                Color32::from_rgba_premultiplied(0, 0, 255, 255),
                Color32::from_rgba_premultiplied(255, 255, 0, 255),
            ],
            [SOLID_UV; 4],
        )),
    });

    // 4. Premultiplied blending, on **28 x 28 squares**.  Square is not
    //    incidental: it makes each quad's shared diagonal run at slope 1, so
    //    pixel centres land exactly on it and the top-left fill rule decides
    //    which of the two triangles owns them.  Without that rule those pixels
    //    blend twice, and with translucent geometry the seam is visible — so
    //    the reference has to contain geometry that can show it.
    for (i, c) in [
        Color32::from_rgba_premultiplied(120, 0, 0, 120),
        Color32::from_rgba_premultiplied(0, 120, 0, 120),
        Color32::from_rgba_premultiplied(0, 0, 120, 120),
    ]
    .into_iter()
    .enumerate()
    {
        let x = 10.0 + i as f32 * 18.0;
        out.push(ClippedPrimitive {
            clip_rect: full,
            primitive: Primitive::Mesh(solid(pos2(x, 64.0), pos2(x + 28.0, 92.0), c)),
        });
    }

    // 5. A clip rect that cuts a quad in half, including a fractional edge.
    out.push(ClippedPrimitive {
        clip_rect: Rect::from_min_max(pos2(96.0, 64.0), pos2(127.5, 92.0)),
        primitive: Primitive::Mesh(solid(
            pos2(96.0, 64.0),
            pos2(152.0, 92.0),
            Color32::from_rgba_premultiplied(220, 180, 40, 255),
        )),
    });

    // 6. A sub-pixel triangle, where the fill rule decides the result.
    let mut tri = Mesh::with_texture(TextureId::Managed(0));
    for pos in [pos2(132.5, 66.25), pos2(148.75, 74.5), pos2(134.0, 88.75)] {
        tri.vertices.push(Vertex {
            pos,
            uv: SOLID_UV,
            color: Color32::from_rgba_premultiplied(240, 240, 240, 200),
        });
    }
    tri.indices.extend_from_slice(&[0, 1, 2]);
    out.push(ClippedPrimitive {
        clip_rect: full,
        primitive: Primitive::Mesh(tri),
    });

    out
}

fn render() -> Vec<u8> {
    let raster = rasterize(&scene(), &texture(), SIZE, 1.0);
    assert_eq!(raster.missing_textures, 0, "the scene's texture is defined");
    assert_eq!(raster.skipped_callbacks, 0);
    let mut png = Vec::new();
    encode_png(&mut png, raster.width, raster.height, &raster.rgba).expect("encode");
    png
}

fn reference_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(REFERENCE)
}

#[test]
fn the_rasterizer_matches_its_committed_reference() {
    // Regenerate with `UPDATE_REFERENCE=1 cargo test --test raster`, and read
    // the diff before committing it: this file changing is either a deliberate
    // rasterizer change or a bug, and nothing else.
    let png = render();
    let path = reference_path();

    if std::env::var_os("UPDATE_REFERENCE").is_some() {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::fs::write(&path, &png).expect("write reference");
        return;
    }

    let want = std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "cannot read {}: {e}\nregenerate with UPDATE_REFERENCE=1 cargo test --test raster",
            path.display()
        )
    });
    assert_eq!(
        png.len(),
        want.len(),
        "the rasterizer's output changed size ({} vs {} bytes)",
        png.len(),
        want.len()
    );
    assert!(
        png == want,
        "the rasterizer no longer matches {}.\n\
         On CI this most likely means the output differs between architectures; \
         locally it means the rasterizer changed.",
        path.display()
    );
}

#[test]
fn rasterizing_twice_gives_the_same_bytes() {
    // The cheap companion to the reference: within one process, nothing about
    // the rasterizer may depend on allocation addresses or iteration order.
    assert_eq!(render(), render());
}

#[test]
fn blending_is_premultiplied_source_over_in_gamma_space() {
    // Pinned as arithmetic rather than only through the reference image, so a
    // failure says *what* is wrong rather than "some pixels moved".
    //
    // egui's preferred framebuffer is not sRGB, so the pipeline blends in gamma
    // space with no colour conversion — which is what makes this a plain
    // weighted sum and keeps `powf` off the path entirely.
    let full = Rect::from_min_max(pos2(0.0, 0.0), pos2(4.0, 4.0));
    let bg = Color32::from_rgba_premultiplied(200, 100, 50, 255);
    let fg = Color32::from_rgba_premultiplied(40, 80, 20, 128);
    let prims = vec![
        ClippedPrimitive {
            clip_rect: full,
            primitive: Primitive::Mesh(solid(pos2(0.0, 0.0), pos2(4.0, 4.0), bg)),
        },
        ClippedPrimitive {
            clip_rect: full,
            primitive: Primitive::Mesh(solid(pos2(0.0, 0.0), pos2(4.0, 4.0), fg)),
        },
    ];
    let out = rasterize(&prims, &texture(), (4, 4), 1.0);

    // src + dst * (1 - src.a), rounded to eight bits.
    let expect = |s: u8, d: u8| -> u8 {
        let (s, d) = (f32::from(s) / 255.0, f32::from(d) / 255.0);
        let inv = 1.0 - f32::from(fg.a()) / 255.0;
        (s + d * inv).mul_add(255.0, 0.5) as u8
    };
    let px = &out.rgba[..4];
    assert_eq!(px[0], expect(fg.r(), bg.r()), "red");
    assert_eq!(px[1], expect(fg.g(), bg.g()), "green");
    assert_eq!(px[2], expect(fg.b(), bg.b()), "blue");
    assert_eq!(px[3], 255, "over an opaque background the result is opaque");
}

#[test]
fn a_clip_rect_bounds_what_is_drawn() {
    // The scene relies on this; asserting it separately means a clipping
    // regression names itself instead of moving pixels in a reference image.
    let clip = Rect::from_min_max(pos2(0.0, 0.0), pos2(2.0, 4.0));
    let prims = vec![ClippedPrimitive {
        clip_rect: clip,
        primitive: Primitive::Mesh(solid(pos2(0.0, 0.0), pos2(4.0, 4.0), Color32::WHITE)),
    }];
    let out = rasterize(&prims, &texture(), (4, 4), 1.0);
    for y in 0..4 {
        for x in 0..4 {
            let px = &out.rgba[(y * 4 + x) * 4..][..4];
            if x < 2 {
                assert_eq!(px[0], 255, "({x},{y}) should be inside the clip");
            } else {
                assert_eq!(px[0], 0, "({x},{y}) should be clipped away");
            }
        }
    }
}

#[test]
fn pixels_per_point_scales_the_output() {
    // A 2x capture is the same scene at twice the resolution, not a crop.
    let full = Rect::from_min_max(pos2(0.0, 0.0), pos2(4.0, 4.0));
    let prims = vec![ClippedPrimitive {
        clip_rect: full,
        primitive: Primitive::Mesh(solid(pos2(0.0, 0.0), pos2(2.0, 2.0), Color32::WHITE)),
    }];
    let out = rasterize(&prims, &texture(), (8, 8), 2.0);
    assert_eq!((out.width, out.height), (8, 8));
    // The quad covered the top-left quarter in points, so it covers the
    // top-left quarter in pixels too.
    let at = |x: usize, y: usize| out.rgba[(y * 8 + x) * 4];
    assert_eq!(at(1, 1), 255, "inside the scaled quad");
    assert_eq!(at(6, 6), 0, "outside it");
}
