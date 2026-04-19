// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! PSK31 (BPSK31 / QPSK31) decode state and processing.

use std::sync::mpsc::SyncSender;

use num_complex::Complex32 as C32;
use orion_sdr::modulate::psk31::{PSK31_BAUD, psk31_sps};
use orion_sdr::sync::psk31_sync::psk31_sync;

use super::{DecodeMode, DecodeResult, Psk31Stream, SPECTRUM_WINDOW_SAMPLES};
pub use orion_sdr::util::{PSK31_BW_HZ, best_sync, spectrum_snr_db};

/// Maximum PSK31 accumulation buffer: caps memory and limits decode latency.
/// 1200 symbols ≈ 38 s at 31.25 baud — comfortably larger than the default
/// transmission (msg×5 + preamble + postamble ≈ 1100 symbols) so a full frame
/// is never truncated in normal use.  If the carrier runs longer the buffer is
/// decoded and flushed at this boundary without waiting for a gap.
pub const PSK31_MAX_ACCUM_SYMS: usize = 1200;

/// Search half-width around the configured carrier (±200 Hz).
pub const SYNC_SEARCH_HZ: f32 = 200.0;

/// Minimum accumulated samples before attempting psk31_sync (64 symbols).
pub const SYNC_MIN_SYMS: usize = 64;

/// Info throttle interval (samples).  Shared with other modes but defined here
/// because PSK31 was the first user.
pub const INFO_INTERVAL: usize = 48_000; // 1 s at 48 kHz

pub struct Psk31State {
    pub iq_buf: Vec<C32>,
    pub stream: Option<Psk31Stream>,
    pub smoothed_snr_db: f32,
    pub info_counter: usize,
}

impl Default for Psk31State {
    fn default() -> Self {
        Self {
            iq_buf: Vec::new(),
            stream: None,
            smoothed_snr_db: 0.0,
            info_counter: 0,
        }
    }
}

impl Psk31State {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        self.iq_buf.clear();
        self.stream = None;
        self.smoothed_snr_db = 0.0;
        self.info_counter = 0;
    }

    #[allow(clippy::too_many_arguments)]
    pub fn process(
        &mut self,
        samples: &[f32],
        is_signal: bool,
        gap_edge: bool,
        mode: DecodeMode,
        carrier_hz: f32,
        fs: f32,
        tx: &SyncSender<DecodeResult>,
    ) {
        let sps = psk31_sps(fs);
        let max_accum = PSK31_MAX_ACCUM_SYMS * sps;
        let mode_label = if mode == DecodeMode::Bpsk31 {
            "BPSK31"
        } else {
            "QPSK31"
        };

        if !is_signal {
            if gap_edge {
                self.info_counter = 0;
                self.smoothed_snr_db = 0.0;
                // Send zeroed Info so the Di bar clears immediately.
                let _ = tx.try_send(DecodeResult::Info {
                    modulation: mode_label.to_owned(),
                    center_hz: carrier_hz,
                    bw_hz: 0.0,
                    snr_db: 0.0,
                });
                if let Some(ref mut stream) = self.stream {
                    // Flush remaining samples + Viterbi tail + varicode.
                    if stream.fed_up_to() < self.iq_buf.len() {
                        let text = stream.feed(&self.iq_buf[stream.fed_up_to()..]);
                        if !text.is_empty() {
                            let _ = tx.try_send(DecodeResult::Text(text));
                        }
                    }
                    let tail = stream.flush();
                    if !tail.is_empty() {
                        let _ = tx.try_send(DecodeResult::Text(tail));
                    }
                }
                self.stream = None;
                self.iq_buf.clear();
            }
        } else {
            self.iq_buf
                .extend(samples.iter().map(|&s| C32::new(s, 0.0)));

            // Try to establish the stream if we haven't yet.
            if self.stream.is_none() && self.iq_buf.len() >= sps * SYNC_MIN_SYMS {
                let margin = if mode == DecodeMode::Bpsk31 { 1.5 } else { 3.0 };
                let base_hz = (carrier_hz - SYNC_SEARCH_HZ).max(0.0);
                let max_hz = carrier_hz + SYNC_SEARCH_HZ;
                let results = psk31_sync(&self.iq_buf, fs, base_hz, max_hz, 4, margin, 256, 5);
                if let Some((_found_hz, time_sym)) = best_sync(&results, carrier_hz, PSK31_BAUD) {
                    let scan_end = ((time_sym + 2) * sps).min(self.iq_buf.len());
                    let onset = self.iq_buf[..scan_end]
                        .iter()
                        .position(|c| c.re * c.re + c.im * c.im > 0.01)
                        .unwrap_or(0);
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
                    let text = stream.feed(&self.iq_buf[start..]);
                    if !text.is_empty() {
                        let _ = tx.try_send(DecodeResult::Text(text));
                    }
                    stream.set_fed_up_to(self.iq_buf.len());
                    self.stream = Some(stream);
                }
            }

            // Feed new samples to the running stream.
            if let Some(ref mut stream) = self.stream {
                let new_end = self.iq_buf.len();
                if stream.fed_up_to() < new_end {
                    let text = stream.feed(&self.iq_buf[stream.fed_up_to()..new_end]);
                    if !text.is_empty() {
                        let _ = tx.try_send(DecodeResult::Text(text));
                    }
                    stream.set_fed_up_to(new_end);
                }
            }

            // Periodic Info updates (~1 s) during signal.
            self.info_counter += samples.len();
            if self.info_counter >= INFO_INTERVAL {
                self.info_counter = 0;
                let tail_start = self.iq_buf.len().saturating_sub(SPECTRUM_WINDOW_SAMPLES);
                let win: Vec<f32> = self.iq_buf[tail_start..].iter().map(|c| c.re).collect();
                let raw_snr = spectrum_snr_db(&win, fs, carrier_hz);
                if self.smoothed_snr_db == 0.0 {
                    self.smoothed_snr_db = raw_snr;
                } else {
                    self.smoothed_snr_db = 0.2 * raw_snr + 0.8 * self.smoothed_snr_db;
                }
                let _ = tx.try_send(DecodeResult::Info {
                    modulation: mode_label.to_owned(),
                    center_hz: carrier_hz,
                    bw_hz: PSK31_BW_HZ,
                    snr_db: self.smoothed_snr_db,
                });
            }

            // Safety cap: discard oldest samples if buffer grows too large.
            if self.iq_buf.len() >= max_accum {
                let keep = max_accum / 2;
                let drop = self.iq_buf.len() - keep;
                self.iq_buf.drain(..drop);
                if let Some(ref mut stream) = self.stream {
                    let new_pos = stream.fed_up_to().saturating_sub(drop);
                    stream.set_fed_up_to(new_pos);
                }
            }
        }
    }
}
