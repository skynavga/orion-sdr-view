// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Integration tests for the COFDM receiver: the complex-baseband tap, frame
//! accounting, and the measured diagnostics the instrumentation panel reads.
//!
//! Two of these exist because the obvious test suite could not tell a working
//! receiver from a broken one:
//!
//! - [`carrier_offset_is_observable`] injects a *known* frequency offset.  Every
//!   other test here runs at zero offset, where a front end that is structurally
//!   incapable of measuring offset returns the right answer anyway.  That is
//!   exactly what the first attempt at this receiver did — see `rx.rs`.
//! - [`real_output_is_the_projection_of_the_decoded_samples`] ties the samples
//!   the decoder sees to the ones the display shows.  Tapping complex baseband
//!   would otherwise leave the upconversion and the real projection untested,
//!   and a bug in either would present as a wrong picture beside a perfect BER
//!   readout.

use num_complex::Complex32 as C32;
use orion_sdr::dsp::Rotator;
use orion_sdr::fec::{FrameMetadata, FramePacket};
use orion_sdr::modulate::{McsTable, OfdmFrameMod};
use orion_sdr_view::source::{
    COFDM_DEFAULT_CN_DB, COFDM_DEFAULT_FS, CofdmBwFraction, CofdmMask, CofdmRx, CofdmShaping,
    CofdmSource, CofdmTaper, MAX_CN_DB, SignalSource, cofdm_default_center_hz,
    cofdm_edge_guard_for, cofdm_link_config,
};

/// The band centre these tests use unless they are specifically moving it.
fn center() -> f32 {
    cofdm_default_center_hz(COFDM_DEFAULT_FS)
}

/// The viewer's real per-render-frame block size, so the tests exercise the
/// same feed granularity the decode thread sees rather than one big buffer.
const BLOCK: usize = 4096;

fn source_with(shaping: CofdmShaping, fraction: CofdmBwFraction, cn_db: f32) -> CofdmSource {
    CofdmSource::new(
        60.0,
        1.0,
        cn_db,
        fraction,
        shaping,
        center(),
        COFDM_DEFAULT_FS,
    )
}

/// A C/N high enough that the injected noise is negligible for a test that
/// wants a clean waveform.  Replaces the pre-`C/N` `noise_amp = 0.0`.
const CLEAN_CN_DB: f32 = MAX_CN_DB;

/// Pulls `n` samples in `BLOCK`-sized bites, exactly as the app does: take the
/// real block for the display, then hand the decoder its complex counterpart.
fn pump(src: &mut CofdmSource, rx: &mut CofdmRx, n: usize) {
    let mut taken = 0;
    while taken < n {
        let want = BLOCK.min(n - taken);
        let _display = src.next_samples(want);
        let iq = src
            .last_samples_iq()
            .expect("COFDM must offer complex baseband")
            .to_vec();
        rx.process(&iq);
        taken += want;
    }
}

/// Samples spanning roughly `frames` COFDM frames at `fraction`.
fn samples_for(fraction: CofdmBwFraction, frames: usize) -> usize {
    // frame = preamble(4*64) + training(n_fft+cp) + header + payload symbols;
    // a generous over-estimate is fine — the tests assert on frames decoded,
    // not on exact buffer arithmetic.
    let _ = fraction;
    frames * 32_000
}

#[test]
fn waveform_is_demodulable() {
    // Run at both extremes: 7/8 is where the image margin is thinnest (120 kHz
    // between the wanted edge and the image), 1/4 is the default.
    for fraction in [CofdmBwFraction::OneQuarter, CofdmBwFraction::SevenEighths] {
        let shaping = CofdmShaping::derived(fraction);
        let mut src = source_with(shaping, fraction, CLEAN_CN_DB);
        let mut rx = CofdmRx::new(&shaping, COFDM_DEFAULT_FS);
        pump(&mut src, &mut rx, samples_for(fraction, 6));

        let stats = rx.stats();
        assert!(
            stats.decoded > 0,
            "{fraction:?}: the transmitted waveform did not decode at all \
             (decoded {}, failed {}, lost {})",
            stats.decoded,
            stats.failed,
            stats.lost
        );
    }
}

#[test]
fn waveform_is_demodulable_with_shaping_on() {
    // The taper and the spectral mask are transmit-side post-passes that eat
    // into the guard budget.  They are pictures until a decoder has to survive
    // them, which is exactly what this asserts.
    let fraction = CofdmBwFraction::OneQuarter;
    let shaping = CofdmShaping {
        enabled: true,
        edge_guard: cofdm_edge_guard_for(fraction),
        include_dc: false,
        taper: CofdmTaper::Quarter,
        mask: CofdmMask::Db60,
    };
    let mut src = source_with(shaping, fraction, CLEAN_CN_DB);
    let mut rx = CofdmRx::new(&shaping, COFDM_DEFAULT_FS);
    pump(&mut src, &mut rx, samples_for(fraction, 6));

    assert!(
        rx.stats().decoded > 0,
        "shaped waveform did not decode: {:?}",
        rx.stats()
    );
}

/// A carrier offset must actually be measurable.
///
/// **This is the test that catches a structurally-blind front end.**  Decoding
/// the real projection cannot pass it: for a real input the Schmidl & Cox
/// correlation reduces to `s[n]*s[n+L]*exp(-j*w0*L)`, one phase shared by every
/// term, so the estimate is a constant.  Measured that way it returned the same
/// -0.0134 Hz for offsets of 0, 50, 200 and 1000 Hz — and a suite that only ever
/// tested at zero offset called that a pass.
#[test]
fn carrier_offset_is_observable() {
    let fraction = CofdmBwFraction::OneQuarter;
    let shaping = CofdmShaping::derived(fraction);
    let mut src = source_with(shaping, fraction, CLEAN_CN_DB);

    let mut seen = Vec::new();
    for &offset_hz in &[0.0f32, 50.0, 200.0] {
        let mut rx = CofdmRx::new(&shaping, COFDM_DEFAULT_FS);
        src.restart();
        // Apply a genuine carrier offset to the complex baseband before it
        // reaches the receiver.
        let mut rot = Rotator::new(offset_hz, COFDM_DEFAULT_FS);
        let mut taken = 0;
        while taken < samples_for(fraction, 4) {
            let _display = src.next_samples(BLOCK);
            let iq: Vec<C32> = src
                .last_samples_iq()
                .unwrap()
                .iter()
                .map(|&c| {
                    let r = rot.next();
                    c * r
                })
                .collect();
            rx.process(&iq);
            taken += BLOCK;
        }
        let est = rx.last().and_then(|f| f.cfo_hz);
        seen.push((offset_hz, est));
        if let Some(est) = est {
            assert!(
                (est - offset_hz).abs() < 20.0,
                "offset {offset_hz} Hz was estimated as {est} Hz"
            );
        }
    }
    // Distinct offsets must produce distinct estimates.  A front end that
    // reports one constant passes every individual comparison above by
    // returning None; this is what makes it fail.
    let estimates: Vec<Option<f32>> = seen.iter().map(|&(_, e)| e).collect();
    assert!(
        estimates.iter().all(|e| e.is_some()),
        "no offset produced a usable estimate: {seen:?}"
    );
    let spread = estimates.iter().flatten().fold(f32::MIN, |m, &v| m.max(v))
        - estimates.iter().flatten().fold(f32::MAX, |m, &v| m.min(v));
    assert!(
        spread > 100.0,
        "estimates barely move across a 200 Hz sweep ({seen:?}) — the front end \
         is reporting a constant, not a measurement"
    );
}

/// The samples the decoder consumes and the samples the display shows must be
/// the same samples.
///
/// Asserted **with noise on**: at zero noise the identity holds even if the
/// complex tap were taken upstream of the impairment, which is precisely the
/// mistake that would leave `CBER` reading zero forever beside a visibly noisy
/// spectrum.
#[test]
fn real_output_is_the_projection_of_the_decoded_samples() {
    let fraction = CofdmBwFraction::OneQuarter;
    let shaping = CofdmShaping::derived(fraction);
    let mut src = source_with(shaping, fraction, 30.0);
    let mut rot = Rotator::new(center(), COFDM_DEFAULT_FS);

    for _ in 0..4 {
        let real = src.next_samples(BLOCK);
        let iq = src.last_samples_iq().expect("complex baseband").to_vec();
        assert_eq!(real.len(), iq.len());
        let mut worst = 0.0f32;
        let mut noise_energy = 0.0f32;
        for (k, (&r, &c)) in real.iter().zip(&iq).enumerate() {
            let p = rot.next();
            let projected = c.re * p.re - c.im * p.im;
            worst = worst.max((r - projected).abs());
            let _ = k;
            noise_energy += c.norm_sqr();
        }
        assert!(noise_energy > 0.0, "buffer is empty; nothing was asserted");
        assert!(
            worst < 1e-3,
            "real output is not re(iq * exp(j*2*pi*f0*k/fs)); worst deviation {worst}"
        );
    }
}

/// A retuned band must still decode, and must still project to the display.
///
/// **A demodulator that differs from the modulator by a frequency offset does
/// not fail loudly — it never acquires, which looks identical to a dead
/// signal.**  That is the same trap `cofdm_link_config` exists to prevent for
/// the numerology; the band centre is a second axis of it.  The reason it is
/// *not* a trap here is structural rather than lucky: the receiver consumes
/// complex baseband, upstream of the upconversion, so the centre is a property
/// of the display path alone.  This test is what says so — and what would fail
/// if the tap were ever moved downstream of the rotator.
#[test]
fn the_receiver_follows_the_band_centre() {
    let fraction = CofdmBwFraction::OneQuarter;
    let shaping = CofdmShaping::derived(fraction);
    // Three centres spanning the legal range at the 1/4 fraction, including one
    // low enough that the widest fractions would not fit there.
    for center_hz in [center() / 2.0, center(), center() * 1.5] {
        let mut src = CofdmSource::new(
            60.0,
            1.0,
            CLEAN_CN_DB,
            fraction,
            shaping,
            center_hz,
            COFDM_DEFAULT_FS,
        );
        assert_eq!(
            src.center_hz(),
            center_hz,
            "centre was clamped unexpectedly"
        );
        let mut rx = CofdmRx::new(&shaping, COFDM_DEFAULT_FS);
        pump(&mut src, &mut rx, samples_for(fraction, 6));

        let stats = rx.stats();
        assert!(
            stats.decoded > 0 && stats.failed == 0 && stats.lost == 0,
            "centre {center_hz} Hz: {stats:?}"
        );

        // ...and the display still shows the projection of what was decoded,
        // now rotated to the new centre.
        //
        // On a *fresh* source, because the reference rotator starts at phase
        // zero and the source's does not restart: it is advanced once per
        // emitted sample and never reset, which is what keeps the phase
        // continuous across block, loop and gap boundaries.  Rewinding the
        // buffer with `restart` leaves the two oscillators out of step by
        // however many samples have been drawn.
        let mut fresh = CofdmSource::new(
            60.0,
            1.0,
            CLEAN_CN_DB,
            fraction,
            shaping,
            center_hz,
            COFDM_DEFAULT_FS,
        );
        let mut rot = Rotator::new(center_hz, COFDM_DEFAULT_FS);
        let real = fresh.next_samples(BLOCK);
        let iq = fresh.last_samples_iq().expect("complex baseband").to_vec();
        let worst = real
            .iter()
            .zip(&iq)
            .map(|(&r, &c)| {
                let p = rot.next();
                (r - (c.re * p.re - c.im * p.im)).abs()
            })
            .fold(0.0f32, f32::max);
        assert!(
            worst < 1e-3,
            "centre {center_hz} Hz: real output is not the projection at that centre \
             (worst deviation {worst})"
        );
    }
}

#[test]
fn diagnostics_are_populated() {
    let fraction = CofdmBwFraction::OneQuarter;
    let shaping = CofdmShaping::derived(fraction);
    let mut src = source_with(shaping, fraction, CLEAN_CN_DB);
    let mut rx = CofdmRx::new(&shaping, COFDM_DEFAULT_FS);
    pump(&mut src, &mut rx, samples_for(fraction, 6));

    let facts = rx.last().expect("a frame should have been decoded");
    assert!(facts.sync_score.is_some(), "sync score missing");
    assert!(facts.cfo_hz.is_some(), "CFO missing");
    assert!(facts.evm_db.is_some(), "EVM missing");
    assert!(facts.channel_ber.is_some(), "CBER missing");
    assert!(facts.inner_ber.is_some(), "IBER missing");

    let score = facts.sync_score.unwrap();
    assert!(
        (0.0..=1.0).contains(&score) && score > 0.5,
        "sync score {score} should clear the acceptance threshold on a clean signal"
    );
}

#[test]
fn iber_never_exceeds_cber() {
    // The inner decoder cannot make things worse: its output error rate sits at
    // or below its input's.  Asserted across the C/N range the settings row
    // actually offers, down to where frames start breaking.
    let fraction = CofdmBwFraction::OneQuarter;
    let shaping = CofdmShaping::derived(fraction);
    for &cn_db in &[CLEAN_CN_DB, 45.0, 31.0, 25.0] {
        let mut src = source_with(shaping, fraction, cn_db);
        let mut rx = CofdmRx::new(&shaping, COFDM_DEFAULT_FS);
        pump(&mut src, &mut rx, samples_for(fraction, 6));

        let Some(facts) = rx.last() else { continue };
        if let (Some(cber), Some(iber)) = (facts.channel_ber, facts.inner_ber) {
            assert!(
                iber <= cber + 1e-9,
                "C/N {cn_db}: IBER {iber} exceeds CBER {cber}"
            );
        }
    }
}

#[test]
fn reset_clears_frame_accounting() {
    let fraction = CofdmBwFraction::OneQuarter;
    let shaping = CofdmShaping::derived(fraction);
    let mut src = source_with(shaping, fraction, CLEAN_CN_DB);
    let mut rx = CofdmRx::new(&shaping, COFDM_DEFAULT_FS);
    pump(&mut src, &mut rx, samples_for(fraction, 4));
    assert!(rx.stats().decoded > 0);

    rx.reset();
    assert_eq!(rx.stats().decoded, 0);
    assert_eq!(rx.stats().failed, 0);
    assert_eq!(rx.stats().lost, 0);
    assert!(rx.last().is_none());
}

#[test]
fn frame_error_rate_is_none_before_any_frame() {
    let shaping = CofdmShaping::derived(CofdmBwFraction::OneQuarter);
    let rx = CofdmRx::new(&shaping, COFDM_DEFAULT_FS);
    assert!(rx.stats().frame_error_rate().is_none());
}

/// Every frame the source transmits must arrive.
///
/// **This is the regression test for the silent frame-drop in orion-sdr's
/// streaming receiver** (`try_one_frame` takes the highest-*ranked* sync
/// candidate rather than the earliest, then drains the buffer past every frame
/// before it).  It must run with noise: at zero noise every preamble scores
/// exactly 1.000, the candidate sort is stable, rank order equals position
/// order, and the bug is invisible.
#[test]
fn no_frames_are_silently_dropped() {
    let fraction = CofdmBwFraction::OneQuarter;
    let shaping = CofdmShaping::derived(fraction);
    // The settings row's default C/N.  An excellent link -- any loss here is a
    // receiver defect, not a channel effect.
    let mut src = source_with(shaping, fraction, COFDM_DEFAULT_CN_DB);
    let mut rx = CofdmRx::new(&shaping, COFDM_DEFAULT_FS);
    pump(&mut src, &mut rx, samples_for(fraction, 8));

    let stats = rx.stats();
    assert!(
        stats.decoded > 0,
        "nothing decoded at all: {stats:?} -- this test cannot say anything"
    );
    assert_eq!(
        stats.lost,
        0,
        "{} of {} frames vanished with {} reported errors -- \
         sequence gaps are the only evidence they ever existed",
        stats.lost,
        stats.expected(),
        stats.failed
    );
}

/// Looping the source's frame buffer must not invent losses.
///
/// The source emits `COFDM_BUFFER_FRAMES` frames and then repeats them, so
/// `sequence_num` wraps from 39 back to 0 — a normal event that must count as
/// zero missing frames. Getting this wrong is easy and quiet: computing the gap
/// with `wrapping_sub` reduces modulo 2^32, which only commutes with `% 40` if
/// 40 divides 2^32. It does not, so every wrap reported exactly `2^32 mod 40`
/// = 16 phantom losses.
///
/// Runs at the 7/8 fraction because its shorter frames reach the wrap soonest;
/// the narrow fractions cannot get there inside a reasonable test and would
/// pass regardless.
#[test]
fn looping_the_buffer_does_not_invent_frame_losses() {
    let fraction = CofdmBwFraction::SevenEighths;
    let shaping = CofdmShaping::derived(fraction);
    let mut src = source_with(shaping, fraction, CLEAN_CN_DB);
    let mut rx = CofdmRx::new(&shaping, COFDM_DEFAULT_FS);
    // Comfortably more than one pass through the 40-frame buffer.
    pump(&mut src, &mut rx, 500_000);

    let stats = rx.stats();
    assert!(
        stats.decoded > 40,
        "must decode past the buffer wrap to test anything, got {stats:?}"
    );
    assert_eq!(
        stats.lost, 0,
        "phantom losses across the buffer wrap: {stats:?}"
    );
}

/// A failed frame is one bad frame, not two.
///
/// The receiver skips past a frame it could not decode, so the next good
/// frame's `sequence_num` is two ahead of the last — and a gap detector that
/// does not know about the failure counts that same frame again. Measured
/// before this was fixed: `failed` and `lost` were *identical* at every noise
/// level from 0.53 to 1.00, so the panel reported exactly twice the true error
/// count.
///
/// The load-bearing assertion is that the accounted-for frame total does not
/// move with noise. The same signal duration carries the same number of frames
/// however many of them decode; a total that grows as the link degrades is
/// double-counting, whatever the individual columns say.
#[test]
fn a_failed_frame_is_not_also_counted_as_a_lost_one() {
    let fraction = CofdmBwFraction::OneQuarter;
    let shaping = CofdmShaping::derived(fraction);
    const SAMPLES: usize = 3_000_000;

    let mut totals = Vec::new();
    // The last value has to be past the FEC cliff or the final assertion is
    // vacuous; the first has to be clean or the baseline frame count is not one.
    for &cn_db in &[CLEAN_CN_DB, 24.0, 19.0] {
        let mut src = source_with(shaping, fraction, cn_db);
        let mut rx = CofdmRx::new(&shaping, COFDM_DEFAULT_FS);
        pump(&mut src, &mut rx, SAMPLES);
        let stats = rx.stats();
        assert_eq!(
            stats.lost, 0,
            "C/N {cn_db}: {} gap-inferred losses that the {} reported \
             failures already account for",
            stats.lost, stats.failed
        );
        totals.push((cn_db, stats.expected(), stats.failed));
    }

    let baseline = totals[0].1;
    for &(cn_db, total, failed) in &totals {
        assert!(
            total.abs_diff(baseline) <= 1,
            "C/N {cn_db}: accounted for {total} frames against {baseline} on a \
             clean link ({failed} failures) — the same burst cannot contain more \
             frames just because more of them broke"
        );
    }
    // And the sweep has to actually reach the errors, or it proves nothing.
    assert!(
        totals.iter().any(|&(_, _, failed)| failed > 0),
        "no frame ever failed; lower the C/N or this test is vacuous"
    );
}

/// The display gain must scale every segment of a frame by the same factor.
///
/// **This is the invariant that broke before.** Until orion-sdr 0.0.57
/// `generate_ofdm_preamble` ignored its config, so the modulator gain reached
/// the payload but not the preamble — at gain 121 the payload came out 4x
/// louder than the preamble it was supposed to be acquired from, the Schmidl &
/// Cox correlator's energy normalisation was swamped, and the sync score
/// collapsed from 1.000 to 0.095. The signal became undecodable while looking
/// perfectly healthy on screen.
///
/// The gain is now *derived* in `render` — normalising the burst to
/// `COFDM_DISPLAY_RMS_DBFS` rather than applying a fitted constant — which
/// makes the uniformity easier to break, not harder: the scalar is computed
/// from the buffer it is then applied to.  It still lives in `render` rather
/// than in `OfdmConfig`, which changes
/// the mechanism delivering it, so the property is asserted directly: each
/// segment of the source's own output, against the same link built at unit
/// scale, must differ by exactly the source's derived display gain.
#[test]
fn the_display_gain_scales_every_segment_alike() {
    let fraction = CofdmBwFraction::OneQuarter;
    let shaping = CofdmShaping::derived(fraction);

    // The same link the source renders, at unit scale.
    let (cfg, preamble) = cofdm_link_config(&shaping, COFDM_DEFAULT_FS);
    assert_eq!(
        cfg.gain, 1.0,
        "the waveform config must carry no display gain"
    );
    let modu = OfdmFrameMod::new(cfg, McsTable::default_ladder(), preamble);
    let reference = modu.modulate_frame(
        &FramePacket::new(FrameMetadata::new(0, 1), vec![0x5a; 184]),
        0,
    );

    // The source's own first frame, effectively noise-free so only the derived
    // display gain differs.
    let mut src = source_with(shaping, fraction, CLEAN_CN_DB);
    let gain = src.display_gain();
    let _ = src.next_samples(reference.len());
    let emitted = src.last_samples_iq().expect("complex baseband").to_vec();
    assert_eq!(emitted.len(), reference.len());

    let rms = |s: &[C32]| (s.iter().map(|c| c.norm_sqr()).sum::<f32>() / s.len() as f32).sqrt();
    let repeats = 4 * 64;
    let training = repeats + 256 + 32;
    for (name, span, tol) in [
        // Preamble and training carry fixed patterns, so these are exact.
        ("preamble", 0..repeats, 0.001),
        ("training", repeats..training, 0.001),
        // The payload's bits differ from the reference's, but QPSK is constant
        // modulus per carrier, so its RMS is near-invariant to them.
        ("payload", training..reference.len(), 0.02),
    ] {
        let scale = rms(&emitted[span.clone()]) / rms(&reference[span]);
        assert!(
            (scale / gain - 1.0).abs() < tol,
            "{name} scaled by {scale:.3}, not {gain} — a gain that reaches \
             some segments and not others is what made this source unacquirable"
        );
    }
}
