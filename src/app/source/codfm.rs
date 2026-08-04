// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::app::settings::{CodfmSettings, SettingsState};
use crate::decode::DecodeMode;
use crate::source::SignalSource;
use crate::source::codfm::{self, CODFM_FS, CODFM_NOMINAL_CENTER, CodfmSource, codfm_occupied_bw};
use crate::source::ft8::Ft8ViewState;

/// Build a fresh `CodfmSource` from current settings.  The sample rate is a
/// fixed source property (`CODFM_FS`), not a settings value.
pub(in crate::app) fn make(settings: &SettingsState) -> CodfmSource {
    CodfmSource::new(
        settings.codfm_gap_secs(),
        settings.codfm_noise_amp(),
        CODFM_FS,
    )
}

/// Push current CODFM settings into the active source if applicable.
pub(in crate::app) fn sync(source: &mut dyn SignalSource, settings: &SettingsState) {
    if let Some(codfm) = source.as_any_mut().downcast_mut::<CodfmSource>() {
        codfm.apply_params(settings.codfm_gap_secs(), settings.codfm_noise_amp());
    }
}

/// Submode line for the top HUD when CODFM is the active source (empty).
pub(in crate::app) fn hud_submode_str(_settings: &SettingsState) -> String {
    codfm::hud_submode_str()
}

pub(super) struct Factory;
impl super::SourceFactory for Factory {
    fn make(&self, settings: &SettingsState) -> Box<dyn SignalSource> {
        Box::new(make(settings))
    }
    fn decode_mode(&self, _: &SettingsState, _: &Ft8ViewState) -> DecodeMode {
        DecodeMode::Codfm
    }
    fn decode_carrier_hz(&self, _settings: &SettingsState) -> f32 {
        // The band center — used for the Di "ctr" readout only.
        CODFM_NOMINAL_CENTER
    }
    fn set_carrier_hz(&self, _settings: &mut SettingsState, _hz: f32) {
        // No-op: CODFM occupies a fixed wideband sub-band, not a single
        // tunable carrier, so the source-lock (L key) does not retune it.
    }

    // ── Wideband viewport preferences ───────────────────────────────────────
    fn nominal_center_hz(&self, _settings: &SettingsState) -> Option<f32> {
        Some(CODFM_NOMINAL_CENTER)
    }
    fn preferred_span_hz(&self, _settings: &SettingsState) -> Option<f32> {
        Some(codfm_occupied_bw(CODFM_FS) * 1.2)
    }
    fn preferred_spec_delta_hz(&self, _settings: &SettingsState) -> Option<f32> {
        Some(codfm_occupied_bw(CODFM_FS) * 0.6)
    }
}
