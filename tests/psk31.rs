//! Integration tests for the PSK31 source and its decode pipeline.

use num_complex::Complex32 as C32;
use orion_sdr::Block;
use orion_sdr::demodulate::psk31::{Bpsk31Demod, Bpsk31Decider};
use orion_sdr::codec::varicode::VaricodeDecoder;
use orion_sdr::sync::psk31_sync::psk31_sync;
use orion_sdr::modulate::psk31::{psk31_sps, PSK31_BAUD};

use orion_sdr_view::decode::{
    DecodeMode, DecodeResult,
    Psk31Stream, SIGNAL_THRESHOLD, PSK31_MAX_ACCUM_SYMS,
    PSK31_BW_HZ, SYNC_SEARCH_HZ, SYNC_MIN_SYMS,
    best_sync, spectrum_snr_db,
};
use orion_sdr_view::source::{Psk31Source, Psk31Mode, SignalSource};

mod common;
use common::ticker::{BufferDecode, TickerSimConfig, run_ticker_sim};

const FS: f32 = 48_000.0;
const CARRIER_HZ: f32 = 12_000.0;

// ── Shared helpers ─────────────────────────────────────────────────────────────

/// Decode a BPSK31 IQ buffer: sync, demod, decide, varicode.
/// Returns (Info result, optional Text result).
fn decode_bpsk31(
    iq: &[C32],
    carrier_hz: f32,
    fs: f32,
    sps: usize,
) -> (DecodeResult, Option<DecodeResult>) {
    let base_hz = (carrier_hz - SYNC_SEARCH_HZ).max(0.0);
    let max_hz  = carrier_hz + SYNC_SEARCH_HZ;
    let results = psk31_sync(iq, fs, base_hz, max_hz, 4, 1.5, 256, 20);

    println!("  psk31_sync: {} candidates", results.len());
    for r in &results {
        println!("    cand: carrier_hz={:.1} time_sym={} score={:.2}", r.carrier_hz, r.time_sym, r.score);
    }

    let real: Vec<f32> = iq.iter().map(|c| c.re).collect();

    let (found_hz, time_sym) = match best_sync(&results, carrier_hz, PSK31_BAUD) {
        Some(r) => r,
        None => {
            let snr = spectrum_snr_db(&real, fs, carrier_hz);
            return (DecodeResult::Info {
                modulation: "BPSK31".to_owned(),
                center_hz:  carrier_hz,
                bw_hz:      PSK31_BW_HZ,
                snr_db:     snr,
            }, None);
        }
    };

    let snr = spectrum_snr_db(&real, fs, found_hz);
    let info = DecodeResult::Info {
        modulation: "BPSK31".to_owned(),
        center_hz:  found_hz,
        bw_hz:      PSK31_BW_HZ,
        snr_db:     snr,
    };

    let scan_end = ((time_sym + 2) * sps).min(iq.len());
    let start = iq[..scan_end]
        .iter()
        .position(|c| c.re * c.re + c.im * c.im > 0.01)
        .unwrap_or(0);
    let max_syms = (iq.len() - start) / sps + 2;
    println!("  bpsk31: found_hz={found_hz:.1} time_sym={time_sym} start={start} max_syms={max_syms} iq.len={}", iq.len());

    let mut soft = vec![0.0_f32; max_syms];
    let wr = Bpsk31Demod::new(fs, carrier_hz, 1.0).process(&iq[start..], &mut soft);
    soft.truncate(wr.out_written);

    let mut bits = vec![0_u8; soft.len()];
    let dr = Bpsk31Decider::new().process(&soft, &mut bits);
    bits.truncate(dr.out_written);

    let text = varicode_decode_bits(&bits);
    println!("  bpsk31: bits.len={} text={:?}", bits.len(), &text[..text.len().min(40)]);
    if text.is_empty() {
        (info, None)
    } else {
        (info, Some(DecodeResult::Text(text)))
    }
}

/// Push bits through a VaricodeDecoder, flushing with two trailing zeros.
fn varicode_decode_bits(bits: &[u8]) -> String {
    let mut vdec = VaricodeDecoder::new();
    for &b in bits { vdec.push_bit(b); }
    vdec.push_bit(0);
    vdec.push_bit(0);
    let mut text = String::new();
    while let Some(ch) = vdec.pop_char() {
        if (0x20..0x7f).contains(&ch) {
            text.push(ch as char);
        }
    }
    text
}

/// Run the streaming decode pipeline on a Psk31Source, return decoded text.
fn run_streaming_decode(
    mode: Psk31Mode,
    msg: &str, repeat: usize, loops: usize, loop_gap: f32,
) -> String {
    use orion_sdr::util::rms;

    let sps = psk31_sps(FS);
    let carrier_hz = CARRIER_HZ;
    let decode_mode = match mode {
        Psk31Mode::Bpsk31 => DecodeMode::Bpsk31,
        Psk31Mode::Qpsk31 => DecodeMode::Qpsk31,
    };

    let mut src = Psk31Source::new(
        carrier_hz, loop_gap, 0.0, mode,
        msg.to_owned(), repeat, FS,
    );

    let mut iq_buf: Vec<C32> = Vec::new();
    let mut stream: Option<Psk31Stream> = None;
    let mut was_signal = false;
    let mut all_text = String::new();

    let text_bytes = (msg.len() * repeat + repeat.saturating_sub(1)) as f32;
    let approx_signal_secs = (64.0 + text_bytes * 11.0 + 32.0) / 31.25;
    let total_samples = ((approx_signal_secs + loop_gap) * loops as f32 + 2.0) * FS;
    let margin = if decode_mode == DecodeMode::Bpsk31 { 1.5 } else { 3.0 };

    for _ in (0..total_samples as usize).step_by(800) {
        let samples = src.next_samples(800);
        let is_signal = rms(&samples) >= SIGNAL_THRESHOLD;
        let gap_edge = !is_signal && was_signal;
        was_signal = is_signal;

        if gap_edge {
            if let Some(ref mut s) = stream {
                if s.fed_up_to() < iq_buf.len() {
                    let text = s.feed(&iq_buf[s.fed_up_to()..]);
                    if !text.is_empty() { all_text.push_str(&text); }
                }
                let tail = s.flush();
                if !tail.is_empty() { all_text.push_str(&tail); }
            }
            stream = None;
            iq_buf.clear();
            continue;
        }

        if !is_signal { continue; }

        iq_buf.extend(samples.iter().map(|&s| C32::new(s, 0.0)));

        if stream.is_none() && iq_buf.len() >= sps * SYNC_MIN_SYMS {
            let base_hz = (carrier_hz - SYNC_SEARCH_HZ).max(0.0);
            let max_hz  = carrier_hz + SYNC_SEARCH_HZ;
            let results = psk31_sync(&iq_buf, FS, base_hz, max_hz, 4, margin, 256, 5);
            if let Some((_found_hz, time_sym)) = best_sync(&results, carrier_hz, PSK31_BAUD) {
                let scan_end = ((time_sym + 2) * sps).min(iq_buf.len());
                let onset = iq_buf[..scan_end]
                    .iter()
                    .position(|c| c.re * c.re + c.im * c.im > 0.01)
                    .unwrap_or(0);
                let start = onset;
                let mut s = match decode_mode {
                    DecodeMode::Bpsk31 => {
                        let mut s = Psk31Stream::new_bpsk(FS, carrier_hz, 1.0);
                        s.set_fed_up_to(start);
                        s
                    }
                    _ => {
                        let mut s = Psk31Stream::new_qpsk(FS, carrier_hz, 1.0);
                        s.set_fed_up_to(start);
                        s
                    }
                };
                let text = s.feed(&iq_buf[start..]);
                if !text.is_empty() { all_text.push_str(&text); }
                s.set_fed_up_to(iq_buf.len());
                stream = Some(s);
            }
        }

        if let Some(ref mut s) = stream
            && s.fed_up_to() < iq_buf.len()
        {
            let text = s.feed(&iq_buf[s.fed_up_to()..]);
            if !text.is_empty() { all_text.push_str(&text); }
            s.set_fed_up_to(iq_buf.len());
        }
    }
    all_text
}

// ── PSK31 decode ──────────────────────────────────────────────────────────────

#[test]
fn psk31_decode_yields_text() {
    const MSG: &str = "CQ CQ CQ DE N0GNR";
    let sps = psk31_sps(FS);

    let mut src = Psk31Source::new(
        CARRIER_HZ, 0.0, 0.0, Psk31Mode::Bpsk31,
        MSG.to_owned(), 3, FS,
    );
    let total = PSK31_MAX_ACCUM_SYMS * sps;
    let samples: Vec<f32> = src.next_samples(total);
    println!("rendered {} samples = {:.1}s, sps={sps}", samples.len(), samples.len() as f32 / FS);

    let iq: Vec<C32> = samples.iter().map(|&s| C32::new(s, 0.0)).collect();
    let (info, text) = decode_bpsk31(&iq, CARRIER_HZ, FS, sps);
    if let DecodeResult::Info { modulation, center_hz, snr_db, .. } = &info {
        println!("Info: {modulation} ctr={center_hz:.0} snr={snr_db:.1}");
    }
    let found_text = matches!(text, Some(DecodeResult::Text(ref t)) if {
        println!("Text: {t:?}");
        !t.is_empty()
    });
    assert!(found_text, "expected non-empty Text result from full-frame decode");
}

// ── Dt ticker simulation ─────────────────────────────────────────────────────

/// Simulate the Dt mode ticker as seen by the viewer: feed Psk31Source blocks
/// through the shared ticker harness, which handles gap detection and the
/// rolling-window decode pipeline.  Uses `decode_bpsk31` as the per-buffer
/// decode callback.
///
/// Parameters match the viewer defaults:
///   BLOCK         = 800 samples (~16.7 ms at 48 kHz, matching SAMPLES_PER_FRAME)
///   gap_secs      = 10 s
///   msg_repeat    = 5
///   noise_amp     = 0 (clean signal for clarity)
///   Two full source loops so we can see repeat behaviour.
#[test]
fn psk31_simulate_dt_ticker() {
    const MSG:      &str  = "CQ CQ CQ DE N0GNR";
    const REPEAT:   usize = 5;
    const BLOCK:    usize = 800;
    const LOOP_GAP: f32   = 10.0;
    const LOOPS:    usize = 2;

    let sps = psk31_sps(FS);
    let max_accum = PSK31_MAX_ACCUM_SYMS * sps;

    let mut src = Psk31Source::new(
        CARRIER_HZ, LOOP_GAP, 0.0, Psk31Mode::Bpsk31,
        MSG.to_owned(), REPEAT, FS,
    );

    let text_bytes         = (MSG.len() * REPEAT + (REPEAT - 1)) as f32;
    let approx_text_syms   = (text_bytes * 11.0) as usize;
    let approx_signal_syms = 64 + approx_text_syms + 32;
    let approx_signal_secs = approx_signal_syms as f32 / 31.25;
    let approx_loop_secs   = approx_signal_secs + LOOP_GAP;
    let total_samples      = ((approx_loop_secs * LOOPS as f32 + 2.0) * FS) as usize;

    println!("  PSK31 message: {MSG:?} × {REPEAT}, carrier={CARRIER_HZ:.0} Hz");
    println!("  max_accum={max_accum} samples ({:.1}s)", PSK31_MAX_ACCUM_SYMS as f32 / 31.25);
    println!("  est. signal frame ≈ {approx_signal_secs:.1}s, loop gap={LOOP_GAP:.0}s");

    let cfg = TickerSimConfig {
        label:         "PSK31 BPSK31",
        block:         BLOCK,
        total_samples,
        fs:            FS,
        max_accum:     Some(max_accum),
    };

    run_ticker_sim(&mut src, &cfg, |iq| {
        let (info, text) = decode_bpsk31(iq, CARRIER_HZ, FS, sps);
        BufferDecode { info: Some(info), text }
    });
}

// ── Streaming decode tests ────────────────────────────────────────────────────

#[test]
fn streaming_decode_bpsk31_5_loops() {
    let text = run_streaming_decode(Psk31Mode::Bpsk31, "CQ CQ CQ DE N0GNR", 5, 5, 15.0);
    println!("BPSK31 decoded ({} chars): {:?}", text.len(), &text[..text.len().min(80)]);
    let errors = text.chars().filter(|c| !"CQ DE N0GNR ".contains(*c)).count();
    assert!(errors == 0, "BPSK31: {errors} unexpected chars in decoded text");
}

#[test]
fn streaming_decode_qpsk31_5_loops() {
    let text = run_streaming_decode(Psk31Mode::Qpsk31, "CQ CQ CQ DE N0GNR", 5, 5, 15.0);
    println!("QPSK31 decoded ({} chars): {:?}", text.len(), &text[..text.len().min(80)]);
    let errors = text.chars().filter(|c| !"CQ DE N0GNR ".contains(*c)).count();
    assert!(errors == 0, "QPSK31: {errors} unexpected chars in decoded text");
    assert!(text.contains("CQ CQ CQ DE N0GNR"), "QPSK31: message not found");
}

#[test]
fn streaming_decode_short_messages_bpsk31() {
    for &(msg, repeat, loops) in &[
        ("A",  1, 3), ("AB", 1, 3), ("CQ DE N0GNR", 1, 3),
        ("CQ", 5, 3), ("CQ CQ CQ DE N0GNR", 5, 2),
    ] {
        let text = run_streaming_decode(Psk31Mode::Bpsk31, msg, repeat, loops, 5.0);
        println!("BPSK31 msg={msg:?} r={repeat}: {:?}", &text);
        let errors = text.chars().filter(|c| !msg.contains(*c) && *c != ' ').count();
        assert!(errors == 0, "BPSK31 msg={msg:?}: {errors} unexpected chars");
        assert!(!text.is_empty(), "BPSK31 msg={msg:?}: no text decoded");
    }
}

#[test]
fn streaming_decode_short_messages_qpsk31() {
    for &(msg, repeat, loops) in &[
        ("A",  1, 3), ("AB", 1, 3), ("CQ DE N0GNR", 1, 3),
        ("CQ", 5, 3), ("CQ CQ CQ DE N0GNR", 5, 2),
    ] {
        let text = run_streaming_decode(Psk31Mode::Qpsk31, msg, repeat, loops, 5.0);
        println!("QPSK31 msg={msg:?} r={repeat}: {:?}", &text);
        let errors = text.chars().filter(|c| !msg.contains(*c) && *c != ' ').count();
        assert!(errors == 0, "QPSK31 msg={msg:?}: {errors} unexpected chars");
        assert!(!text.is_empty(), "QPSK31 msg={msg:?}: no text decoded");
    }
}

#[test]
fn streaming_decode_all_printable_ascii_bpsk31() {
    let msg: String = (32u8..127u8).map(|b| b as char).collect();
    let text = run_streaming_decode(Psk31Mode::Bpsk31, &msg, 1, 1, 5.0);
    println!("BPSK31 all-ASCII decoded ({} chars)", text.len());
    let expected: Vec<char> = msg.chars().collect();
    let got: Vec<char> = text.chars().collect();
    assert!(got.len() >= expected.len(),
        "too few chars: expected {}, got {}", expected.len(), got.len());
    for (i, (g, e)) in got.iter().zip(expected.iter()).enumerate() {
        assert_eq!(g, e, "BPSK31 mismatch at {i}: expected {e:?}, got {g:?}");
    }
}

#[test]
fn streaming_decode_all_printable_ascii_qpsk31() {
    let msg: String = (32u8..127u8).map(|b| b as char).collect();
    let text = run_streaming_decode(Psk31Mode::Qpsk31, &msg, 1, 1, 5.0);
    println!("QPSK31 all-ASCII decoded ({} chars)", text.len());
    let expected: Vec<char> = msg.chars().collect();
    let got: Vec<char> = text.chars().collect();
    assert!(got.len() >= expected.len(),
        "too few chars: expected {}, got {}", expected.len(), got.len());
    for (i, (g, e)) in got.iter().zip(expected.iter()).enumerate() {
        assert_eq!(g, e, "QPSK31 mismatch at {i}: expected {e:?}, got {g:?}");
    }
}
