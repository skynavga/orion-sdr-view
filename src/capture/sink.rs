// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Where a recording's frames go: a numbered PNG each, or a pipe to `ffmpeg`.

use std::io::Write;
use std::path::{Path, PathBuf};

use super::encode::write_png;

/// A destination for constant-rate RGBA frames.
///
/// A trait rather than an enum so a test can substitute a counting sink and
/// drive the whole recorder — resampling, sequencing, drop accounting — with no
/// encoder, no subprocess and no filesystem.
pub trait FrameSink: Send {
    /// Called once, with the first frame's dimensions.
    ///
    /// **The size is not known until then.**  It is the physical surface size,
    /// which depends on the display's scale factor, so it cannot be derived
    /// from the window's logical size at the moment recording starts.
    fn open(&mut self, width: u32, height: u32) -> std::io::Result<()>;

    /// Write one frame, already resampled to the target rate.
    fn write_frame(&mut self, rgba: &[u8]) -> std::io::Result<()>;

    /// Close the destination.  Called once, even if nothing was written.
    fn finish(&mut self) -> std::io::Result<()>;

    /// The artifact produced, for reporting.
    fn output_path(&self) -> PathBuf;
}

/// The command line for piping raw frames to `ffmpeg`.
///
/// Split out so the arguments can be asserted in a test: a wrong `-pix_fmt` or
/// a transposed `-s` produces a video that is merely *wrong* rather than
/// missing, which is the kind of defect that survives a long time.
///
/// `-pix_fmt yuv420p` on the output — not just the input — because H.264 in
/// RGB is unplayable in most consumer players.
pub fn ffmpeg_args(width: u32, height: u32, fps: u32, out: &Path) -> Vec<String> {
    vec![
        "-hide_banner".to_owned(),
        "-loglevel".to_owned(),
        "error".to_owned(),
        "-y".to_owned(),
        "-f".to_owned(),
        "rawvideo".to_owned(),
        "-pix_fmt".to_owned(),
        "rgba".to_owned(),
        "-s".to_owned(),
        format!("{width}x{height}"),
        "-framerate".to_owned(),
        fps.to_string(),
        "-i".to_owned(),
        "-".to_owned(),
        "-c:v".to_owned(),
        "libx264".to_owned(),
        "-preset".to_owned(),
        "veryfast".to_owned(),
        "-pix_fmt".to_owned(),
        "yuv420p".to_owned(),
        out.to_string_lossy().into_owned(),
    ]
}

/// Whether `ffmpeg` can be run.
///
/// **Checked when recording starts, not when the encoder spawns.**  The encoder
/// cannot spawn until the first frame arrives, because that is when the frame
/// size becomes known — so without this check a missing ffmpeg would be
/// discovered a frame *after* the user was told recording had begun.
pub fn ffmpeg_available() -> bool {
    std::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Pipes raw RGBA to `ffmpeg`, which encodes H.264 into an MP4.
///
/// Piping keeps a codec dependency tree out of a DSP tool; ffmpeg is a runtime
/// requirement for video alone.
pub struct FfmpegSink {
    out: PathBuf,
    fps: u32,
    child: Option<std::process::Child>,
}

impl FfmpegSink {
    pub fn new(out: PathBuf, fps: u32) -> Self {
        Self {
            out,
            fps,
            child: None,
        }
    }
}

impl FrameSink for FfmpegSink {
    fn open(&mut self, width: u32, height: u32) -> std::io::Result<()> {
        if let Some(dir) = self.out.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let child = std::process::Command::new("ffmpeg")
            .args(ffmpeg_args(width, height, self.fps, &self.out))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn()?;
        self.child = Some(child);
        Ok(())
    }

    fn write_frame(&mut self, rgba: &[u8]) -> std::io::Result<()> {
        let child = self
            .child
            .as_mut()
            .ok_or_else(|| std::io::Error::other("ffmpeg was not started"))?;
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| std::io::Error::other("ffmpeg stdin is closed"))?;
        stdin.write_all(rgba)
    }

    fn finish(&mut self) -> std::io::Result<()> {
        let Some(mut child) = self.child.take() else {
            return Ok(());
        };
        // Close stdin first: ffmpeg finalizes the container on EOF, so waiting
        // without dropping the pipe would deadlock.
        drop(child.stdin.take());
        let status = child.wait()?;
        if status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other(format!(
                "ffmpeg exited with {status}"
            )))
        }
    }

    fn output_path(&self) -> PathBuf {
        self.out.clone()
    }
}

/// Writes each frame as `NNNNNN.png` inside a directory of its own.
///
/// No external dependency, at a large multiple of the size — useful when
/// ffmpeg is unavailable, or when the frames are wanted individually.
pub struct PngSequenceSink {
    dir: PathBuf,
    size: Option<(u32, u32)>,
    index: u64,
}

impl PngSequenceSink {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            size: None,
            index: 0,
        }
    }
}

impl FrameSink for PngSequenceSink {
    fn open(&mut self, width: u32, height: u32) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        self.size = Some((width, height));
        Ok(())
    }

    fn write_frame(&mut self, rgba: &[u8]) -> std::io::Result<()> {
        let (w, h) = self
            .size
            .ok_or_else(|| std::io::Error::other("the sequence was not opened"))?;
        // Zero-padded and monotonic, so the frames sort into playback order and
        // `ffmpeg -i %06d.png` can pick them up later.
        let path = self.dir.join(format!("{:06}.png", self.index));
        self.index += 1;
        write_png(&path, w, h, rgba)
    }

    fn finish(&mut self) -> std::io::Result<()> {
        Ok(())
    }

    fn output_path(&self) -> PathBuf {
        self.dir.clone()
    }
}
