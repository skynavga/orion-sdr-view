// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The JSON written beside every capture.
//!
//! **A capture outlives the session that made it.**  A PNG on its own says
//! nothing about which source produced it, at what sample rate, over what span,
//! or against what dB scale — all of which a reader needs before the picture
//! means anything.  The same reasoning as the replay dump's header record: the
//! artifact should describe itself rather than depend on someone remembering.

use std::path::Path;

use serde::Serialize;

use crate::utils::format::format_iso8601;

/// What the viewer was showing when the frame was taken.
///
/// Filled by the app at capture time.  Deliberately the *displayed* state — the
/// source, the viewport, the dB scale — rather than internals, because it is
/// there to make the image interpretable, not to dump the program.
#[derive(Debug, Clone, Serialize)]
pub struct SceneInfo {
    pub source: String,
    pub fs_hz: f32,
    pub center_hz: f32,
    pub span_hz: f32,
    pub db_min: f32,
    pub db_max: f32,
    /// Whether the help/settings/instrument overlays were drawn into it.
    pub overlays: bool,
}

/// The sidecar for a single still.
#[derive(Debug, Clone, Serialize)]
pub struct StillMeta {
    pub kind: &'static str,
    pub version: &'static str,
    /// The image this describes, as a bare filename — the two live in the same
    /// directory, so an absolute path here would only go stale when moved.
    pub file: String,
    /// Content time in ISO 8601 extended format, matching the basic-format
    /// stamp in the filename.
    pub time: String,
    pub seq: u64,
    pub width: u32,
    pub height: u32,
    /// Which pane this is, when it is a pane raster rather than a window still.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pane: Option<String>,
    /// The script's label for this capture, if it gave one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(flatten)]
    pub scene: SceneInfo,
}

/// The manifest for one recording.
#[derive(Debug, Clone, Serialize)]
pub struct RecordingMeta {
    pub kind: &'static str,
    pub version: &'static str,
    pub file: String,
    /// Content time of the first frame.
    pub started: String,
    /// Content time of the last frame handed to the writer.
    pub ended: String,
    pub width: u32,
    pub height: u32,
    /// The constant frame rate the file was resampled to.
    pub fps: u32,
    pub frames_written: u64,
    /// Frames the target rate discarded — expected, not a fault.
    pub frames_superseded: u64,
    /// Frames that went missing.  **Recorded even when zero**, so a reader can
    /// tell "none were lost" from "nobody counted".
    pub frames_dropped: u64,
    #[serde(flatten)]
    pub scene: SceneInfo,
}

impl StillMeta {
    pub fn new(
        file: &Path,
        time: std::time::SystemTime,
        offset_min: i32,
        seq: u64,
        width: u32,
        height: u32,
        scene: SceneInfo,
    ) -> Self {
        Self {
            kind: "still",
            version: env!("CARGO_PKG_VERSION"),
            file: file_name_of(file),
            time: format_iso8601(time, offset_min),
            seq,
            width,
            height,
            pane: None,
            label: None,
            scene,
        }
    }
}

/// The sidecar path for an image: the same path with a `.json` extension, so
/// the two sort together and neither can be mistaken for the other.
pub fn sidecar_path(image: &Path) -> std::path::PathBuf {
    image.with_extension("json")
}

fn file_name_of(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Write a metadata value as pretty JSON.
///
/// Pretty rather than compact: unlike the replay dump — which is one record per
/// line precisely so it can be streamed — this is a single object a person
/// opens to find out what they are looking at.
pub fn write_json<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(value).map_err(std::io::Error::other)?;
    std::fs::write(path, text + "\n")
}
