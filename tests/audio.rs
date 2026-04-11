// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use orion_sdr_view::utils::audio::*;

#[test]
fn silence_length() {
    let buf = silence(0.5, 8000.0);
    assert_eq!(buf.len(), 4000);
    assert!(buf.iter().all(|&s| s == 0.0));
}

#[test]
fn sine_burst_length_and_amplitude() {
    let mut phase = 0.0;
    let buf = sine_burst(800.0, 0.1, 0.8, 8000.0, &mut phase);
    assert_eq!(buf.len(), 800);
    // All samples within ±amp
    assert!(buf.iter().all(|&s| s.abs() <= 0.8 + 1e-6));
    // Ramp region (first 40 samples at 8 kHz × 5 ms) starts near zero
    assert!(buf[0].abs() < 0.01);
    // Steady-state region has significant energy
    let mid = buf.len() / 2;
    let rms: f32 = (buf[mid - 10..mid + 10].iter().map(|s| s * s).sum::<f32>() / 20.0).sqrt();
    assert!(rms > 0.3);
}

#[test]
fn sine_burst_phase_continuous() {
    // Two consecutive bursts should carry phase across calls.
    // Use 750 Hz at 8 kHz so the phase doesn't land on an exact cycle.
    let mut phase = 0.0;
    sine_burst(750.0, 0.05, 1.0, 8000.0, &mut phase);
    assert!(phase != 0.0, "phase should advance after first burst");
    let p1 = phase;
    sine_burst(750.0, 0.05, 1.0, 8000.0, &mut phase);
    assert!(phase != p1, "phase should advance after second burst");
}

#[test]
fn morse_known_patterns() {
    // SOS = ··· ——— ···
    assert_eq!(morse('S'), &[false, false, false]);
    assert_eq!(morse('O'), &[true, true, true]);
    assert_eq!(morse('E'), &[false]);           // single dit
    assert_eq!(morse('T'), &[true]);             // single dah
    assert_eq!(morse('0'), &[true, true, true, true, true]);
    assert_eq!(morse('5'), &[false, false, false, false, false]);
}

#[test]
fn morse_unsupported_returns_empty() {
    assert!(morse(' ').is_empty());
    assert!(morse('!').is_empty());
    assert!(morse('a').is_empty()); // lowercase not mapped
}

#[test]
fn gen_morse_cq_produces_audio() {
    let buf = gen_morse_cq(8000.0, 2.0);
    // Should be non-trivial length (message + 2 s gap)
    assert!(buf.len() > 8000 * 2); // at least the trailing gap
    // Trailing gap should be silence
    let gap_samples = (2.0 * 8000.0) as usize;
    let tail = &buf[buf.len() - gap_samples..];
    assert!(tail.iter().all(|&s| s == 0.0));
    // Signal portion has non-zero energy
    let signal = &buf[..buf.len() - gap_samples];
    let peak = signal.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    assert!(peak > 0.5);
}

#[test]
fn write_wav_roundtrip() {
    let samples = sine_burst(440.0, 0.1, 0.5, 8000.0, &mut 0.0);
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("test.wav");
    write_wav(&path, &samples, 8000);

    // Read back and verify
    let mut reader = hound::WavReader::open(&path).expect("open wav");
    let spec = reader.spec();
    assert_eq!(spec.channels, 1);
    assert_eq!(spec.sample_rate, 8000);
    assert_eq!(spec.bits_per_sample, 32);
    let read_samples: Vec<f32> = reader.samples::<f32>().map(|s| s.unwrap()).collect();
    assert_eq!(read_samples.len(), samples.len());
    for (a, b) in samples.iter().zip(read_samples.iter()) {
        assert!((a - b).abs() < 1e-6);
    }
}
