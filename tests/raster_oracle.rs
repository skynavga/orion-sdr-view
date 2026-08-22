// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The CPU rasterizer against the real GPU pipeline.
//!
//! **This is the anti-divergence check.** `src/capture/raster.rs` is a second
//! renderer, and a second renderer's characteristic failure is drifting from the
//! first so gradually that nobody notices — a headless capture that stops
//! looking like the window it claims to show. So the same primitives are
//! rendered through `egui-wgpu`'s own shader and pipeline, offscreen, and the
//! two images are compared.
//!
//! # Why it is `#[ignore]`d
//!
//! It needs a GPU adapter. That is fine here (Metal) and absent on a stock
//! GitHub Actions Linux runner, so CI compiles it — which is the half that
//! catches a refactor breaking it — and does not run it. Same arrangement as
//! `tests/cofdm_link_budget.rs`, for the same reason.
//!
//! ```sh
//! cargo test --release --test raster_oracle -- --ignored --nocapture
//! ```
//!
//! # Why the comparison has a tolerance
//!
//! Not because the arithmetic is uncertain — the fragment stage is a multiply
//! and the blend a weighted sum, both exactly reproducible — but because
//! *coverage* is not. Which pixels a triangle claims along an edge is decided by
//! the hardware's own fill rule, and a disagreement of one pixel on a feathered
//! edge is expected. A systematic error, the kind worth catching, moves far more
//! than that.

#![cfg(feature = "gui")]

mod common;

use egui::epaint::ClippedPrimitive;
use orion_sdr_view::capture::{Textures, rasterize};

/// Fraction of pixels allowed to differ at all.
const MAX_DIFFERING: f64 = 0.02;
/// Largest per-channel difference tolerated on those pixels.
const MAX_CHANNEL_DELTA: u8 = 8;

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl Gpu {
    /// An offscreen device: no surface, no window, no swapchain.
    fn new() -> Option<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .ok()?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("oracle"),
            ..Default::default()
        }))
        .ok()?;
        Some(Self { device, queue })
    }

    /// Render `primitives` exactly as the interactive path does.
    fn render(
        &self,
        primitives: &[ClippedPrimitive],
        deltas: &egui::TexturesDelta,
        size: (u32, u32),
        ppp: f32,
    ) -> Vec<u8> {
        // `Rgba8Unorm`, not the sRGB variant: `preferred_framebuffer_format`
        // picks a non-sRGB target, which is what selects the gamma fragment
        // path the CPU rasterizer implements.  Choosing sRGB here would compare
        // against a pipeline the app never runs.
        const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

        let mut renderer = egui_wgpu::Renderer::new(
            &self.device,
            FORMAT,
            egui_wgpu::RendererOptions {
                // The shader's own four-tap bilinear, which is what the CPU side
                // implements — egui added this flag for exactly this comparison.
                predictable_texture_filtering: true,
                // Off on both sides: it is a deliberate noise function, and
                // reproducing it would test the noise rather than the renderer.
                dithering: false,
                ..Default::default()
            },
        );
        for (id, delta) in &deltas.set {
            renderer.update_texture(&self.device, &self.queue, *id, delta);
        }

        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("oracle-target"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [size.0, size.1],
            pixels_per_point: ppp,
        };
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        renderer.update_buffers(&self.device, &self.queue, &mut encoder, primitives, &screen);
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("oracle-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        // Opaque black, matching the CPU canvas's starting state.
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            renderer.render(&mut pass.forget_lifetime(), primitives, &screen);
        }

        // Readback rows are padded to 256 bytes, so the buffer is wider than the
        // image and the padding has to be stripped after mapping.
        let unpadded = size.0 as usize * 4;
        let padded = unpadded.div_ceil(256) * 256;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("oracle-readback"),
            size: (padded * size.1 as usize) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded as u32),
                    rows_per_image: Some(size.1),
                },
            },
            wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit(Some(encoder.finish()));

        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("the readback should complete");
        rx.recv().expect("map result").expect("map succeeded");

        let data = slice.get_mapped_range();
        let mut out = Vec::with_capacity(unpadded * size.1 as usize);
        for row in 0..size.1 as usize {
            out.extend_from_slice(&data[row * padded..row * padded + unpadded]);
        }
        drop(data);
        buffer.unmap();
        out
    }
}

/// How far apart two images are.
fn compare(cpu: &[u8], gpu: &[u8]) -> (f64, u8) {
    assert_eq!(cpu.len(), gpu.len(), "images differ in size");
    let mut differing = 0usize;
    let mut worst = 0u8;
    for (a, b) in cpu
        .as_chunks::<4>()
        .0
        .iter()
        .zip(gpu.as_chunks::<4>().0.iter())
    {
        let d = (0..4).map(|i| a[i].abs_diff(b[i])).max().unwrap_or(0);
        if d > 0 {
            differing += 1;
            worst = worst.max(d);
        }
    }
    (differing as f64 / (cpu.len() / 4) as f64, worst)
}

/// Drive the app to a steady COFDM display and take the frame's primitives.
fn app_frame(size: (f32, f32)) -> (Vec<ClippedPrimitive>, egui::TexturesDelta, Textures) {
    let mut h = common::harness::Harness::with_defaults();
    h.run_script("0.00 source COFDM\n0.50 key D\n");
    for _ in 0..90 {
        h.idle(1);
    }
    h.frame_primitives(size)
}

#[test]
#[ignore = "needs a GPU adapter; run explicitly with --ignored"]
fn the_cpu_rasterizer_agrees_with_the_gpu_pipeline() {
    let Some(gpu) = Gpu::new() else {
        panic!("no GPU adapter available; this test needs one");
    };
    let size_pt = (640.0f32, 480.0f32);
    let ppp = 1.0f32;
    let size_px = (size_pt.0 as u32, size_pt.1 as u32);

    let (primitives, deltas, textures) = app_frame(size_pt);
    assert!(!primitives.is_empty(), "the frame should have drawn");

    let cpu = rasterize(&primitives, &textures, size_px, ppp);
    assert_eq!(cpu.missing_textures, 0, "every texture should be present");
    let gpu_rgba = gpu.render(&primitives, &deltas, size_px, ppp);

    let (fraction, worst) = compare(&cpu.rgba, &gpu_rgba);
    println!(
        "\n  differing pixels: {:.3}%   worst channel delta: {worst}",
        fraction * 100.0
    );
    assert!(
        fraction <= MAX_DIFFERING,
        "{:.3}% of pixels differ (limit {:.1}%) — the two renderers have diverged",
        fraction * 100.0,
        MAX_DIFFERING * 100.0
    );
    assert!(
        worst <= MAX_CHANNEL_DELTA,
        "worst channel delta {worst} exceeds {MAX_CHANNEL_DELTA} — that is a \
         systematic error rather than edge coverage"
    );
}
