// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use orion_sdr::modulate::{Bpsk31Mod, Qpsk31Mod};
use orion_sdr::util::PSK31_BW_HZ;

use crate::source::{CnNoise, CnReference, NoiseDomain, SignalSource, mean_power};

// ── PSK31 HUD helpers ────────────────────────────────────────────────────────

/// Format the PSK31 submode string shown in the top HUD line:
/// `"  mode b|q  msg c|n"`.
pub fn hud_submode_str(mode: Psk31Mode, msg_is_custom: bool) -> String {
    let mode_ch = match mode {
        Psk31Mode::Bpsk31 => "b",
        Psk31Mode::Qpsk31 => "q",
    };
    let msg_ch = if msg_is_custom { "c" } else { "n" };
    format!("  mode {mode_ch}  msg {msg_ch}")
}

// ── PSK31 constants ───────────────────────────────────────────────────────────

pub const PSK31_DEFAULT_CANNED_TEXT: &str = "CQ CQ CQ DE N0GNR";
pub const PSK31_DEFAULT_CUSTOM_TEXT: &str = "Custom message";
pub const PSK31_DEFAULT_REPEAT: usize = 3;
pub const PSK31_DEFAULT_GAP_SECS: f32 = 15.0;

/// Default C/N (dB), chosen to reproduce the noise floor the pre-`C/N`
/// amplitude default put on screen.  It sits ~9 dB above COFDM's
/// equivalent because a 62.5 Hz signal against noise spread over 24 kHz is a
/// 25.8 dB spreading factor, against COFDM's 9 dB.
pub const PSK31_DEFAULT_CN_DB: f32 = 54.0;

// ── Psk31Mode ─────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub enum Psk31Mode {
    Bpsk31,
    Qpsk31,
}

// ── Psk31Source ───────────────────────────────────────────────────────────────

/// PSK31 signal source (BPSK31 or QPSK31).
///
/// Pre-renders a complete modulated frame (preamble + text + postamble) once
/// at construction. The frame plays once, followed by a configurable silence
/// gap, then repeats indefinitely without reallocation.
pub struct Psk31Source {
    pub carrier_hz: f32,
    pub gap_secs: f32,
    noise: CnNoise,
    pub mode: Psk31Mode,
    /// Text to transmit (ASCII). Repeated `msg_repeat` times per loop.
    pub message: String,
    /// Number of times to repeat `message` before the silence gap.
    pub msg_repeat: usize,
    mod_rate: f32,
    samples: Vec<f32>,
    pos: usize,
    gap_remaining: usize,
    gap_samples: usize,
}

impl Psk31Source {
    pub fn new(
        carrier_hz: f32,
        gap_secs: f32,
        cn_db: f32,
        mode: Psk31Mode,
        message: String,
        msg_repeat: usize,
        mod_rate: f32,
    ) -> Self {
        let gap_samples = (gap_secs * mod_rate) as usize;
        let mut src = Self {
            carrier_hz,
            gap_secs,
            noise: CnNoise::new(cn_db, cn_reference(0.0, mod_rate)),
            mode,
            message,
            msg_repeat: msg_repeat.max(1),
            mod_rate,
            samples: Vec::new(),
            pos: 0,
            gap_remaining: 0,
            gap_samples,
        };
        src.render();
        src
    }

    /// Requested carrier-to-noise ratio, in dB.
    pub fn cn_db(&self) -> f32 {
        self.noise.cn_db()
    }

    /// Per-component standard deviation of the injected noise.
    pub fn noise_sigma(&self) -> f32 {
        self.noise.sigma()
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
        // The rendered burst is the power reference; re-seat it before the next
        // sample is drawn.  PSK31 is constant-envelope and the buffer holds no
        // gap, so its mean is the transmitting power.
        self.noise
            .set_reference(cn_reference(mean_power(&self.samples), self.mod_rate));
    }

    /// Recompute the gap sample count after `gap_secs` changes.
    pub fn update_gap(&mut self) {
        self.gap_samples = (self.gap_secs * self.mod_rate) as usize;
    }

    /// Apply a fresh set of carrier/mode/timing parameters, re-rendering the
    /// frame if anything that affects waveform content changed.  `message` is
    /// intentionally NOT updated here — committed only on explicit text accept.
    pub fn apply_params(
        &mut self,
        carrier_hz: f32,
        gap_secs: f32,
        cn_db: f32,
        mode: Psk31Mode,
        msg_repeat: usize,
    ) {
        let carrier_changed = (self.carrier_hz - carrier_hz).abs() > 0.01;
        let mode_changed = self.mode != mode;
        let repeat_changed = self.msg_repeat != msg_repeat;

        self.carrier_hz = carrier_hz;
        self.gap_secs = gap_secs;
        self.mode = mode;
        self.msg_repeat = msg_repeat.max(1);

        if carrier_changed || mode_changed || repeat_changed {
            self.render();
        }
        // After `render`, so the C/N is derived against the reference the new
        // buffer implies rather than the outgoing one's.
        self.noise.set_cn_db(cn_db);
        self.update_gap();
    }
}

/// The C/N geometry for a PSK31 burst of measured power `signal_power`.
fn cn_reference(signal_power: f32, fs: f32) -> CnReference {
    CnReference {
        signal_power,
        occupied_bw_hz: PSK31_BW_HZ,
        fs,
        domain: NoiseDomain::Real,
    }
}

impl SignalSource for Psk31Source {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn restart(&mut self) {
        self.pos = 0;
        self.gap_remaining = 0;
    }

    fn next_samples(&mut self, n: usize) -> Vec<f32> {
        // The whole rendered message plays.  It used to be truncated at
        // `MAX_SIG_SECS` so the decode-bar timer's fixed-width field could not
        // overflow — a display bound silently cutting a transmission short.
        // The timer marks the overflow now instead.
        let effective_len = self.samples.len();
        let mut out = Vec::with_capacity(n);
        let mut i = 0;
        while i < n {
            if self.gap_remaining > 0 {
                let gap_now = self.gap_remaining.min(n - i);
                for _ in 0..gap_now {
                    let noise = self.noise.next();
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
                    let noise = self.noise.next();
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
