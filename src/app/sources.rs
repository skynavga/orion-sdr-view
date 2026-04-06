use crate::decode::{DecodeMode};
use crate::source::tone::TestToneSource;
use crate::source::amdsb::{AmDsbSource, BuiltinAudio, load_builtin};
use crate::source::psk31::{Psk31Mode, Psk31Source};

use super::{SAMPLE_RATE, SourceMode};
use super::view::ViewApp;

// ── Source construction and sync ──────────────────────────────────────────────

impl ViewApp {
    /// Build a fresh AmDsbSource from current settings values.
    pub(super) fn make_am_source(&self) -> AmDsbSource {
        let (audio, audio_rate) = if self.settings.am_audio_is_custom() {
            // Custom with no path yet — start silent; audio loaded on WAV entry
            (Vec::new(), SAMPLE_RATE)
        } else {
            let builtin = BuiltinAudio::ALL[self.settings.am_audio_idx().min(BuiltinAudio::ALL.len() - 1)];
            load_builtin(builtin)
        };
        AmDsbSource::new(
            audio,
            audio_rate,
            self.settings.am_carrier_hz(),
            self.settings.am_mod_index(),
            self.settings.am_loop_gap_secs(),
            self.settings.am_noise_amp(),
            self.settings.am_msg_repeat(),
            SAMPLE_RATE,
        )
    }

    /// Build a fresh Psk31Source from current settings values.
    pub(super) fn make_psk31_source(&self) -> Psk31Source {
        let mode = match self.settings.psk31_mode_str() {
            "QPSK31" => Psk31Mode::Qpsk31,
            _        => Psk31Mode::Bpsk31,
        };
        Psk31Source::new(
            self.settings.psk31_carrier_hz(),
            self.settings.psk31_loop_gap_secs(),
            self.settings.psk31_noise_amp(),
            mode,
            self.settings.psk31_message().to_owned(),
            self.settings.psk31_msg_repeat(),
            SAMPLE_RATE,
        )
    }

    /// Reload the built-in audio buffer into the active AmDsbSource after the
    /// AM audio toggle changes (Morse ↔ Voice). No-op if source is not AM DSB
    /// or if Custom is selected (user WAV takes precedence).
    pub(super) fn reload_builtin_audio(&mut self) {
        if self.source_mode != SourceMode::AmDsb {
            return;
        }
        if self.settings.am_audio_is_custom() {
            return;
        }
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
    pub(super) fn try_load_wav(&mut self) {
        let path_str = self.settings.wav_path().to_owned();
        if path_str.is_empty() {
            self.settings.set_wav_status(false);
            return;
        }
        match crate::source::amdsb::load_wav_file(std::path::Path::new(&path_str)) {
            Ok((audio, rate)) => {
                if self.source_mode == SourceMode::AmDsb {
                    if let Some(am) = self.source.as_any_mut().downcast_mut::<AmDsbSource>() {
                        am.set_audio(audio, rate);
                    }
                }
                self.settings.set_wav_status(true);
                self.reset_playback();
            }
            Err(_) => {
                self.settings.set_wav_status(false);
            }
        }
    }

    /// Push current settings values into live signal/display state.
    pub(super) fn sync_settings(&mut self) {
        self.db_min = self.settings.db_min();
        self.db_max = self.settings.db_max();
        self.waterfall.db_min = self.settings.db_min();
        self.waterfall.db_max = self.settings.db_max();
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
            let gap_changed = (am.loop_gap_secs - self.settings.am_loop_gap_secs()).abs() > 0.01;
            if gap_changed {
                am.loop_gap_secs = self.settings.am_loop_gap_secs();
                am.update_loop_gap();
            }
            am.noise_amp = self.settings.am_noise_amp();
            am.msg_repeat = self.settings.am_msg_repeat().max(1);
        }

        if let Some(psk31) = self.source.as_any_mut().downcast_mut::<Psk31Source>() {
            let new_mode = match self.settings.psk31_mode_str() {
                "QPSK31" => Psk31Mode::Qpsk31,
                _        => Psk31Mode::Bpsk31,
            };
            let new_repeat      = self.settings.psk31_msg_repeat();
            let carrier_changed = (psk31.carrier_hz - self.settings.psk31_carrier_hz()).abs() > 0.01;
            let mode_changed    = psk31.mode != new_mode;
            let repeat_changed  = psk31.msg_repeat != new_repeat;
            psk31.carrier_hz    = self.settings.psk31_carrier_hz();
            psk31.noise_amp     = self.settings.psk31_noise_amp();
            psk31.loop_gap_secs = self.settings.psk31_loop_gap_secs();
            psk31.mode          = new_mode;
            psk31.msg_repeat    = new_repeat.max(1);
            // message is NOT synced here — it is applied only when the user
            // explicitly accepts the text edit via Enter (see apply_psk31_message).
            if carrier_changed || mode_changed || repeat_changed { psk31.render(); }
            psk31.update_loop_gap();
        }
        self.sync_decode_config();
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

    /// Update the shared DecodeConfig to match the current source mode and carrier.
    pub(super) fn sync_decode_config(&mut self) {
        let mode = match self.source_mode {
            SourceMode::Psk31 => match self.settings.psk31_mode_str() {
                "QPSK31" => DecodeMode::Qpsk31,
                _        => DecodeMode::Bpsk31,
            },
            SourceMode::AmDsb    => DecodeMode::AmDsb,
            SourceMode::TestTone => DecodeMode::TestTone,
        };
        let carrier_hz = match self.source_mode {
            SourceMode::Psk31    => self.settings.psk31_carrier_hz(),
            SourceMode::AmDsb    => self.settings.am_carrier_hz(),
            SourceMode::TestTone => self.settings.freq_hz(),
        };
        if let Ok(mut cfg) = self.decode_config.lock() {
            cfg.mode       = mode;
            cfg.carrier_hz = carrier_hz;
        }
    }
}
