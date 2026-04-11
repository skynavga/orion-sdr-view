// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use orion_sdr_view::source::tone::{TestSignalGen, TestToneSource};
use orion_sdr_view::source::SignalSource;

fn make_gen() -> TestSignalGen {
    TestSignalGen::new(1000.0, 48000.0)
}

#[test]
fn new_defaults() {
    let g = make_gen();
    assert_eq!(g.freq_hz, 1000.0);
    assert_eq!(g.sample_rate, 48000.0);
    assert_eq!(g.tone_amp, 0.65);
    assert_eq!(g.noise_amp, 0.05);
    assert!(!g.cycling);
}

#[test]
fn next_sample_produces_bounded_output() {
    let mut g = make_gen();
    for _ in 0..1000 {
        let s = g.next_sample();
        // tone_amp=0.65 + noise can exceed slightly, but should be reasonable
        assert!(s.abs() < 2.0, "sample out of range: {s}");
    }
}

#[test]
fn next_sample_has_energy() {
    let mut g = make_gen();
    let samples: Vec<f32> = (0..4800).map(|_| g.next_sample()).collect();
    let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
    // With tone_amp=0.65, RMS of a sine is ~0.46; noise adds a bit
    assert!(rms > 0.3, "RMS too low: {rms}");
}

#[test]
fn awgn_distribution() {
    // With noise_amp=1.0 and tone_amp=0.0, output is pure AWGN.
    let mut g = make_gen();
    g.tone_amp = 0.0;
    g.noise_amp = 1.0;
    let samples: Vec<f32> = (0..10000).map(|_| g.next_sample()).collect();
    let mean = samples.iter().sum::<f32>() / samples.len() as f32;
    let variance = samples.iter().map(|s| (s - mean).powi(2)).sum::<f32>() / samples.len() as f32;
    // CLT of 12 uniforms: mean≈0, variance≈1
    assert!(mean.abs() < 0.1, "AWGN mean not near zero: {mean}");
    assert!((variance - 1.0).abs() < 0.2, "AWGN variance not near 1: {variance}");
}

#[test]
fn start_cycling_begins_ramp_down() {
    let mut g = make_gen();
    assert!(!g.cycling);
    g.start_cycling();
    assert!(g.cycling);
    assert_eq!(g.tone_amp, g.amp_max);
    // After many samples, amplitude should decrease (ramp down)
    let ramp_samples = (g.ramp_secs * g.sample_rate) as usize;
    for _ in 0..ramp_samples / 2 {
        g.next_sample();
    }
    assert!(g.tone_amp < g.amp_max, "amplitude should decrease during ramp down");
}

#[test]
fn start_cycling_idempotent() {
    let mut g = make_gen();
    g.start_cycling();
    let amp1 = g.tone_amp;
    g.start_cycling(); // second call should be no-op
    assert_eq!(g.tone_amp, amp1);
}

#[test]
fn stop_cycling_restores_full_amplitude() {
    let mut g = make_gen();
    g.start_cycling();
    // Advance partway through ramp-down
    for _ in 0..1000 {
        g.next_sample();
    }
    g.stop_cycling();
    assert!(!g.cycling);
    assert_eq!(g.tone_amp, g.amp_max);
}

#[test]
fn stop_cycling_idempotent() {
    let mut g = make_gen();
    g.stop_cycling(); // not cycling — should be no-op
    assert!(!g.cycling);
    assert_eq!(g.tone_amp, g.amp_max);
}

#[test]
fn restart_resets_state() {
    let mut g = make_gen();
    g.start_cycling();
    for _ in 0..5000 {
        g.next_sample();
    }
    g.restart();
    assert_eq!(g.tone_amp, g.amp_max);
    // Phase should be zero after restart
    g.noise_amp = 0.0;
    let s = g.next_sample();
    // next_sample computes sin(phase) THEN advances, so first sample is sin(0)=0
    assert!(s.abs() < 0.01, "first sample after restart should be near zero: {s}");
}

#[test]
fn full_cycle_returns_to_peak() {
    let mut g = make_gen();
    g.noise_amp = 0.0; // eliminate noise for deterministic test
    g.start_cycling();
    // One full cycle: ramp_down + pause_low + ramp_up + pause_high
    let cycle_samples = 2 * (g.ramp_secs * g.sample_rate) as usize
        + 2 * (g.pause_secs * g.sample_rate) as usize
        + 4; // small margin for boundary samples
    for _ in 0..cycle_samples {
        g.next_sample();
    }
    // Should be back at amp_max (in PauseHigh or start of next RampDown)
    assert!((g.tone_amp - g.amp_max).abs() < 0.01,
        "amplitude should return to peak after full cycle: {}", g.tone_amp);
}

#[test]
fn test_tone_source_trait() {
    let g = TestSignalGen::new(5000.0, 48000.0);
    let mut src = TestToneSource::new(g);
    assert_eq!(src.sample_rate(), 48000.0);
    let samples = src.next_samples(100);
    assert_eq!(samples.len(), 100);
    // Verify as_any_mut downcast works
    assert!(src.as_any_mut().downcast_mut::<TestToneSource>().is_some());
}

#[test]
fn test_tone_source_restart() {
    let mut g = TestSignalGen::new(1000.0, 48000.0);
    g.noise_amp = 0.0;
    let mut src = TestToneSource::new(g);
    // Generate some samples to advance phase
    src.next_samples(1000);
    src.restart();
    // After restart, first sample should be near zero (sin(0))
    let samples = src.next_samples(1);
    assert!(samples[0].abs() < 0.01);
}
