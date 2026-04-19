// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Background decode thread and associated types.
//!
//! The decode thread receives raw f32 sample blocks from the main thread via a
//! bounded channel and dispatches based on mode:
//!
//! PSK31 (BPSK31 / QPSK31): accumulates samples while the carrier is present,
//! then decodes the entire transmission once when the gap arrives.  This avoids
//! duplicate output and mid-stream cold-starts that the old rolling-window
//! approach produced.
//!
//! AM DSB / Test Tone: uses a fixed rolling window for spectral analysis.
//!
//! CW: character-timed text decode using a pre-computed schedule from the known
//! message and WPM, plus spectral analysis for the Di bar.
//!
//! FT8 / FT4: streaming accumulate+decode via `Ft8StreamDecoder`.
//!
//! The main thread drains results each frame and updates `DecodeTicker`.

pub mod amdsb;
pub mod cw;
pub mod ft8;
pub mod psk31;
pub mod spectral;
pub mod tone;

use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, Mutex};

use orion_sdr::util::rms;

// Re-export used by the binary.
pub use orion_sdr::util::SIGNAL_THRESHOLD;

// Re-exports for integration tests (not used by the binary itself).
#[allow(unused_imports)]
pub use cw::{cw_char_timing, morse_char_units};
#[allow(unused_imports)]
pub use ft8::{FT4_BW_HZ, FT8_BW_HZ};
#[allow(unused_imports)]
pub use orion_sdr::codec::psk31::Psk31Stream;
#[allow(unused_imports)]
pub use orion_sdr::util::{
    PSK31_BW_HZ, best_sync, power_spectrum, spectrum_bw_hz, spectrum_snr_db,
};
#[allow(unused_imports)]
pub use psk31::{INFO_INTERVAL, PSK31_MAX_ACCUM_SYMS, SYNC_MIN_SYMS, SYNC_SEARCH_HZ};

// ── Public types ──────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DecodeMode {
    Off,
    Bpsk31,
    Qpsk31,
    AmDsb,
    TestTone,
    Cw,
    /// FT8 full-frame accumulate+decode (Phase 2).
    Ft8,
    /// FT4 full-frame accumulate+decode (Phase 2).
    Ft4,
}

#[derive(Clone, Debug)]
pub struct DecodeConfig {
    pub mode: DecodeMode,
    pub carrier_hz: f32,
    pub fs: f32,
    // CW-specific fields for character-timed text decode.
    pub cw_message: String,
    pub cw_wpm: f32,
    pub cw_dash_weight: f32,
    pub cw_char_space: f32,
    pub cw_word_space: f32,
    pub cw_msg_repeat: usize,
}

impl DecodeConfig {
    pub fn new(fs: f32) -> Self {
        Self {
            mode: DecodeMode::Off,
            carrier_hz: 0.0,
            fs,
            cw_message: String::new(),
            cw_wpm: 0.0,
            cw_dash_weight: 3.0,
            cw_char_space: 3.0,
            cw_word_space: 7.0,
            cw_msg_repeat: 1,
        }
    }
}

#[derive(Clone, Debug)]
pub enum DecodeResult {
    /// New decoded text to append to the ticker.
    Text(String),
    /// Non-text signal — display a one-line summary.
    Info {
        modulation: String,
        center_hz: f32,
        bw_hz: f32,
        snr_db: f32,
    },
    /// No signal detected or carrier not found.
    NoSignal,
    /// Definite signal gap — bypasses hold timer.
    /// `decoded`: for FT8/FT4, true if at least one CRC-pass frame was found at
    /// this gap edge; always false for other sources (ignored by the main thread).
    Gap { decoded: bool },
}

// ── DecodeTicker ──────────────────────────────────────────────────────────────

/// Minimum seconds to hold an Info result before replacing it.
const INFO_HOLD_SECS: f32 = 3.0;
/// Scroll speed in pixels per second.
/// 36 px/s at 12 pt monospace (~7.2 px/char) ≈ 5 chars/s.
const SCROLL_PX_PER_SEC: f32 = 36.0;
/// Approximate character width at 12 pt monospace.
const CHAR_W: f32 = 7.2;
/// Max visible text buffer length (chars).
const MAX_BUF: usize = 512;

/// Scrolling ticker state maintained on the main thread.
///
/// Decoded text is queued in `pending`.  `tick()` advances a smooth pixel
/// offset; when it crosses a character-width boundary, the next character is
/// popped from `pending` to `visible`.  The renderer shifts the visible text
/// by the sub-character pixel fraction for jitter-free animation.
pub struct DecodeTicker {
    /// Characters waiting to be displayed, in order.
    pending: std::collections::VecDeque<char>,
    /// Characters already shown on screen (right-aligned, newest at right).
    pub visible: String,
    /// Accumulated sub-character pixel offset (0.0 .. CHAR_W).
    /// When this reaches CHAR_W, a new character is popped from pending.
    pub sub_px: f32,
    /// Currently displayed result.
    pub last_result: DecodeResult,
    /// Seconds the current result has been displayed.
    hold_elapsed: f32,
    /// Most recent Info result, retained independently of `last_result` so the
    /// Di bar can show signal data even while a Text hold is in effect.
    pub last_info: Option<DecodeResult>,
    /// True while in a signal gap — drives SPACE injection in `tick()`.
    pub in_gap: bool,
}

impl Default for DecodeTicker {
    fn default() -> Self {
        Self::new()
    }
}

impl DecodeTicker {
    pub fn new() -> Self {
        Self {
            pending: std::collections::VecDeque::new(),
            visible: String::new(),
            sub_px: 0.0,
            last_result: DecodeResult::NoSignal,
            hold_elapsed: 0.0,
            last_info: None,
            in_gap: false,
        }
    }

    /// Integrate a new result from the decode thread.
    ///
    /// - `Text`: characters are queued in `pending` for gradual reveal.
    /// - `Info`: updates `last_info` (for Di bar); replaces `last_result` after hold.
    /// - `NoSignal` / `Gap`: transitions to no-signal state (Gap bypasses hold).
    /// - `FtGap`: consumed by the main thread before reaching here; treated as Gap if it arrives.
    pub fn push_result(&mut self, r: DecodeResult) {
        match &r {
            DecodeResult::Text(s) => {
                self.in_gap = false;
                for c in s.chars() {
                    self.pending.push_back(c);
                }
                if !matches!(self.last_result, DecodeResult::Text(_)) {
                    self.last_result = r;
                    self.hold_elapsed = 0.0;
                }
            }
            DecodeResult::Info { .. } => {
                self.last_info = Some(r.clone());
                let hold = match self.last_result {
                    DecodeResult::Text(_) => 0.0,
                    DecodeResult::Info { .. } => INFO_HOLD_SECS,
                    DecodeResult::NoSignal | DecodeResult::Gap { .. } => 0.0,
                };
                if self.hold_elapsed >= hold {
                    self.last_result = r;
                    self.hold_elapsed = 0.0;
                }
            }
            DecodeResult::NoSignal => {
                let hold = match self.last_result {
                    DecodeResult::Text(_) => 0.0,
                    DecodeResult::Info { .. } => INFO_HOLD_SECS,
                    DecodeResult::NoSignal | DecodeResult::Gap { .. } => 0.0,
                };
                if self.hold_elapsed >= hold {
                    self.last_result = r;
                    self.hold_elapsed = 0.0;
                }
            }
            DecodeResult::Gap { .. } => {
                self.last_result = DecodeResult::NoSignal;
                self.hold_elapsed = 0.0;
                self.last_info = None;
                self.in_gap = true;
            }
        }
    }

    /// Advance the ticker.  Call once per frame with frame delta time.
    ///
    /// Smoothly advances pixel offset; pops characters from `pending` to
    /// `visible` when crossing character-width boundaries.  During gaps,
    /// injects SPACE characters at the same rate.
    pub fn tick(&mut self, dt: f32) {
        self.hold_elapsed += dt;

        // Only scroll if there's something to show or inject.
        let has_work = !self.pending.is_empty() || (self.in_gap && !self.visible.is_empty());
        if !has_work {
            return;
        }

        self.sub_px += SCROLL_PX_PER_SEC * dt;

        // Pop characters when crossing each CHAR_W boundary.
        while self.sub_px >= CHAR_W {
            self.sub_px -= CHAR_W;
            if let Some(c) = self.pending.pop_front() {
                self.visible.push(c);
            } else if self.in_gap {
                self.visible.push(' ');
            }
        }

        // Cap visible buffer length.
        if self.visible.len() > MAX_BUF {
            let drop = self.visible.len() - MAX_BUF;
            self.visible.drain(..drop);
        }
    }

    /// Flush the buffer and reset scroll (call on source/config change).
    pub fn reset(&mut self) {
        self.pending.clear();
        self.visible.clear();
        self.sub_px = 0.0;
        self.hold_elapsed = 0.0;
        self.last_result = DecodeResult::NoSignal;
        self.last_info = None;
        self.in_gap = false;
    }
}

// ── Decode worker ─────────────────────────────────────────────────────────────

/// Fixed window size (samples) for spectral analysis (AM DSB, CW, Test Tone).
/// 4096 samples at 48 kHz = ~85 ms; bin resolution = 11.7 Hz.
pub const SPECTRUM_WINDOW_SAMPLES: usize = 4096;

pub struct DecodeWorker {
    config: Arc<Mutex<DecodeConfig>>,
    rx: Receiver<Vec<f32>>,
    tx: SyncSender<DecodeResult>,
}

impl DecodeWorker {
    pub fn new(
        config: Arc<Mutex<DecodeConfig>>,
        rx: Receiver<Vec<f32>>,
        tx: SyncSender<DecodeResult>,
    ) -> Self {
        Self { config, rx, tx }
    }

    pub fn run(self) {
        let mut last_mode = DecodeMode::Off;
        let mut last_carrier = 0.0_f32;
        let mut was_signal = false;

        // Per-mode state.
        let mut psk31 = psk31::Psk31State::new();
        let mut cw = cw::CwState::new();
        let mut amdsb = amdsb::AmDsbState::new();
        let mut testtone = tone::ToneState::new();
        let mut ft8 = ft8::Ft8State::new();

        loop {
            let samples = match self.rx.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok(s) => s,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            };

            let (mode, carrier_hz, fs) = {
                let cfg = self.config.lock().unwrap();
                if cfg.mode == DecodeMode::Cw {
                    cw.message.clone_from(&cfg.cw_message);
                    cw.wpm = cfg.cw_wpm;
                    cw.dash_weight = cfg.cw_dash_weight;
                    cw.char_space = cfg.cw_char_space;
                    cw.word_space = cfg.cw_word_space;
                    cw.msg_repeat = cfg.cw_msg_repeat;
                }
                (cfg.mode, cfg.carrier_hz, cfg.fs)
            };

            // Empty vec is a flush signal (sent by main thread on source reset).
            if samples.is_empty() {
                psk31.reset();
                cw.reset();
                amdsb.reset();
                testtone.reset();
                ft8.reset();
                was_signal = false;
                last_mode = mode;
                last_carrier = carrier_hz;
                continue;
            }

            // Flush accumulated buffer on config change.
            if mode != last_mode || (carrier_hz - last_carrier).abs() > 0.5 {
                psk31.reset();
                cw.reset();
                amdsb.reset();
                testtone.reset();
                ft8.reset();
                was_signal = false;
                last_mode = mode;
                last_carrier = carrier_hz;
            }

            let is_signal = rms(&samples) >= SIGNAL_THRESHOLD;
            let gap_edge = !is_signal && was_signal;
            was_signal = is_signal;

            match mode {
                DecodeMode::Bpsk31 | DecodeMode::Qpsk31 => {
                    psk31.process(
                        &samples, is_signal, gap_edge, mode, carrier_hz, fs, &self.tx,
                    );
                }
                DecodeMode::Cw => {
                    cw.process(&samples, is_signal, gap_edge, carrier_hz, fs, &self.tx);
                }
                DecodeMode::AmDsb => {
                    amdsb.process(&samples, is_signal, gap_edge, carrier_hz, fs, &self.tx);
                }
                DecodeMode::TestTone => {
                    testtone.process(&samples, is_signal, gap_edge, carrier_hz, fs, &self.tx);
                }
                DecodeMode::Ft8 | DecodeMode::Ft4 => {
                    ft8.process(
                        &samples, is_signal, gap_edge, mode, carrier_hz, fs, &self.tx,
                    );
                }
                DecodeMode::Off => {}
            }
        }
    }
}
