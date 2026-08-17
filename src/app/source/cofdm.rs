// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::app::settings::{CofdmSettings, SettingsState};
use crate::decode::DecodeMode;
use crate::source::SignalSource;
use crate::source::cofdm::{
    self, COFDM_PREFERRED_REF_DB, CofdmShaping, CofdmSource, cofdm_occupied_bw,
};
use crate::source::ft8::Ft8ViewState;
use orion_sdr::modulate::ConstellationOrder;

/// Build a fresh `CofdmSource` from current settings.
///
/// The sample rate comes from settings like everything else now.  It is not a
/// row (see `CofdmConfig::fs_hz`), but it *is* per-source state, and this is the
/// only construction path — so a configured rate reaches the source, and
/// `ViewApp::apply_source_sample_rate` then re-derives Nyquist from what the
/// constructed source reports.
pub(in crate::app) fn make(settings: &SettingsState) -> CofdmSource {
    CofdmSource::new(
        settings.cofdm_sig_secs(),
        settings.cofdm_gap_secs(),
        settings.cofdm_cn_db(),
        settings.cofdm_bw_fraction(),
        settings.cofdm_shaping(),
        settings.cofdm_center_hz(),
        settings.cofdm_fs_hz(),
    )
}

/// Push current COFDM settings into the active source if applicable.
pub(in crate::app) fn sync(source: &mut dyn SignalSource, settings: &SettingsState) {
    if let Some(cofdm) = source.as_any_mut().downcast_mut::<CofdmSource>() {
        cofdm.apply_params(
            settings.cofdm_sig_secs(),
            settings.cofdm_gap_secs(),
            settings.cofdm_cn_db(),
            settings.cofdm_bw_fraction(),
            settings.cofdm_shaping(),
            settings.cofdm_center_hz(),
        );
    }
}

/// Short label for a constellation order, for the decoder pane's heading.
///
/// The order is the one the **receiver recovered** from the frame header, not
/// the one the transmit config was built with, so this is a display of what
/// arrived rather than of what was configured — the same provenance rule the
/// `X` panel's `mod` / `CR` / `GI` fields follow.
pub(in crate::app) fn constellation_label(order: ConstellationOrder) -> &'static str {
    match order {
        ConstellationOrder::Bpsk => "BPSK",
        ConstellationOrder::Qpsk => "QPSK",
        ConstellationOrder::Qam16 => "QAM16",
        ConstellationOrder::Qam64 => "QAM64",
        ConstellationOrder::Qam256 => "QAM256",
    }
}

/// Occupied bandwidth (Hz) for the current settings — reported in the Di bar.
/// Keyed off the *effective* edge guard, which the shaping rows can override
/// away from what the bandwidth fraction alone implies, and which the band
/// centre bounds.
pub(in crate::app) fn occupied_bw_hz(settings: &SettingsState) -> f32 {
    cofdm_occupied_bw(
        settings.cofdm_fs_hz(),
        effective_shaping(settings).edge_guard,
    )
}

/// The *effective* transmit shaping for the current settings.
///
/// The decoder builds both its carrier-plan facts and its receiver from this,
/// through the same `cofdm_link_config` the modulator uses — so the two ends
/// share one definition of the numerology rather than two that must be kept in
/// agreement.
pub(in crate::app) fn effective_shaping(settings: &SettingsState) -> CofdmShaping {
    settings.cofdm_shaping().effective(
        settings.cofdm_bw_fraction(),
        settings.cofdm_center_hz(),
        settings.cofdm_fs_hz(),
    )
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
    fn decode_carrier_hz(&self, settings: &SettingsState) -> f32 {
        // The band center: the Di "ctr" readout, and the frequency the wideband
        // C/N estimator centres its occupied window on.  One value, so a retune
        // cannot leave the measurement looking at the old band.
        settings.cofdm_center_hz()
    }
    fn set_carrier_hz(&self, settings: &mut SettingsState, hz: f32) {
        // The source-lock (L key), which used to be a documented no-op here.
        settings.set_cofdm_center_hz(hz);
    }

    fn cn_db(&self, settings: &SettingsState) -> f32 {
        settings.cofdm_cn_db()
    }

    // ── Wideband viewport preferences ───────────────────────────────────────
    fn nominal_center_hz(&self, settings: &SettingsState) -> Option<f32> {
        Some(settings.cofdm_center_hz())
    }
    fn preferred_span_hz(&self, settings: &SettingsState) -> Option<f32> {
        // Full Nyquist: the bandwidth fraction (a settings toggle) then controls
        // how much of this fixed span the occupied band fills.  Clamped to
        // Nyquist by `FreqView::reframe`.  The horizontal spectrogram follows
        // this same viewport span, so it needs no separate preference.
        //
        // This is also the rule that makes the `Zoom` row a startup default
        // rather than a persistent override: a switch *to* a source that states
        // a preference reframes to it.
        Some(settings.cofdm_fs_hz() / 2.0)
    }
    fn preferred_ref_db(&self, _settings: &SettingsState) -> Option<f32> {
        // Match the ~-15 dB signal peaks produced by the modulator gain.
        Some(COFDM_PREFERRED_REF_DB)
    }
}
