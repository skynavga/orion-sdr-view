// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

pub struct Defaults;
impl Defaults {
    pub const DB_MIN: f32 = -80.0;
    pub const DB_MAX: f32 = -20.0;
    pub const FREQ_HZ: f32 = 12_000.0;
    pub const NOISE_AMP: f32 = 0.05;
    pub const AMP_MAX: f32 = 0.65;
    pub const RAMP_SECS: f32 = 3.0;
    pub const PAUSE_SECS: f32 = 7.0;
    pub const CARRIER_HZ: f32 = 12_000.0;
    pub const MOD_INDEX: f32 = 1.0;
    pub const AM_GAP_SECS: f32 = 7.0;
    pub const AM_NOISE_AMP: f32 = 0.05;
    /// Default ± frequency window (Hz) for the horizontal spectrogram pane.
    pub const SPEC_FREQ_DELTA_HZ: f32 = 2_000.0;
    /// Default time range (seconds) spanned by the full width of the
    /// horizontal spectrogram pane.
    pub const SPEC_TIME_RANGE_SECS: f32 = 10.0;
}
