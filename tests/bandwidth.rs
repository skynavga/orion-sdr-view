//! Bandwidth-measurement tests against AM DSB-modulated signals.
//!
//! Not specific to a single source: these exercise `spectrum_bw_hz` on
//! synthetic tones, band-limited noise, and the built-in Morse/Voice audio
//! fixtures.  AM DSB is used as the modulation carrier because sidebands
//! map directly to audio bandwidth, making expected ranges easy to assert.

use orion_sdr_view::decode::{
    SIGNAL_THRESHOLD, SPECTRUM_WINDOW_SAMPLES, spectrum_bw_hz,
};
use orion_sdr_view::source::{
    AmDsbSource, BuiltinAudio, SignalSource, load_builtin,
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
        for (i, slot) in out.iter_mut().enumerate() {
            *slot += (2.0 * std::f32::consts::PI * f * i as f32 / FS + phase).sin();
        }
    }
    let peak = out.iter().map(|&s| s.abs()).fold(0.0f32, f32::max).max(1e-6);
    out.iter_mut().for_each(|s| *s *= 0.5 / peak);
    out
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
        (800.0..=2_400.0).contains(&bw),
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
        (3_000.0..=7_000.0).contains(&bw),
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
        (800.0..=2_400.0).contains(&bw),
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
        (2_000.0..=8_000.0).contains(&bw),
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
