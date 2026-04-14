// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::decode::DecodeMode;
use crate::source::amdsb::{AmDsbSource, BuiltinAudio, load_builtin};
use crate::source::cw::CwSource;
use crate::source::ft8::{Ft8Mode, Ft8MsgType, Ft8Source};
use crate::source::psk31::{Psk31Mode, Psk31Source};
use crate::source::tone::TestToneSource;

use super::view::ViewApp;
use super::{SAMPLE_RATE, SourceMode};

// ── Source construction and sync ──────────────────────────────────────────────

impl ViewApp {
    /// Build a fresh AmDsbSource from current settings values.
    pub(super) fn make_am_source(&self) -> AmDsbSource {
        let (audio, audio_rate) = if self.settings.am_audio_is_custom() {
            // Custom with no path yet — start silent; audio loaded on WAV entry
            (Vec::new(), SAMPLE_RATE)
        } else {
            let builtin = BuiltinAudio::ALL[self
                .settings
                .am_audio_idx()
                .min(BuiltinAudio::ALL.len() - 1)];
            load_builtin(builtin)
        };
        AmDsbSource::new(
            audio,
            audio_rate,
            self.settings.am_carrier_hz(),
            self.settings.am_mod_index(),
            self.settings.am_gap_secs(),
            self.settings.am_noise_amp(),
            self.settings.am_msg_repeat(),
            SAMPLE_RATE,
        )
    }

    /// Build a fresh CwSource from current settings values.
    pub(super) fn make_cw_source(&self) -> CwSource {
        CwSource::new(
            self.settings.cw_carrier_hz(),
            self.settings.cw_gap_secs(),
            self.settings.cw_noise_amp(),
            self.settings.cw_wpm(),
            self.settings.cw_jitter_pct(),
            self.settings.cw_dash_weight(),
            self.settings.cw_char_space(),
            self.settings.cw_word_space(),
            self.settings.cw_rise_ms(),
            self.settings.cw_fall_ms(),
            self.settings.cw_message().to_owned(),
            self.settings.cw_msg_repeat(),
            SAMPLE_RATE,
        )
    }

    /// Build a fresh Psk31Source from current settings values.
    pub(super) fn make_psk31_source(&self) -> Psk31Source {
        let mode = match self.settings.psk31_mode_str() {
            "QPSK31" => Psk31Mode::Qpsk31,
            _ => Psk31Mode::Bpsk31,
        };
        Psk31Source::new(
            self.settings.psk31_carrier_hz(),
            self.settings.psk31_gap_secs(),
            self.settings.psk31_noise_amp(),
            mode,
            self.settings.psk31_message().to_owned(),
            self.settings.psk31_msg_repeat(),
            SAMPLE_RATE,
        )
    }

    /// Build a fresh Ft8Source from current settings values.
    pub(super) fn make_ft8_source(&self) -> Ft8Source {
        let mode = match self.settings.ft8_mode_str() {
            "FT4" => Ft8Mode::Ft4,
            _ => Ft8Mode::Ft8,
        };
        let msg_type = if self.settings.ft8_msg_is_free_text() {
            Ft8MsgType::FreeText
        } else {
            Ft8MsgType::Standard
        };
        Ft8Source::new(
            self.settings.ft8_carrier_hz(),
            self.settings.ft8_gap_secs(),
            self.settings.ft8_noise_amp(),
            mode,
            msg_type,
            self.settings.ft8_call_to().to_owned(),
            self.settings.ft8_call_de().to_owned(),
            self.settings.ft8_grid().to_owned(),
            self.settings.ft8_free_text().to_owned(),
            self.settings.ft8_msg_repeat(),
            SAMPLE_RATE,
        )
    }

    /// Reload audio after the AM audio toggle changes (Morse / Voice / Custom).
    /// No-op if source is not AM DSB.
    pub(super) fn reload_builtin_audio(&mut self) {
        if self.source_mode != SourceMode::AmDsb {
            return;
        }
        if self.settings.am_audio_is_custom() {
            // Switched TO Custom: try to reload a previously valid path,
            // otherwise go carrier-only (no audio).
            let path_str = self.settings.wav_path().to_owned();
            if !path_str.is_empty() {
                if let Ok((audio, rate)) =
                    crate::source::amdsb::load_wav_file(std::path::Path::new(&path_str))
                {
                    if let Some(am) = self.source.as_any_mut().downcast_mut::<AmDsbSource>() {
                        am.set_audio(audio, rate);
                    }
                    self.settings.set_wav_status(true);
                    self.reset_playback();
                    return;
                }
                // Path exists but file no longer valid — mark failed, go silent.
                self.settings.set_wav_status(false);
            }
            self.clear_am_audio();
            return;
        }
        // Morse or Voice: load built-in audio.
        let audio_idx = self.settings.am_audio_idx();
        self.settings.reset_am_repeat_for_audio(audio_idx);
        let builtin = BuiltinAudio::ALL[audio_idx.min(BuiltinAudio::ALL.len() - 1)];
        let (audio, rate) = load_builtin(builtin);
        if let Some(am) = self.source.as_any_mut().downcast_mut::<AmDsbSource>() {
            am.set_audio(audio, rate);
            am.msg_repeat = self.settings.am_msg_repeat();
        }
        self.reset_playback();
    }

    /// Attempt to load the WAV path from settings into the AM DSB source.
    /// On failure, clear audio to carrier-only and mark the path as failed.
    /// Returns true on success.
    pub(super) fn try_load_wav(&mut self) -> bool {
        let path_str = self.settings.wav_path().to_owned();
        if path_str.is_empty() {
            self.settings.set_wav_status(false);
            self.clear_am_audio();
            return false;
        }
        match crate::source::amdsb::load_wav_file(std::path::Path::new(&path_str)) {
            Ok((audio, rate)) => {
                if self.source_mode == SourceMode::AmDsb
                    && let Some(am) = self.source.as_any_mut().downcast_mut::<AmDsbSource>()
                {
                    am.set_audio(audio, rate);
                }
                self.settings.set_wav_status(true);
                self.reset_playback();
                true
            }
            Err(e) => {
                eprintln!("orion-sdr-view: failed to load {:?}: {}", path_str, e);
                self.settings.set_wav_status(false);
                self.clear_am_audio();
                false
            }
        }
    }

    /// Clear the AM DSB audio buffer to produce carrier-only output.
    fn clear_am_audio(&mut self) {
        if self.source_mode == SourceMode::AmDsb
            && let Some(am) = self.source.as_any_mut().downcast_mut::<AmDsbSource>()
        {
            am.set_audio(Vec::new(), SAMPLE_RATE);
        }
        self.reset_playback();
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

        // Propagate test-tone settings into the active source if applicable
        if let Some(tts) = self.source.as_any_mut().downcast_mut::<TestToneSource>() {
            tts.signal_gen.freq_hz = self.settings.freq_hz();
            tts.signal_gen.noise_amp = self.settings.noise_amp();
            tts.signal_gen.amp_max = self.settings.amp_max();
            tts.signal_gen.ramp_secs = self.settings.ramp_secs();
            tts.signal_gen.pause_secs = self.settings.pause_secs();
        }

        // Propagate AM DSB settings into the active source if applicable
        if let Some(am) = self.source.as_any_mut().downcast_mut::<AmDsbSource>() {
            let carrier_changed = (am.carrier_hz - self.settings.am_carrier_hz()).abs() > 0.5;
            let index_changed = (am.mod_index - self.settings.am_mod_index()).abs() > 0.001;
            am.carrier_hz = self.settings.am_carrier_hz();
            am.mod_index = self.settings.am_mod_index();
            if carrier_changed || index_changed {
                am.rebuild_mod();
            }
            let gap_changed = (am.gap_secs - self.settings.am_gap_secs()).abs() > 0.01;
            if gap_changed {
                am.gap_secs = self.settings.am_gap_secs();
                am.update_gap();
            }
            am.noise_amp = self.settings.am_noise_amp();
            am.msg_repeat = self.settings.am_msg_repeat().max(1);
        }

        if let Some(psk31) = self.source.as_any_mut().downcast_mut::<Psk31Source>() {
            let new_mode = match self.settings.psk31_mode_str() {
                "QPSK31" => Psk31Mode::Qpsk31,
                _ => Psk31Mode::Bpsk31,
            };
            let new_repeat = self.settings.psk31_msg_repeat();
            let carrier_changed =
                (psk31.carrier_hz - self.settings.psk31_carrier_hz()).abs() > 0.01;
            let mode_changed = psk31.mode != new_mode;
            let repeat_changed = psk31.msg_repeat != new_repeat;
            psk31.carrier_hz = self.settings.psk31_carrier_hz();
            psk31.noise_amp = self.settings.psk31_noise_amp();
            psk31.gap_secs = self.settings.psk31_gap_secs();
            psk31.mode = new_mode;
            psk31.msg_repeat = new_repeat.max(1);
            // message is NOT synced here — it is applied only when the user
            // explicitly accepts the text edit via Enter (see apply_psk31_message).
            if carrier_changed || mode_changed || repeat_changed {
                psk31.render();
            }
            psk31.update_gap();
        }

        if let Some(cw) = self.source.as_any_mut().downcast_mut::<CwSource>() {
            let carrier_changed = (cw.carrier_hz - self.settings.cw_carrier_hz()).abs() > 0.01;
            let wpm_changed = (cw.wpm - self.settings.cw_wpm()).abs() > 0.01;
            let jitter_changed = (cw.jitter_pct - self.settings.cw_jitter_pct()).abs() > 0.01;
            let weight_changed = (cw.dash_weight - self.settings.cw_dash_weight()).abs() > 0.01;
            let char_sp_changed = (cw.char_space - self.settings.cw_char_space()).abs() > 0.01;
            let word_sp_changed = (cw.word_space - self.settings.cw_word_space()).abs() > 0.01;
            let rise_changed = (cw.rise_ms - self.settings.cw_rise_ms()).abs() > 0.01;
            let fall_changed = (cw.fall_ms - self.settings.cw_fall_ms()).abs() > 0.01;
            let repeat_changed = cw.msg_repeat != self.settings.cw_msg_repeat();
            cw.carrier_hz = self.settings.cw_carrier_hz();
            cw.wpm = self.settings.cw_wpm();
            cw.jitter_pct = self.settings.cw_jitter_pct();
            cw.dash_weight = self.settings.cw_dash_weight();
            cw.char_space = self.settings.cw_char_space();
            cw.word_space = self.settings.cw_word_space();
            cw.rise_ms = self.settings.cw_rise_ms();
            cw.fall_ms = self.settings.cw_fall_ms();
            cw.noise_amp = self.settings.cw_noise_amp();
            cw.gap_secs = self.settings.cw_gap_secs();
            cw.msg_repeat = self.settings.cw_msg_repeat().max(1);
            // Message is NOT synced here — applied only on explicit text edit accept.
            if carrier_changed
                || wpm_changed
                || jitter_changed
                || weight_changed
                || char_sp_changed
                || word_sp_changed
                || rise_changed
                || fall_changed
                || repeat_changed
            {
                cw.render();
            }
            cw.update_gap();
        }

        if let Some(ft8) = self.source.as_any_mut().downcast_mut::<Ft8Source>() {
            let new_mode = match self.settings.ft8_mode_str() {
                "FT4" => Ft8Mode::Ft4,
                _ => Ft8Mode::Ft8,
            };
            let new_msg_type = if self.settings.ft8_msg_is_free_text() {
                Ft8MsgType::FreeText
            } else {
                Ft8MsgType::Standard
            };
            let new_repeat = self.settings.ft8_msg_repeat();
            let carrier_changed = (ft8.carrier_hz - self.settings.ft8_carrier_hz()).abs() > 0.01;
            let mode_changed = ft8.ft8_mode != new_mode;
            let msg_type_changed = ft8.msg_type != new_msg_type;
            let repeat_changed = ft8.msg_repeat != new_repeat;
            ft8.carrier_hz = self.settings.ft8_carrier_hz();
            ft8.noise_amp = self.settings.ft8_noise_amp();
            ft8.gap_secs = self.settings.ft8_gap_secs();
            ft8.ft8_mode = new_mode;
            ft8.msg_type = new_msg_type;
            ft8.msg_repeat = new_repeat.max(1);
            // free_text is NOT synced here — applied only on explicit text edit accept
            if carrier_changed || mode_changed || msg_type_changed || repeat_changed {
                ft8.render();
            }
            ft8.update_gap();
            self.ft_mode = ft8.ft8_mode;
            self.ft_msg_type = ft8.msg_type;
        }

        self.sync_decode_config();
    }

    /// Cycle the FT8 source between FT8 and FT4 modes (M key).
    pub(super) fn cycle_ft8_mode(&mut self) {
        if let Some(ft8) = self.source.as_any_mut().downcast_mut::<Ft8Source>() {
            ft8.ft8_mode = match ft8.ft8_mode {
                Ft8Mode::Ft8 => Ft8Mode::Ft4,
                Ft8Mode::Ft4 => Ft8Mode::Ft8,
            };
            self.ft_mode = ft8.ft8_mode;
            ft8.render();
        }
        self.sync_decode_config();
        self.reset_playback();
    }

    /// Cycle the FT8 source message type (N key): Standard → FreeText → Standard.
    pub(super) fn cycle_ft8_msg_type(&mut self) {
        if let Some(ft8) = self.source.as_any_mut().downcast_mut::<Ft8Source>() {
            ft8.msg_type = match ft8.msg_type {
                Ft8MsgType::Standard => Ft8MsgType::FreeText,
                Ft8MsgType::FreeText => Ft8MsgType::Standard,
            };
            self.ft_msg_type = ft8.msg_type;
            ft8.render();
        }
        self.reset_playback();
    }

    /// Apply the committed PSK31 message and repeat count to the live source and
    /// re-render.  Called only when the user explicitly accepts the message edit.
    pub(super) fn apply_psk31_message(&mut self) {
        if let Some(psk31) = self.source.as_any_mut().downcast_mut::<Psk31Source>() {
            psk31.message = self.settings.psk31_message().to_owned();
            psk31.render();
        }
        self.reset_playback();
    }

    /// Apply the committed CW message and repeat count to the live source and
    /// re-render.  Called only when the user explicitly accepts the message edit.
    pub(super) fn apply_cw_message(&mut self) {
        if let Some(cw) = self.source.as_any_mut().downcast_mut::<CwSource>() {
            cw.message = self.settings.cw_message().to_owned();
            cw.render();
        }
        self.reset_playback();
    }

    /// Apply the committed FT8 free-text message to the live source and re-render.
    /// Called only when the user explicitly accepts the text edit via Enter.
    pub(super) fn apply_ft8_free_text(&mut self) {
        if let Some(ft8) = self.source.as_any_mut().downcast_mut::<Ft8Source>() {
            ft8.free_text = self.settings.ft8_free_text().to_owned();
            ft8.render();
        }
        self.reset_playback();
    }

    /// Update the shared DecodeConfig to match the current source mode and carrier.
    pub(super) fn sync_decode_config(&mut self) {
        let mode = match self.source_mode {
            SourceMode::Psk31 => match self.settings.psk31_mode_str() {
                "QPSK31" => DecodeMode::Qpsk31,
                _ => DecodeMode::Bpsk31,
            },
            SourceMode::Cw => DecodeMode::Cw,
            SourceMode::AmDsb => DecodeMode::AmDsb,
            SourceMode::TestTone => DecodeMode::TestTone,
            SourceMode::Ft8 => match self.ft_mode {
                Ft8Mode::Ft8 => DecodeMode::Ft8,
                Ft8Mode::Ft4 => DecodeMode::Ft4,
            },
        };
        let carrier_hz = match self.source_mode {
            SourceMode::Psk31 => self.settings.psk31_carrier_hz(),
            SourceMode::Cw => self.settings.cw_carrier_hz(),
            SourceMode::AmDsb => self.settings.am_carrier_hz(),
            SourceMode::TestTone => self.settings.freq_hz(),
            SourceMode::Ft8 => self.settings.ft8_carrier_hz(),
        };
        if let Ok(mut cfg) = self.decode_config.lock() {
            cfg.mode = mode;
            cfg.carrier_hz = carrier_hz;
            if mode == DecodeMode::Cw {
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
