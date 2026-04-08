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
//! The main thread drains results each frame and updates `DecodeTicker`.

use std::sync::{Arc, Mutex};
use std::sync::mpsc::{Receiver, SyncSender};

use num_complex::Complex32 as C32;
use orion_sdr::util::rms;
use orion_sdr::sync::psk31_sync::psk31_sync;
use orion_sdr::modulate::psk31::{psk31_sps, PSK31_BAUD};

// Re-exports from orion-sdr (migrated from local definitions).
pub use orion_sdr::codec::psk31::Psk31Stream;
pub use orion_sdr::util::{
    SIGNAL_THRESHOLD, PSK31_BW_HZ,
    power_spectrum, spectrum_snr_db, spectrum_bw_hz, best_sync,
};

// ── Public types ──────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DecodeMode {
    Off,
    Bpsk31,
    Qpsk31,
    AmDsb,
    TestTone,
    /// FT8 full-frame accumulate+decode (Phase 2).
    Ft8,
    /// FT4 full-frame accumulate+decode (Phase 2).
    #[allow(dead_code)]
    Ft4,
}

#[derive(Clone, Debug)]
pub struct DecodeConfig {
    pub mode:       DecodeMode,
    pub carrier_hz: f32,
    pub fs:         f32,
}

impl DecodeConfig {
    pub fn new(fs: f32) -> Self {
        Self { mode: DecodeMode::Off, carrier_hz: 0.0, fs }
    }
}

#[derive(Clone, Debug)]
pub enum DecodeResult {
    /// New decoded text to append to the ticker.
    Text(String),
    /// Non-text signal — display a one-line summary.
    Info {
        modulation: String,
        center_hz:  f32,
        bw_hz:      f32,
        snr_db:     f32,
    },
    /// No signal detected or carrier not found.
    NoSignal,
    /// Definite signal gap (e.g. inter-loop silence) — bypasses hold timer.
    Gap,
}

// ── DecodeTicker ──────────────────────────────────────────────────────────────

/// Minimum seconds to hold an Info result before replacing it.
const INFO_HOLD_SECS:    f32 = 3.0;
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

impl DecodeTicker {
    pub fn new() -> Self {
        Self {
            pending:      std::collections::VecDeque::new(),
            visible:      String::new(),
            sub_px:       0.0,
            last_result:  DecodeResult::NoSignal,
            hold_elapsed: 0.0,
            last_info:    None,
            in_gap:       false,
        }
    }

    /// Integrate a new result from the decode thread.
    ///
    /// - `Text`: characters are queued in `pending` for gradual reveal.
    /// - `Info`: updates `last_info` (for Di bar); replaces `last_result` after hold.
    /// - `NoSignal` / `Gap`: transitions to no-signal state (Gap bypasses hold).
    pub fn push_result(&mut self, r: DecodeResult) {
        match &r {
            DecodeResult::Text(s) => {
                self.in_gap = false;
                for c in s.chars() {
                    self.pending.push_back(c);
                }
                if !matches!(self.last_result, DecodeResult::Text(_)) {
                    self.last_result  = r;
                    self.hold_elapsed = 0.0;
                }
            }
            DecodeResult::Info { .. } => {
                self.last_info = Some(r.clone());
                let hold = match self.last_result {
                    DecodeResult::Text(_)               => 0.0,
                    DecodeResult::Info { .. }           => INFO_HOLD_SECS,
                    DecodeResult::NoSignal | DecodeResult::Gap => 0.0,
                };
                if self.hold_elapsed >= hold {
                    self.last_result  = r;
                    self.hold_elapsed = 0.0;
                }
            }
            DecodeResult::NoSignal => {
                let hold = match self.last_result {
                    DecodeResult::Text(_)   => 0.0,
                    DecodeResult::Info {..} => INFO_HOLD_SECS,
                    DecodeResult::NoSignal | DecodeResult::Gap => 0.0,
                };
                if self.hold_elapsed >= hold {
                    self.last_result  = r;
                    self.hold_elapsed = 0.0;
                }
            }
            DecodeResult::Gap => {
                self.last_result  = DecodeResult::NoSignal;
                self.hold_elapsed = 0.0;
                self.in_gap       = true;
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
        let has_work = !self.pending.is_empty()
            || (self.in_gap && !self.visible.is_empty());
        if !has_work { return; }

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
        self.sub_px       = 0.0;
        self.hold_elapsed = 0.0;
        self.last_result  = DecodeResult::NoSignal;
        self.last_info    = None;
        self.in_gap       = false;
    }
}

// ── Decode worker ─────────────────────────────────────────────────────────────

/// Maximum PSK31 accumulation buffer: caps memory and limits decode latency.
/// 1200 symbols ≈ 38 s at 31.25 baud — comfortably larger than the default
/// transmission (msg×5 + preamble + postamble ≈ 1100 symbols) so a full frame
/// is never truncated in normal use.  If the carrier runs longer the buffer is
/// decoded and flushed at this boundary without waiting for a gap.
pub const PSK31_MAX_ACCUM_SYMS: usize = 1200;

/// Fixed window size (samples) for AM DSB / Test Tone spectral analysis.
/// 4096 samples at 48 kHz = ~85 ms; bin resolution = 11.7 Hz.
pub const SPECTRUM_WINDOW_SAMPLES: usize = 4096;

/// Search half-width around the configured carrier (±200 Hz).
pub const SYNC_SEARCH_HZ: f32 = 200.0;

/// Minimum accumulated samples before attempting psk31_sync (64 symbols).
pub const SYNC_MIN_SYMS: usize = 64;

pub struct DecodeWorker {
    config: Arc<Mutex<DecodeConfig>>,
    rx:     Receiver<Vec<f32>>,
    tx:     SyncSender<DecodeResult>,
}

impl DecodeWorker {
    pub fn new(
        config: Arc<Mutex<DecodeConfig>>,
        rx:     Receiver<Vec<f32>>,
        tx:     SyncSender<DecodeResult>,
    ) -> Self {
        Self { config, rx, tx }
    }

    pub fn run(self) {
        let mut iq_buf:        Vec<C32> = Vec::new();
        let mut last_mode    = DecodeMode::Off;
        let mut last_carrier = 0.0_f32;
        let mut was_signal   = false;
        // EMA-smoothed BW for AM DSB (α = 0.3 → ~2–3 window time constant).
        let mut smoothed_bw_hz = 0.0_f32;
        // Rolling window state for AM DSB / Test Tone spectral analysis.
        let mut spec_buf: Vec<C32> = Vec::new();
        // Streaming PSK31 decode state (created after first sync, destroyed at gap).
        let mut psk31_stream: Option<Psk31Stream> = None;
        // Sample counter for Info throttling (~250 ms between updates, all modes).
        let mut info_counter: usize = 0;
        const INFO_INTERVAL: usize = 48_000; // 1 s at 48 kHz
        // EMA-smoothed SNR for Di display (α = 0.2, shared across modes).
        let mut smoothed_snr_db = 0.0_f32;

        loop {
            let samples = match self.rx.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok(s)  => s,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)      => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            };

            let (mode, carrier_hz, fs) = {
                let cfg = self.config.lock().unwrap();
                (cfg.mode, cfg.carrier_hz, cfg.fs)
            };

            // Empty vec is a flush signal (sent by main thread on source reset).
            if samples.is_empty() {
                iq_buf.clear();
                spec_buf.clear();
                smoothed_bw_hz  = 0.0;
                smoothed_snr_db = 0.0;
                was_signal      = false;
                info_counter    = 0;
                psk31_stream    = None;
                last_mode       = mode;
                last_carrier    = carrier_hz;
                continue;
            }

            // Flush accumulated buffer on config change.
            if mode != last_mode || (carrier_hz - last_carrier).abs() > 0.5 {
                iq_buf.clear();
                spec_buf.clear();
                smoothed_bw_hz  = 0.0;
                smoothed_snr_db = 0.0;
                was_signal      = false;
                info_counter    = 0;
                psk31_stream    = None;
                last_mode       = mode;
                last_carrier    = carrier_hz;
            }

            let is_signal = rms(&samples) >= SIGNAL_THRESHOLD;
            let gap_edge = !is_signal && was_signal;
            was_signal = is_signal;

            match mode {
                DecodeMode::Bpsk31 | DecodeMode::Qpsk31 => {
                    let sps        = psk31_sps(fs);
                    let max_accum  = PSK31_MAX_ACCUM_SYMS * sps;
                    let mode_label = if mode == DecodeMode::Bpsk31 { "BPSK31" } else { "QPSK31" };

                    if !is_signal {
                        if gap_edge {
                            info_counter    = 0;
                            smoothed_snr_db = 0.0;
                            // Send zeroed Info so the Di bar clears immediately.
                            let _ = self.tx.try_send(DecodeResult::Info {
                                modulation: mode_label.to_owned(),
                                center_hz:  carrier_hz,
                                bw_hz:      0.0,
                                snr_db:     0.0,
                            });
                            if let Some(ref mut stream) = psk31_stream {
                                // Flush remaining samples + Viterbi tail + varicode.
                                if stream.fed_up_to() < iq_buf.len() {
                                    let text = stream.feed(&iq_buf[stream.fed_up_to()..]);
                                    if !text.is_empty() {
                                        let _ = self.tx.try_send(DecodeResult::Text(text));
                                    }
                                }
                                let tail = stream.flush();
                                if !tail.is_empty() {
                                    let _ = self.tx.try_send(DecodeResult::Text(tail));
                                }
                            }
                            psk31_stream = None;
                            iq_buf.clear();
                        }
                    } else {
                        iq_buf.extend(samples.iter().map(|&s| C32::new(s, 0.0)));

                        // Try to establish the stream if we haven't yet.
                        if psk31_stream.is_none()
                            && iq_buf.len() >= sps * SYNC_MIN_SYMS
                        {
                            let margin = if mode == DecodeMode::Bpsk31 { 1.5 } else { 3.0 };
                            let base_hz = (carrier_hz - SYNC_SEARCH_HZ).max(0.0);
                            let max_hz  = carrier_hz + SYNC_SEARCH_HZ;
                            let results = psk31_sync(&iq_buf, fs, base_hz, max_hz, 4, margin, 256, 5);
                            if let Some((_found_hz, time_sym)) = best_sync(&results, carrier_hz, PSK31_BAUD) {
                                let scan_end = ((time_sym + 2) as usize * sps).min(iq_buf.len());
                                let onset = iq_buf[..scan_end]
                                    .iter()
                                    .position(|c| c.re * c.re + c.im * c.im > 0.01)
                                    .unwrap_or(0);
                                // BPSK31: start at exact onset (differential demod
                                // is robust to sub-symbol misalignment).
                                // QPSK31: start at onset for the demod; the
                                // differential detection guard in feed() skips
                                // symbols with near-zero energy (silence prefix).
                                let start = onset;

                                let mut stream = match mode {
                                    DecodeMode::Bpsk31 => {
                                        let mut s = Psk31Stream::new_bpsk(fs, carrier_hz, 1.0);
                                        s.set_fed_up_to(start);
                                        s
                                    }
                                    _ => {
                                        let mut s = Psk31Stream::new_qpsk(fs, carrier_hz, 1.0);
                                        s.set_fed_up_to(start);
                                        s
                                    }
                                };
                                let text = stream.feed(&iq_buf[start..]);
                                if !text.is_empty() {
                                    let _ = self.tx.try_send(DecodeResult::Text(text));
                                }
                                stream.set_fed_up_to(iq_buf.len());
                                psk31_stream = Some(stream);
                            }
                        }

                        // Feed new samples to the running stream.
                        if let Some(ref mut stream) = psk31_stream {
                            let new_end = iq_buf.len();
                            if stream.fed_up_to() < new_end {
                                let text = stream.feed(&iq_buf[stream.fed_up_to()..new_end]);
                                if !text.is_empty() {
                                    let _ = self.tx.try_send(DecodeResult::Text(text));
                                }
                                stream.set_fed_up_to(new_end);
                            }
                        }

                        // Periodic Info updates (~1 s) during signal.
                        info_counter += samples.len();
                        if info_counter >= INFO_INTERVAL {
                            info_counter = 0;
                            let tail_start = iq_buf.len().saturating_sub(SPECTRUM_WINDOW_SAMPLES);
                            let win: Vec<f32> = iq_buf[tail_start..]
                                .iter().map(|c| c.re).collect();
                            let raw_snr = spectrum_snr_db(&win, fs, carrier_hz);
                            if smoothed_snr_db == 0.0 {
                                smoothed_snr_db = raw_snr;
                            } else {
                                smoothed_snr_db = 0.2 * raw_snr + 0.8 * smoothed_snr_db;
                            }
                            let _ = self.tx.try_send(DecodeResult::Info {
                                modulation: mode_label.to_owned(),
                                center_hz:  carrier_hz,
                                bw_hz:      PSK31_BW_HZ,
                                snr_db:     smoothed_snr_db,
                            });
                        }

                        // Safety cap: discard oldest samples if buffer grows too large.
                        if iq_buf.len() >= max_accum {
                            // Stream has already processed everything, safe to truncate.
                            let keep = max_accum / 2;
                            let drop = iq_buf.len() - keep;
                            iq_buf.drain(..drop);
                            if let Some(ref mut stream) = psk31_stream {
                                let new_pos = stream.fed_up_to().saturating_sub(drop);
                                stream.set_fed_up_to(new_pos);
                            }
                        }
                    }
                }

                DecodeMode::AmDsb | DecodeMode::TestTone => {
                    if !is_signal {
                        if gap_edge {
                            spec_buf.clear();
                            info_counter    = 0;
                            smoothed_snr_db = 0.0;
                            smoothed_bw_hz  = 0.0;
                            // Send zeroed Info so the Di bar clears immediately.
                            let label = if mode == DecodeMode::AmDsb { "AM DSB" } else { "Test Tone" };
                            let _ = self.tx.try_send(DecodeResult::Info {
                                modulation: label.to_owned(),
                                center_hz:  carrier_hz,
                                bw_hz:      0.0,
                                snr_db:     0.0,
                            });
                        }
                        continue;
                    }
                    spec_buf.extend(samples.iter().map(|&s| C32::new(s, 0.0)));
                    if spec_buf.len() < SPECTRUM_WINDOW_SAMPLES { continue; }

                    let decode_buf: Vec<C32> = spec_buf[..SPECTRUM_WINDOW_SAMPLES].to_vec();
                    spec_buf.drain(..SPECTRUM_WINDOW_SAMPLES / 2);

                    // EMA accumulation runs at the spectral window rate (~43 ms)
                    // for smoothing accuracy; Info is only sent at INFO_INTERVAL.
                    let real: Vec<f32> = decode_buf.iter().map(|c| c.re).collect();
                    let raw_snr = spectrum_snr_db(&real, fs, carrier_hz);
                    if smoothed_snr_db == 0.0 {
                        smoothed_snr_db = raw_snr;
                    } else {
                        smoothed_snr_db = 0.2 * raw_snr + 0.8 * smoothed_snr_db;
                    }
                    let (label, bw) = match mode {
                        DecodeMode::AmDsb => {
                            let raw_bw = spectrum_bw_hz(&real, fs, carrier_hz, 7.0);
                            if smoothed_bw_hz == 0.0 {
                                smoothed_bw_hz = raw_bw;
                            } else {
                                smoothed_bw_hz = 0.2 * raw_bw + 0.8 * smoothed_bw_hz;
                            }
                            ("AM DSB", smoothed_bw_hz)
                        }
                        _ => {
                            let (_, bin_hz) = power_spectrum(&real, fs);
                            ("Test Tone", bin_hz)
                        }
                    };
                    info_counter += SPECTRUM_WINDOW_SAMPLES / 2; // new samples per window step
                    if info_counter >= INFO_INTERVAL {
                        info_counter = 0;
                        let _ = self.tx.try_send(DecodeResult::Info {
                            modulation: label.to_owned(),
                            center_hz:  carrier_hz,
                            bw_hz:      bw,
                            snr_db:     smoothed_snr_db,
                        });
                    }
                }

                // Phase 2 will add FT8/FT4 full-frame decode branches here.
                DecodeMode::Ft8 | DecodeMode::Ft4 | DecodeMode::Off => {}
            }
        }
    }

}


