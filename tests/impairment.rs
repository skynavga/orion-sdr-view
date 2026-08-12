// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The C/N impairment model and the derived display level.
//!
//! These pin the two properties the change exists to deliver — an impairment
//! that means the same thing on every source, and a display level that is
//! derived rather than fitted — plus the traps that make either easy to get
//! silently wrong.

use orion_sdr::util::rms;
use orion_sdr_view::config::ViewConfig;
use orion_sdr_view::decode::{SPECTRUM_WINDOW_SAMPLES, wb_cn_db};
use orion_sdr_view::source::{
    COFDM_DEFAULT_CN_DB, COFDM_DISPLAY_RMS_DBFS, COFDM_FS, COFDM_NOMINAL_CENTER, CofdmBwFraction,
    CofdmShaping, CofdmSource, MAX_CN_DB, SignalSource, cofdm_occupied_bw, ft8::FT8_DEFAULT_CN_DB,
    tone::TestSignalGen,
};

/// Blocks of this size, matching the viewer's per-frame feed granularity.
const BLOCK: usize = 4096;

/// Mean complex power over `blocks` reads.
fn iq_power(src: &mut CofdmSource, blocks: usize) -> f32 {
    let mut sum = 0.0f64;
    let mut n = 0usize;
    for _ in 0..blocks {
        let _ = src.next_samples(BLOCK);
        let iq = src.last_samples_iq().expect("complex baseband");
        sum += iq.iter().map(|c| c.norm_sqr() as f64).sum::<f64>();
        n += iq.len();
    }
    (sum / n as f64) as f32
}

/// Mean real-projection RMS over `blocks` reads.
fn real_rms(src: &mut CofdmSource, blocks: usize) -> f32 {
    (0..blocks)
        .map(|_| rms(&src.next_samples(BLOCK)))
        .sum::<f32>()
        / blocks as f32
}

fn source(fraction: CofdmBwFraction, cn_db: f32) -> CofdmSource {
    CofdmSource::new(
        600.0,
        600.0,
        cn_db,
        fraction,
        CofdmShaping::default_for(fraction),
        COFDM_FS,
    )
}

/// The C/N the source *achieves*, measured end-to-end from its own output.
///
/// Noise power comes from the gap (where there is no signal) and signal power
/// from the burst with that noise subtracted, so nothing here reuses the
/// arithmetic under test — a sign error or a missing factor of two in
/// `CnReference::sigma_for` shows up as a dB offset.
fn achieved_cn_db(fraction: CofdmBwFraction, cn_db: f32) -> f32 {
    let shaping = CofdmShaping::default_for(fraction);
    let occupied = cofdm_occupied_bw(COFDM_FS, shaping.effective(fraction).edge_guard);

    let mut gap = source(fraction, cn_db);
    gap.advance_time(601.0);
    assert!(!gap.in_signal(), "expected the gap phase");
    let p_noise = iq_power(&mut gap, 40);

    let mut burst = source(fraction, cn_db);
    let p_total = iq_power(&mut burst, 40);
    let p_signal = (p_total - p_noise).max(1e-30);

    // C/N = P_signal / (N0 * B_occupied), N0 = P_noise / fs.  Complex baseband,
    // so the noise is white over the full fs — not over the display's Nyquist.
    let n0 = p_noise / COFDM_FS;
    10.0 * (p_signal / (n0 * occupied)).log10()
}

// ── A. The impairment ───────────────────────────────────────────────────────

#[test]
fn the_achieved_cn_matches_the_requested_cn() {
    // The headline property.  Measured end-to-end, at three settings two
    // decades apart, so an error that scales with level and one that does not
    // are both caught.
    for &cn_db in &[40.0_f32, 30.0, 20.0] {
        let achieved = achieved_cn_db(CofdmBwFraction::OneQuarter, cn_db);
        assert!(
            (achieved - cn_db).abs() < 0.5,
            "requested {cn_db:.1} dB C/N, achieved {achieved:.1} dB"
        );
    }
}

#[test]
fn the_achieved_cn_is_flat_across_bandwidth_fractions() {
    // Every fraction carries a *different* derived display gain, so this is
    // also the display-level-invariance test: if the impairment tracked the
    // normalisation instead of the signal, these would spread with bandwidth.
    //
    // Note what this does NOT claim: the fractions do not fail at the same C/N.
    // Frame duration, preamble correlation energy and common-phase tracking
    // variance all scale with the carrier count, and no impairment knob touches
    // them.
    let mut worst = 0.0f32;
    for &fr in CofdmBwFraction::ALL {
        let achieved = achieved_cn_db(fr, 30.0);
        worst = worst.max((achieved - 30.0).abs());
        assert!(
            (achieved - 30.0).abs() < 0.5,
            "{}: requested 30.0 dB C/N, achieved {achieved:.1} dB",
            fr.label()
        );
    }
    assert!(worst < 0.5, "worst deviation {worst:.2} dB");
}

#[test]
fn the_cn_reference_excludes_the_preamble() {
    // The trap: the preamble is deliberately hotter than the payload, and the
    // prefix is a *bandwidth-dependent* fraction of the frame — so referencing
    // the buffer mean would inject a different C/N at every fraction.
    //
    // Pins the premise (the prefix really is hotter) rather than the
    // implementation, so it stays meaningful if the measurement moves.  The
    // consequence is covered by `the_achieved_cn_is_flat_across_bandwidth_fractions`.
    let fraction = CofdmBwFraction::OneQuarter;
    let mut src = source(fraction, MAX_CN_DB);
    let frame_prefix = 4 * 64 + 256 + 32;
    let _ = src.next_samples(BLOCK);
    let iq = src.last_samples_iq().expect("complex baseband");
    let power =
        |s: &[num_complex::Complex32]| s.iter().map(|c| c.norm_sqr()).sum::<f32>() / s.len() as f32;
    let prefix = power(&iq[..frame_prefix]);
    let payload = power(&iq[frame_prefix..]);
    assert!(
        prefix > payload,
        "preamble+training power {prefix:.5} is not above payload {payload:.5} — \
         if this ever stops holding, the reason for excluding the prefix is gone"
    );
}

#[test]
fn the_injected_noise_is_gaussian() {
    // Uniform noise has the same variance and is equally white, so nothing else
    // here would catch a regression to `xorshift` — but the FEC cliff is a tail
    // phenomenon, and an FER curve measured against uniform noise cannot be
    // compared to a published waterfall.  Kurtosis separates them cleanly:
    // Gaussian is 3.0, uniform is 1.8.
    let mut g = TestSignalGen::new(1000.0, 48_000.0);
    g.tone_amp = 0.0;
    g.set_cn_db(20.0);
    let sigma = g.noise_sigma();
    let s: Vec<f64> = (0..200_000)
        .map(|_| (g.next_sample() / sigma) as f64)
        .collect();
    let mean = s.iter().sum::<f64>() / s.len() as f64;
    let m2 = s.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / s.len() as f64;
    let m4 = s.iter().map(|v| (v - mean).powi(4)).sum::<f64>() / s.len() as f64;
    let kurtosis = m4 / (m2 * m2);
    assert!(
        (kurtosis - 3.0).abs() < 0.15,
        "kurtosis {kurtosis:.3} is not Gaussian (uniform would be 1.8)"
    );
}

#[test]
fn the_noise_floor_does_not_follow_the_tone_ramp() {
    // The C/N is referenced to `amp_max`, not to the live `tone_amp`.  A
    // reference that tracked the ramp would make the noise floor pump with the
    // signal — visibly wrong, since real noise does not follow the carrier —
    // and would divide by zero at the bottom of the cycle.
    let mut g = TestSignalGen::new(1000.0, 48_000.0);
    g.set_cn_db(30.0);
    let at_peak = g.noise_sigma();

    g.tone_amp = 0.0; // as the ramp's trough leaves it
    let at_trough = g.noise_sigma();
    assert_eq!(
        at_peak, at_trough,
        "noise amplitude moved with the live tone amplitude"
    );
    assert!(at_peak > 0.0, "30 dB C/N should inject something");
}

#[test]
fn the_measured_cn_tracks_the_requested_cn() {
    // The panel and the Di line have shown a *measured* C/N since 0.0.21.  It
    // is the same quantity the knob now requests, so the two must agree — and
    // they do not for free.
    //
    // `wb_spectrum_snr_db` runs on the **real projection**, which costs a
    // complex-baseband source exactly 3.01 dB: the signal splits into two
    // mirror lobes while symmetric complex noise merely halves.  That is what
    // `REAL_PROJECTION_CN_OFFSET_DB` corrects, and this test is what stops the
    // constant from being wrong without anyone noticing.
    //
    // The tolerance is wide because the estimator has character of its own: a
    // mean-of-dB in band (a geometric mean, ~2.5 dB under the arithmetic one
    // for periodogram bins) against a median-of-dB out of band (~1.6 dB under),
    // plus in-band bins that contain noise as well as signal.  What must hold
    // tightly is the *tracking*: 10 dB of knob must move the readout 10 dB.
    let fraction = CofdmBwFraction::OneQuarter;
    let shaping = CofdmShaping::default_for(fraction);
    let occupied = cofdm_occupied_bw(COFDM_FS, shaping.effective(fraction).edge_guard);

    let measure = |cn_db: f32| {
        let mut src = source(fraction, cn_db);
        let s = src.next_samples(SPECTRUM_WINDOW_SAMPLES);
        wb_cn_db(&s, COFDM_FS, COFDM_NOMINAL_CENTER, occupied) + REAL_PROJECTION_CN_OFFSET_DB
    };

    // The stated range: the default fraction, 10-30 dB.  Above that the
    // transmit skirt contaminates the noise floor and the reading compresses;
    // at wide occupancies there is barely any out-of-band spectrum left to
    // measure at all.  Both are properties of an out-of-band noise estimate,
    // predate this change, and are documented on `wb_cn_db`.
    for &cn_db in &[30.0_f32, 20.0, 10.0] {
        let measured = measure(cn_db);
        assert!(
            (measured - cn_db).abs() < 3.0,
            "requested {cn_db:.1} dB C/N, panel would read {measured:.1} dB"
        );
    }

    let slope = (measure(30.0) - measure(10.0)) / 20.0;
    assert!(
        (0.7..=1.15).contains(&slope),
        "20 dB of knob moved the readout {:.1} dB (slope {slope:.2})",
        slope * 20.0
    );
}

/// Mirror of the private constant in `source::cofdm::decode`.  Duplicated
/// rather than exported: it is an implementation detail of one estimator, and
/// a test that re-derives it independently is worth more than one that imports
/// the number it is checking.
const REAL_PROJECTION_CN_OFFSET_DB: f32 = 3.0103;

// ── B. The derived display level ────────────────────────────────────────────

#[test]
fn the_display_level_is_met_at_every_bandwidth() {
    // What the fitted `COFDM_GAIN` of 121.0 could not do: measured signal-phase
    // RMS spanned 1.344 to 3.646 across the fractions, a 2.7x spread, because
    // one constant was applied to a signal whose power depends on the carrier
    // count.  Normalising collapses it.
    let target = 10f32.powf(COFDM_DISPLAY_RMS_DBFS / 20.0);
    for &fr in CofdmBwFraction::ALL {
        let mut src = source(fr, MAX_CN_DB);
        let measured = real_rms(&mut src, 20);
        let err_db = 20.0 * (measured / target).log10();
        assert!(
            err_db.abs() < 1.0,
            "{}: signal-phase RMS {measured:.4} is {err_db:+.2} dB from the \
             {target:.4} target",
            fr.label()
        );
    }
}

#[test]
fn the_display_gain_is_not_one_constant() {
    // The point of deriving it: a source whose rendered power tracks its
    // occupied bandwidth needs a *different* scalar per configuration, which is
    // exactly what a fitted constant cannot supply — and what makes DFT-s-OFDM
    // (materially lower PAPR) a structural problem rather than a tuning one.
    let narrow = source(CofdmBwFraction::OneEighth, MAX_CN_DB).display_gain();
    let wide = source(CofdmBwFraction::SevenEighths, MAX_CN_DB).display_gain();
    assert!(
        narrow > wide * 2.0,
        "expected the derived gain to vary with occupancy: 1/8 {narrow:.1}, \
         7/8 {wide:.1}"
    );
}

// ── D. The breaking config change ───────────────────────────────────────────

#[test]
fn a_retired_noise_amp_key_is_refused_not_ignored() {
    // The whole reason the rejection field exists: every field is `Option<T>`
    // and nothing sets `deny_unknown_fields`, so serde would drop `noise_amp`
    // silently and fall back to the `cn_db` default — a config that looks like
    // it loaded while discarding what the user wrote.
    let yaml = r#"
view:
  sources:
    cofdm:
      bandwidth: 1/4
      noise_amp: 0.5
"#;
    let cfg: ViewConfig = serde_yaml::from_str::<serde_yaml::Value>(yaml)
        .ok()
        .and_then(|v| serde_yaml::from_value::<TestFile>(v).ok())
        .map(|f| f.view)
        .expect("fixture parses");
    let errs = cfg.retired_key_errors();
    assert_eq!(errs.len(), 1, "expected exactly one diagnostic: {errs:?}");
    assert!(
        errs[0].contains("cn_db") && errs[0].contains("cofdm"),
        "diagnostic must name the source and the replacement: {}",
        errs[0]
    );
}

#[test]
fn a_current_config_produces_no_retired_key_diagnostics() {
    let yaml = r#"
view:
  sources:
    cofdm:
      bandwidth: 1/4
      cn_db: 30.0
    psk31:
      cn_db: 45.0
"#;
    let cfg: ViewConfig = serde_yaml::from_str::<serde_yaml::Value>(yaml)
        .ok()
        .and_then(|v| serde_yaml::from_value::<TestFile>(v).ok())
        .map(|f| f.view)
        .expect("fixture parses");
    assert!(cfg.retired_key_errors().is_empty());
    assert_eq!(cfg.cofdm_cn_db(), 30.0);
    assert_eq!(cfg.psk31_cn_db(), 45.0);
    // An untouched source falls back to its own default, which differs from
    // COFDM's by ~10 dB because the spreading factors do.
    assert_eq!(cfg.ft8_cn_db(), FT8_DEFAULT_CN_DB);
    const { assert!(FT8_DEFAULT_CN_DB > COFDM_DEFAULT_CN_DB) };
}

#[derive(serde::Deserialize)]
struct TestFile {
    view: ViewConfig,
}
