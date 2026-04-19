// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! CW (Morse code) decode state and processing.

use std::sync::mpsc::SyncSender;

use num_complex::Complex32 as C32;

use crate::source::MAX_SIG_SECS;

use super::psk31::INFO_INTERVAL;
use super::{DecodeResult, SPECTRUM_WINDOW_SAMPLES};
pub use orion_sdr::util::{spectrum_bw_hz, spectrum_snr_db};

// ── CW character timing ─────────────────────────────────────────────────────

/// ITU Morse lookup (subset matching `orion_sdr::codec::morse`).
const MORSE_TABLE: &[(char, &str)] = &[
    ('A', ".-"),
    ('B', "-..."),
    ('C', "-.-."),
    ('D', "-.."),
    ('E', "."),
    ('F', "..-."),
    ('G', "--."),
    ('H', "...."),
    ('I', ".."),
    ('J', ".---"),
    ('K', "-.-"),
    ('L', ".-.."),
    ('M', "--"),
    ('N', "-."),
    ('O', "---"),
    ('P', ".--."),
    ('Q', "--.-"),
    ('R', ".-."),
    ('S', "..."),
    ('T', "-"),
    ('U', "..-"),
    ('V', "...-"),
    ('W', ".--"),
    ('X', "-..-"),
    ('Y', "-.--"),
    ('Z', "--.."),
    ('0', "-----"),
    ('1', ".----"),
    ('2', "..---"),
    ('3', "...--"),
    ('4', "....-"),
    ('5', "....."),
    ('6', "-...."),
    ('7', "--..."),
    ('8', "---.."),
    ('9', "----."),
    ('.', ".-.-.-"),
    (',', "--..--"),
    ('?', "..--.."),
    ('/', "-..-."),
];

/// Compute the duration of a Morse character in units (dot-lengths).
pub fn morse_char_units(c: char, dash_weight: f32) -> Option<f32> {
    let upper = c.to_ascii_uppercase();
    MORSE_TABLE
        .iter()
        .find(|(ch, _)| *ch == upper)
        .map(|(_, pat)| {
            let mut units = 0.0_f32;
            for (i, elem) in pat.chars().enumerate() {
                if i > 0 {
                    units += 1.0; // intra-char gap
                }
                units += match elem {
                    '.' => 1.0,
                    '-' => dash_weight,
                    _ => 0.0,
                };
            }
            units
        })
}

/// Build a schedule of `(character, cumulative_sample_threshold)` pairs for
/// the full CW message (with repeats).  The threshold is the sample offset
/// at which the character has been fully transmitted (body + trailing gap),
/// so the decode worker emits the character when accumulated samples ≥
/// threshold.  Uses nominal (jitter-free) timing.
pub fn cw_char_timing(
    message: &str,
    wpm: f32,
    dash_weight: f32,
    char_space: f32,
    word_space: f32,
    msg_repeat: usize,
    fs: f32,
) -> Vec<(char, usize)> {
    if message.is_empty() || wpm < 1.0 {
        return Vec::new();
    }
    let unit_ms = 1200.0 / wpm;
    let unit_samples = (unit_ms * 1e-3) * fs;
    // The source caps its rendered signal at MAX_SIG_SECS; don't schedule
    // characters beyond that boundary.
    let max_samples = (MAX_SIG_SECS * fs) as usize;

    // Build repeated text the same way CwSource::render() does.
    let repeated: Vec<u8> = std::iter::repeat_n(message.as_bytes(), msg_repeat.max(1))
        .collect::<Vec<_>>()
        .join(b" ".as_ref());
    let text = String::from_utf8_lossy(&repeated);

    let mut schedule = Vec::new();
    let mut cumulative = 0.0_f32;
    let mut pending_gap: Option<f32> = None;

    for c in text.chars() {
        if c.is_ascii_whitespace() {
            if pending_gap.is_some() || !schedule.is_empty() {
                pending_gap = Some(word_space);
            }
            continue;
        }

        let char_units = match morse_char_units(c, dash_weight) {
            Some(u) => u,
            None => continue,
        };

        // Emit pending gap before this character.
        if let Some(gap_units) = pending_gap.take() {
            // Word gaps produce a space in the decoded output.
            if gap_units >= word_space {
                schedule.push((' ', cumulative.round() as usize));
            }
            cumulative += gap_units * unit_samples;
        }

        // Character body.
        cumulative += char_units * unit_samples;

        // Stop if this character lands beyond the source's signal cap.
        if cumulative.round() as usize > max_samples {
            // Emit space + ellipsis to indicate truncation.
            schedule.push((' ', max_samples));
            schedule.push(('\u{2026}', max_samples));
            break;
        }

        // Emit the character at the end of its body (before the trailing
        // char gap, so text appears as soon as the character is keyed).
        schedule.push((c.to_ascii_uppercase(), cumulative.round() as usize));

        // Queue char gap.
        pending_gap = Some(char_space);
    }

    schedule
}

// ── CW decode state ─────────────────────────────────────────────────────────

/// Holdoff multiplier: silence must persist for this many word-space durations
/// before we declare a true transmission gap.  2× word space is long enough
/// to ride through the longest intra-message silence (a single word gap)
/// while still detecting the real inter-transmission gap quickly.
const HOLDOFF_WORD_SPACES: f32 = 2.0;

pub struct CwState {
    // Spectral analysis state (Di bar).
    pub spec_buf: Vec<C32>,
    pub smoothed_snr_db: f32,
    pub smoothed_bw_hz: f32,
    pub info_counter: usize,
    // Character-timed text decode state.
    pub char_schedule: Vec<(char, usize)>,
    pub accum_samples: usize,
    pub next_char_idx: usize,
    // Holdoff: CW keying gaps produce per-block silence that must not be
    // mistaken for a transmission gap.  We track consecutive silent samples
    // and only declare a gap when silence exceeds the holdoff threshold.
    in_signal: bool,
    silence_samples: usize,
    // Snapshot of CW config fields (refreshed each sample block by the worker).
    pub message: String,
    pub wpm: f32,
    pub dash_weight: f32,
    pub char_space: f32,
    pub word_space: f32,
    pub msg_repeat: usize,
    // Config values used to build the current schedule.  When any of these
    // diverge from the live config, the schedule is rebuilt mid-signal.
    sched_wpm: f32,
    sched_dash_weight: f32,
    sched_char_space: f32,
    sched_word_space: f32,
    sched_msg_repeat: usize,
    sched_message: String,
}

impl Default for CwState {
    fn default() -> Self {
        Self {
            spec_buf: Vec::new(),
            smoothed_snr_db: 0.0,
            smoothed_bw_hz: 0.0,
            info_counter: 0,
            char_schedule: Vec::new(),
            accum_samples: 0,
            next_char_idx: 0,
            in_signal: false,
            silence_samples: 0,
            message: String::new(),
            wpm: 0.0,
            dash_weight: 3.0,
            char_space: 3.0,
            word_space: 7.0,
            msg_repeat: 1,
            sched_wpm: 0.0,
            sched_dash_weight: 0.0,
            sched_char_space: 0.0,
            sched_word_space: 0.0,
            sched_msg_repeat: 0,
            sched_message: String::new(),
        }
    }
}

impl CwState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        self.spec_buf.clear();
        self.smoothed_snr_db = 0.0;
        self.smoothed_bw_hz = 0.0;
        self.info_counter = 0;
        self.char_schedule.clear();
        self.accum_samples = 0;
        self.next_char_idx = 0;
        self.in_signal = false;
        self.silence_samples = 0;
    }

    /// Holdoff threshold in samples: silence must exceed this to declare a
    /// real transmission gap.
    fn holdoff_samples(&self, fs: f32) -> usize {
        if self.wpm < 1.0 {
            return 0;
        }
        let unit_ms = 1200.0 / self.wpm;
        let word_gap_ms = unit_ms * self.word_space;
        let holdoff_ms = word_gap_ms * HOLDOFF_WORD_SPACES;
        (holdoff_ms * 1e-3 * fs) as usize
    }

    pub fn process(
        &mut self,
        samples: &[f32],
        is_signal: bool,
        _gap_edge: bool,
        carrier_hz: f32,
        fs: f32,
        tx: &SyncSender<DecodeResult>,
    ) {
        let holdoff = self.holdoff_samples(fs);

        if is_signal {
            // Reset silence counter — we're hearing carrier.
            self.silence_samples = 0;

            if !self.in_signal {
                // True signal onset (first block or after a real gap).
                self.in_signal = true;
            }
        } else {
            // Block is below threshold — could be a keying gap or a real gap.
            self.silence_samples += samples.len();

            if self.in_signal && self.silence_samples > holdoff {
                // Silence has persisted beyond the holdoff — declare real gap.
                self.in_signal = false;
                self.spec_buf.clear();
                self.info_counter = 0;
                self.smoothed_snr_db = 0.0;
                self.smoothed_bw_hz = 0.0;
                self.accum_samples = 0;
                self.next_char_idx = 0;
                self.char_schedule.clear();
                let _ = tx.try_send(DecodeResult::Info {
                    modulation: "CW".to_owned(),
                    center_hz: carrier_hz,
                    bw_hz: 0.0,
                    snr_db: 0.0,
                });
                return;
            }
        }

        if !self.in_signal {
            return;
        }

        // Detect config changes that invalidate the current schedule.
        let config_changed = !self.char_schedule.is_empty()
            && ((self.wpm - self.sched_wpm).abs() > 0.01
                || (self.dash_weight - self.sched_dash_weight).abs() > 0.01
                || (self.char_space - self.sched_char_space).abs() > 0.01
                || (self.word_space - self.sched_word_space).abs() > 0.01
                || self.msg_repeat != self.sched_msg_repeat
                || self.message != self.sched_message);

        if config_changed {
            // Rebuild schedule and restart from the beginning of the new
            // timing.  Reset accum so the new schedule plays from offset 0.
            self.accum_samples = 0;
            self.next_char_idx = 0;
            self.char_schedule.clear();
        }

        // Build character schedule on signal onset or after config change.
        if self.char_schedule.is_empty() {
            self.char_schedule = cw_char_timing(
                &self.message,
                self.wpm,
                self.dash_weight,
                self.char_space,
                self.word_space,
                self.msg_repeat,
                fs,
            );
            self.next_char_idx = 0;
            self.sched_wpm = self.wpm;
            self.sched_dash_weight = self.dash_weight;
            self.sched_char_space = self.char_space;
            self.sched_word_space = self.word_space;
            self.sched_msg_repeat = self.msg_repeat;
            self.sched_message.clone_from(&self.message);
        }

        // Track accumulated samples (including keying-gap blocks while in
        // holdoff) and emit characters at the nominal timing boundaries.
        self.accum_samples += samples.len();
        while self.next_char_idx < self.char_schedule.len() {
            let (ch, threshold) = self.char_schedule[self.next_char_idx];
            if self.accum_samples >= threshold {
                let _ = tx.try_send(DecodeResult::Text(ch.to_string()));
                self.next_char_idx += 1;
            } else {
                break;
            }
        }

        // Spectral analysis for Di bar — only feed blocks with actual signal
        // (skip keying-gap blocks to avoid diluting SNR/BW estimates).
        if !is_signal {
            return;
        }
        self.spec_buf
            .extend(samples.iter().map(|&s| C32::new(s, 0.0)));
        if self.spec_buf.len() >= SPECTRUM_WINDOW_SAMPLES {
            let decode_buf: Vec<C32> = self.spec_buf[..SPECTRUM_WINDOW_SAMPLES].to_vec();
            self.spec_buf.drain(..SPECTRUM_WINDOW_SAMPLES / 2);

            let real: Vec<f32> = decode_buf.iter().map(|c| c.re).collect();
            let raw_snr = spectrum_snr_db(&real, fs, carrier_hz);
            if self.smoothed_snr_db == 0.0 {
                self.smoothed_snr_db = raw_snr;
            } else {
                self.smoothed_snr_db = 0.2 * raw_snr + 0.8 * self.smoothed_snr_db;
            }
            let raw_bw = spectrum_bw_hz(&real, fs, carrier_hz, 7.0);
            if self.smoothed_bw_hz == 0.0 {
                self.smoothed_bw_hz = raw_bw;
            } else {
                self.smoothed_bw_hz = 0.2 * raw_bw + 0.8 * self.smoothed_bw_hz;
            }
            self.info_counter += SPECTRUM_WINDOW_SAMPLES / 2;
            if self.info_counter >= INFO_INTERVAL {
                self.info_counter = 0;
                let _ = tx.try_send(DecodeResult::Info {
                    modulation: "CW".to_owned(),
                    center_hz: carrier_hz,
                    bw_hz: self.smoothed_bw_hz,
                    snr_db: self.smoothed_snr_db,
                });
            }
        }
    }
}
