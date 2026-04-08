//! Integration tests for FT8/FT4 source generation and decode.

use num_complex::Complex32 as C32;
use orion_sdr::util::rms;
use orion_sdr::codec::Ft8StreamDecoder;
use orion_sdr_view::source::{Ft8Source, Ft8Mode, Ft8MsgType, SignalSource};

const FS: f32 = 48_000.0;
const CARRIER_HZ: f32 = 1_500.0;

/// FT8 frame length at 48 kHz (12 kHz native × 4).
/// 12.64 s × 12_000 = 151_680 native samples; × 4 = 606_720 at 48 kHz.
const FT8_FRAME_LEN_48K: usize = 606_720;

/// FT4 frame length at 48 kHz.
/// 5.04 s × 12_000 = 60_480 native samples; × 4 = 241_920 at 48 kHz.
const FT4_FRAME_LEN_48K: usize = 241_920;

// ── Source construction helpers ───────────────────────────────────────────────

fn make_ft8_source(repeat: usize, loop_gap_secs: f32) -> Ft8Source {
    Ft8Source::new(
        CARRIER_HZ,
        loop_gap_secs,
        0.0,
        Ft8Mode::Ft8,
        Ft8MsgType::Standard,
        "CQ".to_owned(),
        "N0GNR".to_owned(),
        "FN31".to_owned(),
        "CQ DX".to_owned(),
        repeat,
        FS,
    )
}

fn make_ft4_source(repeat: usize, loop_gap_secs: f32) -> Ft8Source {
    Ft8Source::new(
        CARRIER_HZ,
        loop_gap_secs,
        0.0,
        Ft8Mode::Ft4,
        Ft8MsgType::Standard,
        "CQ".to_owned(),
        "N0GNR".to_owned(),
        "FN31".to_owned(),
        "CQ DX".to_owned(),
        repeat,
        FS,
    )
}

// ── Source unit tests ─────────────────────────────────────────────────────────

#[test]
fn ft8_source_sample_rate() {
    let src = make_ft8_source(1, 0.0);
    assert_eq!(src.sample_rate(), FS);
}

#[test]
fn ft4_source_sample_rate() {
    let src = make_ft4_source(1, 0.0);
    assert_eq!(src.sample_rate(), FS);
}

/// One FT8 frame (repeat=1, no gap) should produce exactly FT8_FRAME_LEN_48K
/// samples with non-zero power.
#[test]
fn ft8_source_frame_length() {
    let mut src = make_ft8_source(1, 0.0);
    let samples = src.next_samples(FT8_FRAME_LEN_48K);
    assert_eq!(samples.len(), FT8_FRAME_LEN_48K);
    let power: f32 = samples.iter().map(|&s| s * s).sum::<f32>() / samples.len() as f32;
    assert!(power > 0.01, "FT8 frame has near-zero power: {power:.4}");
}

/// One FT4 frame (repeat=1, no gap) should produce exactly FT4_FRAME_LEN_48K
/// samples with non-zero power.
#[test]
fn ft4_source_frame_length() {
    let mut src = make_ft4_source(1, 0.0);
    let samples = src.next_samples(FT4_FRAME_LEN_48K);
    assert_eq!(samples.len(), FT4_FRAME_LEN_48K);
    let power: f32 = samples.iter().map(|&s| s * s).sum::<f32>() / samples.len() as f32;
    assert!(power > 0.01, "FT4 frame has near-zero power: {power:.4}");
}

/// With repeat=2 and no gap the source emits two consecutive signal frames
/// before going silent.
#[test]
fn ft8_source_repeat_two_frames() {
    let mut src = make_ft8_source(2, 0.0);
    // First frame should be signal.
    let frame1 = src.next_samples(FT8_FRAME_LEN_48K);
    assert!(rms(&frame1) > 0.1, "frame 1 should be non-silent");
    // Second frame should also be signal.
    let frame2 = src.next_samples(FT8_FRAME_LEN_48K);
    assert!(rms(&frame2) > 0.1, "frame 2 should be non-silent");
    // Gap (50 ms) should be silent (no noise_amp).
    let gap = src.next_samples(100);
    for (i, &s) in gap.iter().enumerate() {
        assert_eq!(s, 0.0, "gap sample {i} should be 0.0");
    }
}

/// With a 1-second gap the source emits signal then 1 s of silence then signal.
#[test]
fn ft8_source_loop_gap_timing() {
    const GAP_SECS: f32 = 1.0;
    let gap_samples = (GAP_SECS * FS) as usize;

    let mut src = make_ft8_source(1, GAP_SECS);
    // Signal frame.
    let frame = src.next_samples(FT8_FRAME_LEN_48K);
    assert!(rms(&frame) > 0.1, "signal frame should be non-silent");
    // Gap should be silent.
    let gap = src.next_samples(gap_samples);
    for (i, &s) in gap.iter().enumerate() {
        assert_eq!(s, 0.0, "gap sample {i} should be 0.0");
    }
    // Next frame (second loop) should be signal again.
    let frame2 = src.next_samples(FT8_FRAME_LEN_48K);
    assert!(rms(&frame2) > 0.1, "second loop frame should be non-silent");
}

/// Noise amp adds non-zero samples during the gap period.
#[test]
fn ft8_source_noise_in_gap() {
    let mut src = Ft8Source::new(
        CARRIER_HZ, 1.0, 0.05,
        Ft8Mode::Ft8, Ft8MsgType::Standard,
        "CQ".to_owned(), "N0GNR".to_owned(), "FN31".to_owned(), "CQ DX".to_owned(),
        1, FS,
    );
    let _frame = src.next_samples(FT8_FRAME_LEN_48K);
    let gap = src.next_samples(4096);
    let has_noise = gap.iter().any(|&s| s.abs() > 1e-6);
    assert!(has_noise, "gap should contain noise when noise_amp > 0");
}

/// `restart()` resets the source so it plays the frame from the beginning.
#[test]
fn ft8_source_restart() {
    let mut src = make_ft8_source(1, 0.0);
    let a = src.next_samples(1024);
    src.restart();
    let b = src.next_samples(1024);
    assert_eq!(a, b, "samples after restart should match initial samples");
}

// ── Decode integration test ───────────────────────────────────────────────────

/// Modulate a standard FT8 CQ message, downsample 4:1, feed into
/// `Ft8StreamDecoder`, and verify that the decoded text contains the callsign.
///
/// This exercises the full encode → modulate (Ft8Source) → downsample →
/// Ft8StreamDecoder chain that the decode worker uses.
#[test]
fn ft8_decode_standard_message() {
    const CALL_DE: &str = "N0GNR";
    const CALL_TO: &str = "CQ";

    let mut src = Ft8Source::new(
        CARRIER_HZ, 0.0, 0.0,
        Ft8Mode::Ft8, Ft8MsgType::Standard,
        CALL_TO.to_owned(), CALL_DE.to_owned(), "FN31".to_owned(), String::new(),
        1, FS,
    );

    // Collect exactly one frame at 48 kHz.
    let samples_48k = src.next_samples(FT8_FRAME_LEN_48K);

    // Downsample 4:1 to 12 kHz (take every 4th sample — matches decode worker).
    let iq_12k: Vec<C32> = samples_48k
        .iter()
        .step_by(4)
        .map(|&s| C32::new(s, 0.0))
        .collect();

    // Feed into Ft8StreamDecoder.  Search ±50 Hz around CARRIER_HZ.
    let mut dec = Ft8StreamDecoder::new_ft8(
        12_000.0,
        CARRIER_HZ - 50.0,
        CARRIER_HZ + 50.0,
        8,
    );

    let results = dec.feed(&iq_12k);

    assert!(
        !results.is_empty(),
        "Ft8StreamDecoder: expected at least one decode result from noiseless frame"
    );

    // The decoded message should contain the callsign.
    let msg_text = format!("{:?}", results[0].message);
    println!("FT8 decoded: {msg_text}");
    assert!(
        msg_text.contains(CALL_DE),
        "decoded message should contain {CALL_DE}: {msg_text}"
    );
}

/// Same test for FT4.
#[test]
fn ft4_decode_standard_message() {
    const CALL_DE: &str = "N0GNR";
    const CALL_TO: &str = "CQ";

    let mut src = Ft8Source::new(
        CARRIER_HZ, 0.0, 0.0,
        Ft8Mode::Ft4, Ft8MsgType::Standard,
        CALL_TO.to_owned(), CALL_DE.to_owned(), "FN31".to_owned(), String::new(),
        1, FS,
    );

    let samples_48k = src.next_samples(FT4_FRAME_LEN_48K);
    let iq_12k: Vec<C32> = samples_48k
        .iter()
        .step_by(4)
        .map(|&s| C32::new(s, 0.0))
        .collect();

    let mut dec = Ft8StreamDecoder::new_ft4(
        12_000.0,
        CARRIER_HZ - 50.0,
        CARRIER_HZ + 50.0,
        8,
    );

    let results = dec.feed(&iq_12k);

    assert!(
        !results.is_empty(),
        "Ft8StreamDecoder (FT4): expected at least one decode result from noiseless frame"
    );

    let msg_text = format!("{:?}", results[0].message);
    println!("FT4 decoded: {msg_text}");
    assert!(
        msg_text.contains(CALL_DE),
        "FT4 decoded message should contain {CALL_DE}: {msg_text}"
    );
}
