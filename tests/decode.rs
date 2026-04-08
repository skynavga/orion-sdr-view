//! Integration tests for the decode pipeline.

use num_complex::Complex32 as C32;
use orion_sdr::Block;
use orion_sdr::demodulate::psk31::{Bpsk31Demod, Bpsk31Decider};
use orion_sdr::codec::varicode::VaricodeDecoder;
use orion_sdr::sync::psk31_sync::psk31_sync;
use orion_sdr::modulate::psk31::{psk31_sps, PSK31_BAUD};

use orion_sdr_view::decode::{
    DecodeMode, DecodeResult, DecodeTicker,
    Psk31Stream, SIGNAL_THRESHOLD, SPECTRUM_WINDOW_SAMPLES, PSK31_MAX_ACCUM_SYMS,
    PSK31_BW_HZ, SYNC_SEARCH_HZ, SYNC_MIN_SYMS,
    best_sync, spectrum_snr_db, spectrum_bw_hz,
};
use orion_sdr_view::source::{
    AmDsbSource, Psk31Source, Psk31Mode, SignalSource, BuiltinAudio, load_builtin,
};

const FS: f32 = 48_000.0;
const CARRIER_HZ: f32 = 12_000.0;

// ── Shared helpers ─────────────────────────────────────────────────────────────

/// Build an AM DSB signal from a slice of audio samples already at FS.
/// Audio is normalised to peak = 0.9 (matching AmDsbSource behaviour) before
/// modulation so sideband levels are consistent across test helpers.
fn am_dsb_signal(audio: &[f32]) -> Vec<f32> {
    use orion_sdr::modulate::AmDsbMod;
    use orion_sdr::core::AudioToIqChain;
    let peak = audio.iter().map(|&s| s.abs()).fold(0.0f32, f32::max);
    let scale = if peak > 1e-6 { 0.9 / peak } else { 1.0 };
    let norm: Vec<f32> = audio.iter().map(|&s| s * scale).collect();
    let block = AmDsbMod::new(FS, CARRIER_HZ, 1.0, 1.0);
    let mut chain = AudioToIqChain::new(block);
    let iq = chain.process_ref(&norm);
    iq.iter().map(|c| c.re).collect()
}

/// Generate a single sinusoid at `freq_hz` for `n` samples.
fn sine(freq_hz: f32, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| (2.0 * std::f32::consts::PI * freq_hz * i as f32 / FS).sin())
        .collect()
}

/// Generate bandlimited noise covering `lo_hz`..`hi_hz` by summing sines.
/// Uses a fixed set of harmonics spread across the band.
fn band_noise(lo_hz: f32, hi_hz: f32, n: usize) -> Vec<f32> {
    let steps = 20_usize;
    let step = (hi_hz - lo_hz) / steps as f32;
    let mut out = vec![0.0f32; n];
    for k in 0..steps {
        let f = lo_hz + k as f32 * step;
        let phase = k as f32 * 0.7;
        for i in 0..n {
            out[i] += (2.0 * std::f32::consts::PI * f * i as f32 / FS + phase).sin();
        }
    }
    let peak = out.iter().map(|&s| s.abs()).fold(0.0f32, f32::max).max(1e-6);
    out.iter_mut().for_each(|s| *s *= 0.5 / peak);
    out
}

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
        if ch >= 0x20 && ch < 0x7f {
            text.push(ch as char);
        }
    }
    text
}

/// Modulate audio via AmDsbSource (handles native-rate → FS resampling),
/// measure BW on one SPECTRUM_WINDOW_SAMPLES window, and return the result.
/// Returns the maximum BW observed across active windows in the recording.
fn measure_bw_via_source(audio: Vec<f32>, audio_rate: f32) -> f32 {
    let audio_secs = audio.len() as f32 / audio_rate;
    let total = ((audio_secs * FS) as usize).max(SPECTRUM_WINDOW_SAMPLES);
    let mut src = AmDsbSource::new(
        audio, audio_rate, CARRIER_HZ, 1.0, 0.0, 0.0, 1, FS,
    );
    let signal = src.next_samples(total);
    signal
        .chunks_exact(SPECTRUM_WINDOW_SAMPLES)
        .filter(|w| {
            let rms = (w.iter().map(|&s| s*s).sum::<f32>() / w.len() as f32).sqrt();
            rms >= SIGNAL_THRESHOLD
        })
        .map(|w| spectrum_bw_hz(w, FS, CARRIER_HZ, 7.0))
        .fold(0.0f32, f32::max)
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

        if let Some(ref mut s) = stream {
            if s.fed_up_to() < iq_buf.len() {
                let text = s.feed(&iq_buf[s.fed_up_to()..]);
                if !text.is_empty() { all_text.push_str(&text); }
                s.set_fed_up_to(iq_buf.len());
            }
        }
    }
    all_text
}

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
            /*loop_gap_secs=*/ 2.0,
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

// ── Morse-like: single 800 Hz tone ────────────────────────────────────────────

/// AM DSB of a single 800 Hz tone: sidebands at ±800 Hz from carrier.
/// Expected BW ≈ 2 × 800 Hz = 1600 Hz.  Tolerance: 800–2400 Hz.
#[test]
fn bw_morse_tone() {
    let audio = sine(800.0, SPECTRUM_WINDOW_SAMPLES);
    let signal = am_dsb_signal(&audio);
    let bw = spectrum_bw_hz(&signal, FS, CARRIER_HZ, 7.0);
    println!("Morse-tone BW: {bw:.1} Hz");
    assert!(
        bw >= 800.0 && bw <= 2_400.0,
        "Morse-tone BW {bw:.0} Hz not in [800, 2400]"
    );
}

// ── Voice-like: broadband 300–3000 Hz ────────────────────────────────────────

/// AM DSB of broadband voice audio (300–3000 Hz): sidebands span ±300–3000 Hz.
/// Expected BW ≈ 2 × 3000 Hz = 6000 Hz.  Tolerance: 3000–7000 Hz.
#[test]
fn bw_voice_audio() {
    let audio = band_noise(300.0, 3_000.0, SPECTRUM_WINDOW_SAMPLES);
    let signal = am_dsb_signal(&audio);
    let bw = spectrum_bw_hz(&signal, FS, CARRIER_HZ, 7.0);
    println!("Voice BW: {bw:.1} Hz");
    assert!(
        bw >= 3_000.0 && bw <= 7_000.0,
        "Voice BW {bw:.0} Hz not in [3000, 7000]"
    );
}

// ── Built-in audio sources ────────────────────────────────────────────────────

/// AM DSB of the built-in Morse WAV: audio is CW bursts at ~800 Hz.
/// Sidebands sit at carrier ± 800 Hz.  Expected BW: 800–2400 Hz.
#[test]
fn bw_builtin_morse() {
    let (audio, audio_rate) = load_builtin(BuiltinAudio::Morse);
    let bw = measure_bw_via_source(audio, audio_rate);
    println!("Built-in Morse BW: {bw:.1} Hz");
    assert!(
        bw >= 800.0 && bw <= 2_400.0,
        "Morse BW {bw:.0} Hz not in [800, 2400]"
    );
}

/// AM DSB of the built-in Voice WAV: broadband speech up to ~4 kHz.
/// Sidebands span carrier ± audio BW.  Expected BW: 2000–8000 Hz.
#[test]
fn bw_builtin_voice() {
    let (audio, audio_rate) = load_builtin(BuiltinAudio::Voice);
    let bw = measure_bw_via_source(audio, audio_rate);
    println!("Built-in Voice BW: {bw:.1} Hz");
    assert!(
        bw >= 2_000.0 && bw <= 8_000.0,
        "Voice BW {bw:.0} Hz not in [2000, 8000]"
    );
}

// ── Stability: BW doesn't blow up when signal fades ──────────────────────────

/// When audio is near-silence (gap), BW should be small (near carrier only),
/// not artificially inflated by noise.  Upper bound: 1000 Hz.
#[test]
fn bw_silence_stays_small() {
    let audio: Vec<f32> = vec![0.001f32; SPECTRUM_WINDOW_SAMPLES];
    let signal = am_dsb_signal(&audio);
    let bw = spectrum_bw_hz(&signal, FS, CARRIER_HZ, 7.0);
    println!("Silence BW: {bw:.1} Hz");
    assert!(
        bw <= 1_000.0,
        "Silence BW {bw:.0} Hz should be ≤ 1000 Hz (got inflated reading)"
    );
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

/// Simulate the Dt mode ticker as seen by the viewer: feed Psk31Source blocks
/// through gap detection and the rolling-window decode pipeline, printing each
/// event the ticker would receive (with wall-clock timestamp) so we can inspect
/// what text accumulates and when.
///
/// Parameters match the viewer defaults:
///   BLOCK = 800 samples (~16.7 ms at 48 kHz, matching SAMPLES_PER_FRAME)
///   loop_gap_secs = 10 s (PSK31_DEFAULT_LOOP_GAP_SECS)
///   msg_repeat    = 5   (PSK31_DEFAULT_REPEAT)
///   noise_amp     = 0   (clean signal for clarity)
///   Two full source loops so we can see repeat behaviour.
#[test]
fn simulate_dt_ticker() {
    use orion_sdr::util::rms;

    const MSG:       &str  = "CQ CQ CQ DE N0GNR";
    const REPEAT:    usize = 5;
    const BLOCK:     usize = 800;
    const LOOP_GAP:  f32   = 10.0;
    const LOOPS:     usize = 2;

    let sps = psk31_sps(FS);

    let mut src = Psk31Source::new(
        CARRIER_HZ, LOOP_GAP, 0.0, Psk31Mode::Bpsk31,
        MSG.to_owned(), REPEAT, FS,
    );

    let mut iq_buf:     Vec<C32>     = Vec::new();
    let mut ticker:     DecodeTicker = DecodeTicker::new();
    let mut t_secs:     f32          = 0.0;
    let mut was_silent: bool         = true;
    let max_accum = PSK31_MAX_ACCUM_SYMS * sps;

    let text_bytes         = (MSG.len() * REPEAT + (REPEAT - 1)) as f32;
    let approx_text_syms   = (text_bytes * 11.0) as usize;
    let approx_signal_syms = 64 + approx_text_syms + 32;
    let approx_signal_secs = approx_signal_syms as f32 / 31.25;
    let approx_loop_secs   = approx_signal_secs + LOOP_GAP;
    let total_samples      = ((approx_loop_secs * LOOPS as f32 + 2.0) * FS) as usize;

    println!("── Dt ticker simulation ──────────────────────────────────────────");
    println!("  message: {MSG:?} × {REPEAT}, carrier={CARRIER_HZ:.0} Hz, fs={FS:.0}");
    println!("  max_accum={max_accum} samples ({:.1}s), block={BLOCK}",
        PSK31_MAX_ACCUM_SYMS as f32 / 31.25);
    println!("  est. signal frame ≈ {approx_signal_secs:.1}s, loop gap={LOOP_GAP:.0}s");
    println!("  simulating {total_samples} samples ({:.1}s)\n", total_samples as f32 / FS);

    for block_start in (0..total_samples).step_by(BLOCK) {
        let n = BLOCK.min(total_samples - block_start);
        let samples = src.next_samples(n);
        t_secs += n as f32 / FS;

        let is_silent = rms(&samples) < SIGNAL_THRESHOLD;

        if is_silent {
            if !was_silent && !iq_buf.is_empty() {
                let buf = std::mem::take(&mut iq_buf);
                println!("t={t_secs:7.2}s  [GAP: decode {} samples = {:.1}s]",
                    buf.len(), buf.len() as f32 / FS);
                let (info, text) = decode_bpsk31(&buf, CARRIER_HZ, FS, sps);
                if let DecodeResult::Info { ref modulation, center_hz, snr_db, .. } = info {
                    println!("  Info: {modulation} ctr={center_hz:.1}Hz snr={snr_db:.1}dB");
                }
                ticker.push_result(info);
                match text {
                    Some(DecodeResult::Text(ref s)) => {
                        println!("  Text: {:?}", s);
                        ticker.push_result(DecodeResult::Text(s.clone()));
                    }
                    Some(other) => println!("  {:?}", other),
                    None        => println!("  (no text)"),
                }
            }
            ticker.push_result(DecodeResult::Gap);
            was_silent = true;
        } else {
            iq_buf.extend(samples.iter().map(|&s| C32::new(s, 0.0)));
            was_silent = false;

            if iq_buf.len() >= max_accum {
                let buf = std::mem::take(&mut iq_buf);
                println!("t={t_secs:7.2}s  [MAX_ACCUM flush: {} samples]", buf.len());
                let (info, text) = decode_bpsk31(&buf, CARRIER_HZ, FS, sps);
                if let DecodeResult::Info { ref modulation, center_hz, snr_db, .. } = info {
                    println!("  Info: {modulation} ctr={center_hz:.1}Hz snr={snr_db:.1}dB");
                }
                ticker.push_result(info);
                if let Some(DecodeResult::Text(ref s)) = text {
                    println!("  Text: {:?}", s);
                    ticker.push_result(DecodeResult::Text(s.clone()));
                }
            }
        }

        ticker.tick(n as f32 / FS);
    }

    println!("\n── Final ticker buffer ───────────────────────────────────────────");
    println!("  visible: {:?}", ticker.visible);
    println!("  visible.len() = {}", ticker.visible.len());
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

