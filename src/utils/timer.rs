// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Generic loop/phase timer used by the decode bar.
//!
//! Tracks signal-vs-gap phase timing and a wraparound loop counter.  Sources
//! with intra-message keying gaps (e.g. CW) configure a holdoff so brief
//! silences are not treated as transmission gaps; sources with no holdoff
//! transition immediately on the first below-threshold block.

use crate::decode::SIGNAL_THRESHOLD;

pub struct LoopTimer {
    pub in_signal: bool,
    pub phase_secs: f32,
    pub loop_count: u32,
    /// Consecutive seconds of silence (block RMS below threshold).
    silence_secs: f32,
    /// Holdoff duration: silence must exceed this to declare a gap.
    /// Zero disables holdoff (immediate transition).
    holdoff_secs: f32,
    /// Block RMS at or above which the source counts as transmitting.
    /// Per-source; see [`LoopTimer::set_signal_threshold`].
    signal_threshold: f32,
    /// True for one frame on gap → signal transition.
    pub signal_onset: bool,
    /// True for one frame on signal → gap transition (after holdoff).
    pub gap_onset: bool,
}

impl Default for LoopTimer {
    fn default() -> Self {
        Self::new()
    }
}

impl LoopTimer {
    pub fn new() -> Self {
        Self {
            in_signal: false,
            phase_secs: 0.0,
            loop_count: 0,
            silence_secs: 0.0,
            holdoff_secs: 0.0,
            signal_threshold: SIGNAL_THRESHOLD,
            signal_onset: false,
            gap_onset: false,
        }
    }

    pub fn reset(&mut self) {
        self.in_signal = false;
        self.phase_secs = 0.0;
        self.loop_count = 0;
        self.silence_secs = 0.0;
        self.signal_onset = false;
        self.gap_onset = false;
    }

    /// Set the holdoff duration.  Pass 0.0 for modes without keying gaps.
    pub fn set_holdoff(&mut self, secs: f32) {
        self.holdoff_secs = secs.max(0.0);
    }

    /// Set the block-RMS level that counts as signal.
    ///
    /// Defaults to [`SIGNAL_THRESHOLD`], which assumes a unit-scale source.  A
    /// source that is not unit-scale — or whose own noise floor can rise above
    /// that level — must supply its own, or its gaps stop being detected.
    pub fn set_signal_threshold(&mut self, rms: f32) {
        self.signal_threshold = rms.max(0.0);
    }

    /// Call once per frame with the measured block RMS and the frame duration.
    pub fn tick(&mut self, rms: f32, dt: f32) {
        self.tick_active(rms >= self.signal_threshold, dt);
    }

    /// As [`tick`](Self::tick), with the signal/silence decision already made —
    /// for a source that reports its own phase rather than being measured.
    pub fn tick_active(&mut self, active: bool, dt: f32) {
        self.signal_onset = false;
        self.gap_onset = false;

        if active {
            self.silence_secs = 0.0;
            if !self.in_signal {
                self.loop_count = (self.loop_count + 1) % 1000;
                self.in_signal = true;
                self.signal_onset = true;
                self.phase_secs = 0.0;
            } else {
                self.phase_secs += dt;
            }
        } else {
            self.silence_secs += dt;
            if self.in_signal {
                if self.silence_secs > self.holdoff_secs {
                    self.in_signal = false;
                    self.gap_onset = true;
                    self.phase_secs = 0.0;
                } else {
                    self.phase_secs += dt;
                }
            } else {
                self.phase_secs += dt;
            }
        }
    }

    /// Formatted string: `"sig 12.34 s loop 007"`, or `"sig 99.99+s loop 001"`
    /// once the phase has run past what the field can show.
    ///
    /// **The marker slot is always present** — a space when the timer has not
    /// overflowed — so the field width never changes and the right-aligned HUD
    /// cannot reflow.  Same convention as a wrapped error count's `000042+`,
    /// and for the same reason: `99.99` and `99.99+` are very different
    /// readings, and clamping without saying so under-reports silently.
    ///
    /// This is what lets a burst outlast the display.  Sources used to truncate
    /// themselves at [`MAX_SIG_SECS`](crate::source::MAX_SIG_SECS) so this
    /// string could not grow; the bound belongs here, not in the signal.
    pub fn label(&self) -> String {
        let kind = if self.in_signal { "sig" } else { "gap" };
        let over = self.phase_secs > crate::source::MAX_SIG_SECS;
        let secs = self.phase_secs.min(crate::source::MAX_SIG_SECS);
        let mark = if over { '+' } else { ' ' };
        format!("{kind} {secs:05.2}{mark}s loop {:03}", self.loop_count)
    }
}
