// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Integration tests for FT8/FT4 source generation and decode.

use num_complex::Complex32 as C32;
use orion_sdr::codec::Ft8StreamDecoder;
use orion_sdr::util::rms;
use orion_sdr_view::decode::{DecodeResult, FT8_BW_HZ};
use orion_sdr_view::source::ft8::FT8_MOD_BASE_HZ;
use orion_sdr_view::source::{Ft8Mode, Ft8MsgType, Ft8Source, SignalSource};

mod common;
use common::ticker::{BufferDecode, TickerSimConfig, run_ticker_sim};

const FS: f32 = 48_000.0;
const CARRIER_HZ: f32 = 1_500.0;

/// FT8 frame length at 48 kHz (12 kHz native × 4).
/// 12.64 s × 12_000 = 151_680 native samples; × 4 = 606_720 at 48 kHz.
const FT8_FRAME_LEN_48K: usize = 606_720;

/// FT4 frame length at 48 kHz.
/// 5.04 s × 12_000 = 60_480 native samples; × 4 = 241_920 at 48 kHz.
const FT4_FRAME_LEN_48K: usize = 241_920;

// ── Source construction helpers ───────────────────────────────────────────────

fn make_ft8_source(repeat: usize, gap_secs: f32) -> Ft8Source {
    Ft8Source::new(
        CARRIER_HZ,
        gap_secs,
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

fn make_ft4_source(repeat: usize, gap_secs: f32) -> Ft8Source {
    Ft8Source::new(
        CARRIER_HZ,
        gap_secs,
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
        CARRIER_HZ,
        1.0,
        0.05,
        Ft8Mode::Ft8,
        Ft8MsgType::Standard,
        "CQ".to_owned(),
        "N0GNR".to_owned(),
        "FN31".to_owned(),
        "CQ DX".to_owned(),
        1,
        FS,
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
        CARRIER_HZ,
        0.0,
        0.0,
        Ft8Mode::Ft8,
        Ft8MsgType::Standard,
        CALL_TO.to_owned(),
        CALL_DE.to_owned(),
        "FN31".to_owned(),
        String::new(),
        1,
        FS,
    );

    // Collect exactly one frame at 48 kHz.
    let samples_48k = src.next_samples(FT8_FRAME_LEN_48K);

    // Downsample 4:1 to 12 kHz (take every 4th sample — matches decode worker).
    let iq_12k: Vec<C32> = samples_48k
        .iter()
        .step_by(4)
        .map(|&s| C32::new(s, 0.0))
        .collect();

    // Source renders at CARRIER_HZ in the native 12 kHz domain; search there.
    let mut dec = Ft8StreamDecoder::new_ft8(12_000.0, CARRIER_HZ - 50.0, CARRIER_HZ + 50.0, 8);

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
        CARRIER_HZ,
        0.0,
        0.0,
        Ft8Mode::Ft4,
        Ft8MsgType::Standard,
        CALL_TO.to_owned(),
        CALL_DE.to_owned(),
        "FN31".to_owned(),
        String::new(),
        1,
        FS,
    );

    let samples_48k = src.next_samples(FT4_FRAME_LEN_48K);
    let iq_12k: Vec<C32> = samples_48k
        .iter()
        .step_by(4)
        .map(|&s| C32::new(s, 0.0))
        .collect();

    let mut dec = Ft8StreamDecoder::new_ft4(12_000.0, CARRIER_HZ - 50.0, CARRIER_HZ + 50.0, 8);

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

// ── Decode with frequency shift (12 kHz carrier) ─────────────────────────────

/// Modulate at 12 kHz carrier (which internally shifts from 1500 Hz base),
/// then reverse the shift before decimation — matching the decode worker path.
#[test]
fn ft8_decode_shifted_12khz_carrier() {
    const CALL_DE: &str = "N0GNR";
    const SHIFTED_CARRIER: f32 = 12_000.0;

    let mut src = Ft8Source::new(
        SHIFTED_CARRIER,
        0.0,
        0.0,
        Ft8Mode::Ft8,
        Ft8MsgType::Standard,
        "CQ".to_owned(),
        CALL_DE.to_owned(),
        "FN31".to_owned(),
        String::new(),
        1,
        FS,
    );

    let samples_48k = src.next_samples(FT8_FRAME_LEN_48K);

    // Reverse the frequency shift: multiply by exp(-j*2π*shift*t), decimate 4:1.
    let shift_hz = SHIFTED_CARRIER - FT8_MOD_BASE_HZ;
    let phase_inc = 2.0 * std::f32::consts::PI * shift_hz / FS;
    let iq_12k: Vec<C32> = samples_48k
        .iter()
        .enumerate()
        .step_by(4)
        .map(|(i, &s)| {
            let phase = phase_inc * i as f32;
            C32::new(s * phase.cos(), -s * phase.sin())
        })
        .collect();

    let mut dec =
        Ft8StreamDecoder::new_ft8(12_000.0, FT8_MOD_BASE_HZ - 50.0, FT8_MOD_BASE_HZ + 50.0, 8);

    let results = dec.feed(&iq_12k);

    assert!(
        !results.is_empty(),
        "Ft8StreamDecoder: expected decode from 12 kHz shifted frame"
    );

    let msg_text = format!("{:?}", results[0].message);
    println!("FT8 shifted decode: {msg_text}");
    assert!(
        msg_text.contains(CALL_DE),
        "decoded message should contain {CALL_DE}: {msg_text}"
    );
}

/// Same shifted decode test for FT4.
#[test]
fn ft4_decode_shifted_12khz_carrier() {
    const CALL_DE: &str = "N0GNR";
    const SHIFTED_CARRIER: f32 = 12_000.0;

    let mut src = Ft8Source::new(
        SHIFTED_CARRIER,
        0.0,
        0.0,
        Ft8Mode::Ft4,
        Ft8MsgType::Standard,
        "CQ".to_owned(),
        CALL_DE.to_owned(),
        "FN31".to_owned(),
        String::new(),
        1,
        FS,
    );

    let samples_48k = src.next_samples(FT4_FRAME_LEN_48K);

    let shift_hz = SHIFTED_CARRIER - FT8_MOD_BASE_HZ;
    let phase_inc = 2.0 * std::f32::consts::PI * shift_hz / FS;
    let iq_12k: Vec<C32> = samples_48k
        .iter()
        .enumerate()
        .step_by(4)
        .map(|(i, &s)| {
            let phase = phase_inc * i as f32;
            C32::new(s * phase.cos(), -s * phase.sin())
        })
        .collect();

    let mut dec =
        Ft8StreamDecoder::new_ft4(12_000.0, FT8_MOD_BASE_HZ - 50.0, FT8_MOD_BASE_HZ + 50.0, 8);

    let results = dec.feed(&iq_12k);

    assert!(
        !results.is_empty(),
        "Ft8StreamDecoder (FT4): expected decode from 12 kHz shifted frame"
    );

    let msg_text = format!("{:?}", results[0].message);
    println!("FT4 shifted decode: {msg_text}");
    assert!(
        msg_text.contains(CALL_DE),
        "FT4 decoded message should contain {CALL_DE}: {msg_text}"
    );
}

// ── Dt ticker simulation ─────────────────────────────────────────────────────

/// Decode an accumulated 48 kHz IQ buffer containing one or more FT8 frames
/// at `carrier_hz`: reverse the shift to FT8_MOD_BASE_HZ, decimate 4:1 to
/// 12 kHz, feed Ft8StreamDecoder, and return Info + (optional) Text results
/// for the ticker harness.
fn decode_ft8_buffer(iq_48k: &[C32], carrier_hz: f32) -> BufferDecode {
    let shift_hz = carrier_hz - FT8_MOD_BASE_HZ;
    let phase_inc = 2.0 * std::f32::consts::PI * shift_hz / FS;
    let iq_12k: Vec<C32> = iq_48k
        .iter()
        .enumerate()
        .step_by(4)
        .map(|(i, c)| {
            let phase = phase_inc * i as f32;
            let (sin_p, cos_p) = phase.sin_cos();
            C32::new(c.re * cos_p, -c.re * sin_p)
        })
        .collect();

    let mut dec = Ft8StreamDecoder::new_ft8(
        12_000.0,
        FT8_MOD_BASE_HZ - 200.0,
        FT8_MOD_BASE_HZ + 200.0,
        8,
    );
    let results = dec.feed(&iq_12k);
    let tail = dec.flush();

    let mut text_out = String::new();
    for r in results.iter().chain(tail.iter()) {
        let s = format!("{:?}", r.message);
        if !text_out.is_empty() {
            text_out.push(' ');
        }
        text_out.push_str(&s);
    }

    let info = DecodeResult::Info {
        modulation: "FT8".to_owned(),
        center_hz: carrier_hz,
        bw_hz: FT8_BW_HZ,
        snr_db: 0.0,
    };
    let text = if text_out.is_empty() {
        None
    } else {
        Some(DecodeResult::Text(text_out))
    };
    BufferDecode {
        info: Some(info),
        text,
    }
}

/// Simulate the Dt mode ticker for FT8: feed Ft8Source through the shared
/// ticker harness, decode each accumulated burst, and print what the ticker
/// would display.  Parameters: repeat=2, gap=1 s, two full loops.
#[test]
fn ft8_simulate_dt_ticker() {
    const REPEAT: usize = 2;
    const GAP_SECS: f32 = 1.0;
    const LOOPS: usize = 2;
    const BLOCK: usize = 1024;

    let mut src = make_ft8_source(REPEAT, GAP_SECS);

    // One FT8 frame = 12.64 s.  Each loop emits REPEAT frames (~25 s) + GAP.
    let loop_secs = 12.64 * REPEAT as f32 + GAP_SECS;
    let total_samples = ((loop_secs * LOOPS as f32 + 1.0) * FS) as usize;

    let cfg = TickerSimConfig {
        label: "FT8 standard CQ",
        block: BLOCK,
        total_samples,
        fs: FS,
        // FT8 frames are long; rely on gap-edge decoding only.
        max_accum: None,
    };

    let result = run_ticker_sim(&mut src, &cfg, |iq| decode_ft8_buffer(iq, CARRIER_HZ));

    assert!(
        result.gap_decodes >= 1,
        "expected at least one gap-edge decode, got {}",
        result.gap_decodes,
    );
    // The decoded Ft8Message Debug output includes the call_de callsign as a
    // literal string.  Over the ~27 s simulated run, the ticker's scroll
    // animation has time to move those characters from `pending` into
    // `visible`, so the final visible buffer should contain "N0GNR".
    assert!(
        result.ticker.visible.contains("N0GNR"),
        "expected ticker.visible to contain decoded callsign N0GNR, got: {:?}",
        result.ticker.visible,
    );
}

// ── Streaming decode (incremental feed across multiple frames) ────────────────

/// Drive an Ft8Source in small 800-sample blocks through the same decode path
/// the worker uses: incremental carrier-shift reversal (phase accumulator
/// wrapped to `[0, 2π)`), 4:1 decimation to 12 kHz, and `Ft8StreamDecoder::feed`.
/// Gap edges trigger `flush()` + `clear()`.  Over 2 loops × 2 frames the
/// decoder should produce at least 2 `N0GNR` hits — one per loop, regardless
/// of whether both per-loop frames decode individually.
///
/// This exercises the *streaming* contract (block-by-block feeding across
/// frame boundaries and gaps), complementing the one-shot decode tests above
/// which hand `Ft8StreamDecoder` a single complete frame in one call.
#[test]
fn ft8_streaming_decode_multi_loop() {
    const REPEAT: usize = 2;
    const GAP_SECS: f32 = 0.5;
    const LOOPS: usize = 2;
    const BLOCK: usize = 800;

    let mut src = make_ft8_source(REPEAT, GAP_SECS);

    // One FT8 frame = 12.64 s.  Each loop emits REPEAT frames + GAP, plus
    // a small tail so the final gap edge is seen.
    let loop_secs = 12.64 * REPEAT as f32 + GAP_SECS;
    let total_samples = ((loop_secs * LOOPS as f32 + 1.0) * FS) as usize;

    // Incremental phase accumulator for the carrier-shift reversal.
    let shift_hz = CARRIER_HZ - FT8_MOD_BASE_HZ;
    let phase_inc = 2.0 * std::f32::consts::PI * shift_hz / FS;
    let two_pi = 2.0 * std::f32::consts::PI;
    let mut phase = 0.0_f32;
    // Absolute sample index into the source stream, used for i % 4 decimation.
    let mut sample_idx: usize = 0;

    let mut dec = Ft8StreamDecoder::new_ft8(
        12_000.0,
        FT8_MOD_BASE_HZ - 200.0,
        FT8_MOD_BASE_HZ + 200.0,
        8,
    );

    let mut was_signal = false;
    let mut all_messages: Vec<String> = Vec::new();
    let mut gap_edges = 0usize;

    let push_results = |results: Vec<orion_sdr::codec::Ft8DecodeResult>,
                        tag: &str,
                        all_messages: &mut Vec<String>| {
        for r in results {
            let s = format!("{:?}", r.message);
            println!("  [{tag}] {s}");
            all_messages.push(s);
        }
    };

    let mut remaining = total_samples;
    while remaining > 0 {
        let n = BLOCK.min(remaining);
        remaining -= n;
        let samples = src.next_samples(n);

        let is_signal = rms(&samples) >= 0.01;
        let gap_edge = !is_signal && was_signal;
        was_signal = is_signal;

        if gap_edge {
            gap_edges += 1;
            let flush_results = dec.flush();
            println!(
                "[gap #{gap_edges}] flush -> {} results",
                flush_results.len()
            );
            push_results(flush_results, "flush", &mut all_messages);
            dec.clear();
        }

        if is_signal {
            // Reverse the carrier shift and decimate 4:1, in the same
            // incremental fashion as the decode worker.
            let mut downsampled: Vec<C32> = Vec::with_capacity(n / 4 + 1);
            for (k, s) in samples.iter().enumerate() {
                let abs_idx = sample_idx + k;
                if abs_idx.is_multiple_of(4) {
                    let (sin_p, cos_p) = phase.sin_cos();
                    downsampled.push(C32::new(s * cos_p, -s * sin_p));
                }
                phase += phase_inc;
                if phase >= two_pi {
                    phase -= two_pi;
                } else if phase < 0.0 {
                    phase += two_pi;
                }
            }
            let results = dec.feed(&downsampled);
            if !results.is_empty() {
                println!("[mid-signal] feed -> {} results", results.len());
                push_results(results, "feed", &mut all_messages);
                // Mirror the worker: clear so flush() at gap doesn't re-decode.
                dec.clear();
            }
        } else {
            // Still advance phase and sample index across silent blocks so
            // the shift stays coherent with the source timeline.
            for _ in 0..n {
                phase += phase_inc;
                if phase >= two_pi {
                    phase -= two_pi;
                } else if phase < 0.0 {
                    phase += two_pi;
                }
            }
        }

        sample_idx += n;
    }

    println!(
        "total decoded messages: {}  gap_edges: {}",
        all_messages.len(),
        gap_edges,
    );

    let callsign_hits = all_messages.iter().filter(|s| s.contains("N0GNR")).count();
    assert!(
        callsign_hits >= LOOPS,
        "expected ≥ {LOOPS} decoded messages containing N0GNR (one per loop), got {callsign_hits}: {all_messages:?}"
    );
}
