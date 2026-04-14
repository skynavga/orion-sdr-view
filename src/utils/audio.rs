// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

// Phase 8 migration candidate: tone generation, morse encoding, and WAV
// writing utilities.  orion-sdr may already provide some of these; merge
// or replace as appropriate when migrating.

use std::f32::consts::PI;
use std::path::Path;

// ── WAV I/O ───────────────────────────────────────────────────────────────────

/// Write mono 32-bit float PCM WAV at the given sample rate.
pub fn write_wav(path: &Path, samples: &[f32], sample_rate: u32) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(path, spec).expect("create wav");
    for &s in samples {
        writer.write_sample(s).expect("write sample");
    }
    writer.finalize().expect("finalize wav");
}

// ── Tone primitives ───────────────────────────────────────────────────────────

/// Generate a buffer of silence.
pub fn silence(secs: f32, sample_rate: f32) -> Vec<f32> {
    vec![0.0f32; (secs * sample_rate) as usize]
}

/// Generate a windowed sine burst with 5 ms raised-cosine ramps.
///
/// `phase` is carried across calls for phase-continuous bursts.
pub fn sine_burst(
    freq_hz: f32,
    dur_secs: f32,
    amp: f32,
    sample_rate: f32,
    phase: &mut f32,
) -> Vec<f32> {
    let n = (dur_secs * sample_rate) as usize;
    let dphi = 2.0 * PI * freq_hz / sample_rate;
    let ramp = (0.005 * sample_rate) as usize;
    (0..n)
        .map(|i| {
            let env = if i < ramp {
                i as f32 / ramp as f32
            } else if i >= n - ramp {
                (n - i) as f32 / ramp as f32
            } else {
                1.0
            };
            let s = amp * env * phase.sin();
            *phase += dphi;
            if *phase > PI {
                *phase -= 2.0 * PI;
            }
            s
        })
        .collect()
}

// ── Morse code ────────────────────────────────────────────────────────────────

const DIT: f32 = 0.080;
const DAH: f32 = 3.0 * DIT;
const TONE_HZ: f32 = 800.0;
const TONE_AMP: f32 = 0.8;

/// Return the dit/dah pattern for an alphanumeric character.
/// `false` = dit, `true` = dah. Returns empty for unsupported chars.
pub fn morse(c: char) -> &'static [bool] {
    match c {
        'A' => &[false, true],
        'B' => &[true, false, false, false],
        'C' => &[true, false, true, false],
        'D' => &[true, false, false],
        'E' => &[false],
        'F' => &[false, false, true, false],
        'G' => &[true, true, false],
        'H' => &[false, false, false, false],
        'I' => &[false, false],
        'J' => &[false, true, true, true],
        'K' => &[true, false, true],
        'L' => &[false, true, false, false],
        'M' => &[true, true],
        'N' => &[true, false],
        'O' => &[true, true, true],
        'P' => &[false, true, true, false],
        'Q' => &[true, true, false, true],
        'R' => &[false, true, false],
        'S' => &[false, false, false],
        'T' => &[true],
        'U' => &[false, false, true],
        'V' => &[false, false, false, true],
        'W' => &[false, true, true],
        'X' => &[true, false, false, true],
        'Y' => &[true, false, true, true],
        'Z' => &[true, true, false, false],
        '1' => &[false, true, true, true, true],
        '2' => &[false, false, true, true, true],
        '3' => &[false, false, false, true, true],
        '4' => &[false, false, false, false, true],
        '5' => &[false, false, false, false, false],
        '6' => &[true, false, false, false, false],
        '7' => &[true, true, false, false, false],
        '8' => &[true, true, true, false, false],
        '9' => &[true, true, true, true, false],
        '0' => &[true, true, true, true, true],
        _ => &[],
    }
}

/// Render a morse CQ message as audio samples.
///
/// Words are separated by 4-dit gaps, characters by 2-dit gaps, elements
/// by 1-dit gaps.  A trailing silence of `gap_secs` is appended.
pub fn gen_morse_cq(sample_rate: f32, gap_secs: f32) -> Vec<f32> {
    let words: &[&str] = &["CQ", "CQ", "CQ", "DE", "N0GNR", "N0GNR", "K"];
    let mut out: Vec<f32> = Vec::new();
    let mut phase = 0.0f32;

    for (wi, word) in words.iter().enumerate() {
        for (ci, ch) in word.chars().enumerate() {
            let elements = morse(ch);
            for (ei, &is_dah) in elements.iter().enumerate() {
                let dur = if is_dah { DAH } else { DIT };
                out.extend(sine_burst(TONE_HZ, dur, TONE_AMP, sample_rate, &mut phase));
                if ei + 1 < elements.len() {
                    out.extend(silence(DIT, sample_rate));
                }
            }
            if ci + 1 < word.len() {
                out.extend(silence(2.0 * DIT, sample_rate));
            }
        }
        if wi + 1 < words.len() {
            out.extend(silence(4.0 * DIT, sample_rate));
        }
    }
    out.extend(silence(gap_secs, sample_rate));
    out
}
