// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The app's side of capture: issuing the requests, receiving the images, and
//! deciding what to do with each one.
//!
//! The mechanism is `egui::ViewportCommand::Screenshot`, which eframe's wgpu
//! integration services by reading back the **surface texture** — the window's
//! client area.  macOS composites the title bar and border outside it, so
//! window decorations are excluded by construction: no cropping, no scale-factor
//! arithmetic, and no Screen Recording permission prompt.
//!
//! The readback is asynchronous.  `egui-wgpu` copies to a staging buffer and
//! calls `map_async`; a completed frame arrives on a later pass as an
//! `egui::Event::Screenshot`.  So a request and its image are separated by one
//! or more frames, which is why every request carries a
//! [`CaptureTag`](crate::capture::CaptureTag) naming the instant it depicts.

use std::path::PathBuf;

use crate::capture::{
    CaptureTag, CaptureWriter, FfmpegSink, Frame, FrameSink, PngSequenceSink, RecordingMeta,
    SceneInfo, ffmpeg_available, meta, writer,
};
use crate::config::CaptureFormat;
use crate::utils::script::Pane;
use crate::utils::term::Level;

/// What a capture request was for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CaptureKind {
    Still,
    Recording,
}

/// The app's capture state machine.
///
/// Owned by [`ViewApp`](super::ViewApp) and driven from the per-frame state
/// path — never from `draw`, because receiving an image and handing it to a
/// writer is state work, not drawing.
pub(super) struct CaptureController {
    dir: PathBuf,
    fps: u32,
    format: CaptureFormat,
    /// Whether overlays are drawn into a capture.  When false, the frames being
    /// captured are rendered without them — a frame cannot be drawn twice, so
    /// "capture without overlays" means the live window loses them too.
    overlays: bool,
    /// Monotonic across the whole session, so a gap in the numbers arriving is
    /// visible even across a stop and restart.
    next_seq: u64,
    /// Requests issued and not yet answered, oldest first.
    outstanding: Vec<(u64, CaptureKind)>,
    recording: Option<CaptureWriter>,
    /// Set on the frame recording begins, so the manifest can describe it.
    recording_scene: Option<SceneInfo>,
    /// Messages for the user, drained by the app each frame.  Carry a severity
    /// so a missing encoder does not read like a confirmation.
    pending_notices: Vec<(Level, String)>,
}

impl CaptureController {
    pub(super) fn new(dir: PathBuf, fps: u32, format: CaptureFormat, overlays: bool) -> Self {
        Self {
            dir,
            fps,
            format,
            overlays,
            next_seq: 0,
            outstanding: Vec::new(),
            recording: None,
            recording_scene: None,
            pending_notices: Vec::new(),
        }
    }

    /// Override the output directory, e.g. from `--capture <dir>`.
    pub(super) fn set_dir(&mut self, dir: PathBuf) {
        self.dir = dir;
    }

    pub(super) fn dir(&self) -> &std::path::Path {
        &self.dir
    }

    pub(super) fn is_recording(&self) -> bool {
        self.recording.is_some()
    }

    /// Whether overlays should be drawn this frame.
    ///
    /// False only while a capture is in flight and the config asks for clean
    /// frames.  For a still that is a single frame's flicker; for a recording it
    /// holds for the whole session, which is usually what a demo wants.
    pub(super) fn draw_overlays(&self) -> bool {
        self.overlays || (self.outstanding.is_empty() && self.recording.is_none())
    }

    pub(super) fn take_notices(&mut self) -> Vec<(Level, String)> {
        std::mem::take(&mut self.pending_notices)
    }

    fn notify(&mut self, level: Level, msg: impl Into<String>) {
        self.pending_notices.push((level, msg.into()));
    }

    /// The next sequence number, for a capture that does not go through the
    /// request/reply path at all.  Shares the counter so numbers stay unique
    /// across a session however a capture was taken.
    pub(super) fn next_pane_seq(&mut self) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        seq
    }

    /// Ask for one frame.  Returns the tag to attach to the viewport command.
    fn request(&mut self, kind: CaptureKind, content_time: std::time::SystemTime) -> CaptureTag {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.outstanding.push((seq, kind));
        CaptureTag { seq, content_time }
    }

    /// `F` — capture one still.
    pub(super) fn request_still(&mut self, content_time: std::time::SystemTime) -> CaptureTag {
        self.request(CaptureKind::Still, content_time)
    }

    /// The per-frame request while recording, if one is running.
    pub(super) fn request_recording_frame(
        &mut self,
        content_time: std::time::SystemTime,
    ) -> Option<CaptureTag> {
        self.recording.as_ref()?;
        Some(self.request(CaptureKind::Recording, content_time))
    }

    /// `V` — start or stop recording.  Returns true if it is now recording.
    pub(super) fn toggle_recording(&mut self, scene: SceneInfo, offset_min: i32) -> bool {
        if self.recording.is_some() {
            self.stop_recording(offset_min);
            return false;
        }
        self.start_recording(scene, offset_min)
    }

    fn start_recording(&mut self, scene: SceneInfo, offset_min: i32) -> bool {
        let stamp = crate::utils::format::format_stamp(std::time::SystemTime::now(), offset_min);
        let sink: Box<dyn FrameSink> = match self.format {
            CaptureFormat::Mp4 => {
                // Checked here rather than when the encoder spawns: the encoder
                // cannot start until the first frame arrives, since that is when
                // the physical frame size becomes known.  Without this the user
                // would be told recording had begun and find out a frame later
                // that it had not.
                if !ffmpeg_available() {
                    self.notify(
                        Level::Warn,
                        "capture: ffmpeg was not found on PATH, so mp4 recording cannot start \
                         (set capture.format: png for a frame sequence)",
                    );
                    return false;
                }
                Box::new(FfmpegSink::new(
                    self.dir.join(format!("{stamp}.mp4")),
                    self.fps,
                ))
            }
            CaptureFormat::PngSequence => Box::new(PngSequenceSink::new(self.dir.join(&stamp))),
        };
        self.recording = Some(CaptureWriter::start(sink, self.fps));
        self.recording_scene = Some(scene);
        self.notify(
            Level::Info,
            format!("capture: recording to {}", self.dir.display()),
        );
        true
    }

    /// Stop recording and report what it produced — including what it lost.
    pub(super) fn stop_recording(&mut self, offset_min: i32) {
        let Some(w) = self.recording.take() else {
            return;
        };
        // Requests already in flight will never be answered now; forget them so
        // they cannot be mistaken for a sequence gap in a later recording.
        self.outstanding
            .retain(|(_, kind)| *kind != CaptureKind::Recording);
        let report = w.stop();
        let scene = self.recording_scene.take();

        if let Some(err) = &report.error {
            self.notify(Level::Error, format!("capture: recording stopped — {err}"));
        }
        if let (Some((w_px, h_px)), Some(scene)) = (report.size, scene) {
            let (start, end) = report.span;
            let meta = RecordingMeta {
                kind: "recording",
                version: env!("CARGO_PKG_VERSION"),
                file: report
                    .output
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                started: iso(start, offset_min),
                ended: iso(end, offset_min),
                width: w_px,
                height: h_px,
                fps: self.fps,
                frames_written: report.stats.written,
                frames_superseded: report.stats.superseded,
                frames_dropped: report.stats.missing(),
                scene,
            };
            let path = manifest_path(&report.output);
            if let Err(e) = meta::write_json(&path, &meta) {
                self.notify(
                    Level::Error,
                    format!("capture: could not write {}: {e}", path.display()),
                );
            }
        }
        // A recording that lost frames is a warning, not a confirmation: the
        // whole point of counting them is that nobody has to notice on their own.
        let level = if report.stats.missing() > 0 {
            Level::Warn
        } else {
            Level::Info
        };
        self.notify(
            level,
            format!(
                "capture: {} -> {}",
                report.stats.summary(),
                report.output.display()
            ),
        );
    }

    /// One returned image.
    ///
    /// `seq` comes back on the tag, so a frame is matched to its request rather
    /// than assumed to be the next one.
    pub(super) fn on_image(
        &mut self,
        tag: CaptureTag,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
        scene: SceneInfo,
        offset_min: i32,
    ) {
        let Some(pos) = self.outstanding.iter().position(|(s, _)| *s == tag.seq) else {
            // An image nobody asked for, or one arriving after its recording
            // stopped.  Not fatal, but not silent either.
            self.notify(
                Level::Warn,
                format!("capture: unmatched frame {} discarded", tag.seq),
            );
            return;
        };
        let (_, kind) = self.outstanding.remove(pos);
        let frame = Frame {
            width,
            height,
            rgba,
            tag,
        };
        match kind {
            CaptureKind::Still => match writer::write_still(&self.dir, &frame, offset_min, scene) {
                Ok(path) => self.notify(Level::Info, format!("capture: wrote {}", path.display())),
                Err(e) => self.notify(
                    Level::Error,
                    format!("capture: could not write a still: {e}"),
                ),
            },
            CaptureKind::Recording => {
                if let Some(w) = self.recording.as_mut() {
                    w.submit(frame);
                    // The writer stops itself on a resize or an encoder failure;
                    // notice on the next frame rather than piling frames into a
                    // channel nobody is reading.
                    if w.has_stopped() {
                        self.stop_recording(offset_min);
                    }
                }
            }
        }
    }
}

/// What one `pane` capture needs to know.
///
/// Grouped rather than passed as seven arguments, which is both unreadable and
/// easy to transpose at a call site.
pub(super) struct PaneRequest<'a> {
    pub dir: &'a std::path::Path,
    pub pane: Pane,
    pub label: Option<&'a str>,
    pub seq: u64,
    pub now: std::time::SystemTime,
    pub offset_min: i32,
    pub scene: SceneInfo,
}

/// Write one pane's raster and its metadata sidecar, returning the image path.
///
/// Returns `Ok(None)` when the pane has no pixels yet — a run that captures
/// before any spectrum has been processed.  That is a legitimate outcome, not a
/// failure, but the caller is told so it can say so rather than leave a script
/// author wondering where the file went.
pub(super) fn write_pane(
    app: &super::ViewApp,
    req: PaneRequest<'_>,
) -> std::io::Result<Option<PathBuf>> {
    let Some((w, h, rgba)) = pane_raster(app, req.pane) else {
        return Ok(None);
    };
    let stamp = crate::utils::format::format_stamp(req.now, req.offset_min);
    let suffix = match req.label {
        Some(l) => format!("-{}-{l}", req.pane.name()),
        None => format!("-{}", req.pane.name()),
    };
    let path = req.dir.join(format!("{stamp}{suffix}.png"));
    crate::capture::write_png(&path, w, h, &rgba)?;

    let mut m = meta::StillMeta::new(&path, req.now, req.offset_min, req.seq, w, h, req.scene);
    m.kind = "pane";
    m.pane = Some(req.pane.name().to_owned());
    m.label = req.label.map(str::to_owned);
    meta::write_json(&meta::sidecar_path(&path), &m)?;
    Ok(Some(path))
}

/// One pane's CPU-side raster as RGBA, or `None` if it has no pixels yet.
///
/// **No renderer is involved.** Each of these panes keeps its own pixel buffer
/// so the ring arithmetic is assertable without a GPU; this reads the same
/// buffers, in the same display order the painter uses, so what lands in the
/// file is what the pane shows.
pub(super) fn pane_raster(app: &super::ViewApp, pane: Pane) -> Option<(u32, u32, Vec<u8>)> {
    let push = |out: &mut Vec<u8>, c: egui::Color32| out.extend_from_slice(&c.to_array());
    match pane {
        Pane::Waterfall => {
            let wf = app.waterfall();
            let (w, h) = (wf.freq_bins(), wf.filled());
            if w == 0 || h == 0 {
                return None;
            }
            let mut rgba = Vec::with_capacity(w * h * 4);
            for row in wf.rows_in_display_order() {
                for &c in row {
                    push(&mut rgba, c);
                }
            }
            Some((w as u32, h as u32, rgba))
        }
        Pane::Spectrogram => {
            let sg = app.spectrogram();
            let (w, h) = (sg.filled(), sg.freq_rows());
            if w == 0 || h == 0 {
                return None;
            }
            // Columns are yielded newest-first and each runs top to bottom, so
            // this transposes into the row-major order a PNG wants.
            let cols: Vec<Vec<egui::Color32>> = sg.cols_in_display_order().collect();
            let mut rgba = Vec::with_capacity(w * h * 4);
            for y in 0..h {
                for col in &cols {
                    push(&mut rgba, col[y]);
                }
            }
            Some((w as u32, h as u32, rgba))
        }
        Pane::Persistence => {
            let img = app.persistence_image()?;
            let (w, h) = (img.width(), img.height());
            if w == 0 || h == 0 {
                return None;
            }
            Some((w as u32, h as u32, img.as_raw().to_vec()))
        }
        Pane::Constellation => {
            let c = app.constellation();
            if c.is_empty() {
                return None;
            }
            let px = super::constellation::CONST_PX;
            let mut rgba = Vec::with_capacity(px * px * 4);
            for &c in c.pixels_in_display_order() {
                push(&mut rgba, c);
            }
            Some((px as u32, px as u32, rgba))
        }
        Pane::Correction => {
            let m = app.correction();
            let (w, h) = (m.cols(), m.filled());
            if w == 0 || h == 0 {
                return None;
            }
            let mut rgba = Vec::with_capacity(w * h * 4);
            for row in m.rows_in_display_order() {
                for &c in row {
                    push(&mut rgba, c);
                }
            }
            Some((w as u32, h as u32, rgba))
        }
    }
}

fn iso(t: Option<std::time::SystemTime>, offset_min: i32) -> String {
    t.map(|t| crate::utils::format::format_iso8601(t, offset_min))
        .unwrap_or_default()
}

/// The manifest beside a recording.
///
/// A `.mp4` gets `.json` next to it; a PNG-sequence *directory* gets the
/// manifest inside it, where it travels with the frames.
fn manifest_path(output: &std::path::Path) -> PathBuf {
    if output.is_dir() {
        output.join("recording.json")
    } else {
        meta::sidecar_path(output)
    }
}
