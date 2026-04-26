// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use num_complex::Complex32 as C32;
use orion_sdr::codec::MorseEncoder;
use orion_sdr::core::Block;
use orion_sdr::modulate::CwKeyedMod;

use crate::source::{MAX_SIG_SECS, SignalSource};

// ── CW constants ─────────────────────────────────────────────────────────────

pub const CW_DEFAULT_CARRIER_HZ: f32 = 12_000.0;
pub const CW_DEFAULT_GAP_SECS: f32 = 10.0;
pub const CW_DEFAULT_WPM: f32 = 13.0;
pub const CW_DEFAULT_JITTER_PCT: f32 = 5.0;
pub const CW_DEFAULT_DASH_WEIGHT: f32 = 3.0;
pub const CW_DEFAULT_CHAR_SPACE: f32 = 3.0;
pub const CW_DEFAULT_WORD_SPACE: f32 = 7.0;
pub const CW_DEFAULT_RISE_MS: f32 = 5.0;
pub const CW_DEFAULT_FALL_MS: f32 = 5.0;
pub const CW_DEFAULT_NOISE_AMP: f32 = 0.05;
pub const CW_DEFAULT_REPEAT: usize = 3;
pub const CW_DEFAULT_CANNED_TEXT: &str = "CQ CQ CQ DE N0GNR";
pub const CW_DEFAULT_CUSTOM_TEXT: &str = "Custom message";

// ── CwSource ─────────────────────────────────────────────────────────────────

/// CW (Morse code) signal source.
///
/// Pre-renders a complete keyed-carrier frame (MorseEncoder → CwKeyedMod)
/// at construction.  The frame plays once, followed by a configurable
/// silence gap, then repeats indefinitely without reallocation.
pub struct CwSource {
    pub carrier_hz: f32,
    pub gap_secs: f32,
    pub noise_amp: f32,
    pub wpm: f32,
    pub jitter_pct: f32,
    pub dash_weight: f32,
    pub char_space: f32,
    pub word_space: f32,
    pub rise_ms: f32,
    pub fall_ms: f32,
    /// Text to transmit (ASCII).  Repeated `msg_repeat` times per loop.
    pub message: String,
    /// Number of times to repeat `message` before the silence gap.
    pub msg_repeat: usize,
    mod_rate: f32,
    samples: Vec<f32>,
    pos: usize,
    gap_remaining: usize,
    gap_samples: usize,
    rng: u64,
}

impl CwSource {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        carrier_hz: f32,
        gap_secs: f32,
        noise_amp: f32,
        wpm: f32,
        jitter_pct: f32,
        dash_weight: f32,
        char_space: f32,
        word_space: f32,
        rise_ms: f32,
        fall_ms: f32,
        message: String,
        msg_repeat: usize,
        mod_rate: f32,
    ) -> Self {
        let gap_samples = (gap_secs * mod_rate) as usize;
        let mut src = Self {
            carrier_hz,
            gap_secs,
            noise_amp,
            wpm,
            jitter_pct,
            dash_weight,
            char_space,
            word_space,
            rise_ms,
            fall_ms,
            message,
            msg_repeat: msg_repeat.max(1),
            mod_rate,
            samples: Vec::new(),
            pos: 0,
            gap_remaining: 0,
            gap_samples,
            rng: 0x853c_49e6_748f_ea9b,
        };
        src.render();
        src
    }

    /// (Re-)render the modulated frame.  Called at construction and whenever
    /// carrier, wpm, jitter, weighting, spacing, rise/fall, message, or
    /// repeat count changes.
    ///
    /// Pipeline: MorseEncoder → keying envelope → CwKeyedMod → IQ → .re
    pub fn render(&mut self) {
        // Build repeated text: "msg msg msg" (space-separated).
        let repeated: Vec<u8> = std::iter::repeat_n(self.message.as_bytes(), self.msg_repeat)
            .collect::<Vec<_>>()
            .join(b" ".as_ref());
        let text = String::from_utf8_lossy(&repeated);

        // Stage 1: Morse text → keying envelope (0.0 / 1.0).
        let envelope = MorseEncoder::new(self.mod_rate, self.wpm)
            .with_jitter(self.jitter_pct)
            .with_dash_weight(self.dash_weight)
            .with_char_space(self.char_space)
            .with_word_space(self.word_space)
            .encode_text(&text);

        if envelope.is_empty() {
            self.samples = Vec::new();
            self.pos = 0;
            self.gap_remaining = 0;
            return;
        }

        // Stage 2: Keying envelope → IQ via CwKeyedMod.
        let mut modulator =
            CwKeyedMod::new(self.mod_rate, self.carrier_hz, self.rise_ms, self.fall_ms);
        let mut iq = vec![C32::new(0.0, 0.0); envelope.len()];
        modulator.process(&envelope, &mut iq);

        // Take real part as output signal.
        self.samples = iq.into_iter().map(|c| c.re).collect();
        self.pos = 0;
        self.gap_remaining = 0;
    }

    /// Recompute the gap sample count after `gap_secs` changes.
    pub fn update_gap(&mut self) {
        self.gap_samples = (self.gap_secs * self.mod_rate) as usize;
    }

    /// Apply a fresh set of timing/carrier parameters and return change flags.
    /// `message` is intentionally NOT updated here — it is committed only when
    /// the user explicitly accepts a text edit (see app-level glue).
    #[allow(clippy::too_many_arguments)]
    pub fn apply_params(
        &mut self,
        carrier_hz: f32,
        gap_secs: f32,
        noise_amp: f32,
        wpm: f32,
        jitter_pct: f32,
        dash_weight: f32,
        char_space: f32,
        word_space: f32,
        rise_ms: f32,
        fall_ms: f32,
        msg_repeat: usize,
    ) -> CwSyncFlags {
        let carrier_changed = (self.carrier_hz - carrier_hz).abs() > 0.01;
        let wpm_changed = (self.wpm - wpm).abs() > 0.01;
        let jitter_changed = (self.jitter_pct - jitter_pct).abs() > 0.01;
        let weight_changed = (self.dash_weight - dash_weight).abs() > 0.01;
        let char_sp_changed = (self.char_space - char_space).abs() > 0.01;
        let word_sp_changed = (self.word_space - word_space).abs() > 0.01;
        let rise_changed = (self.rise_ms - rise_ms).abs() > 0.01;
        let fall_changed = (self.fall_ms - fall_ms).abs() > 0.01;
        let repeat_changed = self.msg_repeat != msg_repeat;

        self.carrier_hz = carrier_hz;
        self.wpm = wpm;
        self.jitter_pct = jitter_pct;
        self.dash_weight = dash_weight;
        self.char_space = char_space;
        self.word_space = word_space;
        self.rise_ms = rise_ms;
        self.fall_ms = fall_ms;
        self.noise_amp = noise_amp;
        self.gap_secs = gap_secs;
        self.msg_repeat = msg_repeat.max(1);

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
            self.render();
        }
        self.update_gap();

        CwSyncFlags {
            wpm_or_word_space_changed: wpm_changed || word_sp_changed,
        }
    }
}

/// Per-frame sync result for `CwSource::apply_params`.  The caller uses
/// `wpm_or_word_space_changed` to decide whether to refresh the loop-timer
/// holdoff (CW-specific concern).
#[derive(Clone, Copy, Debug)]
pub struct CwSyncFlags {
    pub wpm_or_word_space_changed: bool,
}

impl CwSource {
    fn xorshift(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 11) as f32 * (1.0 / (1u64 << 53) as f32) * 2.0 - 1.0
    }
}

impl SignalSource for CwSource {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn restart(&mut self) {
        self.pos = 0;
        self.gap_remaining = 0;
    }

    fn next_samples(&mut self, n: usize) -> Vec<f32> {
        let max_sig_samples = (MAX_SIG_SECS * self.mod_rate) as usize;
        let effective_len = self.samples.len().min(max_sig_samples);
        let mut out = Vec::with_capacity(n);
        let mut i = 0;
        while i < n {
            if self.gap_remaining > 0 {
                let gap_now = self.gap_remaining.min(n - i);
                for _ in 0..gap_now {
                    let noise = if self.noise_amp > 0.0 {
                        self.noise_amp * self.xorshift()
                    } else {
                        0.0
                    };
                    out.push(noise);
                }
                self.gap_remaining -= gap_now;
                i += gap_now;
                if self.gap_remaining == 0 {
                    self.pos = 0;
                }
            } else if self.pos < effective_len {
                let available = (effective_len - self.pos).min(n - i);
                for k in 0..available {
                    let noise = if self.noise_amp > 0.0 {
                        self.noise_amp * self.xorshift()
                    } else {
                        0.0
                    };
                    out.push(self.samples[self.pos + k] + noise);
                }
                self.pos += available;
                i += available;
                if self.pos >= effective_len {
                    self.gap_remaining = self.gap_samples;
                }
            } else {
                // samples is empty (should not happen after render())
                out.push(0.0);
                i += 1;
            }
        }
        out
    }

    fn sample_rate(&self) -> f32 {
        self.mod_rate
    }
}
