// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::config::Defaults;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct TestToneConfig {
    pub freq_hz: Option<f32>,
    pub cn_db: Option<f32>,
    /// **Retired.**  Present only so a config written before the C/N change
    /// fails loudly instead of being silently ignored: every field here is
    /// `Option<T>` and nothing sets `deny_unknown_fields`, so serde would
    /// otherwise drop this key and quietly fall back to the `cn_db` default —
    /// a config that looks like it loaded while discarding what the user wrote.
    /// See `ViewConfig::retired_key_errors`.
    pub noise_amp: Option<f32>,
    pub amp_max: Option<f32>,
    pub ramp_secs: Option<f32>,
    pub pause_secs: Option<f32>,
}

impl crate::config::ViewConfig {
    pub fn freq_hz(&self) -> f32 {
        self.sources
            .as_ref()
            .and_then(|s| s.test_tone.as_ref())
            .and_then(|t| t.freq_hz)
            .unwrap_or(Defaults::FREQ_HZ)
    }
    pub fn cn_db(&self) -> f32 {
        self.sources
            .as_ref()
            .and_then(|s| s.test_tone.as_ref())
            .and_then(|t| t.cn_db)
            .unwrap_or(crate::source::tone::TONE_DEFAULT_CN_DB)
    }
    pub fn amp_max(&self) -> f32 {
        self.sources
            .as_ref()
            .and_then(|s| s.test_tone.as_ref())
            .and_then(|t| t.amp_max)
            .unwrap_or(Defaults::AMP_MAX)
    }
    pub fn ramp_secs(&self) -> f32 {
        self.sources
            .as_ref()
            .and_then(|s| s.test_tone.as_ref())
            .and_then(|t| t.ramp_secs)
            .unwrap_or(Defaults::RAMP_SECS)
    }
    pub fn pause_secs(&self) -> f32 {
        self.sources
            .as_ref()
            .and_then(|s| s.test_tone.as_ref())
            .and_then(|t| t.pause_secs)
            .unwrap_or(Defaults::PAUSE_SECS)
    }
}
