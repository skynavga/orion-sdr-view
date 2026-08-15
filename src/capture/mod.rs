// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

pub mod common;
pub mod encode;
pub mod meta;
pub mod sink;
pub mod writer;

pub use common::{CaptureStats, CaptureTag, CfrResampler, Frame};
pub use encode::{encode_png, write_png};
pub use meta::{RecordingMeta, SceneInfo, StillMeta, sidecar_path, write_json};
pub use sink::{FfmpegSink, FrameSink, PngSequenceSink, ffmpeg_args, ffmpeg_available};
pub use writer::{CaptureWriter, Recorder, still_name};
