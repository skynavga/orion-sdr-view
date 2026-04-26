// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::app::settings::SettingsState;
use crate::source::SignalSource;
use crate::source::tone::TestToneSource;

/// Push current tone settings into the active source if it is a `TestToneSource`.
pub(in crate::app) fn sync(source: &mut dyn SignalSource, settings: &SettingsState) {
    if let Some(tts) = source.as_any_mut().downcast_mut::<TestToneSource>() {
        tts.signal_gen.apply_params(
            settings.freq_hz(),
            settings.noise_amp(),
            settings.amp_max(),
            settings.ramp_secs(),
            settings.pause_secs(),
        );
    }
}
