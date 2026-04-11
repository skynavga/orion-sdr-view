// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Integration tests for the AM DSB source and its viewer simulation.

use num_complex::Complex32 as C32;

use orion_sdr_view::decode::{
    SIGNAL_THRESHOLD, SPECTRUM_WINDOW_SAMPLES, spectrum_bw_hz, spectrum_snr_db,
};
use orion_sdr_view::source::{
    AmDsbSource, BuiltinAudio, SignalSource, load_builtin,
};

const FS: f32 = 48_000.0;
const CARRIER_HZ: f32 = 12_000.0;

// ── Viewer simulation ─────────────────────────────────────────────────────────

/// Simulate the full decode worker loop for AM DSB: feed samples from
/// AmDsbSource in the same block size the audio callback uses (1024), apply
/// gap detection, rolling window accumulation, EMA smoothing, and print
/// the BW that would be displayed at each decode window boundary.
#[test]
fn simulate_am_dsb_viewer() {
    use orion_sdr::util::rms;
    const BLOCK: usize = 1024;

    for &audio_kind in &[BuiltinAudio::Morse, BuiltinAudio::Voice] {
        let label = match audio_kind { BuiltinAudio::Morse => "Morse", BuiltinAudio::Voice => "Voice" };
        let (audio, audio_rate) = load_builtin(audio_kind);
        let audio_secs = audio.len() as f32 / audio_rate;

        let mut src = AmDsbSource::new(
            audio, audio_rate, CARRIER_HZ, 1.0,
            /*gap_secs=*/ 2.0,
            /*noise_amp=*/ 0.05,
            /*msg_repeat=*/ 1,
            FS,
        );

        let mut iq_buf: Vec<C32> = Vec::new();
        let mut smoothed_bw = 0.0f32;
        let total_out = ((audio_secs + 2.0) * FS) as usize;
        let mut t_secs = 0.0f32;
        let mut window_count = 0usize;

        println!("\n── {label} ({audio_secs:.1}s audio, {total_out} output samples) ──");

        for block_start in (0..total_out).step_by(BLOCK) {
            let n = BLOCK.min(total_out - block_start);
            let samples = src.next_samples(n);
            t_secs += n as f32 / FS;

            if rms(&samples) < SIGNAL_THRESHOLD {
                iq_buf.clear();
                continue;
            }

            iq_buf.extend(samples.iter().map(|&s| C32::new(s, 0.0)));
            if iq_buf.len() < SPECTRUM_WINDOW_SAMPLES { continue; }

            let win: Vec<f32> = iq_buf[..SPECTRUM_WINDOW_SAMPLES].iter().map(|c| c.re).collect();
            iq_buf.drain(..SPECTRUM_WINDOW_SAMPLES / 2);

            let raw_bw = spectrum_bw_hz(&win, FS, CARRIER_HZ, 7.0);
            if smoothed_bw == 0.0 { smoothed_bw = raw_bw; } else { smoothed_bw = 0.3 * raw_bw + 0.7 * smoothed_bw; }

            window_count += 1;
            let snr = spectrum_snr_db(&win, FS, CARRIER_HZ);
            println!("  t={t_secs:5.2}s  raw_bw={raw_bw:7.1}  smoothed={smoothed_bw:7.1}  snr={snr:.1}dB");
        }
        println!("  ({window_count} decode windows)");
    }
}
