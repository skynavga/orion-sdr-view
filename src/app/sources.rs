// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Cross-source orchestration on `ViewApp`: dispatches per-frame sync, source
//! construction, message commits, and FT8 mode cycling to the per-source app
//! modules under `app::source::*`.

use crate::source::SignalSource;
use crate::source::amdsb::AmDsbSource;

use super::SourceMode;
use super::common::source_mode_factory;
use super::settings::{AmDsbSettings, CwSettings, ToneSettings};
use super::source::{amdsb, cw, ft8, psk31, tone};
use super::view::ViewApp;

impl ViewApp {
    /// Build a fresh source for the active `source_mode` from current settings.
    pub(super) fn make_source(&self) -> Box<dyn SignalSource> {
        source_mode_factory(self.source_mode).make(&self.settings)
    }

    /// Push current settings values into live signal/display state.
    pub(super) fn sync_settings(&mut self) {
        self.db_min = self.settings.db_min();
        self.db_max = self.settings.db_max();
        self.waterfall.db_min = self.settings.db_min();
        self.waterfall.db_max = self.settings.db_max();
        self.time_zone_offset_min = self.settings.time_zone_offset_min();
        self.signal_gen.freq_hz = self.settings.freq_hz();
        self.signal_gen.noise_amp = self.settings.noise_amp();
        self.signal_gen.amp_max = self.settings.amp_max();
        self.signal_gen.ramp_secs = self.settings.ramp_secs();
        self.signal_gen.pause_secs = self.settings.pause_secs();

        // Per-source sync — each module no-ops when its source isn't active.
        tone::sync(self.source.as_mut(), &self.settings);
        amdsb::sync(self.source.as_mut(), &self.settings);
        psk31::sync(self.source.as_mut(), &self.settings);
        if let Some(flags) = cw::sync(self.source.as_mut(), &self.settings)
            && flags.wpm_or_word_space_changed
        {
            self.loop_timer.set_holdoff(self.loop_timer_holdoff_secs());
        }
        if let Some((mode, msg_type)) = ft8::sync(self.source.as_mut(), &self.settings) {
            self.ft8_view.mode = mode;
            self.ft8_view.msg_type = msg_type;
        }

        self.sync_decode_config();
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
        self.reset_playback();
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
        self.reset_playback();
        success
    }

    /// Cycle the FT8 source between FT8 and FT4 modes (M key).
    pub(super) fn cycle_ft8_mode(&mut self) {
        if let Some(mode) = ft8::cycle_mode(self.source.as_mut()) {
            self.ft8_view.mode = mode;
        }
        self.sync_decode_config();
        self.reset_playback();
    }

    /// Cycle the FT8 source message type (N key): Standard → FreeText → Standard.
    pub(super) fn cycle_ft8_msg_type(&mut self) {
        if let Some(msg_type) = ft8::cycle_msg_type(self.source.as_mut()) {
            self.ft8_view.msg_type = msg_type;
        }
        self.reset_playback();
    }

    /// Apply the committed PSK31 message to the live source and re-render.
    pub(super) fn apply_psk31_message(&mut self) {
        psk31::apply_message(self.source.as_mut(), &self.settings);
        self.reset_playback();
    }

    /// Apply the committed CW message to the live source and re-render.
    pub(super) fn apply_cw_message(&mut self) {
        cw::apply_message(self.source.as_mut(), &self.settings);
        self.reset_playback();
    }

    /// Apply the committed FT8 free-text message to the live source and re-render.
    pub(super) fn apply_ft8_free_text(&mut self) {
        ft8::apply_free_text(self.source.as_mut(), &self.settings);
        self.reset_playback();
    }

    /// Update the shared `DecodeConfig` to match the current source mode and
    /// carrier.  Source-specific dispatch goes through the per-source
    /// `SourceFactory` impl; the only source-aware branch here is the CW
    /// extra-fields block.
    pub(super) fn sync_decode_config(&mut self) {
        let factory = source_mode_factory(self.source_mode);
        let mode = factory.decode_mode(&self.settings, &self.ft8_view);
        let carrier_hz = factory.decode_carrier_hz(&self.settings);
        if let Ok(mut cfg) = self.decode_config.lock() {
            cfg.mode = mode;
            cfg.carrier_hz = carrier_hz;
            if self.source_mode == SourceMode::Cw {
                cfg.cw_message = self.settings.cw_message().to_owned();
                cfg.cw_wpm = self.settings.cw_wpm();
                cfg.cw_dash_weight = self.settings.cw_dash_weight();
                cfg.cw_char_space = self.settings.cw_char_space();
                cfg.cw_word_space = self.settings.cw_word_space();
                cfg.cw_msg_repeat = self.settings.cw_msg_repeat();
            }
        }
    }
}
