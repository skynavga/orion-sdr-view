// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! FT8/FT4 decode state and processing.

use std::sync::mpsc::SyncSender;

use num_complex::Complex32 as C32;
use orion_sdr::codec::Ft8StreamDecoder;
use orion_sdr::message::{Ft8Message, gridfield_to_str};

use crate::decode::{DecodeMode, DecodeResult};
use crate::source::psk31::INFO_INTERVAL;
pub use orion_sdr::util::spectrum_snr_db;

/// FT8 signal bandwidth: 8 tones × 6.25 Hz spacing = 50 Hz.
pub const FT8_BW_HZ: f32 = 50.0;
/// FT4 signal bandwidth: 4 tones × 20.833 Hz spacing ≈ 83 Hz.
pub const FT4_BW_HZ: f32 = 83.0;
/// Downsample factor: viewer runs at 48 kHz, FT8/FT4 native rate is 12 kHz.
const FT8_UPSAMPLE: usize = 4;
/// Sync search half-width for FT8/FT4 (±200 Hz around configured carrier).
const FT8_SEARCH_HZ: f32 = 200.0;

pub struct Ft8State {
    pub decoder: Option<Ft8StreamDecoder>,
    pub decoded_this_burst: bool,
    pub shift_phase: f32,
    pub smoothed_snr_db: f32,
    pub info_counter: usize,
}

impl Default for Ft8State {
    fn default() -> Self {
        Self {
            decoder: None,
            decoded_this_burst: false,
            shift_phase: 0.0,
            smoothed_snr_db: 0.0,
            info_counter: 0,
        }
    }
}

impl Ft8State {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        self.decoder = None;
        self.decoded_this_burst = false;
        self.shift_phase = 0.0;
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
        let is_ft8 = mode == DecodeMode::Ft8;
        let label = if is_ft8 { "FT8" } else { "FT4" };
        let bw_hz = if is_ft8 { FT8_BW_HZ } else { FT4_BW_HZ };
        let native_fs = fs / FT8_UPSAMPLE as f32; // 12 kHz
        let native_carrier = crate::source::ft8::FT8_MOD_BASE_HZ;

        if !is_signal {
            if gap_edge {
                if let Some(ref mut dec) = self.decoder {
                    let flush_results = if !self.decoded_this_burst {
                        dec.flush()
                    } else {
                        Vec::new()
                    };
                    let decoded = self.decoded_this_burst || !flush_results.is_empty();
                    for r in flush_results {
                        let text = format_ft8_message(&r.message, label);
                        let _ = tx.try_send(DecodeResult::Text(text));
                    }
                    let _ = tx.try_send(DecodeResult::Info {
                        modulation: label.to_owned(),
                        center_hz: carrier_hz,
                        bw_hz: if decoded { bw_hz } else { 0.0 },
                        snr_db: self.smoothed_snr_db,
                    });
                    let _ = tx.try_send(DecodeResult::Gap { decoded });
                    dec.clear();
                }
                self.decoded_this_burst = false;
                self.smoothed_snr_db = 0.0;
                self.info_counter = 0;
            }
        } else {
            // Create decoder on first signal after a gap or mode switch.
            if self.decoder.is_none() {
                let base_hz = (native_carrier - FT8_SEARCH_HZ).max(0.0);
                let max_hz = native_carrier + FT8_SEARCH_HZ;
                self.decoder = Some(if is_ft8 {
                    Ft8StreamDecoder::new_ft8(native_fs, base_hz, max_hz, 8)
                } else {
                    Ft8StreamDecoder::new_ft4(native_fs, base_hz, max_hz, 8)
                });
            }

            // Frequency-shift down from carrier_hz to FT8_MOD_BASE_HZ
            // via complex mixer, then decimate 4:1 from 48 kHz → 12 kHz.
            let shift_hz = carrier_hz - crate::source::ft8::FT8_MOD_BASE_HZ;
            let phase_inc = 2.0 * std::f32::consts::PI * shift_hz / fs;
            let two_pi = 2.0 * std::f32::consts::PI;
            let mut downsampled: Vec<C32> = Vec::with_capacity(samples.len() / FT8_UPSAMPLE + 1);
            for (i, s) in samples.iter().enumerate() {
                if i % FT8_UPSAMPLE == 0 {
                    let (sin_p, cos_p) = self.shift_phase.sin_cos();
                    downsampled.push(C32::new(s * cos_p, -s * sin_p));
                }
                self.shift_phase += phase_inc;
                if self.shift_phase >= two_pi {
                    self.shift_phase -= two_pi;
                } else if self.shift_phase < 0.0 {
                    self.shift_phase += two_pi;
                }
            }

            if let Some(ref mut dec) = self.decoder {
                let results = dec.feed(&downsampled);
                if !results.is_empty() {
                    self.decoded_this_burst = true;
                    dec.clear();
                    for r in results {
                        let text = format_ft8_message(&r.message, label);
                        let _ = tx.try_send(DecodeResult::Text(text));
                    }
                }
            }

            // Periodic SNR update (~1 s) during accumulation.
            self.info_counter += samples.len();
            if self.info_counter >= INFO_INTERVAL {
                self.info_counter = 0;
                if let Some(ref dec) = self.decoder {
                    let real: Vec<f32> = dec.view_buf().iter().map(|c| c.re).collect();
                    let raw_snr = spectrum_snr_db(&real, native_fs, native_carrier);
                    self.smoothed_snr_db = if self.smoothed_snr_db == 0.0 {
                        raw_snr
                    } else {
                        0.2 * raw_snr + 0.8 * self.smoothed_snr_db
                    };
                }
                let _ = tx.try_send(DecodeResult::Info {
                    modulation: label.to_owned(),
                    center_hz: carrier_hz,
                    bw_hz,
                    snr_db: self.smoothed_snr_db,
                });
            }
        }
    }
}

/// Format a decoded `Ft8Message` for the Dt ticker.
fn format_ft8_message(msg: &Ft8Message, _label: &str) -> String {
    match msg {
        Ft8Message::Standard {
            call_to,
            call_de,
            extra,
        } => {
            let extra_str = gridfield_to_str(extra);
            if extra_str.is_empty() || extra_str == "None" {
                format!("{call_to} DE {call_de}")
            } else {
                format!("{call_to} DE {call_de} {extra_str}")
            }
        }
        Ft8Message::FreeText(text) => text.clone(),
        Ft8Message::NonStd {
            call_to, call_de, ..
        } => {
            format!("{call_to} DE {call_de}")
        }
        Ft8Message::Telemetry(data) => data
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<Vec<_>>()
            .join(""),
        Ft8Message::Unknown(_) => "[undecoded]".to_owned(),
    }
}
