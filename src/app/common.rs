// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use eframe::egui;

// ── Constants ─────────────────────────────────────────────────────────────────

/// The colour the HUD paints its *data* in — the status line's readouts, as
/// opposed to the dim grey of a label or the plain white of the title.
///
/// Shared with pane 3's decoder-mode overlays so the two cannot drift: those
/// readouts (`off-scale`, the correction tally, the codeword geometry) are the
/// same kind of thing as `ctr`/`span`/`c/n`, and reading as a different kind of
/// thing was the point of the change.
pub(crate) const HUD_DATA_COL: egui::Color32 = egui::Color32::from_rgb(0, 200, 255);

pub(crate) const PANE_BG: [egui::Color32; 3] = [
    egui::Color32::from_rgb(10, 10, 20),
    egui::Color32::from_rgb(20, 50, 40),
    egui::Color32::from_rgb(40, 30, 60),
];

/// Wash over the part of a pane lying outside `0..nyquist`, once the viewport is
/// allowed to pan past a band edge.
///
/// Premultiplied black, so it darkens whichever `PANE_BG` it lands on rather
/// than replacing it — one constant for all three panes, and the pane keeps its
/// identity while the empty region stops looking like a place data could be.
pub(crate) const OFF_BAND_DIM: egui::Color32 = egui::Color32::from_rgba_premultiplied(0, 0, 0, 160);

/// [`OFF_BAND_DIM`] pre-composited over `PANE_BG[2]`, for the spectrogram.
///
/// That pane's off-band region is *inside* its texture rather than painted over
/// it — rows outside the band are written as pixels — and the texture is opaque,
/// so the wash has to be resolved to a solid colour here: `40·(1−160/255) ≈ 15`,
/// `30· ≈ 11`, `60· ≈ 22`.  Distinct from the colour ramp's floor, which is pure
/// black, so "no band here" cannot be read as "no signal here".
pub(crate) const OFF_BAND_SOLID: egui::Color32 = egui::Color32::from_rgb(15, 11, 22);

/// The band edge itself, drawn where `0` or Nyquist falls inside a pane.
///
/// Dimming alone is too weak a cue in the waterfall, where absent signal is
/// already dark.  A visible edge is what says the band *stops* here rather than
/// merely going quiet, and it is the cue that tells the user which way home is.
pub(crate) const BAND_EDGE_COL: egui::Color32 = egui::Color32::from_rgb(90, 90, 110);

pub(crate) const FFT_SIZE: usize = 1024;
pub(crate) const SAMPLE_RATE: f32 = 48_000.0;
/// Per-frame sample consumption is paced to wall-clock
/// (`dt * source.sample_rate()`, clamped to the bounds below) so time-based
/// playback (gaps, Test Tone ramp/pause, …) is frame-rate independent rather
/// than assuming a fixed 60 fps.
///
/// Lower bound on the per-frame sample budget, so the FFT keeps refreshing even
/// at very high frame rates (tiny `dt`).
pub(crate) const MIN_SAMPLES_PER_FRAME: usize = 128;
/// Upper bound on the per-frame sample budget, so a large `dt` (after a stall or
/// a high-`fs` source) can't dump an unbounded block into the pipeline.
pub(crate) const MAX_SAMPLES_PER_FRAME: usize = 4096;
/// Fixed pixel height of the decode bar (does not participate in pane proportions).
pub const DECODE_BAR_H: f32 = 28.0;

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

// ── Pane 3 mode ───────────────────────────────────────────────────────────────

/// What pane 3 shows, cycled by the `W` key:
///
/// - [`Waterfall`](Self::Waterfall) — the traditional vertical waterfall, time
///   flowing down, full spectrum across the top.
/// - [`Spectrogram`](Self::Spectrogram) — horizontal: frequency on the y-axis
///   around the primary marker, time on the x-axis with "now" at the left.
/// - [`Constellation`](Self::Constellation) — split: the equalizer's output on
///   the left, the inner decoder's per-bit correction map on the right.  Named
///   for its left half, which is what an operator would call the pane.
///
/// **Renamed from `WaterfallMode`.**  Two of the three variants are not
/// waterfalls, and the names now line up with the capture
/// [`Pane`](crate::utils::script::Pane) enum's, so a script's `pane
/// constellation` and the key that selects it agree.
///
/// The cycle is **three long whatever the source is**, even though only COFDM
/// has a receiver to feed the third.  A key whose cycle length depends on the
/// source is worse than an honest empty state, and it lets a capture script
/// select the mode regardless of what is running.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pane3Mode {
    Waterfall,
    Spectrogram,
    Constellation,
}

impl Pane3Mode {
    pub(crate) fn next(self) -> Self {
        match self {
            Self::Waterfall => Self::Spectrogram,
            Self::Spectrogram => Self::Constellation,
            Self::Constellation => Self::Waterfall,
        }
    }
}

// ── Source mode ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceMode {
    TestTone,
    Cw,
    AmDsb,
    Psk31,
    Ft8,
    Cofdm,
}

impl SourceMode {
    pub const ALL: &'static [SourceMode] = &[
        SourceMode::TestTone,
        SourceMode::Cw,
        SourceMode::AmDsb,
        SourceMode::Psk31,
        SourceMode::Ft8,
        SourceMode::Cofdm,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SourceMode::TestTone => "Test Tone",
            SourceMode::Cw => "CW",
            SourceMode::AmDsb => "AM DSB",
            SourceMode::Psk31 => "PSK31",
            SourceMode::Ft8 => "FT8",
            SourceMode::Cofdm => "COFDM",
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|&m| m == self).unwrap_or(0)
    }

    pub fn next(self) -> Self {
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

/// The keys a script's `set` directive may name for a source.
///
/// Through the factory table for the same reason everything else here is: a new
/// source brings its own keys with it, and no list needs editing.
pub(super) fn source_set_keys(mode: SourceMode) -> &'static [super::settings::SetKey] {
    source_mode_factory(mode).set_keys()
}
