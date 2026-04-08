use orion_sdr::modulate::{Ft8Mod, Ft4Mod};
use orion_sdr::codec::{Ft8Codec, Ft4Codec};
use orion_sdr::message::{Ft8Message, CallsignHashTable, GridField, pack77};

use super::SignalSource;

// ── FT8 constants ─────────────────────────────────────────────────────────────

pub const FT8_DEFAULT_CARRIER_HZ:    f32  = 1500.0;
pub const FT8_DEFAULT_LOOP_GAP_SECS: f32  = 15.0;
pub const FT8_DEFAULT_REPEAT:        usize = 3;
pub const FT8_DEFAULT_CALL_TO:       &str = "CQ";
pub const FT8_DEFAULT_CALL_DE:       &str = "N0GNR";
pub const FT8_DEFAULT_GRID:          &str = "FN31";
pub const FT8_DEFAULT_FREE_TEXT:     &str = "CQ DX";

/// Native FT8/FT4 sample rate used by the modulators.
const FT8_NATIVE_FS: f32 = 12_000.0;

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
    pub carrier_hz:    f32,
    pub loop_gap_secs: f32,
    pub noise_amp:     f32,
    pub ft8_mode:      Ft8Mode,
    pub msg_type:      Ft8MsgType,
    pub msg_repeat:    usize,

    // Standard message fields
    pub call_to: String,
    pub call_de: String,
    pub grid:    String,

    // FreeText field
    pub free_text: String,

    // Internal
    samples:          Vec<f32>,   // pre-rendered frame at 48 kHz
    mod_rate:         f32,        // viewer sample rate (48 kHz)
    pos:              usize,
    gap_remaining:    usize,
    loop_gap_samples: usize,
    play_count:       usize,      // how many times the frame has played in this cycle
    rng:              u64,
}

impl Ft8Source {
    pub fn new(
        carrier_hz:    f32,
        loop_gap_secs: f32,
        noise_amp:     f32,
        ft8_mode:      Ft8Mode,
        msg_type:      Ft8MsgType,
        call_to:       String,
        call_de:       String,
        grid:          String,
        free_text:     String,
        msg_repeat:    usize,
        mod_rate:      f32,
    ) -> Self {
        let loop_gap_samples = (loop_gap_secs * mod_rate) as usize;
        let mut src = Self {
            carrier_hz,
            loop_gap_secs,
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
            loop_gap_samples,
            play_count: 0,
            rng: 0x853c_49e6_748f_ea9b,
        };
        src.render();
        src
    }

    /// (Re-)render the modulated frame at 48 kHz.
    ///
    /// Renders at 12 kHz (native FT8/FT4 rate) then upsamples 4x via linear
    /// interpolation.  Called at construction and on any parameter change.
    pub fn render(&mut self) {
        let msg = self.build_message();
        let mut ht = CallsignHashTable::new();
        let payload = match pack77(&msg, &mut ht) {
            Some(p) => p,
            None    => [0u8; 10],  // fall back to all-zeros frame
        };

        let native: Vec<f32> = match self.ft8_mode {
            Ft8Mode::Ft8 => {
                let frame = Ft8Codec::encode(&payload);
                Ft8Mod::new(FT8_NATIVE_FS, self.carrier_hz, 0.0, 1.0)
                    .modulate(&frame)
                    .into_iter()
                    .map(|c| c.re)
                    .collect()
            }
            Ft8Mode::Ft4 => {
                let frame = Ft4Codec::encode(&payload);
                Ft4Mod::new(FT8_NATIVE_FS, self.carrier_hz, 0.0, 1.0)
                    .modulate(&frame)
                    .into_iter()
                    .map(|c| c.re)
                    .collect()
            }
        };

        // Upsample 4x: linear interpolation between adjacent native samples.
        // Output length = (native.len() - 1) * 4 + 1, but we round down to
        // native.len() * 4 by treating the last sample as its own group.
        let up_len = native.len() * 4;
        let mut up = Vec::with_capacity(up_len);
        for i in 0..native.len() {
            let a = native[i];
            let b = if i + 1 < native.len() { native[i + 1] } else { a };
            up.push(a);
            up.push(a + (b - a) * 0.25);
            up.push(a + (b - a) * 0.50);
            up.push(a + (b - a) * 0.75);
        }

        self.samples = up;
        self.pos = 0;
        self.gap_remaining = 0;
        self.play_count = 0;
    }

    /// Recompute the loop gap sample count after `loop_gap_secs` changes.
    #[allow(dead_code)]
    pub fn update_loop_gap(&mut self) {
        self.loop_gap_samples = (self.loop_gap_secs * self.mod_rate) as usize;
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
            } else if self.pos < self.samples.len() {
                let available = (self.samples.len() - self.pos).min(n - i);
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
                if self.pos >= self.samples.len() {
                    self.play_count += 1;
                    if self.play_count < self.msg_repeat {
                        // Play the frame again.
                        self.pos = 0;
                    } else {
                        // All repeats done: enter gap.
                        self.gap_remaining = self.loop_gap_samples;
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
