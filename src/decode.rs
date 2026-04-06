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
use orion_sdr::{Block, util::rms};
use orion_sdr::demodulate::psk31::{Bpsk31Demod, Bpsk31Decider, Qpsk31Demod};
use orion_sdr::codec::psk31_conv::{StreamingViterbi, DQPSK_EXP};
use orion_sdr::codec::varicode::VaricodeDecoder;
use orion_sdr::sync::psk31_sync::{psk31_sync, Psk31SyncResult};
use orion_sdr::modulate::psk31::{psk31_sps, PSK31_BAUD};
use rustfft::{FftPlanner, num_complex::Complex};

// ── Public types ──────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DecodeMode {
    Off,
    Bpsk31,
    Qpsk31,
    AmDsb,
    TestTone,
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

/// PSK31 bandwidth: raised-cosine pulse shaping gives exactly 2× the baud rate.
pub const PSK31_BW_HZ: f32 = PSK31_BAUD * 2.0; // 62.5 Hz

/// RMS threshold below which a sample block is treated as silence (loop gap).
/// Must sit above the AWGN noise floor (~0.029 at noise_amp=0.05) and below
/// the modulated signal level (~0.5+ for AM DSB / PSK31 at gain=1.0).
/// Public so main.rs can apply the same threshold for in-frame gap detection.
pub const SIGNAL_THRESHOLD: f32 = 0.1;

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

/// Persistent state for streaming PSK31 decode within one transmission.
/// Created after the first successful `psk31_sync`, destroyed at gap edge.
///
/// BPSK31: fully incremental — Bpsk31Decider produces hard bits instantly,
/// which are pushed through the VaricodeDecoder character by character.
///
/// QPSK31: the Qpsk31Decider accumulates soft symbols for Viterbi.  We
/// periodically run Viterbi on the full accumulation and send only the
/// delta (new bits since last flush) through the VaricodeDecoder.
pub enum Psk31Stream {
    Bpsk {
        demod:     Bpsk31Demod,
        decider:   Bpsk31Decider,
        vdec:      VaricodeDecoder,
        fed_up_to: usize,
    },
    Qpsk {
        demod:      Qpsk31Demod,
        viterbi:    StreamingViterbi,
        vdec:       VaricodeDecoder,
        fed_up_to:  usize,
    },
}

impl Psk31Stream {
    pub fn fed_up_to(&self) -> usize {
        match self {
            Psk31Stream::Bpsk { fed_up_to, .. } => *fed_up_to,
            Psk31Stream::Qpsk { fed_up_to, .. } => *fed_up_to,
        }
    }

    pub fn set_fed_up_to(&mut self, v: usize) {
        match self {
            Psk31Stream::Bpsk { fed_up_to, .. } => *fed_up_to = v,
            Psk31Stream::Qpsk { fed_up_to, .. } => *fed_up_to = v,
        }
    }

    /// Feed new IQ samples through the demod chain.
    /// Returns any newly decoded printable characters.
    pub fn feed(&mut self, iq: &[C32]) -> String {
        if iq.is_empty() { return String::new(); }

        match self {
            Psk31Stream::Bpsk { demod, decider, vdec, .. } => {
                let max_syms = iq.len() / 32 + 4;
                let mut soft = vec![0.0_f32; max_syms];
                let wr = demod.process(iq, &mut soft);
                soft.truncate(wr.out_written);

                let mut bits = vec![0_u8; soft.len()];
                let dr = decider.process(&soft, &mut bits);
                bits.truncate(dr.out_written);

                let mut text = String::new();
                for &b in &bits {
                    vdec.push_bit(b);
                    while let Some(ch) = vdec.pop_char() {
                        if ch >= 0x20 && ch < 0x7f {
                            text.push(ch as char);
                        }
                    }
                }
                text
            }
            Psk31Stream::Qpsk { demod, viterbi, vdec, .. } => {
                // Demod IQ → differential DQPSK products, feed each symbol
                // through the streaming Viterbi → varicode decoder.
                let max_soft = iq.len() / 32 + 8;
                let mut soft = vec![0.0_f32; max_soft];
                let wr = demod.process(iq, &mut soft);
                soft.truncate(wr.out_written);

                let mut text = String::new();
                let n_syms = soft.len() / 2;
                for i in 0..n_syms {
                    let d_re = soft[i * 2];
                    let d_im = soft[i * 2 + 1];
                    // Skip near-zero symbols (silence/startup).
                    if d_re * d_re + d_im * d_im < 0.01 { continue; }

                    if let Some(b) = viterbi.feed_symbol(d_re, d_im) {
                        vdec.push_bit(b);
                        while let Some(ch) = vdec.pop_char() {
                            if ch >= 0x20 && ch < 0x7f {
                                text.push(ch as char);
                            }
                        }
                    }
                }
                text
            }
        }
    }

    /// Flush the decoder to emit the last character(s).
    pub fn flush(&mut self) -> String {
        match self {
            Psk31Stream::Bpsk { vdec, .. } => {
                vdec.push_bit(0);
                vdec.push_bit(0);
                let mut text = String::new();
                while let Some(ch) = vdec.pop_char() {
                    if ch >= 0x20 && ch < 0x7f {
                        text.push(ch as char);
                    }
                }
                text
            }
            Psk31Stream::Qpsk { viterbi, vdec, .. } => {
                // Flush Viterbi tail + varicode.
                let mut text = String::new();
                for b in viterbi.flush() {
                    vdec.push_bit(b);
                    while let Some(ch) = vdec.pop_char() {
                        if ch >= 0x20 && ch < 0x7f {
                            text.push(ch as char);
                        }
                    }
                }
                vdec.push_bit(0);
                vdec.push_bit(0);
                while let Some(ch) = vdec.pop_char() {
                    if ch >= 0x20 && ch < 0x7f {
                        text.push(ch as char);
                    }
                }
                text
            }
        }
    }

}

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
                            if let Some((_found_hz, time_sym)) = best_sync(&results, carrier_hz) {
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
                                    DecodeMode::Bpsk31 => Psk31Stream::Bpsk {
                                        demod:     Bpsk31Demod::new(fs, carrier_hz, 1.0),
                                        decider:   Bpsk31Decider::new(),
                                        vdec:      VaricodeDecoder::new(),
                                        fed_up_to: start,
                                    },
                                    _ => Psk31Stream::Qpsk {
                                        demod:     Qpsk31Demod::new(fs, carrier_hz, 1.0),
                                        viterbi:   StreamingViterbi::new(&DQPSK_EXP),
                                        vdec:      VaricodeDecoder::new(),
                                        fed_up_to: start,
                                    },
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

                DecodeMode::Off => {}
            }
        }
    }

}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Pick the best sync result within 2 × PSK31_BAUD of `carrier_hz`.
/// Primary sort: earliest time_sym (more data available for the demodulator,
/// and the preamble is essential for varicode frame lock).
/// Secondary sort: smallest frequency offset (for tie-breaking among concurrent starts).
/// Returns `(carrier_hz, time_sym)`.
pub fn best_sync(results: &[Psk31SyncResult], carrier_hz: f32) -> Option<(f32, usize)> {
    results
        .iter()
        .filter(|r| (r.carrier_hz - carrier_hz).abs() <= 2.0 * PSK31_BAUD)
        .min_by(|a, b| {
            let da = (a.carrier_hz - carrier_hz).abs();
            let db = (b.carrier_hz - carrier_hz).abs();
            a.time_sym.cmp(&b.time_sym)
                .then(da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal))
        })
        .map(|r| (r.carrier_hz, r.time_sym))
}

/// Compute a power spectrum (dB, same scaling as the display) from real samples,
/// using an FFT whose size is the next power of two ≥ `samples.len()` clamped to
/// a maximum of 4096 for speed.  Returns `(power_db_bins, bin_hz)`.
fn power_spectrum(samples: &[f32], fs: f32) -> (Vec<f32>, f32) {
    let n = samples.len().next_power_of_two().min(4096).max(64);
    let bin_hz = fs / n as f32;

    // Hann window + zero-pad.
    let mut buf: Vec<Complex<f32>> = (0..n)
        .map(|i| {
            let s = if i < samples.len() { samples[i] } else { 0.0 };
            let w = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / n as f32).cos();
            Complex { re: s * w, im: 0.0 }
        })
        .collect();

    FftPlanner::new().plan_fft_forward(n).process(&mut buf);

    let scale = 1.0 / n as f32;
    let bins = n / 2 + 1;
    let power_db: Vec<f32> = buf[..bins]
        .iter()
        .map(|c| {
            let mag_sq = (c.re * c.re + c.im * c.im) * scale * scale;
            10.0 * (mag_sq + 1e-12_f32).log10()
        })
        .collect();

    (power_db, bin_hz)
}

/// Estimate SNR (dB) at `carrier_hz` using the same power-spectrum approach as
/// the display: peak bin power vs median of bins 10–50 bins away from the peak.
pub fn spectrum_snr_db(samples: &[f32], fs: f32, carrier_hz: f32) -> f32 {
    let (power_db, bin_hz) = power_spectrum(samples, fs);
    let n_bins = power_db.len();
    if n_bins < 3 { return 0.0; }

    let peak_bin = ((carrier_hz / bin_hz).round() as usize).min(n_bins - 1);

    // Find the actual peak within ±3 bins of expected (AFC tolerance).
    let search_r = 3_usize;
    let lo = peak_bin.saturating_sub(search_r);
    let hi = (peak_bin + search_r).min(n_bins - 1);
    let sig_bin = (lo..=hi)
        .max_by(|&a, &b| power_db[a].partial_cmp(&power_db[b]).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap_or(peak_bin);

    let sig_db = power_db[sig_bin];

    // Noise floor: collect bins at least 10 bins away from the signal bin,
    // excluding DC (bin 0).  Use the median of those bins.
    let guard = 10_usize;
    let mut noise_bins: Vec<f32> = power_db
        .iter()
        .enumerate()
        .filter(|&(i, _)| i > 0 && (i as isize - sig_bin as isize).unsigned_abs() >= guard)
        .map(|(_, &v)| v)
        .collect();

    if noise_bins.is_empty() { return 0.0; }
    noise_bins.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let noise_db = noise_bins[noise_bins.len() / 2]; // median

    sig_db - noise_db
}

/// Estimate AM DSB occupied bandwidth (Hz).
///
/// AM DSB has a strong carrier with audio sidebands spreading outward on each
/// side.  BW is measured as the span from the outer −7 dB point of the LSB to
/// the outer −7 dB point of the USB, where −7 dB is relative to each
/// sideband's own peak (≈ 20 % power level, matching the visual BW markers).
///
/// Strategy: carrier-relative outer-edge scan.
///
/// Assumes audio has been normalised to peak ≈ 0.9 before modulation.
/// With mod_index = 1.0 the sidebands peak at −6 dB relative to the carrier.
/// We scan outward from the carrier guard and find the outermost bin on each
/// side still within `carrier_drop_db` of the carrier peak.  This threshold
/// sits below the sideband peaks for both CW tones (strong narrow peaks) and
/// broadband voice (many moderate bins), and above the AWGN noise floor.
///
/// −20 dB from carrier = 1 % of carrier power.  For normalised audio:
///   - CW tone: sidebands at −6 dB → well above −20 dB cutoff.
///   - Voice:   broadband sidebands at −6 to −15 dB per formant → above cutoff.
///   - Silence: no modulation → sideband bins at noise floor (< −40 dB) → below.
pub fn spectrum_bw_hz(samples: &[f32], fs: f32, carrier_hz: f32, _threshold_db: f32) -> f32 {
    let search_hz         = 4_000.0_f32;
    let carrier_drop_db   = 35.0_f32;   // outermost bin within 35 dB of carrier
    let carrier_guard_bins = 3_usize;

    let (power_db, bin_hz) = power_spectrum(samples, fs);
    let n_bins = power_db.len();
    if n_bins < 3 { return bin_hz; }

    // Locate the carrier bin.
    let nominal_bin = ((carrier_hz / bin_hz).round() as usize).min(n_bins - 1);
    let cr = 3_usize;
    let c_lo = nominal_bin.saturating_sub(cr);
    let c_hi = (nominal_bin + cr).min(n_bins - 1);
    let carrier_bin = (c_lo..=c_hi)
        .max_by(|&a, &b| power_db[a].partial_cmp(&power_db[b]).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap_or(nominal_bin);

    let cutoff = power_db[carrier_bin] - carrier_drop_db;
    let search_bins = (search_hz / bin_hz).ceil() as usize;

    // Left edge: outermost LSB bin above cutoff.
    let lsb_lo = carrier_bin.saturating_sub(search_bins);
    let lsb_hi = carrier_bin.saturating_sub(carrier_guard_bins);
    let left_edge = if lsb_lo < lsb_hi {
        (lsb_lo..=lsb_hi).find(|&i| power_db[i] >= cutoff).unwrap_or(carrier_bin)
    } else {
        carrier_bin
    };

    // Right edge: outermost USB bin above cutoff.
    let usb_lo = (carrier_bin + carrier_guard_bins).min(n_bins - 1);
    let usb_hi = (carrier_bin + search_bins).min(n_bins - 1);
    let right_edge = if usb_lo < usb_hi {
        (usb_lo..=usb_hi).rfind(|&i| power_db[i] >= cutoff).unwrap_or(carrier_bin)
    } else {
        carrier_bin
    };

    ((right_edge.max(left_edge) - left_edge + 1) as f32) * bin_hz
}

