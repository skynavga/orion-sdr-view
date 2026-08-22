// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Cross-source orchestration on `ViewApp`: dispatches per-frame sync, source
//! construction, message commits, and FT8 mode cycling to the per-source app
//! modules under `app::source::*`.

use crate::source::SignalSource;
use crate::source::amdsb::AmDsbSource;

use super::SourceMode;
use super::common::source_mode_factory;
use super::settings::{AmDsbSettings, CwSettings, Ft8Settings, ToneSettings};
use super::source::{amdsb, cofdm, cw, dvbt, ft8, psk31, tone};
use super::view::ViewApp;

impl ViewApp {
    /// Build a fresh source for the active `source_mode` from current settings.
    pub(super) fn make_source(&self) -> Box<dyn SignalSource> {
        source_mode_factory(self.source_mode).make(&self.settings)
    }

    /// Push current settings values into live signal/display state.
    pub(super) fn sync_settings(&mut self) {
        // The source's preferred spectrum scale, *before* the rows are read
        // below.  Guarded on the preference having moved, so `[` / `]` and the
        // `dB min` / `dB max` rows keep working — what changes it is a source
        // switch or a settings row the preference is derived from, which for
        // DVB-T is `Bandwidth`.  That row can move the reference without moving
        // the sample rate, so this cannot ride on the rate guard below.
        if self.source_scale() != self.applied_scale {
            self.apply_source_scale();
        }
        self.db_min = self.settings.db_min();
        self.db_max = self.settings.db_max();
        self.waterfall.db_min = self.settings.db_min();
        self.waterfall.db_max = self.settings.db_max();
        self.time_zone_offset_min = self.settings.time_zone_offset_min();
        self.signal_gen.freq_hz = self.settings.freq_hz();
        self.signal_gen.set_cn_db(self.settings.cn_db());
        self.signal_gen.amp_max = self.settings.amp_max();
        self.signal_gen.ramp_secs = self.settings.ramp_secs();
        self.signal_gen.pause_secs = self.settings.pause_secs();

        // Per-source sync — each module no-ops when its source isn't active.
        tone::sync(self.source.as_mut(), &self.settings);
        amdsb::sync(self.source.as_mut(), &self.settings);
        psk31::sync(self.source.as_mut(), &self.settings);
        cofdm::sync(self.source.as_mut(), &self.settings);
        dvbt::sync(self.source.as_mut(), &self.settings);
        if let Some(flags) = cw::sync(self.source.as_mut(), &self.settings)
            && flags.wpm_or_word_space_changed
        {
            self.loop_timer.set_holdoff(self.loop_timer_holdoff_secs());
        }
        if let Some((mode, msg_type)) = ft8::sync(self.source.as_mut(), &self.settings) {
            self.ft8_view.mode = mode;
            self.ft8_view.msg_type = msg_type;
        }

        // A source whose settings can move its *rate* needs the display
        // re-derived, not just the waveform re-rendered.  DVB-T's `Bandwidth`
        // toggle is the first that can: it spans 24x, and without this the
        // frequency axis, the `Zoom` bound and every bin-indexed pane would keep
        // drawing against the outgoing Nyquist.
        //
        // Guarded on the rate having actually moved, because
        // `apply_source_sample_rate` clears the waterfall, persistence,
        // spectrogram and the two decoder rasters — history at the old scaling
        // cannot be redrawn at the new one, but neither should a `C/N` nudge
        // wipe it.  Uniform rather than per-source: it asks what the constructed
        // source reports, which is the same rule the method itself follows.
        if self.source.sample_rate() != self.applied_fs {
            self.apply_source_sample_rate();
            self.reframe_for_source();
        }

        self.sync_decode_config();
    }

    /// Apply a script's `set` directive, and flow it through the same paths a
    /// popover edit flows through.
    ///
    /// `as_default` is the difference between the two spellings: an untimed
    /// `set` is a *configuration*, so it moves the row's default too and the
    /// source is rebuilt from it exactly as `--config` would have built it; a
    /// timed one is an *interaction*, so it moves the value and re-syncs, which
    /// is what a nudge does.  Rebuilding on a mid-run edit would be wrong rather
    /// than merely heavy — it resets the burst timer and flushes the decode
    /// pipeline, so a C/N sweep would restart the waveform at every step and
    /// measure a sequence of first bursts instead of one degrading link.
    ///
    /// Returns the value the row settled on when it clamped, for the caller to
    /// report.
    pub fn apply_set(
        &mut self,
        target: crate::app::settings::SetTarget,
        value: &str,
        as_default: bool,
    ) -> Result<Option<f32>, String> {
        let outcome = self.settings.apply_set(target, value, as_default)?;
        if outcome.is_text && outcome.is_active_source {
            // A live source holds a *rendered* waveform of the old message, so
            // the row alone changes nothing audible.  This is the path Enter in
            // the popover takes, dispatched rather than re-derived.
            source_mode_factory(self.source_mode)
                .apply_message(self.source.as_mut(), &self.settings);
            self.restart_source();
        } else if as_default {
            self.restart_source();
        } else {
            self.sync_settings();
        }
        // The `Zoom` row is a live control the viewport owns rather than reads,
        // so it is pushed exactly where the popover's key handler pushes it.
        self.freq_view.set_zoom_ratio(self.settings.zoom_ratio());
        Ok(outcome.clamped_to)
    }

    /// Reload audio after the AM audio toggle changes (Morse / Voice / Custom).
    /// No-op if source is not AM DSB.
    pub(super) fn reload_builtin_audio(&mut self) {
        if self.source_mode != SourceMode::AmDsb {
            return;
        }
        match amdsb::reload_audio(&mut self.settings) {
            Some((audio, rate)) => {
                amdsb::set_audio(
                    self.source.as_mut(),
                    audio,
                    rate,
                    self.settings.am_msg_repeat(),
                );
            }
            None => amdsb::clear_audio(self.source.as_mut()),
        }
        self.restart_source();
    }

    /// Attempt to load the WAV path from settings into the AM DSB source.
    /// Returns true on success.
    pub(super) fn try_load_wav(&mut self) -> bool {
        let Some(load) = amdsb::try_load_wav(&mut self.settings) else {
            return false;
        };
        let success = matches!(load, amdsb::WavLoad::Loaded { .. });
        match load {
            amdsb::WavLoad::Loaded { audio, rate } => {
                if self.source_mode == SourceMode::AmDsb
                    && let Some(am) = self.source.as_any_mut().downcast_mut::<AmDsbSource>()
                {
                    am.set_audio(audio, rate);
                }
            }
            amdsb::WavLoad::Cleared => {
                if self.source_mode == SourceMode::AmDsb {
                    amdsb::clear_audio(self.source.as_mut());
                }
            }
        }
        self.restart_source();
        success
    }

    /// Cycle the FT8 source between FT8 and FT4 modes (M key).  Cycles the
    /// settings toggle row; restart_source() then flows the change through
    /// sync_settings → ft8::sync → Ft8Source::apply_params.
    pub(super) fn cycle_ft8_mode(&mut self) {
        self.settings.cycle_ft8_mode();
        self.restart_source();
    }

    /// Cycle the FT8 source message type (N key): Standard → FreeText → Standard.
    /// Same pattern as cycle_ft8_mode — cycle the settings row, let
    /// restart_source flow the change through.
    pub(super) fn cycle_ft8_msg_type(&mut self) {
        self.settings.cycle_ft8_msg_type();
        self.restart_source();
    }

    /// Apply the committed PSK31 message to the live source and re-render.
    pub(super) fn apply_psk31_message(&mut self) {
        psk31::apply_message(self.source.as_mut(), &self.settings);
        self.restart_source();
    }

    /// Apply the committed CW message to the live source and re-render.
    pub(super) fn apply_cw_message(&mut self) {
        cw::apply_message(self.source.as_mut(), &self.settings);
        self.restart_source();
    }

    /// Apply the committed FT8 free-text message to the live source and re-render.
    pub(super) fn apply_ft8_free_text(&mut self) {
        ft8::apply_free_text(self.source.as_mut(), &self.settings);
        self.restart_source();
    }

    /// Update the shared `DecodeConfig` to match the current source mode and
    /// carrier.  Source-specific dispatch goes through the per-source
    /// `SourceFactory` impl; the only source-aware branch here is the CW
    /// extra-fields block.
    pub(super) fn sync_decode_config(&mut self) {
        let factory = source_mode_factory(self.source_mode);
        let mode = factory.decode_mode(&self.settings, &self.ft8_view);
        let carrier_hz = factory.decode_carrier_hz(&self.settings);
        // Read off the live source before the lock, since it needs `&mut self`:
        // the DVB-T receiver's frame trim length and the source's full-scale
        // reference are both render-time quantities, so settings cannot supply
        // them.  `None` whenever the active source is not DVB-T.
        let dvbt_frame_facts = dvbt::frame_facts(self.source.as_mut());
        if let Ok(mut cfg) = self.decode_config.lock() {
            cfg.mode = mode;
            cfg.carrier_hz = carrier_hz;
            // Both sides must agree on where a burst ends.
            cfg.signal_threshold = self.signal_threshold();
            if self.source_mode == SourceMode::Cw {
                cfg.cw_message = self.settings.cw_message().to_owned();
                cfg.cw_wpm = self.settings.cw_wpm();
                cfg.cw_dash_weight = self.settings.cw_dash_weight();
                cfg.cw_char_space = self.settings.cw_char_space();
                cfg.cw_word_space = self.settings.cw_word_space();
                cfg.cw_msg_repeat = self.settings.cw_msg_repeat();
            }
            if self.source_mode == SourceMode::Cofdm {
                cfg.cofdm_bw_hz = cofdm::occupied_bw_hz(&self.settings);
                cfg.cofdm_shaping = cofdm::effective_shaping(&self.settings);
                // **The only decode-config field driven by display state.**
                // Everything else here comes from settings; this one asks
                // "is anyone looking?", because the probe's cost is only
                // worth paying while pane 3 is drawing it.  Reading `self`
                // rather than `self.settings` is what that costs, and the
                // block already reads `self.source_mode`.
                cfg.cofdm_probe = self.pane3_wants_probe();
            }
            if self.source_mode == SourceMode::DvbT {
                cfg.dvbt_bw_hz = dvbt::occupied_bw_hz(&self.settings);
                cfg.dvbt_link = dvbt::link(&self.settings);
                if let Some((payload_len, full_scale)) = dvbt_frame_facts {
                    cfg.dvbt_frame_payload_len = payload_len;
                    cfg.dvbt_full_scale = full_scale;
                }
                cfg.dvbt_probe = self.pane3_wants_probe();
            }
        }
    }
}
