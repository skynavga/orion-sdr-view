// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use super::Defaults;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct DisplayConfig {
    pub db_min: Option<f32>,
    pub db_max: Option<f32>,
    pub time_zone: Option<String>,
    /// Half-width (± Hz) of the frequency window shown by the horizontal
    /// spectrogram pane, centered on the primary marker frequency.
    pub spec_freq_delta_hz: Option<f32>,
    /// Time range (seconds) covered by the full width of the horizontal
    /// spectrogram pane.
    pub spec_time_range_secs: Option<f32>,
}

/// Parsed time-zone mode for the display settings row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TzMode {
    /// UTC (offset = 0).
    Utc,
    /// System local time — offset resolved from the OS at query time.
    Local,
    /// Explicit signed offset in minutes.
    Explicit(i32),
}

impl super::ViewConfig {
    pub fn db_min(&self) -> f32 {
        self.display
            .as_ref()
            .and_then(|d| d.db_min)
            .unwrap_or(Defaults::DB_MIN)
    }
    pub fn db_max(&self) -> f32 {
        self.display
            .as_ref()
            .and_then(|d| d.db_max)
            .unwrap_or(Defaults::DB_MAX)
    }
    pub fn spec_freq_delta_hz(&self) -> f32 {
        self.display
            .as_ref()
            .and_then(|d| d.spec_freq_delta_hz)
            .unwrap_or(Defaults::SPEC_FREQ_DELTA_HZ)
    }
    pub fn spec_time_range_secs(&self) -> f32 {
        self.display
            .as_ref()
            .and_then(|d| d.spec_time_range_secs)
            .unwrap_or(Defaults::SPEC_TIME_RANGE_SECS)
    }

    /// Returns the parsed `time_zone` mode from the YAML config.
    ///
    /// Accepted `time_zone:` values:
    /// - missing or `"utc"` → `TzMode::Utc`
    /// - `"local"` → `TzMode::Local`
    /// - `"+HH:MM"` / `"-HH:MM"` → `TzMode::Explicit(minutes)`
    ///
    /// Anything else is treated as UTC after a soft warning on stderr.
    pub fn time_zone_mode(&self) -> TzMode {
        let raw = match self.display.as_ref().and_then(|d| d.time_zone.as_deref()) {
            Some(s) => s,
            None => return TzMode::Utc,
        };
        parse_time_zone_mode(raw).unwrap_or_else(|| {
            eprintln!("config: unrecognized time_zone {:?}, using UTC", raw,);
            TzMode::Utc
        })
    }

    /// Returns the effective UTC offset in minutes for the configured mode,
    /// resolving `local` against the current system offset.  Clamped to the
    /// display range `[-12*60, 14*60]` minutes.
    pub fn time_zone_offset_min(&self) -> i32 {
        match self.time_zone_mode() {
            TzMode::Utc => 0,
            TzMode::Local => crate::utils::time::local_utc_offset_min(),
            TzMode::Explicit(min) => min,
        }
    }
}

/// Parse a `time_zone` config value into a [`TzMode`].
///
/// Returns `None` for unrecognized input so the caller can warn and fall
/// back to a default.
pub(super) fn parse_time_zone_mode(raw: &str) -> Option<TzMode> {
    let s = raw.trim();
    if s.eq_ignore_ascii_case("utc") || s.is_empty() {
        return Some(TzMode::Utc);
    }
    if s.eq_ignore_ascii_case("local") {
        return Some(TzMode::Local);
    }
    parse_offset_hhmm(s).map(TzMode::Explicit)
}

/// Parse a `+HH:MM` / `-HH:MM` offset string to minutes, or `None` on error.
fn parse_offset_hhmm(s: &str) -> Option<i32> {
    let (sign, rest) = match s.as_bytes().first()? {
        b'+' => (1, &s[1..]),
        b'-' => (-1, &s[1..]),
        _ => return None,
    };
    let (h_str, m_str) = rest.split_once(':')?;
    let h: i32 = h_str.parse().ok()?;
    let m: i32 = m_str.parse().ok()?;
    if !(0..=14).contains(&h) || !(0..60).contains(&m) {
        return None;
    }
    let total = sign * (h * 60 + m);
    if !(-12 * 60..=14 * 60).contains(&total) {
        return None;
    }
    Some(total)
}

/// Format an offset-in-minutes as a user-facing string:
/// - `0` → `"utc"`
/// - positive → `"+HH:MM"`
/// - negative → `"-HH:MM"`
pub fn format_offset_min(min: i32) -> String {
    if min == 0 {
        return "utc".to_owned();
    }
    let sign = if min > 0 { '+' } else { '-' };
    let abs = min.unsigned_abs() as i32;
    let h = abs / 60;
    let m = abs % 60;
    format!("{sign}{h:02}:{m:02}")
}
