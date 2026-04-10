use orion_sdr::modulate::{Bpsk31Mod, Qpsk31Mod};

use super::{SignalSource, MAX_SIG_SECS};

// ── PSK31 constants ───────────────────────────────────────────────────────────

pub const PSK31_DEFAULT_CANNED_TEXT: &str = "CQ CQ CQ DE N0GNR";
pub const PSK31_DEFAULT_CUSTOM_TEXT: &str = "Custom message";
pub const PSK31_DEFAULT_REPEAT: usize = 3;
pub const PSK31_DEFAULT_GAP_SECS: f32 = 15.0;

// ── Psk31Mode ─────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub enum Psk31Mode { Bpsk31, Qpsk31 }

// ── Psk31Source ───────────────────────────────────────────────────────────────

/// PSK31 signal source (BPSK31 or QPSK31).
///
/// Pre-renders a complete modulated frame (preamble + text + postamble) once
/// at construction. The frame plays once, followed by a configurable silence
/// gap, then repeats indefinitely without reallocation.
pub struct Psk31Source {
    pub carrier_hz:    f32,
    pub gap_secs:      f32,
    pub noise_amp:     f32,
    pub mode:          Psk31Mode,
    /// Text to transmit (ASCII). Repeated `msg_repeat` times per loop.
    pub message:       String,
    /// Number of times to repeat `message` before the silence gap.
    pub msg_repeat:    usize,
    mod_rate:          f32,
    samples:           Vec<f32>,
    pos:               usize,
    gap_remaining:     usize,
    gap_samples:       usize,
    rng:               u64,
}

impl Psk31Source {
    pub fn new(
        carrier_hz: f32,
        gap_secs: f32,
        noise_amp: f32,
        mode: Psk31Mode,
        message: String,
        msg_repeat: usize,
        mod_rate: f32,
    ) -> Self {
        let gap_samples = (gap_secs * mod_rate) as usize;
        let mut src = Self {
            carrier_hz,
            gap_secs,
            noise_amp,
            mode,
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

    /// (Re-)render the modulated frame. Called at construction and whenever
    /// carrier, mode, message, or repeat count changes.
    ///
    /// The text fed to the modulator is `message` repeated `msg_repeat` times,
    /// separated by a single space, all within one preamble/postamble envelope.
    pub fn render(&mut self) {
        // Build the repeated text: "msg msg msg" (space-separated).
        let repeated: Vec<u8> = std::iter::repeat_n(self.message.as_bytes(), self.msg_repeat)
            .collect::<Vec<_>>()
            .join(b" ".as_ref());

        self.samples = match self.mode {
            Psk31Mode::Bpsk31 => {
                let iq = Bpsk31Mod::new(self.mod_rate, self.carrier_hz, 1.0)
                    .modulate_text(&repeated, 64, 32);
                iq.into_iter().map(|c| c.re).collect()
            }
            Psk31Mode::Qpsk31 => {
                let iq = Qpsk31Mod::new(self.mod_rate, self.carrier_hz, 1.0)
                    .modulate_text(&repeated, 64, 32);
                iq.into_iter().map(|c| c.re).collect()
            }
        };
        self.pos = 0;
        self.gap_remaining = 0;
    }

    /// Recompute the gap sample count after `gap_secs` changes.
    pub fn update_gap(&mut self) {
        self.gap_samples = (self.gap_secs * self.mod_rate) as usize;
    }

    fn xorshift(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 11) as f32 * (1.0 / (1u64 << 53) as f32) * 2.0 - 1.0
    }
}

impl SignalSource for Psk31Source {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn restart(&mut self) {
        self.pos = 0;
        self.gap_remaining = 0;
    }

    fn next_samples(&mut self, n: usize) -> Vec<f32> {
        let max_sig_samples = (MAX_SIG_SECS * self.mod_rate) as usize;
        // Truncate the effective playback length so the signal burst never
        // exceeds MAX_SIG_SECS (keeps the decode-bar timer within bounds).
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
