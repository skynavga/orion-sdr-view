// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::app::SAMPLE_RATE;
use crate::app::settings::{Psk31Settings, SettingsState};
use crate::source::SignalSource;
use crate::source::psk31::{self, Psk31Mode, Psk31Source};

/// Build a fresh `Psk31Source` from current settings.
pub(in crate::app) fn make(settings: &SettingsState) -> Psk31Source {
    Psk31Source::new(
        settings.psk31_carrier_hz(),
        settings.psk31_gap_secs(),
        settings.psk31_noise_amp(),
        settings_mode(settings),
        settings.psk31_message().to_owned(),
        settings.psk31_msg_repeat(),
        SAMPLE_RATE,
    )
}

/// Push current PSK31 settings into the active source if applicable.
pub(in crate::app) fn sync(source: &mut dyn SignalSource, settings: &SettingsState) {
    if let Some(psk31) = source.as_any_mut().downcast_mut::<Psk31Source>() {
        psk31.apply_params(
            settings.psk31_carrier_hz(),
            settings.psk31_gap_secs(),
            settings.psk31_noise_amp(),
            settings_mode(settings),
            settings.psk31_msg_repeat(),
        );
    }
}

/// Apply the committed PSK31 message to the live source and re-render.
pub(in crate::app) fn apply_message(source: &mut dyn SignalSource, settings: &SettingsState) {
    if let Some(psk31) = source.as_any_mut().downcast_mut::<Psk31Source>() {
        psk31.message = settings.psk31_message().to_owned();
        psk31.render();
    }
}

/// Submode line for the top HUD when PSK31 is the active source.
pub(in crate::app) fn hud_submode_str(settings: &SettingsState) -> String {
    let msg_is_custom = settings.psk31_msg_mode_str() == "Custom";
    psk31::hud_submode_str(settings_mode(settings), msg_is_custom)
}

fn settings_mode(settings: &SettingsState) -> Psk31Mode {
    match settings.psk31_mode_str() {
        "QPSK31" => Psk31Mode::Qpsk31,
        _ => Psk31Mode::Bpsk31,
    }
}
