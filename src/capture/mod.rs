// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

pub mod common;
pub mod encode;
pub mod meta;
/// The CPU rasterizer for egui's tessellated output.
///
/// The one part of `capture` that is not render-stack-free: it consumes
/// `epaint` meshes, so it needs the `gui` feature that provides them.  Frames
/// arrive at everything below it as plain RGBA, which is why the encoder,
/// metadata and writer stay usable without it.
#[cfg(feature = "gui")]
pub mod raster;
pub mod sink;
pub mod writer;

pub use common::{CaptureStats, CaptureTag, CfrResampler, Frame};
pub use encode::{encode_png, write_png};
pub use meta::{RecordingMeta, SceneInfo, StillMeta, sidecar_path, write_json};
#[cfg(feature = "gui")]
pub use raster::{Raster, Texture, Textures, rasterize};
pub use sink::{FfmpegSink, FrameSink, PngSequenceSink, ffmpeg_args, ffmpeg_available};
pub use writer::{CaptureWriter, Recorder, still_name};
