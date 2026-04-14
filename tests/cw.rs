// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Integration tests for the CW source and decode timing.

use orion_sdr_view::decode::{cw_char_timing, morse_char_units};
use orion_sdr_view::source::SignalSource;
use orion_sdr_view::source::cw::{
    CW_DEFAULT_CARRIER_HZ, CW_DEFAULT_CHAR_SPACE, CW_DEFAULT_DASH_WEIGHT, CW_DEFAULT_FALL_MS,
    CW_DEFAULT_GAP_SECS, CW_DEFAULT_JITTER_PCT, CW_DEFAULT_NOISE_AMP, CW_DEFAULT_REPEAT,
    CW_DEFAULT_RISE_MS, CW_DEFAULT_WORD_SPACE, CW_DEFAULT_WPM, CwSource,
};

const FS: f32 = 48_000.0;

fn make_default_source(message: &str) -> CwSource {
    CwSource::new(
        CW_DEFAULT_CARRIER_HZ,
        CW_DEFAULT_GAP_SECS,
        CW_DEFAULT_NOISE_AMP,
        CW_DEFAULT_WPM,
        CW_DEFAULT_JITTER_PCT,
        CW_DEFAULT_DASH_WEIGHT,
        CW_DEFAULT_CHAR_SPACE,
        CW_DEFAULT_WORD_SPACE,
        CW_DEFAULT_RISE_MS,
        CW_DEFAULT_FALL_MS,
        message.to_owned(),
        CW_DEFAULT_REPEAT,
        FS,
    )
}

fn make_clean_source(message: &str) -> CwSource {
    CwSource::new(
        CW_DEFAULT_CARRIER_HZ,
        CW_DEFAULT_GAP_SECS,
        0.0, // no noise
        CW_DEFAULT_WPM,
        0.0, // no jitter
        CW_DEFAULT_DASH_WEIGHT,
        CW_DEFAULT_CHAR_SPACE,
        CW_DEFAULT_WORD_SPACE,
        CW_DEFAULT_RISE_MS,
        CW_DEFAULT_FALL_MS,
        message.to_owned(),
        1,
        FS,
    )
}

// ── Basic signal generation ──────────────────────────────────────────────────

#[test]
fn cw_source_produces_samples() {
    let mut src = make_default_source("CQ CQ CQ DE N0GNR");
    let samples = src.next_samples(4800); // 100 ms
    assert_eq!(samples.len(), 4800);
    // Signal should be present in the first burst.
    let peak = samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    assert!(peak > 0.1, "expected signal, peak={peak}");
}

#[test]
fn cw_source_gap_has_only_noise() {
    // Use a very short message with no noise to verify the gap is silent.
    let mut src = CwSource::new(
        CW_DEFAULT_CARRIER_HZ,
        1.0,  // 1 second gap
        0.0,  // no noise
        30.0, // fast WPM for short signal
        0.0,
        CW_DEFAULT_DASH_WEIGHT,
        CW_DEFAULT_CHAR_SPACE,
        CW_DEFAULT_WORD_SPACE,
        CW_DEFAULT_RISE_MS,
        CW_DEFAULT_FALL_MS,
        "E".to_owned(), // shortest Morse character
        1,
        FS,
    );

    // Consume enough to get past the signal burst into the gap.
    // 'E' at 30 WPM = 1 dot = 40 ms = 1920 samples, plus rise/fall envelope.
    // Consume 10000 samples to be safely into the gap.
    let _ = src.next_samples(10000);

    // Now read some gap samples.
    let gap_samples = src.next_samples(4800);
    let max_abs = gap_samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    assert!(max_abs < 1e-6, "expected silence in gap, max_abs={max_abs}");
}

// ── Restart ──────────────────────────────────────────────────────────────────

#[test]
fn cw_source_restart() {
    let mut src = make_clean_source("SOS");
    let first = src.next_samples(4800);
    src.restart();
    let second = src.next_samples(4800);
    assert_eq!(first, second, "restart should replay from beginning");
}

// ── Render idempotent ────────────────────────────────────────────────────────

#[test]
fn cw_source_render_idempotent() {
    let mut src = make_clean_source("CQ DE N0GNR");
    let len1 = {
        let samples = src.next_samples(500_000);
        samples.len()
    };
    src.render();
    let len2 = {
        let samples = src.next_samples(500_000);
        samples.len()
    };
    assert_eq!(len1, len2, "render() should produce same-length output");
}

// ── WPM affects signal duration ──────────────────────────────────────────────

#[test]
fn cw_source_wpm_affects_duration() {
    // Slower WPM should produce a longer signal burst for the same message.
    let mut slow = CwSource::new(
        CW_DEFAULT_CARRIER_HZ,
        CW_DEFAULT_GAP_SECS,
        0.0,
        5.0, // slow
        0.0,
        CW_DEFAULT_DASH_WEIGHT,
        CW_DEFAULT_CHAR_SPACE,
        CW_DEFAULT_WORD_SPACE,
        CW_DEFAULT_RISE_MS,
        CW_DEFAULT_FALL_MS,
        "SOS".to_owned(),
        1,
        FS,
    );
    let mut fast = CwSource::new(
        CW_DEFAULT_CARRIER_HZ,
        CW_DEFAULT_GAP_SECS,
        0.0,
        25.0, // fast
        0.0,
        CW_DEFAULT_DASH_WEIGHT,
        CW_DEFAULT_CHAR_SPACE,
        CW_DEFAULT_WORD_SPACE,
        CW_DEFAULT_RISE_MS,
        CW_DEFAULT_FALL_MS,
        "SOS".to_owned(),
        1,
        FS,
    );

    // Count non-silence samples in first burst.
    let slow_burst: Vec<f32> = slow.next_samples(5_000_000);
    let fast_burst: Vec<f32> = fast.next_samples(5_000_000);

    let slow_active = slow_burst.iter().filter(|s| s.abs() > 0.001).count();
    let fast_active = fast_burst.iter().filter(|s| s.abs() > 0.001).count();

    assert!(
        slow_active > fast_active * 3,
        "5 WPM signal ({slow_active} active) should be much longer than 25 WPM ({fast_active} active)"
    );
}

// ── Sample rate ──────────────────────────────────────────────────────────────

#[test]
fn cw_source_sample_rate() {
    let src = make_default_source("E");
    assert_eq!(src.sample_rate(), FS);
}

// ── Empty message ────────────────────────────────────────────────────────────

#[test]
fn cw_source_empty_message() {
    // Should not panic; produces gap-only output.
    let mut src = make_clean_source("");
    let samples = src.next_samples(4800);
    assert_eq!(samples.len(), 4800);
}

// ── Morse character units ───────────────────────────────────────────────────

#[test]
fn morse_char_units_dot() {
    // 'E' = "." → 1 unit
    let u = morse_char_units('E', 3.0).unwrap();
    assert!((u - 1.0).abs() < 1e-6);
}

#[test]
fn morse_char_units_dash() {
    // 'T' = "-" → dash_weight units
    let u = morse_char_units('T', 3.0).unwrap();
    assert!((u - 3.0).abs() < 1e-6);
}

#[test]
fn morse_char_units_letter_a() {
    // 'A' = ".-" → dot(1) + intra(1) + dash(3) = 5
    let u = morse_char_units('A', 3.0).unwrap();
    assert!((u - 5.0).abs() < 1e-6);
}

#[test]
fn morse_char_units_sos() {
    // 'S' = "..." → 3 dots + 2 intra-gaps = 5 units
    let u = morse_char_units('S', 3.0).unwrap();
    assert!((u - 5.0).abs() < 1e-6);
    // 'O' = "---" → 3 dashes(9) + 2 intra-gaps(2) = 11 units
    let u = morse_char_units('O', 3.0).unwrap();
    assert!((u - 11.0).abs() < 1e-6);
}

#[test]
fn morse_char_units_custom_weight() {
    // 'T' = "-" with dash_weight=3.5 → 3.5 units
    let u = morse_char_units('T', 3.5).unwrap();
    assert!((u - 3.5).abs() < 1e-6);
}

#[test]
fn morse_char_units_unknown() {
    assert!(morse_char_units('~', 3.0).is_none());
}

#[test]
fn morse_char_units_case_insensitive() {
    assert_eq!(morse_char_units('a', 3.0), morse_char_units('A', 3.0));
}

// ── CW character timing schedule ────────────────────────────────────────────

#[test]
fn cw_timing_single_char() {
    // 'E' at 20 WPM, fs=48000, no repeat
    // 1 unit = 1200/20 = 60 ms = 2880 samples
    // 'E' body = 1 unit = 2880 samples
    let sched = cw_char_timing("E", 20.0, 3.0, 3.0, 7.0, 1, FS);
    assert_eq!(sched.len(), 1);
    assert_eq!(sched[0].0, 'E');
    let unit_samples = (1200.0 / 20.0 * 1e-3 * FS) as usize;
    assert_eq!(sched[0].1, unit_samples);
}

#[test]
fn cw_timing_two_chars() {
    // "SOS" at 20 WPM
    let sched = cw_char_timing("SOS", 20.0, 3.0, 3.0, 7.0, 1, FS);
    assert_eq!(sched.len(), 3);
    assert_eq!(sched[0].0, 'S');
    assert_eq!(sched[1].0, 'O');
    assert_eq!(sched[2].0, 'S');
    // Thresholds must be strictly increasing.
    assert!(sched[0].1 < sched[1].1);
    assert!(sched[1].1 < sched[2].1);
}

#[test]
fn cw_timing_word_gap() {
    // "E E" — word gap between the two E's
    let unit = 1200.0 / 20.0 * 1e-3 * FS;
    let sched = cw_char_timing("E E", 20.0, 3.0, 3.0, 7.0, 1, FS);
    assert_eq!(sched.len(), 2);
    // First 'E': body = 1 unit
    let first_end = (1.0 * unit).round() as usize;
    assert_eq!(sched[0].1, first_end);
    // Second 'E': word_gap(7) + body(1) = 8 units after first
    let second_end = ((1.0 + 7.0 + 1.0) * unit).round() as usize;
    assert_eq!(sched[1].1, second_end);
}

#[test]
fn cw_timing_repeat() {
    // "E" repeated 3 times → "E E E" (space-joined)
    let sched = cw_char_timing("E", 20.0, 3.0, 3.0, 7.0, 3, FS);
    assert_eq!(sched.len(), 3);
    // All three should be 'E'
    for (ch, _) in &sched {
        assert_eq!(*ch, 'E');
    }
    assert!(sched[0].1 < sched[1].1);
    assert!(sched[1].1 < sched[2].1);
}

#[test]
fn cw_timing_empty_message() {
    let sched = cw_char_timing("", 20.0, 3.0, 3.0, 7.0, 1, FS);
    assert!(sched.is_empty());
}

#[test]
fn cw_timing_unknown_chars_skipped() {
    // "~" is not in Morse table
    let sched = cw_char_timing("~", 20.0, 3.0, 3.0, 7.0, 1, FS);
    assert!(sched.is_empty());
}

#[test]
fn cw_timing_mixed_known_unknown() {
    // "E~S" → only E and S
    let sched = cw_char_timing("E~S", 20.0, 3.0, 3.0, 7.0, 1, FS);
    assert_eq!(sched.len(), 2);
    assert_eq!(sched[0].0, 'E');
    assert_eq!(sched[1].0, 'S');
}

#[test]
fn cw_timing_low_wpm() {
    let sched = cw_char_timing("E", 0.5, 3.0, 3.0, 7.0, 1, FS);
    assert!(sched.is_empty(), "wpm < 1.0 should produce empty schedule");
}

#[test]
fn cw_timing_char_space_affects_gap() {
    // Compare char_space=3.0 vs 4.0 for "AB"
    let sched_3 = cw_char_timing("AB", 20.0, 3.0, 3.0, 7.0, 1, FS);
    let sched_4 = cw_char_timing("AB", 20.0, 3.0, 4.0, 7.0, 1, FS);
    // With larger char_space, 'B' should appear later.
    assert!(sched_4[1].1 > sched_3[1].1);
    // 'A' timing should be identical (no gap before first char).
    assert_eq!(sched_3[0].1, sched_4[0].1);
}
