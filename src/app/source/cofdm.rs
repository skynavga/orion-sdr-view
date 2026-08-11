// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::app::settings::{CofdmSettings, SettingsState};
use crate::decode::DecodeMode;
use crate::source::SignalSource;
use crate::source::cofdm::{
    self, COFDM_FS, COFDM_NOMINAL_CENTER, COFDM_PREFERRED_REF_DB, CofdmShaping, CofdmSource,
    cofdm_occupied_bw,
};
use crate::source::ft8::Ft8ViewState;

/// Build a fresh `CofdmSource` from current settings.  The sample rate is a
/// fixed source property (`COFDM_FS`), not a settings value.
pub(in crate::app) fn make(settings: &SettingsState) -> CofdmSource {
    CofdmSource::new(
        settings.cofdm_sig_secs(),
        settings.cofdm_gap_secs(),
        settings.cofdm_noise_amp(),
        settings.cofdm_bw_fraction(),
        settings.cofdm_shaping(),
        COFDM_FS,
    )
}

/// Push current COFDM settings into the active source if applicable.
pub(in crate::app) fn sync(source: &mut dyn SignalSource, settings: &SettingsState) {
    if let Some(cofdm) = source.as_any_mut().downcast_mut::<CofdmSource>() {
        cofdm.apply_params(
            settings.cofdm_sig_secs(),
            settings.cofdm_gap_secs(),
            settings.cofdm_noise_amp(),
            settings.cofdm_bw_fraction(),
            settings.cofdm_shaping(),
        );
    }
}

/// Occupied bandwidth (Hz) for the current settings — reported in the Di bar.
/// Keyed off the *effective* edge guard, which the shaping rows can override
/// away from what the bandwidth fraction alone implies.
pub(in crate::app) fn occupied_bw_hz(settings: &SettingsState) -> f32 {
    let fraction = settings.cofdm_bw_fraction();
    let guard = settings.cofdm_shaping().effective(fraction).edge_guard;
    cofdm_occupied_bw(COFDM_FS, guard)
}

/// The *effective* transmit shaping for the current settings.
///
/// The decoder builds both its carrier-plan facts and its receiver from this,
/// through the same `cofdm_link_config` the modulator uses — so the two ends
/// share one definition of the numerology rather than two that must be kept in
/// agreement.
pub(in crate::app) fn effective_shaping(settings: &SettingsState) -> CofdmShaping {
    settings
        .cofdm_shaping()
        .effective(settings.cofdm_bw_fraction())
}

/// Submode line for the top HUD when COFDM is the active source.
pub(in crate::app) fn hud_submode_str(settings: &SettingsState) -> String {
    cofdm::hud_submode_str(settings.cofdm_bw_fraction(), &settings.cofdm_shaping())
}

pub(super) struct Factory;
impl super::SourceFactory for Factory {
    fn make(&self, settings: &SettingsState) -> Box<dyn SignalSource> {
        Box::new(make(settings))
    }
    fn decode_mode(&self, _: &SettingsState, _: &Ft8ViewState) -> DecodeMode {
        DecodeMode::Cofdm
    }
    fn decode_carrier_hz(&self, _settings: &SettingsState) -> f32 {
        // The band center — used for the Di "ctr" readout only.
        COFDM_NOMINAL_CENTER
    }
    fn set_carrier_hz(&self, _settings: &mut SettingsState, _hz: f32) {
        // No-op: COFDM occupies a fixed wideband sub-band, not a single
        // tunable carrier, so the source-lock (L key) does not retune it.
    }

    // ── Wideband viewport preferences ───────────────────────────────────────
    fn nominal_center_hz(&self, _settings: &SettingsState) -> Option<f32> {
        Some(COFDM_NOMINAL_CENTER)
    }
    fn preferred_span_hz(&self, _settings: &SettingsState) -> Option<f32> {
        // Full Nyquist: the bandwidth fraction (a settings toggle) then controls
        // how much of this fixed span the occupied band fills.  Clamped to
        // Nyquist by `FreqView::reframe`.  The horizontal spectrogram follows
        // this same viewport span, so it needs no separate preference.
        Some(COFDM_FS / 2.0)
    }
    fn preferred_ref_db(&self, _settings: &SettingsState) -> Option<f32> {
        // Match the ~-15 dB signal peaks produced by the modulator gain.
        Some(COFDM_PREFERRED_REF_DB)
    }
}
