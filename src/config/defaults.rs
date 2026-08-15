// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

pub struct Defaults;
impl Defaults {
    pub const DB_MIN: f32 = -80.0;
    /// Spectrum scale top (dBFS) — the display reference level.
    ///
    /// Every source but COFDM uses this; COFDM overrides it with
    /// `COFDM_PREFERRED_REF_DB`, because its burst is normalised to a
    /// deliberately low RMS to leave OFDM crest factor inside full scale.
    pub const DB_MAX: f32 = -15.0;
    pub const FREQ_HZ: f32 = 12_000.0;
    pub const AMP_MAX: f32 = 0.65;
    pub const RAMP_SECS: f32 = 3.0;
    pub const PAUSE_SECS: f32 = 7.0;
    pub const CARRIER_HZ: f32 = 12_000.0;
    pub const MOD_INDEX: f32 = 1.0;
    pub const AM_GAP_SECS: f32 = 7.0;
    /// Default time range (seconds) spanned by the full width of the
    /// horizontal spectrogram pane.
    pub const SPEC_TIME_RANGE_SECS: f32 = 10.0;
    /// Startup viewport zoom ratio (1.0 = full span, 0..Nyquist).
    ///
    /// Expressed as a ratio rather than a span in Hz so one value is portable
    /// across sources: "open at 4x" means the same thing at 48 kHz and at
    /// 1.92 MHz, where a span in Hz would need re-clamping per source and would
    /// mean something different on each.
    pub const ZOOM: f32 = 1.0;

    /// Where captures are written, relative to the working directory.
    ///
    /// Beside the project rather than in `$HOME`: captures are usually taken
    /// *of* something being worked on, so they belong next to it and are easy
    /// to add to a `.gitignore`.  `~/` is still expanded if configured that
    /// way.  The directory is created on first use rather than at startup, so a
    /// session that never captures leaves no trace.
    pub const CAPTURE_DIR: &'static str = "./capture";
    /// Overlays appear in a capture by default: a still of the settings or
    /// instrument panel is exactly what documentation wants, and they are in
    /// the render target already.
    pub const CAPTURE_OVERLAYS: bool = true;
    /// Video frame rate.  30 rather than the display's 60 because the readback
    /// is ~16 MB a frame at 2x scale, and 60 fps of that is ~950 MB/s.
    pub const CAPTURE_FPS: u32 = 30;
}
