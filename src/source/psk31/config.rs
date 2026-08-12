// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::config::Defaults;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Psk31Config {
    pub mode: Option<String>,
    pub carrier_hz: Option<f32>,
    pub gap_secs: Option<f32>,
    pub cn_db: Option<f32>,
    /// **Retired.**  Present only so a config written before the C/N change
    /// fails loudly instead of being silently ignored: every field here is
    /// `Option<T>` and nothing sets `deny_unknown_fields`, so serde would
    /// otherwise drop this key and quietly fall back to the `cn_db` default —
    /// a config that looks like it loaded while discarding what the user wrote.
    /// See `ViewConfig::retired_key_errors`.
    pub noise_amp: Option<f32>,
    pub canned_text: Option<String>,
    pub custom_text: Option<String>,
    pub msg_repeat: Option<u32>,
}

impl crate::config::ViewConfig {
    pub fn psk31_mode(&self) -> &str {
        self.sources
            .as_ref()
            .and_then(|s| s.psk31.as_ref())
            .and_then(|p| p.mode.as_deref())
            .unwrap_or("BPSK31")
    }
    pub fn psk31_carrier_hz(&self) -> f32 {
        self.sources
            .as_ref()
            .and_then(|s| s.psk31.as_ref())
            .and_then(|p| p.carrier_hz)
            .unwrap_or(Defaults::CARRIER_HZ)
    }
    pub fn psk31_gap_secs(&self) -> f32 {
        self.sources
            .as_ref()
            .and_then(|s| s.psk31.as_ref())
            .and_then(|p| p.gap_secs)
            .unwrap_or(crate::source::psk31::PSK31_DEFAULT_GAP_SECS)
    }
    pub fn psk31_cn_db(&self) -> f32 {
        self.sources
            .as_ref()
            .and_then(|s| s.psk31.as_ref())
            .and_then(|p| p.cn_db)
            .unwrap_or(crate::source::psk31::PSK31_DEFAULT_CN_DB)
    }
    pub fn psk31_canned_text(&self) -> &str {
        self.sources
            .as_ref()
            .and_then(|s| s.psk31.as_ref())
            .and_then(|p| p.canned_text.as_deref())
            .unwrap_or(crate::source::psk31::PSK31_DEFAULT_CANNED_TEXT)
    }
    pub fn psk31_custom_text(&self) -> &str {
        self.sources
            .as_ref()
            .and_then(|s| s.psk31.as_ref())
            .and_then(|p| p.custom_text.as_deref())
            .unwrap_or(crate::source::psk31::PSK31_DEFAULT_CUSTOM_TEXT)
    }
    pub fn psk31_msg_repeat(&self) -> usize {
        self.sources
            .as_ref()
            .and_then(|s| s.psk31.as_ref())
            .and_then(|p| p.msg_repeat)
            .map(|v| (v as usize).max(1))
            .unwrap_or(crate::source::psk31::PSK31_DEFAULT_REPEAT)
    }
}
