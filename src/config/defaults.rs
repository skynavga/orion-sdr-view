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
}
