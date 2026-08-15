// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The `view.capture` block: where captures go, and how video is encoded.

use std::path::PathBuf;

use serde::Deserialize;

use super::Defaults;

#[derive(Debug, Deserialize)]
pub struct CaptureConfig {
    /// Output directory, relative to the working directory unless absolute.  A
    /// leading `~/` expands against `$HOME`; the directory is created on the
    /// first capture, not at startup.
    pub dir: Option<String>,
    /// Include the help, settings and instrument overlays in a capture.
    pub overlays: Option<bool>,
    /// Video frame rate, in frames per second.
    pub fps: Option<u32>,
    /// Video container: `mp4` (piped to ffmpeg) or `png` (a frame sequence).
    pub format: Option<String>,
}

/// What a video recording produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CaptureFormat {
    /// H.264 in MP4, encoded by piping raw frames to `ffmpeg`.
    #[default]
    Mp4,
    /// A numbered PNG per frame, in a directory of its own.  No external
    /// dependency, at a large multiple of the size.
    PngSequence,
}

impl CaptureFormat {
    /// Parse a `format:` value.  Unrecognised spellings fall back to the
    /// default rather than failing the load, like every other key in this
    /// schema — see [`ViewConfig`](super::ViewConfig).
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "png" | "png-sequence" | "png_sequence" => Self::PngSequence,
            _ => Self::Mp4,
        }
    }
}

/// Expand a leading `~/` against `$HOME`.
///
/// Only a leading `~/` (and a bare `~`), which is the whole of what a shell
/// would have done had the value not come from a config file.  A `~user` form
/// needs the password database and is not supported.
pub fn expand_tilde(s: &str) -> PathBuf {
    let Some(rest) = s.strip_prefix('~') else {
        return PathBuf::from(s);
    };
    if !(rest.is_empty() || rest.starts_with('/')) {
        return PathBuf::from(s);
    }
    match std::env::var_os("HOME") {
        Some(home) => PathBuf::from(home).join(rest.trim_start_matches('/')),
        // No `$HOME` to expand against: leave it alone rather than silently
        // writing to a directory literally named `~`.
        None => PathBuf::from(s),
    }
}

impl super::ViewConfig {
    /// Where captures are written.
    pub fn capture_dir(&self) -> PathBuf {
        let raw = self
            .capture
            .as_ref()
            .and_then(|c| c.dir.as_deref())
            .unwrap_or(Defaults::CAPTURE_DIR);
        expand_tilde(raw)
    }

    /// Whether overlays appear in a capture.
    pub fn capture_overlays(&self) -> bool {
        self.capture
            .as_ref()
            .and_then(|c| c.overlays)
            .unwrap_or(Defaults::CAPTURE_OVERLAYS)
    }

    /// Video frame rate.  Clamped to a sane range: zero would divide by zero in
    /// the resampler, and anything above the display's own rate can only
    /// duplicate frames.
    pub fn capture_fps(&self) -> u32 {
        self.capture
            .as_ref()
            .and_then(|c| c.fps)
            .unwrap_or(Defaults::CAPTURE_FPS)
            .clamp(1, 240)
    }

    /// Video container.
    pub fn capture_format(&self) -> CaptureFormat {
        self.capture
            .as_ref()
            .and_then(|c| c.format.as_deref())
            .map_or(CaptureFormat::default(), CaptureFormat::parse)
    }
}
