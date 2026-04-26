// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::app::SAMPLE_RATE;
use crate::app::settings::SettingsState;
use crate::source::SignalSource;
use crate::source::ft8::{Ft8Mode, Ft8MsgType, Ft8Source};

/// Build a fresh `Ft8Source` from current settings.
pub(in crate::app) fn make(settings: &SettingsState) -> Ft8Source {
    Ft8Source::new(
        settings.ft8_carrier_hz(),
        settings.ft8_gap_secs(),
        settings.ft8_noise_amp(),
        settings_mode(settings),
        settings_msg_type(settings),
        settings.ft8_call_to().to_owned(),
        settings.ft8_call_de().to_owned(),
        settings.ft8_grid().to_owned(),
        settings.ft8_free_text().to_owned(),
        settings.ft8_msg_repeat(),
        SAMPLE_RATE,
    )
}

/// Push current FT8 settings into the active source if applicable.  Returns
/// the live `(mode, msg_type)` so the caller can update its cached values.
pub(in crate::app) fn sync(
    source: &mut dyn SignalSource,
    settings: &SettingsState,
) -> Option<(Ft8Mode, Ft8MsgType)> {
    let ft8 = source.as_any_mut().downcast_mut::<Ft8Source>()?;
    ft8.apply_params(
        settings.ft8_carrier_hz(),
        settings.ft8_gap_secs(),
        settings.ft8_noise_amp(),
        settings_mode(settings),
        settings_msg_type(settings),
        settings.ft8_msg_repeat(),
    );
    Some((ft8.ft8_mode, ft8.msg_type))
}

/// Apply the committed FT8 free-text message to the live source and re-render.
pub(in crate::app) fn apply_free_text(source: &mut dyn SignalSource, settings: &SettingsState) {
    if let Some(ft8) = source.as_any_mut().downcast_mut::<Ft8Source>() {
        ft8.free_text = settings.ft8_free_text().to_owned();
        ft8.render();
    }
}

/// Cycle the active FT8 source between FT8 ↔ FT4.  Returns the new mode (or
/// `None` if the source isn't FT8).
pub(in crate::app) fn cycle_mode(source: &mut dyn SignalSource) -> Option<Ft8Mode> {
    let ft8 = source.as_any_mut().downcast_mut::<Ft8Source>()?;
    ft8.ft8_mode = ft8.ft8_mode.cycle();
    ft8.render();
    Some(ft8.ft8_mode)
}

/// Cycle the active FT8 source's message type Standard ↔ FreeText.
pub(in crate::app) fn cycle_msg_type(source: &mut dyn SignalSource) -> Option<Ft8MsgType> {
    let ft8 = source.as_any_mut().downcast_mut::<Ft8Source>()?;
    ft8.msg_type = ft8.msg_type.cycle();
    ft8.render();
    Some(ft8.msg_type)
}

fn settings_mode(settings: &SettingsState) -> Ft8Mode {
    match settings.ft8_mode_str() {
        "FT4" => Ft8Mode::Ft4,
        _ => Ft8Mode::Ft8,
    }
}

fn settings_msg_type(settings: &SettingsState) -> Ft8MsgType {
    if settings.ft8_msg_is_free_text() {
        Ft8MsgType::FreeText
    } else {
        Ft8MsgType::Standard
    }
}
