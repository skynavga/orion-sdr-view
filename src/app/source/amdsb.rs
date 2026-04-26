// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::path::Path;

use crate::app::SAMPLE_RATE;
use crate::app::settings::SettingsState;
use crate::source::SignalSource;
use crate::source::amdsb::{self, AmDsbSource, BuiltinAudio, load_builtin, load_wav_file};

/// Build a fresh `AmDsbSource` from current settings.
pub(in crate::app) fn make(settings: &SettingsState) -> AmDsbSource {
    let (audio, audio_rate) = if settings.am_audio_is_custom() {
        // Custom with no path yet — start silent; audio loaded on WAV entry.
        (Vec::new(), SAMPLE_RATE)
    } else {
        let builtin = BuiltinAudio::ALL[settings.am_audio_idx().min(BuiltinAudio::ALL.len() - 1)];
        load_builtin(builtin)
    };
    AmDsbSource::new(
        audio,
        audio_rate,
        settings.am_carrier_hz(),
        settings.am_mod_index(),
        settings.am_gap_secs(),
        settings.am_noise_amp(),
        settings.am_msg_repeat(),
        SAMPLE_RATE,
    )
}

/// Push current AM DSB settings into the active source if applicable.
pub(in crate::app) fn sync(source: &mut dyn SignalSource, settings: &SettingsState) {
    if let Some(am) = source.as_any_mut().downcast_mut::<AmDsbSource>() {
        am.apply_params(
            settings.am_carrier_hz(),
            settings.am_mod_index(),
            settings.am_gap_secs(),
            settings.am_noise_amp(),
            settings.am_msg_repeat(),
        );
    }
}

/// Result of `try_load_wav`: indicates which side-effect to apply.
pub(in crate::app) enum WavLoad {
    Loaded { audio: Vec<f32>, rate: f32 },
    Cleared,
}

/// Try to load the WAV path from settings.  On success, returns the decoded
/// audio.  On failure, clears settings WAV status and asks the caller to
/// silence the source.
pub(in crate::app) fn try_load_wav(settings: &mut SettingsState) -> Option<WavLoad> {
    let path_str = settings.wav_path().to_owned();
    if path_str.is_empty() {
        settings.set_wav_status(false);
        return Some(WavLoad::Cleared);
    }
    match load_wav_file(Path::new(&path_str)) {
        Ok((audio, rate)) => {
            settings.set_wav_status(true);
            Some(WavLoad::Loaded { audio, rate })
        }
        Err(e) => {
            eprintln!("orion-sdr-view: failed to load {:?}: {}", path_str, e);
            settings.set_wav_status(false);
            Some(WavLoad::Cleared)
        }
    }
}

/// Compute the audio payload to push into an AmDsbSource after the AM-audio
/// toggle changes.  Returns `None` if the active toggle is "Custom" and the
/// path failed to load (caller should silence the source).
pub(in crate::app) fn reload_audio(settings: &mut SettingsState) -> Option<(Vec<f32>, f32)> {
    if settings.am_audio_is_custom() {
        let path_str = settings.wav_path().to_owned();
        if !path_str.is_empty() {
            if let Ok((audio, rate)) = load_wav_file(Path::new(&path_str)) {
                settings.set_wav_status(true);
                return Some((audio, rate));
            }
            settings.set_wav_status(false);
        }
        return None;
    }
    // Morse or Voice: load built-in audio.
    let audio_idx = settings.am_audio_idx();
    settings.reset_am_repeat_for_audio(audio_idx);
    let builtin = BuiltinAudio::ALL[audio_idx.min(BuiltinAudio::ALL.len() - 1)];
    Some(load_builtin(builtin))
}

/// Set audio on the active source if it is an `AmDsbSource`.  Also refreshes
/// `msg_repeat` (which depends on the audio toggle for built-ins).
pub(in crate::app) fn set_audio(
    source: &mut dyn SignalSource,
    audio: Vec<f32>,
    rate: f32,
    msg_repeat: usize,
) {
    if let Some(am) = source.as_any_mut().downcast_mut::<AmDsbSource>() {
        am.set_audio(audio, rate);
        am.msg_repeat = msg_repeat;
    }
}

/// Silence an `AmDsbSource` (carrier-only output).
pub(in crate::app) fn clear_audio(source: &mut dyn SignalSource) {
    if let Some(am) = source.as_any_mut().downcast_mut::<AmDsbSource>() {
        am.set_audio(Vec::new(), SAMPLE_RATE);
    }
}

/// Submode line for the top HUD when AM DSB is the active source.
pub(in crate::app) fn hud_submode_str(settings: &SettingsState) -> String {
    amdsb::hud_submode_str(settings.am_audio_str())
}
