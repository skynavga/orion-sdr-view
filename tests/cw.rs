// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Integration tests for the CW source and decode timing.

use orion_sdr::util::rms;
use orion_sdr_view::decode::{SIGNAL_THRESHOLD, cw_char_timing, morse_char_units};
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
    // Schedule: E, ' ', E  (space emitted at word boundary)
    assert_eq!(sched.len(), 3);
    // First 'E': body = 1 unit
    let first_end = (1.0 * unit).round() as usize;
    assert_eq!(sched[0].0, 'E');
    assert_eq!(sched[0].1, first_end);
    // Space emitted at the start of the word gap (same threshold as first E's end).
    assert_eq!(sched[1].0, ' ');
    assert_eq!(sched[1].1, first_end);
    // Second 'E': word_gap(7) + body(1) = 8 units after first
    let second_end = ((1.0 + 7.0 + 1.0) * unit).round() as usize;
    assert_eq!(sched[2].0, 'E');
    assert_eq!(sched[2].1, second_end);
}

#[test]
fn cw_timing_repeat() {
    // "E" repeated 3 times → "E E E" (space-joined)
    let sched = cw_char_timing("E", 20.0, 3.0, 3.0, 7.0, 3, FS);
    // Schedule: E, ' ', E, ' ', E  (spaces at word boundaries)
    assert_eq!(sched.len(), 5);
    assert_eq!(sched[0].0, 'E');
    assert_eq!(sched[1].0, ' ');
    assert_eq!(sched[2].0, 'E');
    assert_eq!(sched[3].0, ' ');
    assert_eq!(sched[4].0, 'E');
    // Thresholds must be strictly non-decreasing.
    for w in sched.windows(2) {
        assert!(w[0].1 <= w[1].1);
    }
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

// ── Round-trip: signal detection diagnostics ────────────────────────────────

/// Count signal/gap transitions when processing a CW source block-by-block.
///
/// CW keying has inter-element and inter-character silences within a single
/// transmission.  If the decode worker uses per-block `rms >= SIGNAL_THRESHOLD`
/// without any holdoff, these keying gaps cause dozens of false gap-edge
/// transitions — resetting the character schedule each time.
///
/// This test documents the problem: a single "SOS" transmission should produce
/// exactly 1 signal onset and 1 gap edge, but naïve per-block RMS detection
/// produces many more.
#[test]
fn cw_signal_transitions_per_block() {
    let block = 800; // typical audio callback size

    for &(msg, wpm) in &[("SOS", 13.0), ("CQ CQ CQ DE N0GNR", 13.0), ("SOS", 25.0)] {
        let mut src = CwSource::new(
            CW_DEFAULT_CARRIER_HZ,
            CW_DEFAULT_GAP_SECS,
            0.0, // no noise — isolates the keying structure
            wpm,
            0.0, // no jitter
            CW_DEFAULT_DASH_WEIGHT,
            CW_DEFAULT_CHAR_SPACE,
            CW_DEFAULT_WORD_SPACE,
            CW_DEFAULT_RISE_MS,
            CW_DEFAULT_FALL_MS,
            msg.to_owned(),
            1,
            FS,
        );

        // Run enough blocks to cover the signal + some gap.
        let total = (FS * 30.0) as usize; // 30 seconds
        let mut was_signal = false;
        let mut onset_count = 0usize;
        let mut gap_edge_count = 0usize;

        for block_start in (0..total).step_by(block) {
            let n = block.min(total - block_start);
            let samples = src.next_samples(n);
            let is_signal = rms(&samples) >= SIGNAL_THRESHOLD;
            if is_signal && !was_signal {
                onset_count += 1;
            }
            if !is_signal && was_signal {
                gap_edge_count += 1;
            }
            was_signal = is_signal;
        }

        println!("msg={msg:?} wpm={wpm}: onsets={onset_count} gap_edges={gap_edge_count}");
        // Document the problem: ideally onset_count == 1 and gap_edge_count == 1,
        // but CW keying gaps within the message cause many more transitions.
        // This test passes regardless — it's diagnostic.  The actual assertion
        // is in the round-trip decode tests below, which will fail until the
        // decode worker is fixed.
        assert!(
            onset_count >= 1,
            "msg={msg:?} wpm={wpm}: expected at least 1 signal onset"
        );
    }
}

// ── Round-trip: CW decode simulation ────────────────────────────────────────

/// Simulate the decode worker's CW arm: process CwSource block-by-block,
/// using the same holdoff logic as `CwState::process()` to ride through
/// keying gaps without resetting.  Returns the decoded characters as a String.
///
/// This mirrors the logic in `src/decode/cw.rs` but without spectral
/// analysis (Di bar) — we only care about the Dt text.
#[allow(clippy::too_many_arguments)]
fn run_cw_decode(
    src: &mut CwSource,
    message: &str,
    wpm: f32,
    dash_weight: f32,
    char_space: f32,
    word_space: f32,
    msg_repeat: usize,
    total_secs: f32,
    block: usize,
) -> CwDecodeResult {
    let total = (total_secs * FS) as usize;
    let mut decoded = String::new();
    let mut onset_count = 0usize;
    let mut gap_edge_count = 0usize;

    // CW decode state (mirrors CwState fields).
    let mut cw_char_schedule: Vec<(char, usize)> = Vec::new();
    let mut cw_accum_samples: usize = 0;
    let mut cw_next_char_idx: usize = 0;
    let mut in_signal = false;
    let mut silence_samples: usize = 0;

    // Holdoff threshold: 2× word space in samples.
    let holdoff = if wpm >= 1.0 {
        let unit_ms = 1200.0 / wpm;
        let word_gap_ms = unit_ms * word_space;
        (word_gap_ms * 2.0 * 1e-3 * FS) as usize
    } else {
        0
    };

    for block_start in (0..total).step_by(block) {
        let n = block.min(total - block_start);
        let samples = src.next_samples(n);
        let is_signal = rms(&samples) >= SIGNAL_THRESHOLD;

        if is_signal {
            silence_samples = 0;
            if !in_signal {
                in_signal = true;
                onset_count += 1;
            }
        } else {
            silence_samples += samples.len();
            if in_signal && silence_samples > holdoff {
                // Real gap — reset.
                in_signal = false;
                gap_edge_count += 1;
                cw_accum_samples = 0;
                cw_next_char_idx = 0;
                cw_char_schedule.clear();
                continue;
            }
        }

        if !in_signal {
            continue;
        }

        // Build character schedule on signal onset.
        if cw_char_schedule.is_empty() && cw_accum_samples == 0 {
            cw_char_schedule = cw_char_timing(
                message,
                wpm,
                dash_weight,
                char_space,
                word_space,
                msg_repeat,
                FS,
            );
            cw_next_char_idx = 0;
        }

        // Track accumulated samples (including keying-gap blocks) and emit
        // characters at the nominal timing boundaries.
        cw_accum_samples += samples.len();
        while cw_next_char_idx < cw_char_schedule.len() {
            let (ch, threshold) = cw_char_schedule[cw_next_char_idx];
            if cw_accum_samples >= threshold {
                decoded.push(ch);
                cw_next_char_idx += 1;
            } else {
                break;
            }
        }
    }

    CwDecodeResult {
        decoded,
        onset_count,
        gap_edge_count,
    }
}

struct CwDecodeResult {
    decoded: String,
    onset_count: usize,
    gap_edge_count: usize,
}

/// Build the expected decoded string from the character schedule, which
/// respects the `MAX_SIG_SECS` source cap that truncates slow signals.
fn expected_cw_text(message: &str, wpm: f32, msg_repeat: usize, dash_weight: f32) -> String {
    let sched = cw_char_timing(
        message,
        wpm,
        dash_weight,
        CW_DEFAULT_CHAR_SPACE,
        CW_DEFAULT_WORD_SPACE,
        msg_repeat,
        FS,
    );
    sched
        .iter()
        .map(|(ch, _)| *ch)
        .collect::<String>()
        .trim_end()
        .to_owned()
}

/// Round-trip decode of "SOS" at default WPM (clean, no noise, no jitter).
#[test]
fn cw_round_trip_clean() {
    let msg = "SOS";
    let wpm = CW_DEFAULT_WPM;
    let mut src = CwSource::new(
        CW_DEFAULT_CARRIER_HZ,
        CW_DEFAULT_GAP_SECS,
        0.0,
        wpm,
        0.0,
        CW_DEFAULT_DASH_WEIGHT,
        CW_DEFAULT_CHAR_SPACE,
        CW_DEFAULT_WORD_SPACE,
        CW_DEFAULT_RISE_MS,
        CW_DEFAULT_FALL_MS,
        msg.to_owned(),
        1,
        FS,
    );

    let result = run_cw_decode(
        &mut src,
        msg,
        wpm,
        CW_DEFAULT_DASH_WEIGHT,
        CW_DEFAULT_CHAR_SPACE,
        CW_DEFAULT_WORD_SPACE,
        1,
        30.0,
        800,
    );

    let expected = expected_cw_text(msg, wpm, 1, CW_DEFAULT_DASH_WEIGHT);
    println!(
        "clean: decoded={:?} expected={:?} onsets={} gap_edges={}",
        result.decoded, expected, result.onset_count, result.gap_edge_count
    );
    assert!(
        result.decoded.starts_with(&expected),
        "decoded text should start with expected: decoded={:?} expected={:?}",
        result.decoded,
        expected,
    );
}

/// Round-trip decode at varied WPM: 5, 13, 25 WPM.
#[test]
fn cw_round_trip_varied_wpm() {
    for &wpm in &[5.0, 13.0, 25.0] {
        let msg = "CQ DE N0GNR";
        let mut src = CwSource::new(
            CW_DEFAULT_CARRIER_HZ,
            CW_DEFAULT_GAP_SECS,
            0.0,
            wpm,
            0.0,
            CW_DEFAULT_DASH_WEIGHT,
            CW_DEFAULT_CHAR_SPACE,
            CW_DEFAULT_WORD_SPACE,
            CW_DEFAULT_RISE_MS,
            CW_DEFAULT_FALL_MS,
            msg.to_owned(),
            1,
            FS,
        );

        // 5 WPM is slow — need more time for the full message.
        let total_secs = if wpm <= 5.0 { 120.0 } else { 60.0 };
        let result = run_cw_decode(
            &mut src,
            msg,
            wpm,
            CW_DEFAULT_DASH_WEIGHT,
            CW_DEFAULT_CHAR_SPACE,
            CW_DEFAULT_WORD_SPACE,
            1,
            total_secs,
            800,
        );

        let expected = expected_cw_text(msg, wpm, 1, CW_DEFAULT_DASH_WEIGHT);
        println!(
            "wpm={wpm}: decoded={:?} expected={:?} onsets={} gap_edges={}",
            result.decoded, expected, result.onset_count, result.gap_edge_count
        );
        assert!(
            result.decoded.starts_with(&expected),
            "wpm={wpm}: decoded text should start with expected: decoded={:?} expected={:?}",
            result.decoded,
            expected,
        );
    }
}

/// Round-trip decode with noise added to the signal.
#[test]
fn cw_round_trip_with_noise() {
    let msg = "CQ CQ CQ DE N0GNR";
    let wpm = CW_DEFAULT_WPM;
    let mut src = CwSource::new(
        CW_DEFAULT_CARRIER_HZ,
        CW_DEFAULT_GAP_SECS,
        CW_DEFAULT_NOISE_AMP, // 0.05
        wpm,
        0.0,
        CW_DEFAULT_DASH_WEIGHT,
        CW_DEFAULT_CHAR_SPACE,
        CW_DEFAULT_WORD_SPACE,
        CW_DEFAULT_RISE_MS,
        CW_DEFAULT_FALL_MS,
        msg.to_owned(),
        1,
        FS,
    );

    let result = run_cw_decode(
        &mut src,
        msg,
        wpm,
        CW_DEFAULT_DASH_WEIGHT,
        CW_DEFAULT_CHAR_SPACE,
        CW_DEFAULT_WORD_SPACE,
        1,
        60.0,
        800,
    );

    let expected = expected_cw_text(msg, wpm, 1, CW_DEFAULT_DASH_WEIGHT);
    println!(
        "noise: decoded={:?} expected={:?} onsets={} gap_edges={}",
        result.decoded, expected, result.onset_count, result.gap_edge_count
    );
    assert!(
        result.decoded.starts_with(&expected),
        "decoded text with noise should start with expected: decoded={:?} expected={:?}",
        result.decoded,
        expected,
    );
}

/// Round-trip decode with msg_repeat=3.
#[test]
fn cw_round_trip_repeated_message() {
    let msg = "SOS";
    let wpm = CW_DEFAULT_WPM;
    let repeat = 3;
    let mut src = CwSource::new(
        CW_DEFAULT_CARRIER_HZ,
        CW_DEFAULT_GAP_SECS,
        0.0,
        wpm,
        0.0,
        CW_DEFAULT_DASH_WEIGHT,
        CW_DEFAULT_CHAR_SPACE,
        CW_DEFAULT_WORD_SPACE,
        CW_DEFAULT_RISE_MS,
        CW_DEFAULT_FALL_MS,
        msg.to_owned(),
        repeat,
        FS,
    );

    let result = run_cw_decode(
        &mut src,
        msg,
        wpm,
        CW_DEFAULT_DASH_WEIGHT,
        CW_DEFAULT_CHAR_SPACE,
        CW_DEFAULT_WORD_SPACE,
        repeat,
        60.0,
        800,
    );

    let expected = expected_cw_text(msg, wpm, repeat, CW_DEFAULT_DASH_WEIGHT);
    println!(
        "repeat={repeat}: decoded={:?} expected={:?} onsets={} gap_edges={}",
        result.decoded, expected, result.onset_count, result.gap_edge_count
    );
    assert!(
        result.decoded.starts_with(&expected),
        "repeated message should start with expected: decoded={:?} expected={:?}",
        result.decoded,
        expected,
    );
}

/// Round-trip decode with different block sizes (256, 800, 2048, 4096).
/// Verifies decoding is robust to audio callback granularity.
#[test]
fn cw_round_trip_varied_block_size() {
    let msg = "CQ DE N0GNR";
    let wpm = CW_DEFAULT_WPM;

    for &block in &[256, 800, 2048, 4096] {
        let mut src = CwSource::new(
            CW_DEFAULT_CARRIER_HZ,
            CW_DEFAULT_GAP_SECS,
            0.0,
            wpm,
            0.0,
            CW_DEFAULT_DASH_WEIGHT,
            CW_DEFAULT_CHAR_SPACE,
            CW_DEFAULT_WORD_SPACE,
            CW_DEFAULT_RISE_MS,
            CW_DEFAULT_FALL_MS,
            msg.to_owned(),
            1,
            FS,
        );

        let result = run_cw_decode(
            &mut src,
            msg,
            wpm,
            CW_DEFAULT_DASH_WEIGHT,
            CW_DEFAULT_CHAR_SPACE,
            CW_DEFAULT_WORD_SPACE,
            1,
            60.0,
            block,
        );

        let expected = expected_cw_text(msg, wpm, 1, CW_DEFAULT_DASH_WEIGHT);
        println!(
            "block={block}: decoded={:?} expected={:?} onsets={} gap_edges={}",
            result.decoded, expected, result.onset_count, result.gap_edge_count
        );
        assert!(
            result.decoded.starts_with(&expected),
            "block={block}: decoded text should start with expected: decoded={:?} expected={:?}",
            result.decoded,
            expected,
        );
    }
}

// ── Slow round-trip tests (run with `cargo test -- --ignored`) ──────────────

/// Multi-loop decode at 5 WPM with per-loop diagnostics.
///
/// At 5 WPM the keying is very slow (~240 ms per dot unit), so the full
/// message "CQ CQ CQ DE N0GNR" × 3 repeats takes ~100+ seconds per loop.
/// This test runs several loops to verify that every loop decodes the
/// complete message without truncation at the gap boundary.
///
#[test]
fn cw_round_trip_slow_5wpm() {
    let msg = "CQ CQ CQ DE N0GNR";
    let wpm = 5.0_f32;
    let repeat = 3;
    let gap_secs = 10.0;
    let block = 800;
    let loops = 3;

    let mut src = CwSource::new(
        CW_DEFAULT_CARRIER_HZ,
        gap_secs,
        0.0, // no noise
        wpm,
        0.0, // no jitter — isolate timing issues
        CW_DEFAULT_DASH_WEIGHT,
        CW_DEFAULT_CHAR_SPACE,
        CW_DEFAULT_WORD_SPACE,
        CW_DEFAULT_RISE_MS,
        CW_DEFAULT_FALL_MS,
        msg.to_owned(),
        repeat,
        FS,
    );

    let expected_per_loop = expected_cw_text(msg, wpm, repeat, CW_DEFAULT_DASH_WEIGHT);
    println!(
        "expected per loop: {:?} ({} chars)",
        expected_per_loop,
        expected_per_loop.len()
    );

    // Estimate total duration: message body + gap per loop.
    // At 5 WPM, 1 unit = 240 ms.  Rough estimate: ~150 units per "CQ CQ CQ DE N0GNR",
    // × 3 repeats ≈ 450 units + word gaps ≈ 500 units ≈ 120s.  Plus 10s gap.
    let secs_per_loop = 140.0 + gap_secs;
    let total_secs = secs_per_loop * loops as f32 + 20.0; // margin
    let total = (total_secs * FS) as usize;
    println!("total duration: {total_secs:.0}s ({total} samples), {loops} loops");

    // Holdoff: 2× word space.
    let unit_ms = 1200.0 / wpm;
    let holdoff = (unit_ms * CW_DEFAULT_WORD_SPACE * 2.0 * 1e-3 * FS) as usize;

    // Decode state.
    let mut cw_char_schedule: Vec<(char, usize)> = Vec::new();
    let mut cw_accum_samples: usize = 0;
    let mut cw_next_char_idx: usize = 0;
    let mut in_signal = false;
    let mut silence_samples: usize = 0;

    // Per-loop collection.
    let mut current_loop_text = String::new();
    let mut loop_texts: Vec<String> = Vec::new();
    let mut loop_num = 0usize;

    let mut total_processed = 0usize;

    for block_start in (0..total).step_by(block) {
        let n = block.min(total - block_start);
        let samples = src.next_samples(n);
        total_processed += n;
        let t_secs = total_processed as f32 / FS;
        let is_signal = rms(&samples) >= SIGNAL_THRESHOLD;

        if is_signal {
            silence_samples = 0;
            if !in_signal {
                in_signal = true;
                loop_num += 1;
                println!("t={t_secs:7.1}s  [ONSET] loop {loop_num}");
            }
        } else {
            silence_samples += samples.len();
            if in_signal && silence_samples > holdoff {
                in_signal = false;
                // Emit any remaining scheduled characters.
                while cw_next_char_idx < cw_char_schedule.len() {
                    let (ch, threshold) = cw_char_schedule[cw_next_char_idx];
                    println!(
                        "  MISSED char {:?} threshold={} accum={}  (delta={})",
                        ch,
                        threshold,
                        cw_accum_samples,
                        threshold as i64 - cw_accum_samples as i64,
                    );
                    cw_next_char_idx += 1;
                    _ = threshold;
                }
                let sched_len = cw_char_schedule.len();
                let decoded_count = current_loop_text.len();
                println!(
                    "t={t_secs:7.1}s  [GAP]   loop {loop_num}: decoded {decoded_count}/{sched_len} chars: {:?}",
                    &current_loop_text
                );
                loop_texts.push(std::mem::take(&mut current_loop_text));
                cw_accum_samples = 0;
                cw_next_char_idx = 0;
                cw_char_schedule.clear();
                continue;
            }
        }

        if !in_signal {
            continue;
        }

        if cw_char_schedule.is_empty() && cw_accum_samples == 0 {
            cw_char_schedule = cw_char_timing(
                msg,
                wpm,
                CW_DEFAULT_DASH_WEIGHT,
                CW_DEFAULT_CHAR_SPACE,
                CW_DEFAULT_WORD_SPACE,
                repeat,
                FS,
            );
            cw_next_char_idx = 0;
            println!(
                "  schedule: {} entries, last threshold={}",
                cw_char_schedule.len(),
                cw_char_schedule.last().map_or(0, |e| e.1),
            );
        }

        cw_accum_samples += samples.len();
        while cw_next_char_idx < cw_char_schedule.len() {
            let (ch, threshold) = cw_char_schedule[cw_next_char_idx];
            if cw_accum_samples >= threshold {
                current_loop_text.push(ch);
                cw_next_char_idx += 1;
            } else {
                break;
            }
        }
    }

    // Flush any in-progress loop.
    if !current_loop_text.is_empty() {
        println!(
            "  [END] partial loop {}: {:?}",
            loop_num + 1,
            current_loop_text
        );
        loop_texts.push(current_loop_text);
    }

    println!("\n── Per-loop results ──");
    for (i, text) in loop_texts.iter().enumerate() {
        let matches = text == &expected_per_loop;
        let status = if matches { "OK" } else { "MISMATCH" };
        println!("  loop {}: [{status}] {:?}", i + 1, text);
    }

    // Assert that at least `loops` complete loops decoded correctly.
    let complete = loop_texts
        .iter()
        .filter(|t| t.starts_with(&expected_per_loop))
        .count();
    assert!(
        complete >= loops,
        "expected >= {loops} complete loop decodes, got {complete}",
    );
}
