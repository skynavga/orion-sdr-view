// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::app::SAMPLE_RATE;
use crate::app::settings::SettingsState;
use crate::source::SignalSource;
use crate::source::cw::{CwSource, CwSyncFlags};

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
