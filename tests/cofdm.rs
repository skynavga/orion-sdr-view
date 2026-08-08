// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Integration tests for the COFDM (wideband coded-OFDM) source: signal generation,
//! the `SignalSource` trait surface, per-source sample rate, occupied
//! bandwidth, out-of-band spectral shaping, and the frame-rate-independent
//! (dt-driven) signal/gap timing.

use orion_sdr_view::source::{
    COFDM_FS, COFDM_MAX_EDGE_GUARD, COFDM_MIN_EDGE_GUARD, COFDM_NOMINAL_CENTER,
    COFDM_SHAPING_SLACK, CofdmBwFraction, CofdmMask, CofdmShaping, CofdmSource, CofdmTaper,
    SignalSource, cofdm_edge_guard_for, cofdm_occupied_bw, cofdm_occupied_half,
};

/// Default construction used by most tests: 2 s signal, 1 s gap, no noise so
/// signal/silence are unambiguous, 1/4 bandwidth, shaping off (so these tests
/// exercise the same waveform they always did).
fn make() -> CofdmSource {
    make_with(CofdmShaping::derived(CofdmBwFraction::OneQuarter))
}

/// Like [`make`], with an explicit shaping configuration.
fn make_with(shaping: CofdmShaping) -> CofdmSource {
    CofdmSource::new(
        2.0,
        1.0,
        0.0,
        CofdmBwFraction::OneQuarter,
        shaping,
        COFDM_FS,
    )
}

/// RMS of a sample block.
fn rms(s: &[f32]) -> f32 {
    (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt()
}

// ── Trait surface / sample rate ────────────────────────────────────────────

#[test]
fn reports_native_sample_rate() {
    let mut src = make();
    // COFDM runs at its own high fs, not the viewer's 48 kHz.
    assert_eq!(src.sample_rate(), COFDM_FS);
    assert_eq!(COFDM_FS, 1_920_000.0);
    // downcast round-trips.
    assert!(src.as_any_mut().downcast_mut::<CofdmSource>().is_some());
}

#[test]
fn next_samples_returns_exactly_n() {
    let mut src = make();
    for n in [0usize, 1, 800, 4096] {
        assert_eq!(src.next_samples(n).len(), n);
    }
}

// ── Signal generation ──────────────────────────────────────────────────────

#[test]
fn signal_phase_has_energy() {
    let mut src = make();
    // Starts in the signal phase; the COFDM buffer must carry real energy.
    let s = src.next_samples(65_536);
    assert!(rms(&s) > 0.1, "signal RMS {} too low", rms(&s));
}

#[test]
fn occupied_bandwidth_scales_with_fraction() {
    // The analytic occupied BW must grow monotonically with the fraction and
    // stay inside the Nyquist band for every option.
    let nyquist = COFDM_FS / 2.0;
    let mut prev = 0.0;
    for &fr in CofdmBwFraction::ALL {
        let bw = cofdm_occupied_bw(COFDM_FS, cofdm_edge_guard_for(fr));
        assert!(bw > prev, "bw not increasing at {}", fr.label());
        assert!(bw < nyquist, "bw {} exceeds Nyquist at {}", bw, fr.label());
        // Band centered on the nominal center stays within [0, Nyquist].
        assert!(COFDM_NOMINAL_CENTER - bw / 2.0 >= 0.0);
        assert!(COFDM_NOMINAL_CENTER + bw / 2.0 <= nyquist);
        prev = bw;
    }
}

// ── dt-driven signal/gap timing (frame-rate independent) ───────────────────

/// Advance the source by `dt`-second steps for `duration` seconds, recording
/// (time, in_signal) at each phase transition.
fn run_phases(src: &mut CofdmSource, dt: f32, duration: f32) -> Vec<(f32, bool)> {
    let mut t = 0.0;
    let mut prev = src.in_signal();
    let mut transitions = Vec::new();
    while t < duration {
        src.advance_time(dt);
        let _ = src.next_samples(800);
        let now = src.in_signal();
        if now != prev {
            transitions.push((t, now));
            prev = now;
        }
        t += dt;
    }
    transitions
}

#[test]
fn phase_durations_follow_wall_clock() {
    // sig 2 s, gap 1 s → flips at 2 s (→gap), 3 s (→sig), 5 s (→gap).
    let mut src = make();
    assert!(src.in_signal(), "starts in signal phase");
    let tr = run_phases(&mut src, 0.05, 6.0);
    assert!(tr.len() >= 3, "expected ≥3 transitions, got {:?}", tr);
    assert!(
        (tr[0].0 - 2.0).abs() < 0.1 && !tr[0].1,
        "sig→gap ~2s: {:?}",
        tr[0]
    );
    assert!(
        (tr[1].0 - 3.0).abs() < 0.1 && tr[1].1,
        "gap→sig ~3s: {:?}",
        tr[1]
    );
    assert!(
        (tr[2].0 - 5.0).abs() < 0.1 && !tr[2].1,
        "sig→gap ~5s: {:?}",
        tr[2]
    );
}

#[test]
fn timing_is_frame_rate_independent() {
    // The first sig→gap flip must land at ~2 s of wall-clock regardless of the
    // dt step size (i.e. regardless of frame rate).
    for &dt in &[0.008_f32, 0.016, 0.05] {
        let mut src = make();
        let tr = run_phases(&mut src, dt, 3.0);
        assert!(!tr.is_empty(), "no transition at dt={dt}");
        assert!(
            (tr[0].0 - 2.0).abs() <= dt + 0.001,
            "sig→gap at {:.3}s for dt={dt} (want ~2s)",
            tr[0].0
        );
    }
}

#[test]
fn signal_then_gap_output_matches_phase() {
    // No noise: the signal phase yields energy, the gap phase yields silence.
    let mut src = make();
    // Consume ~1 s into the signal phase.
    src.advance_time(1.0);
    assert!(src.in_signal());
    assert!(
        rms(&src.next_samples(4096)) > 0.1,
        "signal phase should have energy"
    );
    // Cross into the gap phase.
    src.advance_time(1.5);
    assert!(!src.in_signal());
    assert_eq!(
        rms(&src.next_samples(4096)),
        0.0,
        "gap phase should be silent"
    );
}

#[test]
fn restart_returns_to_signal_start() {
    let mut src = make();
    src.advance_time(2.5); // into the gap phase
    assert!(!src.in_signal());
    src.restart();
    assert!(src.in_signal(), "restart returns to the signal phase");
}

// ── Out-of-band spectral shaping ───────────────────────────────────────────
//
// Three composing levers (see `orion-sdr/docs/modulate.md`): the edge-carrier
// guard moves the strongest `sinc` generators inward, the symbol-window taper
// softens the symbol seam, and the baseband mask attenuates the skirt directly
// in the frequency domain.  At COFDM's numerology (`n_fft` 256, `cp_len` 32)
// the shared guard budget is only 16 samples, so the mask is necessarily short
// and its payoff is measured in tens of dB far out rather than the ~60 dB a
// long-guard profile reaches.

/// Frequency bands of the *display* spectrum (real signal, 0…960 kHz) at the
/// 1/4 fraction, whose occupied band is 360…600 kHz.
const IN_BAND: (f32, f32) = (380_000.0, 580_000.0);
/// A few carriers past the band edge — where the taper acts, and where the
/// mask deliberately leaves its own transition unattenuated.
const NEAR_SKIRT: (f32, f32) = (630_000.0, 780_000.0);
/// Far out, where the mask dominates.
const FAR_SKIRT: (f32, f32) = (900_000.0, 955_000.0);

/// Welch-averaged power spectrum of a real signal through a 4-term
/// Blackman–Harris window (sidelobes ≈ −92 dB).
///
/// The window is not incidental: a rectangular slice leaks roughly 35 dB below
/// its in-band power, which would measure the *analysis*'s own sidelobes rather
/// than the mask's stop band.  Any claim about deep attenuation has to be read
/// through a window whose sidelobes sit below the attenuation being claimed.
fn bh_power_spectrum(samples: &[f32], n: usize) -> Vec<f32> {
    use rustfft::FftPlanner;
    use rustfft::num_complex::Complex;
    const A: [f32; 4] = [0.35875, 0.48829, 0.14128, 0.01168];
    let segments = samples.len() / n;
    let fft = FftPlanner::new().plan_fft_forward(n);
    let mut acc = vec![0.0f32; n / 2];
    for s in 0..segments {
        let mut buf: Vec<Complex<f32>> = (0..n)
            .map(|i| {
                let x = core::f32::consts::TAU * i as f32 / n as f32;
                let w = A[0] - A[1] * x.cos() + A[2] * (2.0 * x).cos() - A[3] * (3.0 * x).cos();
                Complex::new(samples[s * n + i] * w, 0.0)
            })
            .collect();
        fft.process(&mut buf);
        for (k, a) in acc.iter_mut().enumerate() {
            *a += buf[k].norm_sqr();
        }
    }
    acc.iter().map(|&p| p / segments as f32).collect()
}

/// Mean power (dB) of `spec` over a frequency band.
fn band_db(spec: &[f32], n: usize, band: (f32, f32)) -> f32 {
    let bin = |f: f32| ((f / COFDM_FS) * n as f32).round() as usize;
    let (lo, hi) = (bin(band.0), bin(band.1).min(spec.len() - 1));
    let mean = spec[lo..=hi].iter().sum::<f32>() / (hi - lo + 1) as f32;
    10.0 * (mean + 1e-30).log10()
}

/// FFT size for the shaping measurements: 7.5 kHz subcarriers at 1.92 MHz need
/// fine resolution to separate the band edge from the skirt.
const SPEC_N: usize = 8192;

/// Render a shaping configuration and return its display power spectrum.
fn spectrum_of(shaping: CofdmShaping) -> Vec<f32> {
    let mut src = make_with(shaping);
    bh_power_spectrum(&src.next_samples(1 << 20), SPEC_N)
}

/// A shaping set at the 1/4 fraction's own edge guard.
fn shaping(taper: CofdmTaper, mask: CofdmMask) -> CofdmShaping {
    CofdmShaping {
        enabled: true,
        edge_guard: cofdm_edge_guard_for(CofdmBwFraction::OneQuarter),
        include_dc: false,
        taper,
        mask,
    }
}

#[test]
fn derived_edge_guard_reproduces_the_fraction_band() {
    // The bandwidth toggle IS the edge-guard lever: the guard it implies must
    // put the occupied band back where the fraction asked for it, to within the
    // subcarrier spacing the carrier count is quantized to.
    let spacing = COFDM_FS / 256.0;
    for &fr in CofdmBwFraction::ALL {
        let bw = cofdm_occupied_bw(COFDM_FS, cofdm_edge_guard_for(fr));
        let want = fr.value() * COFDM_FS / 2.0;
        assert!(
            (bw - want).abs() <= spacing,
            "{}: guard band {bw} Hz vs fraction {want} Hz",
            fr.label()
        );
    }
}

#[test]
fn edge_guard_override_narrows_the_occupied_band() {
    // Nudging the guard past what the fraction implies takes carriers off both
    // edges — the Di bar's BW readout has to follow the guard, not the label.
    let g = cofdm_edge_guard_for(CofdmBwFraction::OneQuarter);
    let spacing = COFDM_FS / 256.0;
    assert_eq!(cofdm_occupied_half(g + 8), cofdm_occupied_half(g) - 8);
    let narrowed = cofdm_occupied_bw(COFDM_FS, g) - cofdm_occupied_bw(COFDM_FS, g + 8);
    assert!(
        (narrowed - 16.0 * spacing).abs() < 1.0,
        "narrowed {narrowed} Hz"
    );
}

#[test]
fn disabled_shaping_resolves_to_the_fraction_defaults() {
    // With `Shaping` off the four shaping rows are ignored entirely: whatever
    // they hold, the rendered configuration is the fraction's own.
    let fr = CofdmBwFraction::SevenEighths;
    let stale = CofdmShaping {
        enabled: false,
        edge_guard: 3,
        include_dc: true,
        taper: CofdmTaper::ThreeEighths,
        mask: CofdmMask::Db80,
    };
    assert_eq!(stale.effective(fr), CofdmShaping::derived(fr));
    assert_eq!(stale.effective(fr).edge_guard, cofdm_edge_guard_for(fr));
    assert!(
        stale.mask_filter(16).is_none(),
        "disabled shaping has no mask"
    );
}

#[test]
fn no_reachable_setting_overruns_the_guard_budget() {
    // The taper and the mask's group delay share `COFDM_SHAPING_SLACK`. The
    // `Taper` toggle deliberately stops at 3/8 for this reason: 1/2 would spend
    // the whole budget and silently drop the mask while the row still named a
    // stop-band depth.
    for &fr in CofdmBwFraction::ALL {
        let occupied_half = cofdm_occupied_half(cofdm_edge_guard_for(fr));
        for &taper in CofdmTaper::ALL {
            for &mask in CofdmMask::ALL {
                let sh = CofdmShaping {
                    enabled: true,
                    edge_guard: cofdm_edge_guard_for(fr),
                    include_dc: false,
                    taper,
                    mask,
                };
                let delay = sh.mask_filter(occupied_half).map_or(0, |m| m.group_delay());
                assert!(
                    taper.roll_off() + delay <= COFDM_SHAPING_SLACK,
                    "{} / {} / {}: roll_off {} + delay {delay} > {COFDM_SHAPING_SLACK}",
                    fr.label(),
                    taper.label(),
                    mask.label(),
                    taper.roll_off(),
                );
                // A mask that was asked for is never silently dropped.
                assert_eq!(
                    sh.mask_filter(occupied_half).is_some(),
                    mask != CofdmMask::Off,
                    "{} / {} lost its mask",
                    taper.label(),
                    mask.label()
                );
            }
        }
    }
}

#[test]
fn shaping_cuts_out_of_band_energy_and_leaves_the_band_alone() {
    let off = spectrum_of(CofdmShaping::derived(CofdmBwFraction::OneQuarter));
    let on = spectrum_of(shaping(CofdmTaper::Quarter, CofdmMask::Db60));

    let in_band = band_db(&off, SPEC_N, IN_BAND) - band_db(&on, SPEC_N, IN_BAND);
    assert!(
        in_band.abs() < 1.0,
        "shaping moved in-band power by {in_band:.2} dB"
    );

    let far = band_db(&off, SPEC_N, FAR_SKIRT) - band_db(&on, SPEC_N, FAR_SKIRT);
    assert!(far > 12.0, "far skirt dropped only {far:.2} dB");

    let near = band_db(&off, SPEC_N, NEAR_SKIRT) - band_db(&on, SPEC_N, NEAR_SKIRT);
    assert!(near > 2.5, "near skirt dropped only {near:.2} dB");
}

#[test]
fn taper_and_mask_stack() {
    // They attack different things — the taper the symbol seam, the mask the
    // spectrum directly — so together they beat either alone.  Measured far
    // out, where the mask is the stronger lever.
    let taper_only = band_db(
        &spectrum_of(shaping(CofdmTaper::Quarter, CofdmMask::Off)),
        SPEC_N,
        FAR_SKIRT,
    );
    let mask_only = band_db(
        &spectrum_of(shaping(CofdmTaper::Off, CofdmMask::Db60)),
        SPEC_N,
        FAR_SKIRT,
    );
    let both = band_db(
        &spectrum_of(shaping(CofdmTaper::Quarter, CofdmMask::Db60)),
        SPEC_N,
        FAR_SKIRT,
    );
    let baseline = band_db(
        &spectrum_of(CofdmShaping::derived(CofdmBwFraction::OneQuarter)),
        SPEC_N,
        FAR_SKIRT,
    );
    assert!(
        taper_only < baseline - 2.0,
        "taper alone: {taper_only:.2} vs {baseline:.2}"
    );
    assert!(
        mask_only < baseline - 2.0,
        "mask alone: {mask_only:.2} vs {baseline:.2}"
    );
    assert!(
        both < taper_only - 2.0,
        "both {both:.2} vs taper {taper_only:.2}"
    );
    assert!(
        both < mask_only - 2.0,
        "both {both:.2} vs mask {mask_only:.2}"
    );
}

#[test]
fn every_shaping_knob_reaches_the_rendered_buffer() {
    // Guards the settings→source wiring: each parameter must change the samples
    // (and `apply_params` must notice, rather than keeping a stale buffer).
    let base = shaping(CofdmTaper::Quarter, CofdmMask::Db60);
    let variants = [
        CofdmShaping {
            include_dc: true,
            ..base
        },
        CofdmShaping {
            edge_guard: base.edge_guard + 8,
            ..base
        },
        CofdmShaping {
            taper: CofdmTaper::Off,
            ..base
        },
        CofdmShaping {
            mask: CofdmMask::Off,
            ..base
        },
        CofdmShaping {
            enabled: false,
            ..base
        },
    ];
    let reference = make_with(base).next_samples(4096);
    for (i, v) in variants.into_iter().enumerate() {
        let mut src = make_with(base);
        src.apply_params(2.0, 1.0, 0.0, CofdmBwFraction::OneQuarter, v);
        assert_ne!(
            src.next_samples(4096),
            reference,
            "variant {i} changed nothing"
        );
    }
}

#[test]
fn edge_guard_range_brackets_every_fraction() {
    // The `Edge guard` row's range has to admit every guard the `Bandwidth`
    // toggle can seed into it, or re-seeding would clamp and the two rows would
    // disagree about the band.
    for &fr in CofdmBwFraction::ALL {
        let g = cofdm_edge_guard_for(fr);
        assert!(
            (COFDM_MIN_EDGE_GUARD..=COFDM_MAX_EDGE_GUARD).contains(&g),
            "{}: guard {g} outside {COFDM_MIN_EDGE_GUARD}..={COFDM_MAX_EDGE_GUARD}",
            fr.label()
        );
    }
}

#[test]
fn narrowest_guard_keeps_the_band_inside_the_display() {
    // Below `COFDM_MIN_EDGE_GUARD` the occupied band, once upconverted to the
    // nominal center, would run past 0 / Nyquist and fold back on itself.
    let bw = cofdm_occupied_bw(COFDM_FS, COFDM_MIN_EDGE_GUARD);
    assert!(
        COFDM_NOMINAL_CENTER - bw / 2.0 > 0.0 && COFDM_NOMINAL_CENTER + bw / 2.0 < COFDM_FS / 2.0,
        "widest band {bw} Hz does not fit around {COFDM_NOMINAL_CENTER} Hz"
    );
}

#[test]
fn every_fraction_renders_with_shaping_on() {
    // Exercises the whole render path — plan, taper, mask sizing, upconversion —
    // across the reachable band widths and both guard extremes, which is where a
    // clamped filter design or an empty carrier set would surface.
    let mut cases: Vec<(String, CofdmShaping)> = CofdmBwFraction::ALL
        .iter()
        .map(|&fr| {
            (
                fr.label().to_string(),
                CofdmShaping::default_for(fr).effective(fr),
            )
        })
        .collect();
    for guard in [COFDM_MIN_EDGE_GUARD, COFDM_MAX_EDGE_GUARD] {
        cases.push((
            format!("guard {guard}"),
            CofdmShaping {
                edge_guard: guard,
                ..CofdmShaping::default_for(CofdmBwFraction::OneQuarter)
            },
        ));
    }
    for (name, sh) in cases {
        let mut src = make_with(sh);
        let s = src.next_samples(65_536);
        assert!(s.iter().all(|v| v.is_finite()), "{name}: non-finite sample");
        assert!(rms(&s) > 0.1, "{name}: RMS {} too low", rms(&s));
    }
}
