// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use orion_sdr::waveform::dvb_t::DvbTLinkParams;

use crate::app::settings::{DvbTSettings, SettingsState};
use crate::decode::DecodeMode;
use crate::source::SignalSource;
use crate::source::dvbt::{self, DVBT_PREFERRED_REF_DB, DvbTSource};
use crate::source::ft8::Ft8ViewState;

/// Build a fresh `DvbTSource` from current settings.
///
/// **No sample-rate argument**, unlike COFDM's `make`.  For DVB-T the rate is
/// the bandwidth (`fs = BW · 2048/1705`) with the 2K structure fixed above it,
/// so the bandwidth toggle carries it and there is nothing separate to pass.
pub(in crate::app) fn make(settings: &SettingsState) -> DvbTSource {
    DvbTSource::new(
        settings.dvbt_sig_secs(),
        settings.dvbt_gap_secs(),
        settings.dvbt_cn_db(),
        settings.dvbt_bandwidth(),
        settings.dvbt_link(),
        settings.dvbt_shaping(),
        settings.dvbt_center_hz(),
    )
}

/// Push current DVB-T settings into the active source if applicable.
pub(in crate::app) fn sync(source: &mut dyn SignalSource, settings: &SettingsState) {
    if let Some(dvbt) = source.as_any_mut().downcast_mut::<DvbTSource>() {
        dvbt.apply_params(
            settings.dvbt_sig_secs(),
            settings.dvbt_gap_secs(),
            settings.dvbt_cn_db(),
            settings.dvbt_bandwidth(),
            settings.dvbt_link(),
            settings.dvbt_shaping(),
            settings.dvbt_center_hz(),
        );
    }
}

/// Occupied bandwidth (Hz) for the current settings — reported in the Di bar.
///
/// A constant of the bandwidth mode, not something the shaping can move: DVB-T's
/// extreme active carriers are mandatory continual pilots, so there is no
/// edge-guard lever and the occupancy is fixed at 1705/2048 of the waveform's
/// rate.  That is why this takes settings alone where COFDM's takes the
/// effective shaping too.
pub(in crate::app) fn occupied_bw_hz(settings: &SettingsState) -> f32 {
    settings.dvbt_bandwidth().occupied_hz()
}

// **No `effective_shaping` counterpart to COFDM's**, deliberately.  There, the
// shaping *is* the carrier plan, so the receiver has to be built from the same
// resolved set as the modulator or it never acquires.  Here the shaping is
// transmit-only: the occupied band is fixed at 1705/2048, the receiver's window
// back-off is a constant zero, and the scattered-pilot equalizer absorbs the
// mask as it would any other channel.  So the decode config carries the link
// parameters and nothing about shaping.

/// The frame geometry and display scale the receiver must be told, read off the
/// **live source** rather than re-derived from settings.
///
/// Both are render-time quantities.  `frame_payload_len` is deterministic in the
/// link parameters and could be recomputed — but a stream demodulator trims each
/// frame's payload to the length it is given, so a value that disagreed with the
/// rendered buffer would truncate silently instead of failing; taking it from the
/// source makes that disagreement unrepresentable.  `full_scale` is the measured
/// peak of the rendered buffer and cannot be derived at all.
///
/// Returns `None` when the active source is not DVB-T, which is the caller's
/// signal to leave the decode config alone.
pub(in crate::app) fn frame_facts(source: &mut dyn SignalSource) -> Option<(usize, f32)> {
    let dvbt = source.as_any_mut().downcast_mut::<DvbTSource>()?;
    Some((dvbt.frame_payload_len(), dvbt.full_scale()))
}

/// Submode line for the top HUD when DVB-T is the active source.
pub(in crate::app) fn hud_submode_str(settings: &SettingsState) -> String {
    dvbt::hud_submode_str(
        settings.dvbt_bandwidth(),
        settings.dvbt_link(),
        &settings.dvbt_shaping(),
    )
}

pub(super) struct Factory;
impl super::SourceFactory for Factory {
    fn make(&self, settings: &SettingsState) -> Box<dyn SignalSource> {
        Box::new(make(settings))
    }
    fn decode_mode(&self, _: &SettingsState, _: &Ft8ViewState) -> DecodeMode {
        DecodeMode::DvbT
    }
    fn decode_carrier_hz(&self, settings: &SettingsState) -> f32 {
        // The band centre, in display-rate terms: the Di "ctr" readout and the
        // frequency the wideband C/N estimator centres its occupied window on.
        settings.dvbt_center_hz()
    }
    fn set_carrier_hz(&self, settings: &mut SettingsState, hz: f32) {
        settings.set_dvbt_center_hz(hz);
    }

    fn set_keys(&self) -> &'static [crate::app::settings::SetKey] {
        crate::app::settings::DVBT_SET_KEYS
    }

    fn cn_db(&self, settings: &SettingsState) -> f32 {
        settings.dvbt_cn_db()
    }

    // ── Wideband viewport preferences ───────────────────────────────────────
    fn nominal_center_hz(&self, settings: &SettingsState) -> Option<f32> {
        Some(settings.dvbt_center_hz())
    }
    fn preferred_span_hz(&self, settings: &SettingsState) -> Option<f32> {
        // Full Nyquist of the *display* rate, so the occupied band fills 83% of
        // the window — which is as much of it as DVB-T can be made to fill,
        // since the band width is not a lever.  Clamped to Nyquist by
        // `FreqView::reframe`.
        Some(settings.dvbt_bandwidth().display_fs() / 2.0)
    }
    fn preferred_ref_db(&self, _settings: &SettingsState) -> Option<f32> {
        Some(DVBT_PREFERRED_REF_DB)
    }
}

/// The link parameters as one value, for callers that want the triple rather
/// than three accessor calls.
pub(in crate::app) fn link(settings: &SettingsState) -> DvbTLinkParams {
    settings.dvbt_link()
}
