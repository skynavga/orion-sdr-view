// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::app::SAMPLE_RATE;
use crate::app::settings::{CwSettings, SettingsState};
use crate::source::SignalSource;
use crate::source::cw::{self, CwSource, CwSyncFlags};

/// Build a fresh `CwSource` from current settings.
pub(in crate::app) fn make(settings: &SettingsState) -> CwSource {
    CwSource::new(
        settings.cw_carrier_hz(),
        settings.cw_gap_secs(),
        settings.cw_noise_amp(),
        settings.cw_wpm(),
        settings.cw_jitter_pct(),
        settings.cw_dash_weight(),
        settings.cw_char_space(),
        settings.cw_word_space(),
        settings.cw_rise_ms(),
        settings.cw_fall_ms(),
        settings.cw_message().to_owned(),
        settings.cw_msg_repeat(),
        SAMPLE_RATE,
    )
}

/// Push current CW settings into the active source if applicable.  Returns
/// `None` if the source is not a `CwSource`; otherwise returns the per-frame
/// sync flags so the caller can refresh CW-specific state (loop-timer holdoff).
pub(in crate::app) fn sync(
    source: &mut dyn SignalSource,
    settings: &SettingsState,
) -> Option<CwSyncFlags> {
    let cw = source.as_any_mut().downcast_mut::<CwSource>()?;
    Some(cw.apply_params(
        settings.cw_carrier_hz(),
        settings.cw_gap_secs(),
        settings.cw_noise_amp(),
        settings.cw_wpm(),
        settings.cw_jitter_pct(),
        settings.cw_dash_weight(),
        settings.cw_char_space(),
        settings.cw_word_space(),
        settings.cw_rise_ms(),
        settings.cw_fall_ms(),
        settings.cw_msg_repeat(),
    ))
}

/// Apply the committed CW message to the live source and re-render.
pub(in crate::app) fn apply_message(source: &mut dyn SignalSource, settings: &SettingsState) {
    if let Some(cw) = source.as_any_mut().downcast_mut::<CwSource>() {
        cw.message = settings.cw_message().to_owned();
        cw.render();
    }
}

/// Loop-timer holdoff for the active CW settings (zero for non-CW callers).
pub(in crate::app) fn holdoff_secs(settings: &SettingsState) -> f32 {
    cw::holdoff_secs(settings.cw_wpm(), settings.cw_word_space())
}

/// Format the opening ticker delimiter injected on a CW signal-onset edge:
/// `"|| HH:MM:SS.mmm | "`.  `onset` is the captured rising-edge time;
/// `time_zone_offset_min` is the configured display offset.
pub(in crate::app) fn format_open_delimiter(
    onset: std::time::SystemTime,
    time_zone_offset_min: i32,
) -> String {
    let ts = crate::utils::format::format_time(onset, time_zone_offset_min);
    let ts_str = if ts.is_empty() {
        "--:--:--.---".to_owned()
    } else {
        ts
    };
    format!("|| {ts_str} | ")
}

/// Submode line for the top HUD when CW is the active source.
pub(in crate::app) fn hud_submode_str(settings: &SettingsState) -> String {
    let msg_is_custom = settings.cw_msg_mode_str() == "Custom";
    cw::hud_submode_str(msg_is_custom, settings.cw_wpm())
}
