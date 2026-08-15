// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! PNG encoding for captured frames.

use std::io::{BufWriter, Write};
use std::path::Path;

/// Write RGBA8 pixels as a PNG.
///
/// `rgba` is row-major, top row first, `width * height * 4` bytes — the layout
/// `epaint::ColorImage` already stores, so the app hands its readback straight
/// through with no repacking.
///
/// **Alpha is written as-is.**  `Color32` is premultiplied, but a surface
/// readback is opaque throughout, so there is nothing to un-premultiply and
/// doing it anyway would only introduce rounding.  A frame that ever arrives
/// with real transparency would need that step added here.
pub fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> std::io::Result<()> {
    let expected = width as usize * height as usize * 4;
    if rgba.len() != expected {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "frame is {} bytes, but {width}x{height} RGBA needs {expected}",
                rgba.len()
            ),
        ));
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let file = std::fs::File::create(path)?;
    encode_png(BufWriter::new(file), width, height, rgba)
}

/// Encode to any sink.  Separate from [`write_png`] so a test can encode into a
/// `Vec<u8>` and decode it back without touching the filesystem.
pub fn encode_png<W: Write>(out: W, width: u32, height: u32, rgba: &[u8]) -> std::io::Result<()> {
    let mut encoder = png::Encoder::new(out, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    writer
        .write_image_data(rgba)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    writer
        .finish()
        .map_err(|e| std::io::Error::other(e.to_string()))
}
