// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Integration tests for the CW source.

use orion_sdr_view::source::cw::{
    CW_DEFAULT_CARRIER_HZ, CW_DEFAULT_CHAR_SPACE, CW_DEFAULT_DASH_WEIGHT, CW_DEFAULT_FALL_MS,
    CW_DEFAULT_GAP_SECS, CW_DEFAULT_JITTER_PCT, CW_DEFAULT_NOISE_AMP, CW_DEFAULT_REPEAT,
    CW_DEFAULT_RISE_MS, CW_DEFAULT_WPM, CW_DEFAULT_WORD_SPACE, CwSource,
};
use orion_sdr_view::source::SignalSource;

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
        1.0, // 1 second gap
        0.0, // no noise
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
    assert!(
        max_abs < 1e-6,
        "expected silence in gap, max_abs={max_abs}"
    );
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
