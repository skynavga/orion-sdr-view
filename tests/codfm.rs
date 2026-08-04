// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Integration tests for the CODFM (wideband COFDM) source: signal generation,
//! the `SignalSource` trait surface, per-source sample rate, occupied
//! bandwidth, and the frame-rate-independent (dt-driven) signal/gap timing.

use orion_sdr_view::source::{
    CODFM_FS, CODFM_NOMINAL_CENTER, CodfmBwFraction, CodfmSource, SignalSource, codfm_occupied_bw,
};

/// Default construction used by most tests: 2 s signal, 1 s gap, no noise so
/// signal/silence are unambiguous, 1/4 bandwidth.
fn make() -> CodfmSource {
    CodfmSource::new(2.0, 1.0, 0.0, CodfmBwFraction::OneQuarter, CODFM_FS)
}

/// RMS of a sample block.
fn rms(s: &[f32]) -> f32 {
    (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt()
}

// ── Trait surface / sample rate ────────────────────────────────────────────

#[test]
fn reports_native_sample_rate() {
    let mut src = make();
    // CODFM runs at its own high fs, not the viewer's 48 kHz.
    assert_eq!(src.sample_rate(), CODFM_FS);
    assert_eq!(CODFM_FS, 1_920_000.0);
    // downcast round-trips.
    assert!(src.as_any_mut().downcast_mut::<CodfmSource>().is_some());
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
    let nyquist = CODFM_FS / 2.0;
    let mut prev = 0.0;
    for &fr in CodfmBwFraction::ALL {
        let bw = codfm_occupied_bw(CODFM_FS, fr);
        assert!(bw > prev, "bw not increasing at {}", fr.label());
        assert!(bw < nyquist, "bw {} exceeds Nyquist at {}", bw, fr.label());
        // Band centered on the nominal center stays within [0, Nyquist].
        assert!(CODFM_NOMINAL_CENTER - bw / 2.0 >= 0.0);
        assert!(CODFM_NOMINAL_CENTER + bw / 2.0 <= nyquist);
        prev = bw;
    }
}

// ── dt-driven signal/gap timing (frame-rate independent) ───────────────────

/// Advance the source by `dt`-second steps for `duration` seconds, recording
/// (time, in_signal) at each phase transition.
fn run_phases(src: &mut CodfmSource, dt: f32, duration: f32) -> Vec<(f32, bool)> {
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
