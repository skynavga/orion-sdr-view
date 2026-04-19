// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

mod draw;
mod sources;
mod view;

pub(super) mod freqview;
pub(super) mod persistence;
pub(super) mod settings;
pub(super) mod spectrogram;
pub(super) mod spectrum;
pub(super) mod waterfall;

pub(crate) use view::ViewApp;

use eframe::egui;

// ── Constants ─────────────────────────────────────────────────────────────────

pub(super) const PANE_BG: [egui::Color32; 3] = [
    egui::Color32::from_rgb(10, 10, 20),
    egui::Color32::from_rgb(20, 50, 40),
    egui::Color32::from_rgb(40, 30, 60),
];

pub(super) const FFT_SIZE: usize = 1024;
pub(super) const SAMPLE_RATE: f32 = 48_000.0;
/// Number of new samples fed per frame, targeting ~60 fps.
pub(super) const SAMPLES_PER_FRAME: usize = (SAMPLE_RATE / 60.0) as usize;
/// Fixed pixel height of the decode bar (does not participate in pane proportions).
pub(crate) const DECODE_BAR_H: f32 = 28.0;

// ── Loop timer ────────────────────────────────────────────────────────────────

use crate::decode::SIGNAL_THRESHOLD;

/// Tracks signal/gap phase timing and loop iteration count for the decode bar.
///
/// For modes with keying gaps (CW), a holdoff prevents brief intra-message
/// silences from being treated as transmission gaps.  Set `holdoff_secs` > 0
/// to enable; silence must persist longer than the holdoff before the timer
/// transitions to gap.
pub(super) struct LoopTimer {
    pub(super) in_signal: bool,
    pub(super) phase_secs: f32,
    pub(super) loop_count: u32,
    /// Consecutive seconds of silence (block RMS below threshold).
    silence_secs: f32,
    /// Holdoff duration: silence must exceed this to declare a gap.
    /// Zero disables holdoff (immediate transition, suitable for PSK31/FT8/etc.).
    holdoff_secs: f32,
    /// Set to `true` for one frame on gap→signal transition, `false` otherwise.
    pub(super) signal_onset: bool,
    /// Set to `true` for one frame on signal→gap transition (after holdoff),
    /// `false` otherwise.
    pub(super) gap_onset: bool,
}

impl LoopTimer {
    pub(super) fn new() -> Self {
        Self {
            in_signal: false,
            phase_secs: 0.0,
            loop_count: 0,
            silence_secs: 0.0,
            holdoff_secs: 0.0,
            signal_onset: false,
            gap_onset: false,
        }
    }

    pub(super) fn reset(&mut self) {
        self.in_signal = false;
        self.phase_secs = 0.0;
        self.loop_count = 0;
        self.silence_secs = 0.0;
        self.signal_onset = false;
        self.gap_onset = false;
    }

    /// Set the holdoff duration.  Call when the source mode or CW parameters
    /// change.  Pass 0.0 for modes without keying gaps.
    pub(super) fn set_holdoff(&mut self, secs: f32) {
        self.holdoff_secs = secs.max(0.0);
    }

    /// Call once per frame with the measured block RMS and the frame duration.
    pub(super) fn tick(&mut self, rms: f32, dt: f32) {
        let active = rms >= SIGNAL_THRESHOLD;
        self.signal_onset = false;
        self.gap_onset = false;

        if active {
            self.silence_secs = 0.0;
            if !self.in_signal {
                // Gap → signal transition.
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
                    // Silence persisted beyond holdoff — real gap.
                    self.in_signal = false;
                    self.gap_onset = true;
                    self.phase_secs = 0.0;
                } else {
                    // Within holdoff — still count as signal time.
                    self.phase_secs += dt;
                }
            } else {
                self.phase_secs += dt;
            }
        }
    }

    /// Formatted string: "sig 12.34s loop 007" or "gap 02.00s loop 007".
    /// Phase seconds are zero-padded and clamped to 99.99 to keep the width stable.
    pub(super) fn label(&self) -> String {
        let kind = if self.in_signal { "sig" } else { "gap" };
        let secs = self.phase_secs.min(99.99);
        format!("{kind} {secs:05.2}s loop {:03}", self.loop_count)
    }
}

// ── Decode bar mode ───────────────────────────────────────────────────────────

/// Three-state decode bar: off → info-only → text-only → off (cycles with D).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum DecodeBarMode {
    /// Bar hidden.
    Off,
    /// Bar visible; shows only signal info (modulation, freq, BW, SNR).
    Info,
    /// Bar visible; shows only decoded text ticker.
    Text,
}

impl DecodeBarMode {
    /// Cycle to the next mode.  `has_text` gates whether Text mode is reachable:
    /// non-text sources (Test Tone, AM DSB) skip straight from Info back to Off.
    pub(super) fn next(self, has_text: bool) -> Self {
        match self {
            Self::Off => Self::Info,
            Self::Info => {
                if has_text {
                    Self::Text
                } else {
                    Self::Off
                }
            }
            Self::Text => Self::Off,
        }
    }
    pub(super) fn is_visible(self) -> bool {
        self != Self::Off
    }
}

// ── Waterfall mode (pane 3) ───────────────────────────────────────────────────

/// Pane 3 layout: traditional vertical waterfall (time flows down, full
/// spectrum across the top) or horizontal spectrogram (frequency on the
/// y-axis around the primary marker, time on the x-axis with "now" at
/// the left).  Cycled by the `W` key.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum WaterfallMode {
    Vertical,
    Horizontal,
}

impl WaterfallMode {
    pub(super) fn next(self) -> Self {
        match self {
            Self::Vertical => Self::Horizontal,
            Self::Horizontal => Self::Vertical,
        }
    }
}

// ── Source mode ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SourceMode {
    TestTone,
    Cw,
    AmDsb,
    Psk31,
    Ft8,
}

impl SourceMode {
    pub(super) const ALL: &'static [SourceMode] = &[
        SourceMode::TestTone,
        SourceMode::Cw,
        SourceMode::AmDsb,
        SourceMode::Psk31,
        SourceMode::Ft8,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            SourceMode::TestTone => "Test Tone",
            SourceMode::Cw => "CW",
            SourceMode::AmDsb => "AM DSB",
            SourceMode::Psk31 => "PSK31",
            SourceMode::Ft8 => "FT8",
        }
    }

    pub(super) fn index(self) -> usize {
        Self::ALL.iter().position(|&m| m == self).unwrap_or(0)
    }

    pub(super) fn next(self) -> Self {
        let idx = (self.index() + 1) % Self::ALL.len();
        Self::ALL[idx]
    }
}
