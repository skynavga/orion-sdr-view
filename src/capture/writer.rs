// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The recording pipeline: a bounded queue, a writer thread, and the frame
//! accounting that keeps a recording honest about what it lost.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{SyncSender, TrySendError};

use super::common::{CaptureStats, CfrResampler, Frame};
use super::sink::FrameSink;

/// How many frames may be in flight to the writer.
///
/// **Small on purpose.**  A 2400x1656 readback is 15.9 MB, so a queue of four
/// is 64 MB of resident memory that buys 66 ms of slack at 60 fps.  A deep
/// queue would trade a bounded, *counted* frame drop for unbounded memory
/// growth and a stall on the render thread — the wrong way round for a display
/// that must stay responsive.
pub const QUEUE_DEPTH: usize = 4;

/// The filename for a still: an ISO 8601 basic-format stamp plus `.png`.
///
/// e.g. `20260815T112233.456Z.png`.  Sorts into capture order, carries its own
/// offset, and needs no quoting in a shell.
pub fn still_name(t: std::time::SystemTime, offset_min: i32) -> String {
    format!("{}.png", crate::utils::format::format_stamp(t, offset_min))
}

/// The name for a recording, given an extension (`mp4`) or none (a directory of
/// PNGs).
pub fn recording_name(t: std::time::SystemTime, offset_min: i32, ext: Option<&str>) -> String {
    let stamp = crate::utils::format::format_stamp(t, offset_min);
    match ext {
        Some(e) => format!("{stamp}.{e}"),
        None => stamp,
    }
}

/// Drives one recording: resamples to a constant rate, feeds the sink, counts
/// what was lost.
///
/// Synchronous and sink-agnostic, so a test can drive it directly and assert on
/// the accounting without a thread, an encoder or a filesystem.
pub struct Recorder {
    sink: Box<dyn FrameSink>,
    resampler: CfrResampler,
    stats: CaptureStats,
    size: Option<(u32, u32)>,
    /// The sequence number expected next, for spotting a frame that vanished
    /// without the queue reporting it full.
    next_seq: Option<u64>,
    first_time: Option<std::time::SystemTime>,
    last_time: Option<std::time::SystemTime>,
}

/// Why a recording stopped early.
#[derive(Debug)]
pub enum RecordError {
    /// The frame size changed mid-recording — the window was resized, or moved
    /// between displays of different scale factors.
    ///
    /// Fatal rather than papered over: a rawvideo stream carries no way to
    /// signal a resolution change, so feeding ffmpeg a differently-sized frame
    /// produces a corrupt file rather than an error.
    SizeChanged {
        from: (u32, u32),
        to: (u32, u32),
    },
    Io(std::io::Error),
}

impl std::fmt::Display for RecordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SizeChanged { from, to } => write!(
                f,
                "the window changed size mid-recording ({}x{} to {}x{}); \
                 a video stream cannot carry that",
                from.0, from.1, to.0, to.1
            ),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for RecordError {}

impl From<std::io::Error> for RecordError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl Recorder {
    pub fn new(sink: Box<dyn FrameSink>, fps: u32) -> Self {
        Self {
            sink,
            resampler: CfrResampler::new(fps),
            stats: CaptureStats::default(),
            size: None,
            next_seq: None,
            first_time: None,
            last_time: None,
        }
    }

    /// Take one frame off the queue.
    pub fn push(&mut self, frame: &Frame) -> Result<(), RecordError> {
        match self.size {
            None => {
                self.sink.open(frame.width, frame.height)?;
                self.size = Some((frame.width, frame.height));
                self.first_time = Some(frame.tag.content_time);
            }
            Some(size) if size != (frame.width, frame.height) => {
                return Err(RecordError::SizeChanged {
                    from: size,
                    to: (frame.width, frame.height),
                });
            }
            Some(_) => {}
        }

        // A hole in the sequence that the queue-full count does not explain is
        // a frame lost somewhere else.  Counted separately for the same reason
        // `CofdmRxStats` separates `failed` from `lost`: a drop nobody reported
        // is invisible without the sequence numbers, and silence reads as a
        // clean recording.
        if let Some(expected) = self.next_seq
            && frame.tag.seq > expected
        {
            self.stats.lost += frame.tag.seq - expected;
        }
        self.next_seq = Some(frame.tag.seq + 1);
        self.last_time = Some(frame.tag.content_time);
        self.stats.queued += 1;

        let times = self.resampler.admit(frame.tag.content_time);
        if times == 0 {
            self.stats.superseded += 1;
        }
        for _ in 0..times {
            self.sink.write_frame(&frame.rgba)?;
            self.stats.written += 1;
        }
        Ok(())
    }

    /// Record that the render thread could not hand a frame over.
    pub fn note_queue_full(&mut self, n: u64) {
        self.stats.dropped_full += n;
    }

    pub fn stats(&self) -> CaptureStats {
        self.stats
    }

    pub fn output_path(&self) -> PathBuf {
        self.sink.output_path()
    }

    pub fn span(&self) -> (Option<std::time::SystemTime>, Option<std::time::SystemTime>) {
        (self.first_time, self.last_time)
    }

    pub fn size(&self) -> Option<(u32, u32)> {
        self.size
    }

    /// Close the sink and report.
    pub fn finish(mut self) -> Result<CaptureStats, RecordError> {
        self.sink.finish()?;
        Ok(self.stats)
    }
}

/// What the writer thread reports when it stops.
pub struct CaptureReport {
    pub stats: CaptureStats,
    pub output: PathBuf,
    pub size: Option<(u32, u32)>,
    pub span: (Option<std::time::SystemTime>, Option<std::time::SystemTime>),
    /// Set when the recording stopped itself — a resize, or an encoder failure.
    pub error: Option<String>,
}

/// A recording running on its own thread behind a bounded queue.
///
/// The queue is what keeps encoding off the render thread; the bound is what
/// keeps a slow encoder from growing memory without limit.  A frame that will
/// not fit is dropped **and counted**, never silently discarded and never
/// waited on.
pub struct CaptureWriter {
    tx: Option<SyncSender<Frame>>,
    handle: Option<std::thread::JoinHandle<CaptureReport>>,
    /// Frames the queue refused, tallied here because the recorder is on the
    /// far side of it and cannot see them.
    dropped_full: u64,
}

impl CaptureWriter {
    /// Start a recording.
    pub fn start(sink: Box<dyn FrameSink>, fps: u32) -> Self {
        let (tx, rx) = std::sync::mpsc::sync_channel::<Frame>(QUEUE_DEPTH);
        let handle = std::thread::spawn(move || {
            let mut recorder = Recorder::new(sink, fps);
            let mut error = None;
            while let Ok(frame) = rx.recv() {
                if let Err(e) = recorder.push(&frame) {
                    error = Some(e.to_string());
                    break;
                }
            }
            // Drain whatever is still queued so the count is not distorted by
            // frames that were handed over and then abandoned.
            let size = recorder.size();
            let span = recorder.span();
            let output = recorder.output_path();
            let stats = match recorder.finish() {
                Ok(s) => s,
                Err(e) => {
                    error.get_or_insert_with(|| e.to_string());
                    CaptureStats::default()
                }
            };
            CaptureReport {
                stats,
                output,
                size,
                span,
                error,
            }
        });
        Self {
            tx: Some(tx),
            handle: Some(handle),
            dropped_full: 0,
        }
    }

    /// Hand a frame over, or count it as dropped.
    ///
    /// Never blocks: the caller is the render thread, and a stall there would
    /// change the very thing being recorded.
    pub fn submit(&mut self, frame: Frame) {
        let Some(tx) = self.tx.as_ref() else {
            self.dropped_full += 1;
            return;
        };
        match tx.try_send(frame) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => self.dropped_full += 1,
            // The writer thread has stopped — a resize or an encoder failure.
            // Count it; `stop` will surface the reason.
            Err(TrySendError::Disconnected(_)) => self.dropped_full += 1,
        }
    }

    /// Whether the writer thread has stopped of its own accord.
    pub fn has_stopped(&self) -> bool {
        self.handle.as_ref().is_some_and(|h| h.is_finished())
    }

    /// Stop the recording and collect the report.
    pub fn stop(mut self) -> CaptureReport {
        // Dropping the sender is what ends the thread's `recv` loop.
        self.tx = None;
        let mut report = match self.handle.take() {
            Some(h) => h.join().unwrap_or_else(|_| CaptureReport {
                stats: CaptureStats::default(),
                output: PathBuf::new(),
                size: None,
                span: (None, None),
                error: Some("the capture writer panicked".to_owned()),
            }),
            None => CaptureReport {
                stats: CaptureStats::default(),
                output: PathBuf::new(),
                size: None,
                span: (None, None),
                error: None,
            },
        };
        report.stats.dropped_full += self.dropped_full;
        report
    }
}

/// Write a still and its metadata sidecar, returning the image path.
pub fn write_still(
    dir: &Path,
    frame: &Frame,
    offset_min: i32,
    scene: super::meta::SceneInfo,
) -> std::io::Result<PathBuf> {
    let path = dir.join(still_name(frame.tag.content_time, offset_min));
    super::encode::write_png(&path, frame.width, frame.height, &frame.rgba)?;
    let meta = super::meta::StillMeta::new(
        &path,
        frame.tag.content_time,
        offset_min,
        frame.tag.seq,
        frame.width,
        frame.height,
        scene,
    );
    super::meta::write_json(&super::meta::sidecar_path(&path), &meta)?;
    Ok(path)
}
