// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Shared harness for Dt-mode ticker simulation tests.
//!
//! Drives a [`SignalSource`] through block-level gap detection and feeds
//! accumulated IQ buffers into a caller-supplied decode callback, exactly
//! mirroring the decode worker loop in `src/decode/mod.rs`.  Results are
//! pushed into a [`DecodeTicker`] so individual source tests can assert on
//! the final `visible` buffer.
//!
//! This file lives under `tests/common/` so multiple top-level test crates
//! (`tests/psk31.rs`, `tests/ft8.rs`) can share the loop without duplicating
//! the gap-detection bookkeeping.

use num_complex::Complex32 as C32;
use orion_sdr::util::rms;

use orion_sdr_view::decode::{DecodeResult, DecodeTicker, SIGNAL_THRESHOLD};
use orion_sdr_view::source::SignalSource;

/// Result of decoding an accumulated IQ buffer at a gap edge or max-accum flush.
pub struct BufferDecode {
    pub info: Option<DecodeResult>,
    pub text: Option<DecodeResult>,
}

/// Configuration for [`run_ticker_sim`].
pub struct TickerSimConfig<'a> {
    pub label:         &'a str,
    /// Audio callback block size in samples (e.g. 800 for PSK31, 1024 for FT8).
    pub block:         usize,
    /// How many samples to run the simulation over.
    pub total_samples: usize,
    /// Viewer sample rate (Hz).
    pub fs:            f32,
    /// Optional mid-signal flush size in samples.  When the IQ buffer exceeds
    /// this size while still in signal, the harness invokes `decode_fn` with
    /// the current buffer and resets it.  `None` disables mid-signal flushing
    /// (only gap-edge decodes will occur).
    pub max_accum:     Option<usize>,
}

/// Final state returned from a ticker simulation run.
pub struct TickerSimResult {
    pub ticker:        DecodeTicker,
    /// Total number of gap-edge decodes performed.
    pub gap_decodes:   usize,
    /// Total number of mid-signal max-accum decodes performed.
    pub accum_decodes: usize,
}

/// Run the Dt-mode ticker simulation against `source`, using `decode_fn` to
/// turn each accumulated IQ buffer into (Info, Text) results.
///
/// `decode_fn` is called once per gap edge (signal → silence transition) and
/// once per `max_accum` overflow while in signal.  Whatever it returns is
/// pushed to the ticker in the usual Info-then-Text order.
pub fn run_ticker_sim<S, F>(
    source:    &mut S,
    cfg:       &TickerSimConfig<'_>,
    mut decode_fn: F,
) -> TickerSimResult
where
    S: SignalSource + ?Sized,
    F: FnMut(&[C32]) -> BufferDecode,
{
    let mut iq_buf:     Vec<C32>     = Vec::new();
    let mut ticker:     DecodeTicker = DecodeTicker::new();
    let mut t_secs:     f32          = 0.0;
    let mut was_silent: bool         = true;
    let mut gap_decodes:   usize     = 0;
    let mut accum_decodes: usize     = 0;

    println!("── Dt ticker simulation: {} ──", cfg.label);
    println!(
        "  block={}  total_samples={} ({:.1}s)  max_accum={:?}",
        cfg.block,
        cfg.total_samples,
        cfg.total_samples as f32 / cfg.fs,
        cfg.max_accum,
    );

    for block_start in (0..cfg.total_samples).step_by(cfg.block) {
        let n = cfg.block.min(cfg.total_samples - block_start);
        let samples = source.next_samples(n);
        t_secs += n as f32 / cfg.fs;

        let is_silent = rms(&samples) < SIGNAL_THRESHOLD;

        if is_silent {
            if !was_silent && !iq_buf.is_empty() {
                let buf = std::mem::take(&mut iq_buf);
                println!(
                    "t={t_secs:7.2}s  [GAP: decode {} samples = {:.1}s]",
                    buf.len(),
                    buf.len() as f32 / cfg.fs,
                );
                push_decode(&mut ticker, decode_fn(&buf));
                gap_decodes += 1;
            }
            ticker.push_result(DecodeResult::Gap { decoded: false });
            was_silent = true;
        } else {
            iq_buf.extend(samples.iter().map(|&s| C32::new(s, 0.0)));
            was_silent = false;

            if let Some(max_accum) = cfg.max_accum
                && iq_buf.len() >= max_accum
            {
                let buf = std::mem::take(&mut iq_buf);
                println!(
                    "t={t_secs:7.2}s  [MAX_ACCUM flush: {} samples]",
                    buf.len(),
                );
                push_decode(&mut ticker, decode_fn(&buf));
                accum_decodes += 1;
            }
        }

        ticker.tick(n as f32 / cfg.fs);
    }

    println!("── Final ticker buffer ──");
    println!("  visible: {:?}", ticker.visible);
    println!("  visible.len() = {}", ticker.visible.len());

    TickerSimResult { ticker, gap_decodes, accum_decodes }
}

fn push_decode(ticker: &mut DecodeTicker, d: BufferDecode) {
    if let Some(info) = d.info {
        if let DecodeResult::Info { ref modulation, center_hz, snr_db, .. } = info {
            println!("  Info: {modulation} ctr={center_hz:.1}Hz snr={snr_db:.1}dB");
        }
        ticker.push_result(info);
    }
    match d.text {
        Some(DecodeResult::Text(ref s)) => {
            println!("  Text: {:?}", s);
            ticker.push_result(DecodeResult::Text(s.clone()));
        }
        Some(other) => println!("  {:?}", other),
        None        => println!("  (no text)"),
    }
}
