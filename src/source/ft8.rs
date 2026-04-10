use num_complex::Complex32 as C32;
use orion_sdr::modulate::{Ft8Mod, Ft4Mod};
use orion_sdr::codec::{Ft8Codec, Ft4Codec};
use orion_sdr::message::{Ft8Message, CallsignHashTable, GridField, pack77};

use super::{SignalSource, MAX_SIG_SECS};

// ── FT8 constants ─────────────────────────────────────────────────────────────

pub const FT8_DEFAULT_CARRIER_HZ:    f32 = 12_000.0;
pub const FT8_DEFAULT_GAP_SECS: f32 = 15.0;
pub const FT8_DEFAULT_CALL_TO:       &str = "CQ";
pub const FT8_DEFAULT_CALL_DE:       &str = "N0GNR";
pub const FT8_DEFAULT_GRID:          &str = "FN31";
pub const FT8_DEFAULT_FREE_TEXT:     &str = "CQ DX";

/// Native FT8/FT4 sample rate used by the modulators.
const FT8_NATIVE_FS: f32 = 12_000.0;

/// Fixed baseband frequency for FT8/FT4 modulation (must be < FT8_NATIVE_FS / 2).
/// The rendered signal is frequency-shifted from this base up to `carrier_hz`.
pub const FT8_MOD_BASE_HZ: f32 = 1_500.0;

// ── Ft8Mode ───────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Ft8Mode { Ft8, Ft4 }

// ── Ft8MsgType ────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Ft8MsgType { Standard, FreeText }

// ── Ft8Source ─────────────────────────────────────────────────────────────────

/// FT8/FT4 signal source.
///
/// Pre-renders a complete modulated frame at 12 kHz, then upsamples 4x
/// to the viewer's 48 kHz sample rate.  The frame plays `msg_repeat` times
/// followed by a configurable silence gap, then repeats indefinitely.
#[allow(dead_code)]
pub struct Ft8Source {
    pub carrier_hz:  f32,
    pub gap_secs:    f32,
    pub noise_amp:   f32,
    pub ft8_mode:    Ft8Mode,
    pub msg_type:    Ft8MsgType,
    pub msg_repeat:  usize,

    // Standard message fields
    pub call_to: String,
    pub call_de: String,
    pub grid:    String,

    // FreeText field
    pub free_text: String,

    // Internal
    samples:       Vec<f32>,   // pre-rendered frame at 48 kHz
    mod_rate:      f32,        // viewer sample rate (48 kHz)
    pos:           usize,
    gap_remaining: usize,
    gap_samples:   usize,
    play_count:    usize,      // how many times the frame has played in this cycle
    rng:           u64,
}

impl Ft8Source {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        carrier_hz: f32,
        gap_secs:   f32,
        noise_amp:  f32,
        ft8_mode:   Ft8Mode,
        msg_type:   Ft8MsgType,
        call_to:    String,
        call_de:    String,
        grid:       String,
        free_text:  String,
        msg_repeat: usize,
        mod_rate:   f32,
    ) -> Self {
        let gap_samples = (gap_secs * mod_rate) as usize;
        let mut src = Self {
            carrier_hz,
            gap_secs,
            noise_amp,
            ft8_mode,
            msg_type,
            msg_repeat: msg_repeat.max(1),
            call_to,
            call_de,
            grid,
            free_text,
            samples: Vec::new(),
            mod_rate,
            pos: 0,
            gap_remaining: 0,
            gap_samples,
            play_count: 0,
            rng: 0x853c_49e6_748f_ea9b,
        };
        src.render();
        src
    }

    /// (Re-)render the modulated frame at 48 kHz.
    ///
    /// Modulates at `FT8_MOD_BASE_HZ` (1500 Hz) in the native 12 kHz domain,
    /// upsamples 4× with a windowed-sinc anti-image FIR, then frequency-shifts
    /// the result up to `carrier_hz` so the signal appears at the desired
    /// spectral position in the 48 kHz viewer.  The decode worker reverses the
    /// shift before decimating back to 12 kHz.
    pub fn render(&mut self) {
        let msg = self.build_message();
        let mut ht = CallsignHashTable::new();
        let payload = pack77(&msg, &mut ht).unwrap_or_default();

        let native_iq: Vec<_> = match self.ft8_mode {
            Ft8Mode::Ft8 => {
                let frame = Ft8Codec::encode(&payload);
                Ft8Mod::new(FT8_NATIVE_FS, FT8_MOD_BASE_HZ, 0.0, 1.0).modulate(&frame)
            }
            Ft8Mode::Ft4 => {
                let frame = Ft4Codec::encode(&payload);
                Ft4Mod::new(FT8_NATIVE_FS, FT8_MOD_BASE_HZ, 0.0, 1.0).modulate(&frame)
            }
        };

        // Upsample 4× (complex) with a windowed-sinc lowpass FIR.
        // Cutoff fc = fs_native/2 / fs_out = 6/48 = 0.125 (normalised to fs_out).
        // 63-tap Hann window; gain scaled ×L=4 to restore unity passband gain.
        const L:     usize = 4;
        const NTAPS: usize = 63;
        let     fc = 0.125_f32;

        let mut h = [0.0_f32; NTAPS];
        let half = (NTAPS / 2) as isize;
        for (k, hk) in h.iter_mut().enumerate() {
            let n = k as isize - half;
            let sinc = if n == 0 {
                2.0 * fc
            } else {
                (2.0 * std::f32::consts::PI * fc * n as f32).sin()
                    / (std::f32::consts::PI * n as f32)
            };
            let w = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * k as f32
                / (NTAPS - 1) as f32).cos());
            *hk = sinc * w * L as f32;
        }

        // Zero-insert complex IQ then convolve with the real FIR kernel.
        let up_len = native_iq.len() * L;
        let mut sparse = vec![C32::new(0.0, 0.0); up_len];
        for (i, &s) in native_iq.iter().enumerate() {
            sparse[i * L] = s;
        }

        let mut up_iq = vec![C32::new(0.0, 0.0); up_len];
        for (i, slot) in up_iq.iter_mut().enumerate() {
            let mut acc = C32::new(0.0, 0.0);
            for (j, &hj) in h.iter().enumerate() {
                let idx = i as isize - j as isize + half;
                if idx >= 0 && (idx as usize) < up_len {
                    acc += sparse[idx as usize] * hj;
                }
            }
            *slot = acc;
        }

        // Frequency-shift from FT8_MOD_BASE_HZ up to carrier_hz and take real part.
        // Complex shift: x(t) * exp(j*2π*Δf*t) gives a clean single-sideband shift.
        //
        // Accumulate phase incrementally and wrap to [0, 2π) each step so the
        // argument passed to cos/sin stays small.  Computing `phase_inc * i as f32`
        // directly loses f32 precision over long frames (~600k samples) and
        // produces slowly-varying range-reduction errors in cos/sin that appear
        // as a growing comb of sidebands around the carrier.
        let shift_hz = self.carrier_hz - FT8_MOD_BASE_HZ;
        let phase_inc = 2.0 * std::f32::consts::PI * shift_hz / self.mod_rate;
        let two_pi = 2.0 * std::f32::consts::PI;
        let mut phase = 0.0_f32;
        let up: Vec<f32> = up_iq.iter().map(|&s| {
            let mixer = C32::new(phase.cos(), phase.sin());
            let y = (s * mixer).re;
            phase += phase_inc;
            if phase >= two_pi { phase -= two_pi; }
            y
        }).collect();

        self.samples = up;
        self.pos = 0;
        self.gap_remaining = 0;
        self.play_count = 0;
    }

    /// Recompute the gap sample count after `gap_secs` changes.
    #[allow(dead_code)]
    pub fn update_gap(&mut self) {
        self.gap_samples = (self.gap_secs * self.mod_rate) as usize;
    }

    fn build_message(&self) -> Ft8Message {
        match self.msg_type {
            Ft8MsgType::Standard => Ft8Message::Standard {
                call_to: self.call_to.clone(),
                call_de: self.call_de.clone(),
                extra:   GridField::Grid(self.grid.clone()),
            },
            Ft8MsgType::FreeText => Ft8Message::FreeText(self.free_text.clone()),
        }
    }

    fn xorshift(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 11) as f32 * (1.0 / (1u64 << 53) as f32) * 2.0 - 1.0
    }
}

impl SignalSource for Ft8Source {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }

    fn restart(&mut self) {
        self.pos = 0;
        self.gap_remaining = 0;
        self.play_count = 0;
    }

    fn next_samples(&mut self, n: usize) -> Vec<f32> {
        // Cap total signal samples per burst so the decode-bar timer cannot
        // overflow the fixed-width "sig NN.NN" display.
        let max_sig_samples = (MAX_SIG_SECS * self.mod_rate) as usize;
        let frame_len = self.samples.len();
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
                    // Gap done: start next repetition or re-arm gap for next cycle.
                    self.pos = 0;
                    self.play_count = 0;
                }
            } else if self.pos < frame_len {
                // Samples already emitted in this burst across prior repeats + this frame.
                let emitted = self.play_count * frame_len + self.pos;
                let remaining_budget = max_sig_samples.saturating_sub(emitted);
                if remaining_budget == 0 {
                    // Hit the signal-duration cap mid-burst — truncate and enter gap.
                    self.gap_remaining = self.gap_samples;
                    continue;
                }
                let available = (frame_len - self.pos)
                    .min(n - i)
                    .min(remaining_budget);
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
                if self.pos >= frame_len {
                    self.play_count += 1;
                    if self.play_count < self.msg_repeat {
                        // Play the frame again.
                        self.pos = 0;
                    } else {
                        // All repeats done: enter gap.
                        self.gap_remaining = self.gap_samples;
                    }
                }
            } else {
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
