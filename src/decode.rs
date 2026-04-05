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
use orion_sdr::demodulate::psk31::{Bpsk31Demod, Bpsk31Decider, Qpsk31Demod, Qpsk31Decider};
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
const PSK31_BW_HZ: f32 = PSK31_BAUD * 2.0; // 62.5 Hz

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
const PSK31_MAX_ACCUM_SYMS: usize = 1200;

/// Fixed window size (samples) for AM DSB / Test Tone spectral analysis.
/// 4096 samples at 48 kHz = ~85 ms; bin resolution = 11.7 Hz.
const SPECTRUM_WINDOW_SAMPLES: usize = 4096;

/// Search half-width around the configured carrier (±200 Hz).
const SYNC_SEARCH_HZ: f32 = 200.0;

/// Minimum accumulated samples before attempting psk31_sync (64 symbols).
const SYNC_MIN_SYMS: usize = 64;

/// Persistent state for streaming PSK31 decode within one transmission.
/// Created after the first successful `psk31_sync`, destroyed at gap edge.
struct Psk31Stream {
    demod:   Bpsk31Demod,
    decider: Bpsk31Decider,
    vdec:    VaricodeDecoder,
    /// Sample offset into `iq_buf` where the next feed should start.
    fed_up_to: usize,
}

impl Psk31Stream {
    /// Feed new IQ samples through the demod → decider → varicode chain.
    /// Returns any newly decoded printable characters.
    fn feed(&mut self, iq: &[C32]) -> String {
        if iq.is_empty() { return String::new(); }

        let sps_est = iq.len(); // upper bound for output sizing
        let max_syms = sps_est / 32 + 4; // generous
        let mut soft = vec![0.0_f32; max_syms];
        let wr = self.demod.process(iq, &mut soft);
        soft.truncate(wr.out_written);

        let mut bits = vec![0_u8; soft.len()];
        let dr = self.decider.process(&soft, &mut bits);
        bits.truncate(dr.out_written);

        let mut text = String::new();
        for &b in &bits {
            self.vdec.push_bit(b);
            while let Some(ch) = self.vdec.pop_char() {
                if ch >= 0x20 && ch < 0x7f {
                    text.push(ch as char);
                }
            }
        }
        text
    }

    /// Flush the varicode decoder with trailing zeros to emit the last char.
    fn flush(&mut self) -> String {
        self.vdec.push_bit(0);
        self.vdec.push_bit(0);
        let mut text = String::new();
        while let Some(ch) = self.vdec.pop_char() {
            if ch >= 0x20 && ch < 0x7f {
                text.push(ch as char);
            }
        }
        text
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
                                if stream.fed_up_to < iq_buf.len() {
                                    let remaining = &iq_buf[stream.fed_up_to..];
                                    let text = stream.feed(remaining);
                                    if !text.is_empty() {

                                        let _ = self.tx.try_send(DecodeResult::Text(text));
                                    }
                                }
                                let tail = stream.flush();
                                if !tail.is_empty() {

                                    let _ = self.tx.try_send(DecodeResult::Text(tail));
                                }
                            } else if mode == DecodeMode::Qpsk31 && !iq_buf.is_empty() {
                                // QPSK31 fallback: batch decode at gap edge.
                                let (info, text) = self.decode_qpsk31(&iq_buf, carrier_hz, fs, sps);
                                let _ = self.tx.try_send(info);
                                if let Some(t) = text { let _ = self.tx.try_send(t); }
                            }
                            psk31_stream = None;
                            iq_buf.clear();
                        }
                    } else {
                        iq_buf.extend(samples.iter().map(|&s| C32::new(s, 0.0)));

                        // Try to establish the stream if we haven't yet.
                        if psk31_stream.is_none()
                            && iq_buf.len() >= sps * SYNC_MIN_SYMS
                            && mode == DecodeMode::Bpsk31
                        {
                            let base_hz = (carrier_hz - SYNC_SEARCH_HZ).max(0.0);
                            let max_hz  = carrier_hz + SYNC_SEARCH_HZ;
                            let results = psk31_sync(&iq_buf, fs, base_hz, max_hz, 4, 1.5, 256, 5);
                            if let Some((_found_hz, time_sym)) = best_sync(&results, carrier_hz) {
                                // Find the first full symbol of established carrier.
                                // Scan for the first sample whose instantaneous power
                                // exceeds a meaningful threshold (well above numerical
                                // noise, but below the carrier's RMS).  Then round UP
                                let scan_end = ((time_sym + 2) as usize * sps).min(iq_buf.len());
                                let start = iq_buf[..scan_end]
                                    .iter()
                                    .position(|c| c.re * c.re + c.im * c.im > 0.01)
                                    .unwrap_or(0);

                                let mut stream = Psk31Stream {
                                    demod:   Bpsk31Demod::new(fs, carrier_hz, 1.0),
                                    decider: Bpsk31Decider::new(),
                                    vdec:    VaricodeDecoder::new(),
                                    fed_up_to: start,
                                };
                                // Feed everything from onset to current end.
                                let text = stream.feed(&iq_buf[start..]);
                                if !text.is_empty() {
                                    let _ = self.tx.try_send(DecodeResult::Text(text));
                                }
                                stream.fed_up_to = iq_buf.len();
                                psk31_stream = Some(stream);
                            }
                        }

                        // Feed new samples to the running stream.
                        if let Some(ref mut stream) = psk31_stream {
                            let new_end = iq_buf.len();
                            if stream.fed_up_to < new_end {
                                let new_slice = &iq_buf[stream.fed_up_to..new_end];
                                let text = stream.feed(new_slice);
                                if !text.is_empty() {

                                    let _ = self.tx.try_send(DecodeResult::Text(text));
                                }
                                stream.fed_up_to = new_end;
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
                                stream.fed_up_to = stream.fed_up_to.saturating_sub(drop);
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

    #[cfg(test)]
    fn decode_bpsk31(
        &self,
        iq: &[C32],
        carrier_hz: f32,
        fs: f32,
        sps: usize,
    ) -> (DecodeResult, Option<DecodeResult>) {
        let base_hz = (carrier_hz - SYNC_SEARCH_HZ).max(0.0);
        let max_hz  = carrier_hz + SYNC_SEARCH_HZ;

        #[cfg(not(test))]
        let results = psk31_sync(iq, fs, base_hz, max_hz, 4, 1.5, 256, 5);
        #[cfg(test)]
        let results = psk31_sync(iq, fs, base_hz, max_hz, 4, 1.5, 256, 20);

        #[cfg(test)]
        {
            println!("  psk31_sync: {} candidates", results.len());
            for r in &results {
                println!("    cand: carrier_hz={:.1} time_sym={} score={:.2}", r.carrier_hz, r.time_sym, r.score);
            }
        }

        let real: Vec<f32> = iq.iter().map(|c| c.re).collect();

        let (found_hz, time_sym) = match best_sync(&results, carrier_hz) {
            Some(r) => r,
            None => {
                let snr = spectrum_snr_db(&real, fs, carrier_hz);
                return (DecodeResult::Info {
                    modulation: "BPSK31".to_owned(),
                    center_hz:  carrier_hz,
                    bw_hz:      PSK31_BW_HZ,
                    snr_db:     snr,
                }, None);
            }
        };

        let snr = spectrum_snr_db(&real, fs, found_hz);
        let info = DecodeResult::Info {
            modulation: "BPSK31".to_owned(),
            center_hz:  found_hz,
            bw_hz:      PSK31_BW_HZ,
            snr_db:     snr,
        };

        // Start demodulation at the true signal onset.  When time_sym=0 the
        // Scan for the first sample with meaningful carrier energy, then
        // round up to the next symbol boundary for clean Hann-window alignment.
        let scan_end = ((time_sym + 2) * sps).min(iq.len());
        let start = iq[..scan_end]
            .iter()
            .position(|c| c.re * c.re + c.im * c.im > 0.01)
            .unwrap_or(0);
        let max_syms = (iq.len() - start) / sps + 2;
        #[cfg(test)]
        println!("  bpsk31: found_hz={found_hz:.1} time_sym={time_sym} start={start} max_syms={max_syms} iq.len={}", iq.len());
        let mut soft = vec![0.0_f32; max_syms];
        let wr = Bpsk31Demod::new(fs, carrier_hz, 1.0).process(&iq[start..], &mut soft);
        soft.truncate(wr.out_written);

        let mut bits = vec![0_u8; soft.len()];
        let dr = Bpsk31Decider::new().process(&soft, &mut bits);
        bits.truncate(dr.out_written);

        let text = varicode_decode_bits(&bits);
        #[cfg(test)]
        println!("  bpsk31: bits.len={} text={:?}", bits.len(), &text[..text.len().min(40)]);
        if text.is_empty() {
            (info, None)
        } else {
            (info, Some(DecodeResult::Text(text)))
        }
    }

    fn decode_qpsk31(
        &self,
        iq: &[C32],
        carrier_hz: f32,
        fs: f32,
        sps: usize,
    ) -> (DecodeResult, Option<DecodeResult>) {
        let base_hz = (carrier_hz - SYNC_SEARCH_HZ).max(0.0);
        let max_hz  = carrier_hz + SYNC_SEARCH_HZ;

        let results = psk31_sync(iq, fs, base_hz, max_hz, 4, 3.0, 256, 5);

        let real: Vec<f32> = iq.iter().map(|c| c.re).collect();

        let (found_hz, time_sym) = match best_sync(&results, carrier_hz) {
            Some(r) => r,
            None => {
                let snr = spectrum_snr_db(&real, fs, carrier_hz);
                return (DecodeResult::Info {
                    modulation: "QPSK31".to_owned(),
                    center_hz:  carrier_hz,
                    bw_hz:      PSK31_BW_HZ,
                    snr_db:     snr,
                }, None);
            }
        };

        let snr = spectrum_snr_db(&real, fs, found_hz);
        let info = DecodeResult::Info {
            modulation: "QPSK31".to_owned(),
            center_hz:  found_hz,
            bw_hz:      PSK31_BW_HZ,
            snr_db:     snr,
        };

        // Start demodulation at the detected carrier onset (same rationale as BPSK31).
        let start = (time_sym * sps).min(iq.len());
        let max_soft = ((iq.len() - start) / sps + 2) * 2;
        let mut soft = vec![0.0_f32; max_soft];
        let wr = Qpsk31Demod::new(fs, carrier_hz, 1.0).process(&iq[start..], &mut soft);
        soft.truncate(wr.out_written);

        let mut decider = Qpsk31Decider::new();
        decider.process(&soft, &mut vec![]);
        let mut decoded_bits = Vec::new();
        decider.flush(&mut decoded_bits);

        let text = varicode_decode_bits(&decoded_bits);
        if text.is_empty() {
            (info, None)
        } else {
            (info, Some(DecodeResult::Text(text)))
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Pick the best sync result within 2 × PSK31_BAUD of `carrier_hz`.
/// Primary sort: earliest time_sym (more data available for the demodulator,
/// and the preamble is essential for varicode frame lock).
/// Secondary sort: smallest frequency offset (for tie-breaking among concurrent starts).
/// Returns `(carrier_hz, time_sym)`.
fn best_sync(results: &[Psk31SyncResult], carrier_hz: f32) -> Option<(f32, usize)> {
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

/// Push bits through a `VaricodeDecoder`, flushing with two trailing zeros,
/// and return a String of printable ASCII characters.
fn varicode_decode_bits(bits: &[u8]) -> String {
    let mut vdec = VaricodeDecoder::new();
    for &b in bits { vdec.push_bit(b); }
    // Two trailing zeros flush the last character (inter-character boundary).
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
fn spectrum_snr_db(samples: &[f32], fs: f32, carrier_hz: f32) -> f32 {
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
fn spectrum_bw_hz(samples: &[f32], fs: f32, carrier_hz: f32, _threshold_db: f32) -> f32 {
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

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use orion_sdr::modulate::AmDsbMod;
    use orion_sdr::core::AudioToIqChain;

    const FS: f32 = 48_000.0;
    const CARRIER_HZ: f32 = 12_000.0;

    /// Build an AM DSB signal from a slice of audio samples already at FS.
    /// Audio is normalised to peak = 0.9 (matching AmDsbSource behaviour) before
    /// modulation so sideband levels are consistent across test helpers.
    fn am_dsb_signal(audio: &[f32]) -> Vec<f32> {
        let peak = audio.iter().map(|&s| s.abs()).fold(0.0f32, f32::max);
        let scale = if peak > 1e-6 { 0.9 / peak } else { 1.0 };
        let norm: Vec<f32> = audio.iter().map(|&s| s * scale).collect();
        let block = AmDsbMod::new(FS, CARRIER_HZ, 1.0, 1.0);
        let mut chain = AudioToIqChain::new(block);
        let iq = chain.process_ref(&norm);
        iq.iter().map(|c| c.re).collect()
    }

    /// Generate a single sinusoid at `freq_hz` for `n` samples.
    fn sine(freq_hz: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * freq_hz * i as f32 / FS).sin())
            .collect()
    }

    /// Generate bandlimited noise covering `lo_hz`..`hi_hz` by summing sines.
    /// Uses a fixed set of harmonics spread across the band.
    fn band_noise(lo_hz: f32, hi_hz: f32, n: usize) -> Vec<f32> {
        // 20 harmonics spread linearly across the band
        let steps = 20_usize;
        let step = (hi_hz - lo_hz) / steps as f32;
        let mut out = vec![0.0f32; n];
        for k in 0..steps {
            let f = lo_hz + k as f32 * step;
            // vary phase so harmonics don't all peak at the same point
            let phase = k as f32 * 0.7;
            for i in 0..n {
                out[i] += (2.0 * std::f32::consts::PI * f * i as f32 / FS + phase).sin();
            }
        }
        // normalise to peak ≈ 0.5
        let peak = out.iter().map(|&s| s.abs()).fold(0.0f32, f32::max).max(1e-6);
        out.iter_mut().for_each(|s| *s *= 0.5 / peak);
        out
    }

    // ── Viewer simulation ─────────────────────────────────────────────────────

    /// Simulate the full decode worker loop for AM DSB: feed samples from
    /// AmDsbSource in the same block size the audio callback uses (1024), apply
    /// gap detection, rolling window accumulation, EMA smoothing, and print
    /// the BW that would be displayed at each decode window boundary.
    #[test]
    fn simulate_am_dsb_viewer() {
        use crate::source::{AmDsbSource, SignalSource, BuiltinAudio, load_builtin};
        const BLOCK: usize = 1024;   // audio callback block size

        for &audio_kind in &[BuiltinAudio::Morse, BuiltinAudio::Voice] {
            #[cfg(feature = "bw-sim")]
            let label = match audio_kind { BuiltinAudio::Morse => "Morse", BuiltinAudio::Voice => "Voice" };
            let (audio, audio_rate) = load_builtin(audio_kind);
            let audio_secs = audio.len() as f32 / audio_rate;

            let mut src = AmDsbSource::new(
                audio, audio_rate, CARRIER_HZ, 1.0,
                /*loop_gap_secs=*/ 2.0,
                /*noise_amp=*/ 0.05,
                /*msg_repeat=*/ 1,
                FS,
            );

            // Replicate DecodeWorker state
            let mut iq_buf: Vec<C32> = Vec::new();
            let mut smoothed_bw = 0.0f32;
            let total_out = ((audio_secs + 2.0) * FS) as usize;
            #[cfg(feature = "bw-sim")]
            let mut t_secs = 0.0f32;
            #[cfg(feature = "bw-sim")]
            let mut window_count = 0;

            #[cfg(feature = "bw-sim")]
            println!("\n── {label} ({audio_secs:.1}s audio, {total_out} output samples) ──");

            for block_start in (0..total_out).step_by(BLOCK) {
                let n = BLOCK.min(total_out - block_start);
                let samples = src.next_samples(n);
                #[cfg(feature = "bw-sim")]
                { t_secs += n as f32 / FS; }

                // Gap detection
                if orion_sdr::util::rms(&samples) < SIGNAL_THRESHOLD {
                    iq_buf.clear();
                    continue;
                }

                iq_buf.extend(samples.iter().map(|&s| C32::new(s, 0.0)));
                if iq_buf.len() < SPECTRUM_WINDOW_SAMPLES { continue; }

                // Take one window, keep second half for overlap
                let win: Vec<f32> = iq_buf[..SPECTRUM_WINDOW_SAMPLES].iter().map(|c| c.re).collect();
                iq_buf.drain(..SPECTRUM_WINDOW_SAMPLES / 2);

                let raw_bw = spectrum_bw_hz(&win, FS, CARRIER_HZ, 7.0);
                if smoothed_bw == 0.0 { smoothed_bw = raw_bw; } else { smoothed_bw = 0.3 * raw_bw + 0.7 * smoothed_bw; }

                #[cfg(feature = "bw-sim")]
                { window_count += 1; }
                #[cfg(feature = "bw-sim")]
                { let snr = spectrum_snr_db(&win, FS, CARRIER_HZ);
                  println!("  t={t_secs:5.2}s  raw_bw={raw_bw:7.1}  smoothed={smoothed_bw:7.1}  snr={snr:.1}dB"); }
            }
            #[cfg(feature = "bw-sim")]
            println!("  ({window_count} decode windows)");
        }
    }

    // ── Morse-like: single 800 Hz tone ────────────────────────────────────────

    /// AM DSB of a single 800 Hz tone: sidebands at ±800 Hz from carrier.
    /// Expected BW ≈ 2 × 800 Hz = 1600 Hz.  Tolerance: 800–2400 Hz.
    #[test]
    fn bw_morse_tone() {
        let audio = sine(800.0, SPECTRUM_WINDOW_SAMPLES);
        let signal = am_dsb_signal(&audio);
        let bw = spectrum_bw_hz(&signal, FS, CARRIER_HZ, 7.0);
        println!("Morse-tone BW: {bw:.1} Hz");
        assert!(
            bw >= 800.0 && bw <= 2_400.0,
            "Morse-tone BW {bw:.0} Hz not in [800, 2400]"
        );
    }

    // ── Voice-like: broadband 300–3000 Hz ─────────────────────────────────────

    /// AM DSB of broadband voice audio (300–3000 Hz): sidebands span ±300–3000 Hz.
    /// Expected BW ≈ 2 × 3000 Hz = 6000 Hz.  Tolerance: 3000–7000 Hz.
    #[test]
    fn bw_voice_audio() {
        let audio = band_noise(300.0, 3_000.0, SPECTRUM_WINDOW_SAMPLES);
        let signal = am_dsb_signal(&audio);
        let bw = spectrum_bw_hz(&signal, FS, CARRIER_HZ, 7.0);
        println!("Voice BW: {bw:.1} Hz");
        assert!(
            bw >= 3_000.0 && bw <= 7_000.0,
            "Voice BW {bw:.0} Hz not in [3000, 7000]"
        );
    }

    // ── Built-in audio sources ────────────────────────────────────────────────

    /// AM DSB of the built-in Morse WAV: audio is CW bursts at ~800 Hz.
    /// Sidebands sit at carrier ± 800 Hz.  Expected BW: 800–2400 Hz.
    #[test]
    fn bw_builtin_morse() {
        let (audio, audio_rate) = crate::source::load_builtin(crate::source::BuiltinAudio::Morse);
        let bw = measure_bw_via_source(audio, audio_rate);
        println!("Built-in Morse BW: {bw:.1} Hz");
        assert!(
            bw >= 800.0 && bw <= 2_400.0,
            "Morse BW {bw:.0} Hz not in [800, 2400]"
        );
    }

    /// AM DSB of the built-in Voice WAV: broadband speech up to ~4 kHz.
    /// Sidebands span carrier ± audio BW.  Expected BW: 2000–8000 Hz.
    #[test]
    fn bw_builtin_voice() {
        let (audio, audio_rate) = crate::source::load_builtin(crate::source::BuiltinAudio::Voice);
        let bw = measure_bw_via_source(audio, audio_rate);
        println!("Built-in Voice BW: {bw:.1} Hz");
        assert!(
            bw >= 2_000.0 && bw <= 8_000.0,
            "Voice BW {bw:.0} Hz not in [2000, 8000]"
        );
    }

    // ── Stability: BW doesn't blow up when signal fades ───────────────────────

    /// When audio is near-silence (gap), BW should be small (near carrier only),
    /// not artificially inflated by noise.  Upper bound: 1000 Hz.
    #[test]
    fn bw_silence_stays_small() {
        // Very low amplitude audio — simulates carrier-off transition
        let audio: Vec<f32> = vec![0.001f32; SPECTRUM_WINDOW_SAMPLES];
        let signal = am_dsb_signal(&audio);
        let bw = spectrum_bw_hz(&signal, FS, CARRIER_HZ, 7.0);
        println!("Silence BW: {bw:.1} Hz");
        assert!(
            bw <= 1_000.0,
            "Silence BW {bw:.0} Hz should be ≤ 1000 Hz (got inflated reading)"
        );
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    /// Modulate audio via AmDsbSource (handles native-rate → FS resampling),
    /// measure BW on one SPECTRUM_WINDOW_SAMPLES window, and return the result.
    /// Returns the maximum BW observed across active windows in the recording.
    /// Using the max rather than median because the decode bar displays the most
    /// recently active window, which tends to be near the peak of the signal.
    fn measure_bw_via_source(audio: Vec<f32>, audio_rate: f32) -> f32 {
        use crate::source::{AmDsbSource, SignalSource};
        let audio_secs = audio.len() as f32 / audio_rate;
        let total = ((audio_secs * FS) as usize).max(SPECTRUM_WINDOW_SAMPLES);
        let mut src = AmDsbSource::new(
            audio, audio_rate, CARRIER_HZ, 1.0, 0.0, 0.0, 1, FS,
        );
        let signal = src.next_samples(total);
        signal
            .chunks_exact(SPECTRUM_WINDOW_SAMPLES)
            .filter(|w| {
                let rms = (w.iter().map(|&s| s*s).sum::<f32>() / w.len() as f32).sqrt();
                rms >= SIGNAL_THRESHOLD
            })
            .map(|w| spectrum_bw_hz(w, FS, CARRIER_HZ, 7.0))
            .fold(0.0f32, f32::max)
    }

    #[test]
    fn psk31_decode_yields_text() {
        use crate::source::{Psk31Source, Psk31Mode, SignalSource};
        use orion_sdr::modulate::psk31::psk31_sps;

        const MSG: &str = "CQ CQ CQ DE N0GNR";
        let sps = psk31_sps(FS);

        // Generate a complete transmission (no gap — caller supplies all signal).
        let mut src = Psk31Source::new(
            CARRIER_HZ, 0.0, 0.0, Psk31Mode::Bpsk31,
            MSG.to_owned(), 3, FS,
        );
        // Render the full frame: source stops producing signal after the frame.
        let total = PSK31_MAX_ACCUM_SYMS * sps;
        let samples: Vec<f32> = src.next_samples(total);
        println!("rendered {} samples = {:.1}s, sps={sps}", samples.len(), samples.len() as f32 / FS);

        let iq: Vec<C32> = samples.iter().map(|&s| C32::new(s, 0.0)).collect();
        let worker = DecodeWorker::new(
            std::sync::Arc::new(std::sync::Mutex::new(DecodeConfig::new(FS))),
            std::sync::mpsc::sync_channel(1).1,
            std::sync::mpsc::sync_channel(1).0,
        );

        // Decode the whole frame as one shot (mirrors the new accumulate-then-decode path).
        let (info, text) = worker.decode_bpsk31(&iq, CARRIER_HZ, FS, sps);
        if let DecodeResult::Info { modulation, center_hz, snr_db, .. } = &info {
            println!("Info: {modulation} ctr={center_hz:.0} snr={snr_db:.1}");
        }
        let found_text = matches!(text, Some(DecodeResult::Text(ref t)) if {
            println!("Text: {t:?}");
            !t.is_empty()
        });
        assert!(found_text, "expected non-empty Text result from full-frame decode");
    }

    /// Simulate the Dt mode ticker as seen by the viewer: feed Psk31Source blocks
    /// through gap detection and the rolling-window decode pipeline, printing each
    /// event the ticker would receive (with wall-clock timestamp) so we can inspect
    /// what text accumulates and when.
    ///
    /// Parameters match the viewer defaults:
    ///   BLOCK = 800 samples (~16.7 ms at 48 kHz, matching SAMPLES_PER_FRAME)
    ///   loop_gap_secs = 10 s (PSK31_DEFAULT_LOOP_GAP_SECS)
    ///   msg_repeat    = 5   (PSK31_DEFAULT_REPEAT)
    ///   noise_amp     = 0   (clean signal for clarity)
    ///   Two full source loops so we can see repeat behaviour.
    #[test]
    fn simulate_dt_ticker() {
        use crate::source::{Psk31Source, Psk31Mode, SignalSource};
        use orion_sdr::modulate::psk31::psk31_sps;
        use orion_sdr::util::rms;

        const MSG:       &str  = "CQ CQ CQ DE N0GNR";
        const REPEAT:    usize = 5;
        const BLOCK:     usize = 800;   // SAMPLES_PER_FRAME
        const LOOP_GAP:  f32   = 10.0;  // PSK31_DEFAULT_LOOP_GAP_SECS
        const LOOPS:     usize = 2;     // how many full source loops to simulate

        let sps = psk31_sps(FS);

        let mut src = Psk31Source::new(
            CARRIER_HZ, LOOP_GAP, 0.0, Psk31Mode::Bpsk31,
            MSG.to_owned(), REPEAT, FS,
        );

        let worker = DecodeWorker::new(
            std::sync::Arc::new(std::sync::Mutex::new(DecodeConfig::new(FS))),
            std::sync::mpsc::sync_channel(1).1,
            std::sync::mpsc::sync_channel(1).0,
        );

        // Replicate the new decode worker: accumulate during signal, decode once at gap.
        let mut iq_buf:     Vec<C32>     = Vec::new();
        let mut ticker:     DecodeTicker = DecodeTicker::new();
        let mut t_secs:     f32          = 0.0;
        let mut was_silent: bool         = true;
        let max_accum = PSK31_MAX_ACCUM_SYMS * sps;

        // Estimate total samples for LOOPS complete source cycles.
        let text_bytes         = (MSG.len() * REPEAT + (REPEAT - 1)) as f32;
        let approx_text_syms   = (text_bytes * 11.0) as usize;
        let approx_signal_syms = 64 + approx_text_syms + 32;
        let approx_signal_secs = approx_signal_syms as f32 / 31.25;
        let approx_loop_secs   = approx_signal_secs + LOOP_GAP;
        let total_samples      = ((approx_loop_secs * LOOPS as f32 + 2.0) * FS) as usize;

        println!("── Dt ticker simulation ──────────────────────────────────────────");
        println!("  message: {MSG:?} × {REPEAT}, carrier={CARRIER_HZ:.0} Hz, fs={FS:.0}");
        println!("  max_accum={max_accum} samples ({:.1}s), block={BLOCK}",
            PSK31_MAX_ACCUM_SYMS as f32 / 31.25);
        println!("  est. signal frame ≈ {approx_signal_secs:.1}s, loop gap={LOOP_GAP:.0}s");
        println!("  simulating {total_samples} samples ({:.1}s)\n", total_samples as f32 / FS);

        for block_start in (0..total_samples).step_by(BLOCK) {
            let n = BLOCK.min(total_samples - block_start);
            let samples = src.next_samples(n);
            t_secs += n as f32 / FS;

            let is_silent = rms(&samples) < SIGNAL_THRESHOLD;

            if is_silent {
                if !was_silent && !iq_buf.is_empty() {
                    let buf = std::mem::take(&mut iq_buf);
                    println!("t={t_secs:7.2}s  [GAP: decode {} samples = {:.1}s]",
                        buf.len(), buf.len() as f32 / FS);
                    let (info, text) = worker.decode_bpsk31(&buf, CARRIER_HZ, FS, sps);
                    if let DecodeResult::Info { ref modulation, center_hz, snr_db, .. } = info {
                        println!("  Info: {modulation} ctr={center_hz:.1}Hz snr={snr_db:.1}dB");
                    }
                    ticker.push_result(info);
                    match text {
                        Some(DecodeResult::Text(ref s)) => {
                            println!("  Text: {:?}", s);
                            ticker.push_result(DecodeResult::Text(s.clone()));
                        }
                        Some(other) => println!("  {:?}", other),
                        None        => println!("  (no text)"),
                    }
                }
                ticker.push_result(DecodeResult::Gap);
                was_silent = true;
            } else {
                iq_buf.extend(samples.iter().map(|&s| C32::new(s, 0.0)));
                was_silent = false;

                if iq_buf.len() >= max_accum {
                    let buf = std::mem::take(&mut iq_buf);
                    println!("t={t_secs:7.2}s  [MAX_ACCUM flush: {} samples]", buf.len());
                    let (info, text) = worker.decode_bpsk31(&buf, CARRIER_HZ, FS, sps);
                    if let DecodeResult::Info { ref modulation, center_hz, snr_db, .. } = info {
                        println!("  Info: {modulation} ctr={center_hz:.1}Hz snr={snr_db:.1}dB");
                    }
                    ticker.push_result(info);
                    if let Some(DecodeResult::Text(ref s)) = text {
                        println!("  Text: {:?}", s);
                        ticker.push_result(DecodeResult::Text(s.clone()));
                    }
                }
            }

            ticker.tick(n as f32 / FS);
        }

        println!("\n── Final ticker buffer ───────────────────────────────────────────");
        println!("  visible: {:?}", ticker.visible);
        println!("  visible.len() = {}", ticker.visible.len());
    }

    /// Simulate the streaming PSK31 decode path from the decode worker across
    /// 3 loops, printing every Text result with timestamps to diagnose errors.
    #[test]
    fn simulate_streaming_decode() {
        use crate::source::{Psk31Source, Psk31Mode, SignalSource};
        use orion_sdr::modulate::psk31::psk31_sps;
        use orion_sdr::util::rms;

        const MSG:       &str  = "CQ CQ CQ DE N0GNR";
        const REPEAT:    usize = 5;
        const BLOCK:     usize = 800;
        const LOOP_GAP:  f32   = 15.0;
        const LOOPS:     usize = 5;

        let sps = psk31_sps(FS);
        let carrier_hz = CARRIER_HZ;

        let mut src = Psk31Source::new(
            carrier_hz, LOOP_GAP, 0.0, Psk31Mode::Bpsk31,
            MSG.to_owned(), REPEAT, FS,
        );

        let mut iq_buf: Vec<C32> = Vec::new();
        let mut stream: Option<Psk31Stream> = None;
        let mut was_signal = false;
        let mut t_secs = 0.0_f32;
        let mut all_text = String::new();
        let mut loop_count = 0_u32;

        let text_bytes = (MSG.len() * REPEAT + (REPEAT - 1)) as f32;
        let approx_signal_secs = (64.0 + text_bytes * 11.0 + 32.0) / 31.25;
        let total_samples = ((approx_signal_secs + LOOP_GAP) * LOOPS as f32 + 2.0) * FS;

        println!("── Streaming decode simulation ({LOOPS} loops) ─────────────────────");
        println!("  sps={sps}, carrier={carrier_hz:.0}, gap={LOOP_GAP:.0}s\n");

        for _ in (0..total_samples as usize).step_by(BLOCK) {
            let n = BLOCK;
            let samples = src.next_samples(n);
            t_secs += n as f32 / FS;

            let is_signal = rms(&samples) >= SIGNAL_THRESHOLD;
            let gap_edge = !is_signal && was_signal;
            let signal_edge = is_signal && !was_signal;
            was_signal = is_signal;

            if signal_edge {
                loop_count += 1;
                println!("t={t_secs:7.2}s  ── signal ON (loop {loop_count}) ──");
            }

            if gap_edge {
                println!("t={t_secs:7.2}s  ── gap edge ──");
                // Flush stream
                if let Some(ref mut s) = stream {
                    if s.fed_up_to < iq_buf.len() {
                        let remaining = &iq_buf[s.fed_up_to..];
                        let energy: f32 = remaining.iter()
                            .map(|c| c.re * c.re + c.im * c.im).sum::<f32>()
                            / remaining.len().max(1) as f32;
                        if energy > 1e-6 {
                            let text = s.feed(remaining);
                            if !text.is_empty() {
                                println!("  flush-feed: {:?}", &text);
                                all_text.push_str(&text);
                            }
                        }
                    }
                    let tail = s.flush();
                    if !tail.is_empty() {
                        println!("  flush-tail: {:?}", &tail);
                        all_text.push_str(&tail);
                    }
                }
                stream = None;
                iq_buf.clear();
                println!("  all_text so far ({} chars): {:?}",
                    all_text.len(), &all_text[all_text.len().saturating_sub(60)..]);
                continue;
            }

            if !is_signal { continue; }

            iq_buf.extend(samples.iter().map(|&s| C32::new(s, 0.0)));

            // Try to establish stream
            if stream.is_none() && iq_buf.len() >= sps * SYNC_MIN_SYMS {
                let base_hz = (carrier_hz - SYNC_SEARCH_HZ).max(0.0);
                let max_hz  = carrier_hz + SYNC_SEARCH_HZ;
                let results = psk31_sync(&iq_buf, FS, base_hz, max_hz, 4, 1.5, 256, 5);
                if let Some((_found_hz, time_sym)) = best_sync(&results, carrier_hz) {
                    let scan_end = ((time_sym + 2) as usize * sps).min(iq_buf.len());
                    let onset = iq_buf[..scan_end]
                        .iter()
                        .position(|c| c.re * c.re + c.im * c.im > 0.01)
                        .unwrap_or(0);
                    let start = onset;
                    println!("t={t_secs:7.2}s  sync: time_sym={time_sym} start={start} iq_buf.len={}",
                        iq_buf.len());
                    let mut s = Psk31Stream {
                        demod:   Bpsk31Demod::new(FS, carrier_hz, 1.0),
                        decider: Bpsk31Decider::new(),
                        vdec:    VaricodeDecoder::new(),
                        fed_up_to: start,
                    };
                    let text = s.feed(&iq_buf[start..]);
                    if !text.is_empty() {
                        println!("  initial: {:?}", &text);
                        all_text.push_str(&text);
                    }
                    s.fed_up_to = iq_buf.len();
                    stream = Some(s);
                }
            }

            // Feed new samples
            if let Some(ref mut s) = stream {
                let new_end = iq_buf.len();
                if s.fed_up_to < new_end {
                    let new_slice = &iq_buf[s.fed_up_to..new_end];
                    let block_energy: f32 = new_slice.iter()
                        .map(|c| c.re * c.re + c.im * c.im).sum::<f32>()
                        / new_slice.len() as f32;
                    if block_energy > 1e-6 {
                        let text = s.feed(new_slice);
                        if !text.is_empty() {
                            println!("t={t_secs:7.2}s  text: {:?}", &text);
                            all_text.push_str(&text);
                        }
                    } else {
                        println!("t={t_secs:7.2}s  SKIP low-energy block ({block_energy:.2e})");
                    }
                    s.fed_up_to = new_end;
                }
            }
        }

        println!("\n── All decoded text ({} chars) ──", all_text.len());
        println!("{:?}", all_text);

        // Verify: should contain the message repeated without errors.
        let expected_single = "CQ CQ CQ DE N0GNR";
        let error_count = all_text.chars()
            .filter(|c| !expected_single.contains(*c) && *c != ' ')
            .count();
        println!("Non-message chars: {error_count}");
        assert!(error_count == 0, "found {error_count} unexpected characters in decoded text");
    }

    /// Test streaming decode with short messages (1 char, 1 repeat) and
    /// varying message lengths to verify the decode pipeline isn't tied
    /// to a particular message size.
    #[test]
    fn streaming_decode_short_messages() {
        use crate::source::{Psk31Source, Psk31Mode, SignalSource};
        use orion_sdr::modulate::psk31::psk31_sps;
        use orion_sdr::util::rms;

        let sps = psk31_sps(FS);
        let carrier_hz = CARRIER_HZ;

        let cases: &[(&str, usize, usize)] = &[
            ("A",                  1, 3),  // 1 char, 1 repeat, 3 loops
            ("AB",                 1, 3),  // 2 chars, 1 repeat
            ("CQ DE N0GNR",       1, 3),  // medium, 1 repeat
            ("CQ",                5, 3),  // short, 5 repeats
            ("CQ CQ CQ DE N0GNR", 5, 2),  // default, 2 loops
        ];

        for &(msg, repeat, loops) in cases {
            println!("\n── msg={msg:?} repeat={repeat} loops={loops} ──");

            let mut src = Psk31Source::new(
                carrier_hz, 5.0, 0.0, Psk31Mode::Bpsk31,
                msg.to_owned(), repeat, FS,
            );

            let mut iq_buf: Vec<C32> = Vec::new();
            let mut stream: Option<Psk31Stream> = None;
            let mut was_signal = false;
            let mut all_text = String::new();

            // Estimate duration
            let text_bytes = (msg.len() * repeat + (repeat.saturating_sub(1))) as f32;
            let approx_signal_secs = (64.0 + text_bytes * 11.0 + 32.0) / 31.25;
            let total_samples = ((approx_signal_secs + 5.0) * loops as f32 + 2.0) * FS;

            for _ in (0..total_samples as usize).step_by(800) {
                let samples = src.next_samples(800);

                let is_signal = rms(&samples) >= SIGNAL_THRESHOLD;
                let gap_edge = !is_signal && was_signal;
                was_signal = is_signal;

                if gap_edge {
                    if let Some(ref mut s) = stream {
                        if s.fed_up_to < iq_buf.len() {
                            let text = s.feed(&iq_buf[s.fed_up_to..]);
                            if !text.is_empty() { all_text.push_str(&text); }
                        }
                        let tail = s.flush();
                        if !tail.is_empty() { all_text.push_str(&tail); }
                    }
                    stream = None;
                    iq_buf.clear();
                    continue;
                }

                if !is_signal { continue; }

                iq_buf.extend(samples.iter().map(|&s| C32::new(s, 0.0)));

                if stream.is_none() && iq_buf.len() >= sps * SYNC_MIN_SYMS {
                    let base_hz = (carrier_hz - SYNC_SEARCH_HZ).max(0.0);
                    let max_hz  = carrier_hz + SYNC_SEARCH_HZ;
                    let results = psk31_sync(&iq_buf, FS, base_hz, max_hz, 4, 1.5, 256, 5);
                    if let Some((_found_hz, time_sym)) = best_sync(&results, carrier_hz) {
                        let scan_end = ((time_sym + 2) * sps).min(iq_buf.len());
                        let start = iq_buf[..scan_end]
                            .iter()
                            .position(|c| c.re * c.re + c.im * c.im > 0.01)
                            .unwrap_or(0);
                        let mut s = Psk31Stream {
                            demod:   Bpsk31Demod::new(FS, carrier_hz, 1.0),
                            decider: Bpsk31Decider::new(),
                            vdec:    VaricodeDecoder::new(),
                            fed_up_to: start,
                        };
                        let text = s.feed(&iq_buf[start..]);
                        if !text.is_empty() { all_text.push_str(&text); }
                        s.fed_up_to = iq_buf.len();
                        stream = Some(s);
                    }
                }

                if let Some(ref mut s) = stream {
                    if s.fed_up_to < iq_buf.len() {
                        let text = s.feed(&iq_buf[s.fed_up_to..]);
                        if !text.is_empty() { all_text.push_str(&text); }
                        s.fed_up_to = iq_buf.len();
                    }
                }
            }

            println!("  decoded: {:?}", &all_text);
            // Verify only chars from the message appear.
            let error_count = all_text.chars()
                .filter(|c| !msg.contains(*c) && *c != ' ')
                .count();
            println!("  non-message chars: {error_count}");
            assert!(error_count == 0,
                "msg={msg:?} repeat={repeat}: found {error_count} unexpected chars in {:?}",
                &all_text);
            // Verify we got at least some text.
            assert!(!all_text.is_empty(),
                "msg={msg:?} repeat={repeat}: no text decoded at all");
        }
    }

    /// Full modulate → demodulate → varicode roundtrip for all printable ASCII.
    /// Generates a PSK31 signal containing every printable character (32–126),
    /// runs the streaming decode pipeline, and verifies every character is
    /// recovered without errors.
    #[test]
    fn streaming_decode_all_printable_ascii() {
        use crate::source::{Psk31Source, Psk31Mode, SignalSource};
        use orion_sdr::modulate::psk31::psk31_sps;
        use orion_sdr::util::rms;

        let sps = psk31_sps(FS);
        let carrier_hz = CARRIER_HZ;

        // Build a message containing every printable ASCII character (32–126).
        let msg: String = (32u8..127u8).map(|b| b as char).collect();
        println!("msg ({} chars): {:?}", msg.len(), &msg);

        let mut src = Psk31Source::new(
            carrier_hz, 5.0, 0.0, Psk31Mode::Bpsk31,
            msg.clone(), 1, FS,
        );

        let mut iq_buf: Vec<C32> = Vec::new();
        let mut stream: Option<Psk31Stream> = None;
        let mut was_signal = false;
        let mut all_text = String::new();

        // Generous duration: 95 chars × ~11 syms × 1536 sps + preamble/postamble + gap.
        let total_samples = ((95.0 * 11.0 + 96.0) / 31.25 + 10.0) * FS;

        for _ in (0..total_samples as usize).step_by(800) {
            let samples = src.next_samples(800);

            let is_signal = rms(&samples) >= SIGNAL_THRESHOLD;
            let gap_edge = !is_signal && was_signal;
            was_signal = is_signal;

            if gap_edge {
                if let Some(ref mut s) = stream {
                    if s.fed_up_to < iq_buf.len() {
                        let text = s.feed(&iq_buf[s.fed_up_to..]);
                        if !text.is_empty() { all_text.push_str(&text); }
                    }
                    let tail = s.flush();
                    if !tail.is_empty() { all_text.push_str(&tail); }
                }
                stream = None;
                iq_buf.clear();
                continue;
            }

            if !is_signal { continue; }

            iq_buf.extend(samples.iter().map(|&s| C32::new(s, 0.0)));

            if stream.is_none() && iq_buf.len() >= sps * SYNC_MIN_SYMS {
                let base_hz = (carrier_hz - SYNC_SEARCH_HZ).max(0.0);
                let max_hz  = carrier_hz + SYNC_SEARCH_HZ;
                let results = psk31_sync(&iq_buf, FS, base_hz, max_hz, 4, 1.5, 256, 5);
                if let Some((_found_hz, time_sym)) = best_sync(&results, carrier_hz) {
                    let scan_end = ((time_sym + 2) * sps).min(iq_buf.len());
                    let start = iq_buf[..scan_end]
                        .iter()
                        .position(|c| c.re * c.re + c.im * c.im > 0.01)
                        .unwrap_or(0);
                    let mut s = Psk31Stream {
                        demod:   Bpsk31Demod::new(FS, carrier_hz, 1.0),
                        decider: Bpsk31Decider::new(),
                        vdec:    VaricodeDecoder::new(),
                        fed_up_to: start,
                    };
                    let text = s.feed(&iq_buf[start..]);
                    if !text.is_empty() { all_text.push_str(&text); }
                    s.fed_up_to = iq_buf.len();
                    stream = Some(s);
                }
            }

            if let Some(ref mut s) = stream {
                if s.fed_up_to < iq_buf.len() {
                    let text = s.feed(&iq_buf[s.fed_up_to..]);
                    if !text.is_empty() { all_text.push_str(&text); }
                    s.fed_up_to = iq_buf.len();
                }
            }
        }

        println!("decoded ({} chars): {:?}", all_text.len(), &all_text);

        // Verify: the first 95 characters must match the full printable ASCII set.
        // (The source loops, so extra characters from loop 2 may follow — ignore them.)
        let expected: Vec<char> = (32u8..127u8).map(|b| b as char).collect();
        let got: Vec<char> = all_text.chars().collect();
        assert!(got.len() >= expected.len(),
            "too few chars: expected at least {}, got {}", expected.len(), got.len());
        for (i, (g, e)) in got.iter().zip(expected.iter()).enumerate() {
            assert_eq!(g, e, "mismatch at index {}: expected {:?} ({}), got {:?} ({})",
                i, e, *e as u32, g, *g as u32);
        }
    }
}
