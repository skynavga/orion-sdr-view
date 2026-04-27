// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use eframe::egui;

// ── Constants ─────────────────────────────────────────────────────────────────

pub(crate) const PANE_BG: [egui::Color32; 3] = [
    egui::Color32::from_rgb(10, 10, 20),
    egui::Color32::from_rgb(20, 50, 40),
    egui::Color32::from_rgb(40, 30, 60),
];

pub(crate) const FFT_SIZE: usize = 1024;
pub(crate) const SAMPLE_RATE: f32 = 48_000.0;
/// Number of new samples fed per frame, targeting ~60 fps.
pub(crate) const SAMPLES_PER_FRAME: usize = (SAMPLE_RATE / 60.0) as usize;
/// Fixed pixel height of the decode bar (does not participate in pane proportions).
pub(crate) const DECODE_BAR_H: f32 = 28.0;

// ── Decode bar mode ───────────────────────────────────────────────────────────

/// Three-state decode bar: off → info-only → text-only → off (cycles with D).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum DecodeBarMode {
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
    pub(crate) fn next(self, has_text: bool) -> Self {
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
    pub(crate) fn is_visible(self) -> bool {
        self != Self::Off
    }
}

// ── Waterfall mode (pane 3) ───────────────────────────────────────────────────

/// Pane 3 layout: traditional vertical waterfall (time flows down, full
/// spectrum across the top) or horizontal spectrogram (frequency on the
/// y-axis around the primary marker, time on the x-axis with "now" at
/// the left).  Cycled by the `W` key.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum WaterfallMode {
    Vertical,
    Horizontal,
}

impl WaterfallMode {
    pub(crate) fn next(self) -> Self {
        match self {
            Self::Vertical => Self::Horizontal,
            Self::Horizontal => Self::Vertical,
        }
    }
}

// ── Source mode ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SourceMode {
    TestTone,
    Cw,
    AmDsb,
    Psk31,
    Ft8,
}

impl SourceMode {
    pub(crate) const ALL: &'static [SourceMode] = &[
        SourceMode::TestTone,
        SourceMode::Cw,
        SourceMode::AmDsb,
        SourceMode::Psk31,
        SourceMode::Ft8,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            SourceMode::TestTone => "Test Tone",
            SourceMode::Cw => "CW",
            SourceMode::AmDsb => "AM DSB",
            SourceMode::Psk31 => "PSK31",
            SourceMode::Ft8 => "FT8",
        }
    }

    pub(crate) fn index(self) -> usize {
        Self::ALL.iter().position(|&m| m == self).unwrap_or(0)
    }

    pub(crate) fn next(self) -> Self {
        let idx = (self.index() + 1) % Self::ALL.len();
        Self::ALL[idx]
    }
}

/// Borrow the static `SourceFactory` for a given source mode.  Adding a new
/// source: extend `SourceMode` and push a `Factory` impl into
/// `app::source::FACTORIES`.  No edit to this function.
pub(super) fn source_mode_factory(
    mode: SourceMode,
) -> &'static (dyn super::source::SourceFactory + Sync) {
    super::source::FACTORIES[mode.index()]
}
