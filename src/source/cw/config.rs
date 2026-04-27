// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct CwConfig {
    pub wpm: Option<f32>,
    pub jitter_pct: Option<f32>,
    pub dash_weight: Option<f32>,
    pub char_space: Option<f32>,
    pub word_space: Option<f32>,
    pub rise_ms: Option<f32>,
    pub fall_ms: Option<f32>,
    pub carrier_hz: Option<f32>,
    pub gap_secs: Option<f32>,
    pub noise_amp: Option<f32>,
    pub canned_text: Option<String>,
    pub custom_text: Option<String>,
    pub msg_repeat: Option<u32>,
}

impl crate::config::ViewConfig {
    fn cw_cfg(&self) -> Option<&CwConfig> {
        self.sources.as_ref().and_then(|s| s.cw.as_ref())
    }
    pub fn cw_wpm(&self) -> f32 {
        self.cw_cfg()
            .and_then(|c| c.wpm)
            .unwrap_or(crate::source::cw::CW_DEFAULT_WPM)
    }
    pub fn cw_jitter_pct(&self) -> f32 {
        self.cw_cfg()
            .and_then(|c| c.jitter_pct)
            .unwrap_or(crate::source::cw::CW_DEFAULT_JITTER_PCT)
    }
    pub fn cw_dash_weight(&self) -> f32 {
        self.cw_cfg()
            .and_then(|c| c.dash_weight)
            .unwrap_or(crate::source::cw::CW_DEFAULT_DASH_WEIGHT)
    }
    pub fn cw_char_space(&self) -> f32 {
        self.cw_cfg()
            .and_then(|c| c.char_space)
            .unwrap_or(crate::source::cw::CW_DEFAULT_CHAR_SPACE)
    }
    pub fn cw_word_space(&self) -> f32 {
        self.cw_cfg()
            .and_then(|c| c.word_space)
            .unwrap_or(crate::source::cw::CW_DEFAULT_WORD_SPACE)
    }
    pub fn cw_rise_ms(&self) -> f32 {
        self.cw_cfg()
            .and_then(|c| c.rise_ms)
            .unwrap_or(crate::source::cw::CW_DEFAULT_RISE_MS)
    }
    pub fn cw_fall_ms(&self) -> f32 {
        self.cw_cfg()
            .and_then(|c| c.fall_ms)
            .unwrap_or(crate::source::cw::CW_DEFAULT_FALL_MS)
    }
    pub fn cw_carrier_hz(&self) -> f32 {
        self.cw_cfg()
            .and_then(|c| c.carrier_hz)
            .unwrap_or(crate::source::cw::CW_DEFAULT_CARRIER_HZ)
    }
    pub fn cw_gap_secs(&self) -> f32 {
        self.cw_cfg()
            .and_then(|c| c.gap_secs)
            .unwrap_or(crate::source::cw::CW_DEFAULT_GAP_SECS)
    }
    pub fn cw_noise_amp(&self) -> f32 {
        self.cw_cfg()
            .and_then(|c| c.noise_amp)
            .unwrap_or(crate::source::cw::CW_DEFAULT_NOISE_AMP)
    }
    pub fn cw_canned_text(&self) -> &str {
        self.cw_cfg()
            .and_then(|c| c.canned_text.as_deref())
            .unwrap_or(crate::source::cw::CW_DEFAULT_CANNED_TEXT)
    }
    pub fn cw_custom_text(&self) -> &str {
        self.cw_cfg()
            .and_then(|c| c.custom_text.as_deref())
            .unwrap_or(crate::source::cw::CW_DEFAULT_CUSTOM_TEXT)
    }
    pub fn cw_msg_repeat(&self) -> usize {
        self.cw_cfg()
            .and_then(|c| c.msg_repeat)
            .map(|v| (v as usize).max(1))
            .unwrap_or(crate::source::cw::CW_DEFAULT_REPEAT)
    }
}
