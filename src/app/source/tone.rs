// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::app::SAMPLE_RATE;
use crate::app::settings::{SettingsState, ToneSettings};
use crate::decode::DecodeMode;
use crate::source::SignalSource;
use crate::source::ft8::Ft8ViewState;
use crate::source::tone::{TestSignalGen, TestToneSource};

/// Build a fresh `TestToneSource` from current settings.
pub(in crate::app) fn make(settings: &SettingsState) -> TestToneSource {
    TestToneSource::new(TestSignalGen::new(settings.freq_hz(), SAMPLE_RATE))
}

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

pub(super) struct Factory;
impl super::SourceFactory for Factory {
    fn make(&self, settings: &SettingsState) -> Box<dyn SignalSource> {
        Box::new(make(settings))
    }
    fn decode_mode(&self, _: &SettingsState, _: &Ft8ViewState) -> DecodeMode {
        DecodeMode::TestTone
    }
    fn decode_carrier_hz(&self, settings: &SettingsState) -> f32 {
        settings.freq_hz()
    }
    fn set_carrier_hz(&self, settings: &mut SettingsState, hz: f32) {
        settings.set_freq_hz(hz);
    }
}
