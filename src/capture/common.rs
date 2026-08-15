// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Capture types shared by the app and the writer: what a frame is, what it is
//! tagged with, and the constant-frame-rate resampler that turns a stream of
//! irregularly-timed frames into a video whose duration is honest.

use std::time::SystemTime;

/// What travels with a capture request and comes back attached to the image.
///
/// **The timestamp is of the content, not of the callback.**  The command is
/// issued during frame N and the image returns one or more frames later, so
/// stamping it on arrival would smear the timeline by the readback latency.
/// This is the instant the frame's `dt` advance was applied — the state the
/// picture actually depicts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureTag {
    pub seq: u64,
    pub content_time: SystemTime,
}

/// One captured frame, decoupled from egui.
///
/// Carries plain RGBA rather than a `ColorImage` so everything below this point
/// — resampling, encoding, the writer thread — is testable without a render
/// stack, and so the writer cannot accidentally depend on the UI layer.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// Row-major RGBA8, top row first.  `width * height * 4` bytes.
    pub rgba: Vec<u8>,
    pub tag: CaptureTag,
}

impl Frame {
    /// Bytes a frame of this size occupies — the number that governs every
    /// buffering decision here.  A 2400x1656 readback is 15.9 MB.
    pub fn byte_len(width: u32, height: u32) -> usize {
        width as usize * height as usize * 4
    }

    /// Whether the buffer length matches the stated dimensions.
    pub fn is_well_formed(&self) -> bool {
        self.rgba.len() == Self::byte_len(self.width, self.height)
    }
}

/// What a recording did, reported when it stops.
///
/// **Drops are counted, never silent.**  This repo has expensive precedent:
/// `CofdmRxStats::lost` exists because a receiver that quietly discarded frames
/// read as a *perfect link*.  A capture that drops a third of its frames and
/// reports success is the same failure in a different costume.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CaptureStats {
    /// Frames handed to the writer.
    pub queued: u64,
    /// Frames the render thread could not hand over, because the writer was
    /// still busy and the bounded queue was full.
    pub dropped_full: u64,
    /// Frames missing from the sequence for any other reason — the readback
    /// itself losing one.  A gap the drop count does not already explain.
    pub lost: u64,
    /// Frames actually written, after constant-frame-rate resampling.  Larger
    /// than `queued` when the content clock ran slower than the target rate and
    /// frames had to be duplicated to fill the timeline.
    pub written: u64,
    /// Frames the resampler discarded because a later one landed in the same
    /// slot.  Not a fault: this is what recording a 60 fps display at 30 fps
    /// means.
    pub superseded: u64,
}

impl CaptureStats {
    /// Frames that went missing, however they went missing.
    pub fn missing(&self) -> u64 {
        self.dropped_full + self.lost
    }

    /// A one-line report for the user, naming losses explicitly.
    pub fn summary(&self) -> String {
        let mut s = format!("{} frames written", self.written);
        if self.superseded > 0 {
            let _ = std::fmt::Write::write_fmt(
                &mut s,
                format_args!(", {} superseded at the target rate", self.superseded),
            );
        }
        if self.missing() > 0 {
            let _ = std::fmt::Write::write_fmt(
                &mut s,
                format_args!(
                    ", {} DROPPED ({} queue-full, {} lost)",
                    self.missing(),
                    self.dropped_full,
                    self.lost
                ),
            );
        }
        s
    }
}

/// Maps irregularly-timed frames onto fixed slots `k / fps` from the first one.
///
/// **The file is constant frame rate; the resampling happens here.**  ffmpeg's
/// rawvideo demuxer assumes CFR, so the alternative would be a variable-rate
/// stream plus a timestamp sidecar to reconstruct it from.  Resampling instead
/// buys two things worth having anyway: the bookkeeping *is* the drop
/// accounting, and the video's wall-clock duration matches the session it
/// recorded.
#[derive(Debug, Clone)]
pub struct CfrResampler {
    fps: f64,
    origin: Option<SystemTime>,
    /// Number of slots emitted so far, i.e. the index of the next empty one.
    emitted: u64,
}

impl CfrResampler {
    pub fn new(fps: u32) -> Self {
        Self {
            fps: f64::from(fps.max(1)),
            origin: None,
            emitted: 0,
        }
    }

    /// How many times this frame should be written.
    ///
    /// * `0` — a later frame already reached this slot, so this one is
    ///   superseded.  Recording a 60 fps display at 30 fps drops every other
    ///   frame here, which is the intent rather than a loss.
    /// * `1` — the ordinary case.
    /// * `>1` — the content clock skipped slots, so this frame fills them.  A
    ///   stall, or a display slower than the target rate.
    ///
    /// Time is taken from the content tag, so a frame that sat in the queue
    /// still lands in the slot it depicts.
    pub fn admit(&mut self, content_time: SystemTime) -> u64 {
        let origin = *self.origin.get_or_insert(content_time);
        // `duration_since` saturates on a non-monotonic step backwards, which
        // puts the frame in the earliest slot rather than panicking.
        let elapsed = content_time
            .duration_since(origin)
            .unwrap_or_default()
            .as_secs_f64();
        // The highest slot this instant has reached, so slots `0..=reached`
        // are now covered and `emitted` of them are already written.
        let reached = (elapsed * self.fps).floor().max(0.0) as u64;
        let count = (reached + 1).saturating_sub(self.emitted);
        self.emitted += count;
        count
    }

    /// Slots written so far.
    pub fn emitted(&self) -> u64 {
        self.emitted
    }
}
