// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Per-frame view state cached on the main thread for FT8/FT4 sources.
//!
//! Tracks frame counts, the most recent decoded-frame timestamp, the pending
//! signal-onset time (captured on rising edge, consumed on Gap{decoded:true}),
//! and the cached sub-mode + message-type (mirrored from the live source for
//! HUD/draw access without downcasting).
//!
//! This state is FT8-specific and lives in the lib so the per-frame update
//! logic — which has no dependency on UI settings or egui — stays out of the
//! bin's `view.rs` and `draw.rs`.

use std::time::SystemTime;

use crate::utils::format::format_time;

use super::source::{Ft8Mode, Ft8MsgType};

/// Frame count clamp: HUD shows three digits ("frm 999"), so wrap at 999.
const FRAME_COUNT_MAX: u32 = 999;
/// Placeholder timestamp shown when no onset has been captured yet.
const PLACEHOLDER_TIMESTAMP: &str = "--:--:--.---";

pub struct Ft8ViewState {
    /// Number of CRC-pass frames decoded since the last reset.
    pub frame_count: u32,
    /// Number of CRC-fail (or otherwise empty) decode attempts since reset.
    pub err_count: u32,
    /// Formatted timestamp of the most recently decoded frame (HH:MM:SS.mmm).
    pub last_timestamp: String,
    /// Wall-clock time when the current signal burst started — captured on
    /// rising edge, taken when the matching `Gap{decoded:true}` arrives.
    pub pending_onset: Option<SystemTime>,
    /// Cached sub-mode (FT8 vs. FT4) — mirrors the live source.
    pub mode: Ft8Mode,
    /// Cached message type (Standard vs. FreeText) — mirrors the live source.
    pub msg_type: Ft8MsgType,
}

impl Default for Ft8ViewState {
    fn default() -> Self {
        Self::new()
    }
}

impl Ft8ViewState {
    pub fn new() -> Self {
        Self {
            frame_count: 0,
            err_count: 0,
            last_timestamp: String::new(),
            pending_onset: None,
            mode: Ft8Mode::Ft8,
            msg_type: Ft8MsgType::Standard,
        }
    }

    /// Clear counters, timestamp, and pending onset — but preserve cached
    /// mode/msg_type (those are reset by the caller on source switch if needed).
    pub fn reset(&mut self) {
        self.frame_count = 0;
        self.err_count = 0;
        self.last_timestamp.clear();
        self.pending_onset = None;
    }

    /// Reset cached mode/msg_type to defaults.  Called when switching INTO FT8.
    pub fn reset_to_defaults(&mut self) {
        self.mode = Ft8Mode::Ft8;
        self.msg_type = Ft8MsgType::Standard;
    }

    /// Capture signal onset on rising edge.  Idempotent within a burst.
    pub fn on_signal_rising_edge(&mut self) {
        self.pending_onset = Some(SystemTime::now());
    }

    /// Increment the frame counter and consume `pending_onset` to set
    /// `last_timestamp`.  Call when a Gap{decoded:true} arrives.
    pub fn on_decoded_frame(&mut self, time_zone_offset_min: i32) {
        self.frame_count = self.frame_count.saturating_add(1).min(FRAME_COUNT_MAX);
        if let Some(onset) = self.pending_onset.take() {
            self.last_timestamp = format_time(onset, time_zone_offset_min);
        }
    }

    /// Increment the error counter.  Call when Gap{decoded:false} arrives.
    pub fn on_failed_frame(&mut self) {
        self.err_count = self.err_count.saturating_add(1).min(FRAME_COUNT_MAX);
    }

    /// Wrap a decoded frame's text with timestamp delimiters for the ticker:
    /// `"|| HH:MM:SS.mmm | <text> ||"`.  Uses `pending_onset` if available,
    /// falling back to `last_timestamp`.
    pub fn format_decoded_text(&self, text: &str, time_zone_offset_min: i32) -> String {
        let ts = if let Some(onset) = self.pending_onset {
            format_time(onset, time_zone_offset_min)
        } else {
            self.last_timestamp.clone()
        };
        let ts_str = if ts.is_empty() {
            PLACEHOLDER_TIMESTAMP.to_owned()
        } else {
            ts
        };
        format!("|| {ts_str} | {text} ||")
    }
}
