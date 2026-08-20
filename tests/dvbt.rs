// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Integration tests for the DVB-T source: frame geometry, the ×2 display
//! oversampling that DVB-T's 83% occupancy forces, buffer sizing across the
//! bandwidth modes, the derived display level, and the dt-driven signal/gap
//! timing.
//!
//! Two of these guard things that would otherwise fail *silently*:
//!
//! - [`the_payload_fills_a_conformant_sixty_eight_symbol_frame`] — one TS packet
//!   too many pushes the frame to 69 symbols and the signal stops being DVB-T,
//!   while everything still renders and decodes.
//! - [`even_display_samples_are_the_waveforms_own`] — the decoder reads the
//!   even-indexed samples of the display buffer.  An interpolator that was not
//!   half-band would leave that subsequence subtly filtered, and the receiver
//!   would degrade rather than break.

use num_complex::Complex32 as C32;
use orion_sdr::dsp::Rotator;
use orion_sdr::modulate::DvbTFrameMod;
use orion_sdr::waveform::dvb_t::{
    DVB_T_N_FFT, DvbTFrameParams, DvbTLinkParams, GuardInterval, dvb_t_occupied_bw,
};
use orion_sdr::waveform::dvb_t_ts::TS_PAYLOAD_LEN;
use orion_sdr_view::source::{
    DVBT_DEFAULT_CN_DB, DVBT_DEFAULT_CODE_RATE, DVBT_DEFAULT_CONSTELLATION, DVBT_DEFAULT_GUARD,
    DVBT_DISPLAY_OVERSAMPLE, DVBT_DISPLAY_RMS_DBFS, DVBT_SYMBOLS_PER_FRAME, DvbTBandwidth,
    DvbTShaping, DvbTSource, MAX_CN_DB, SignalSource, dvbt_center_bounds, dvbt_clamp_center,
    dvbt_default_center_hz, dvbt_frame_payload_bytes, dvbt_super_frame_samples,
};

/// The default link: G1/32, QPSK, r3/4.
fn link() -> DvbTLinkParams {
    DvbTLinkParams {
        guard: DVBT_DEFAULT_GUARD,
        constellation: DVBT_DEFAULT_CONSTELLATION,
        code_rate: DVBT_DEFAULT_CODE_RATE,
    }
}

/// The band centre these tests use unless they are specifically moving it.
fn center(bw: DvbTBandwidth) -> f32 {
    dvbt_default_center_hz(bw.display_fs())
}

/// Default construction: 2 s signal, 1 s gap, no meaningful noise, shaping off
/// so the waveform under test is the bare one.
fn make() -> DvbTSource {
    make_with(DvbTBandwidth::Bw1MHz, link(), DvbTShaping::off())
}

fn make_with(bw: DvbTBandwidth, link: DvbTLinkParams, shaping: DvbTShaping) -> DvbTSource {
    DvbTSource::new(2.0, 1.0, MAX_CN_DB, bw, link, shaping, center(bw))
}

fn rms(s: &[f32]) -> f32 {
    (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt()
}

// ── Trait surface / sample rate ────────────────────────────────────────────

#[test]
fn reports_the_oversampled_display_rate() {
    let mut src = make();
    let bw = DvbTBandwidth::Bw1MHz;
    assert_eq!(src.sample_rate(), bw.display_fs());
    assert_eq!(src.waveform_fs(), bw.fs());
    assert_eq!(src.sample_rate(), src.waveform_fs() * 2.0);
    assert!(src.as_any_mut().downcast_mut::<DvbTSource>().is_some());
}

/// The reason the display rate is not the waveform's: a DVB-T band is 83% of its
/// own sample rate, so at 1× it cannot fit the one-sided `0..fs/2` span the
/// viewer draws, at any centre frequency.  This is the arithmetic that forced
/// [`DVBT_DISPLAY_OVERSAMPLE`], and it is asserted rather than commented because
/// the plan this source was built from got it wrong.
#[test]
fn the_band_does_not_fit_at_the_waveforms_own_rate() {
    for &bw in DvbTBandwidth::ALL {
        let occupied = dvb_t_occupied_bw(bw.fs());
        assert!(
            occupied > bw.fs() / 2.0,
            "{}: {occupied} should exceed the 1x Nyquist span",
            bw.label()
        );
        assert!(
            occupied < bw.display_fs() / 2.0,
            "{}: {occupied} should fit the oversampled span",
            bw.label()
        );
    }
}

#[test]
fn the_occupied_band_is_the_nominal_channel_width() {
    for &bw in DvbTBandwidth::ALL {
        let src = make_with(bw, link(), DvbTShaping::off());
        let want = bw.occupied_hz();
        let got = src.occupied_bw_hz();
        assert!(
            (got - want).abs() < want * 1e-3,
            "{}: occupied {got} vs nominal {want}",
            bw.label()
        );
    }
}

// ── Frame geometry ─────────────────────────────────────────────────────────

/// The payload must fill a frame that stays exactly 68 symbols, and must be the
/// *largest* such payload.
///
/// Both halves matter and neither is visible at run time.  One TS packet more
/// and `DvbTFrameMod` sizes the frame to 69 symbols — still renders, still
/// decodes, no longer DVB-T.  One fewer and the receiver decodes a shorter
/// prefix for no reason, paying the whole-frame diagnostics cost against a
/// fraction of a frame's payload.
///
/// Asserted against the modulator's own symbol count rather than against a
/// restatement of the coding arithmetic, because the arithmetic is exactly what
/// would drift.
#[test]
fn the_payload_fills_a_conformant_sixty_eight_symbol_frame() {
    for &constellation in orion_sdr_view::source::dvbt::DVBT_CONSTELLATIONS {
        for &code_rate in orion_sdr_view::source::dvbt::DVBT_CODE_RATES {
            let link = DvbTLinkParams {
                guard: GuardInterval::G1_32,
                constellation,
                code_rate,
            };
            let payload_len = dvbt_frame_payload_bytes(link);
            assert!(payload_len > 0);
            let n_symbols = |bytes: usize| {
                DvbTFrameMod::new(DvbTFrameParams {
                    link,
                    frame_number: 0,
                    cell_id: 0,
                })
                .modulate(&vec![0u8; bytes])
                .n_symbols
            };
            assert_eq!(
                n_symbols(payload_len),
                DVBT_SYMBOLS_PER_FRAME,
                "{constellation:?} {code_rate:?}: {payload_len} bytes is not a 68-symbol frame"
            );
            assert_eq!(
                n_symbols(payload_len + TS_PAYLOAD_LEN),
                DVBT_SYMBOLS_PER_FRAME + 1,
                "{constellation:?} {code_rate:?}: {payload_len} bytes leaves room for another packet"
            );
        }
    }
}

/// The crest factor, and the reason it is 20 dB worse than COFDM's.
///
/// This is the measurement `DVBT_DISPLAY_RMS_DBFS` and `DvbTSource::full_scale`
/// are both built on, so the shape is pinned rather than described.  Each frame
/// codes independently, so its Forney(12,17) outer interleaver both **fills**
/// and **drains** inside the frame: symbols 0-6 and 61-67 carry branch registers
/// that are largely empty, which the convolutional coder turns into a
/// near-constant frequency-domain vector and the IFFT into an impulse.  The 54
/// symbols between them are ordinary 10-13 dB OFDM.
///
/// If a future orion-sdr carries interleaver state across frames (§4.7's
/// byte-continuous stream, currently deferred) this test is what will notice.
#[test]
fn the_crest_factor_is_the_interleaver_fill_and_drain() {
    let bw = DvbTBandwidth::Bw333kHz;
    let mut src = make_with(bw, link(), DvbTShaping::off());
    let sps = (DVB_T_N_FFT + DVBT_DEFAULT_GUARD.cp_len_2k()) * DVBT_DISPLAY_OVERSAMPLE;
    let b = src.next_samples(sps * DVBT_SYMBOLS_PER_FRAME);
    let r = rms(&b);
    let crest = |s: usize| {
        20.0 * (b[s * sps..(s + 1) * sps]
            .iter()
            .fold(0.0f32, |m, x| m.max(x.abs()))
            / r)
            .log10()
    };

    assert!(crest(0) > 25.0, "the fill should peak: symbol 0 at {}", crest(0));
    assert!(
        crest(DVBT_SYMBOLS_PER_FRAME - 1) > 25.0,
        "the drain should peak: symbol 67 at {}",
        crest(DVBT_SYMBOLS_PER_FRAME - 1)
    );
    for s in 8..=59 {
        let c = crest(s);
        assert!(c < 16.0, "symbol {s} crest {c} is not ordinary OFDM");
    }
    // The whole-burst crest, which is what the display level has to live with.
    let burst = 20.0 * (b.iter().fold(0.0f32, |m, x| m.max(x.abs())) / r).log10();
    assert!((25.0..40.0).contains(&burst), "burst crest {burst}");
}

/// `full_scale` must bound the real projection, so `overload` is a measurement
/// rather than a permanent warning.
#[test]
fn full_scale_bounds_the_real_projection() {
    for &bw in &[DvbTBandwidth::Bw333kHz, DvbTBandwidth::Bw1MHz] {
        let mut src = make_with(bw, link(), DvbTShaping::off());
        let fs = src.full_scale();
        let b = src.next_samples(300_000);
        let peak = b.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(
            peak <= fs * 1.01,
            "{}: peak {peak} exceeds full scale {fs}",
            bw.label()
        );
        // And it is not slack by an order of magnitude — a `full_scale` far
        // above the real swing would make `lvl` and `peak` both meaningless.
        assert!(
            peak > fs * 0.3,
            "{}: peak {peak} against full scale {fs}",
            bw.label()
        );
    }
}

#[test]
fn a_super_frame_is_four_frames_of_the_guards_symbol_length() {
    for &guard in orion_sdr_view::source::dvbt::DVBT_GUARDS {
        let want = 4 * 68 * (DVB_T_N_FFT + guard.cp_len_2k());
        assert_eq!(dvbt_super_frame_samples(guard), want);
    }
    // The measured table in the plan, so a change upstream to the 2K geometry
    // shows up here rather than as a buffer that quietly changed size.
    assert_eq!(dvbt_super_frame_samples(GuardInterval::G1_32), 574_464);
    assert_eq!(dvbt_super_frame_samples(GuardInterval::G1_4), 696_320);
}

// ── ×2 interpolation ───────────────────────────────────────────────────────

/// The half-band property, which is what lets the decoder read the waveform's
/// own samples out of the display buffer instead of the source keeping two.
///
/// Asserted through the source rather than against the private interpolator: the
/// claim that matters is about `last_samples_iq`, and the identity below is what
/// the receiver depends on.
#[test]
fn even_display_samples_are_the_waveforms_own() {
    let bw = DvbTBandwidth::Bw1MHz;
    let mut src = DvbTSource::new(
        60.0,
        1.0,
        30.0,
        bw,
        link(),
        DvbTShaping::off(),
        center(bw),
    );
    // An independent rotator run over the *display* stream, exactly as the
    // source's is — so this checks the projection rather than re-deriving the
    // oscillator's phase convention.
    let mut rot = Rotator::new(src.center_hz(), src.sample_rate());

    for _ in 0..4 {
        let n = 4096;
        let real = src.next_samples(n);
        let iq = src.last_samples_iq().expect("complex baseband").to_vec();
        assert_eq!(iq.len(), n / DVBT_DISPLAY_OVERSAMPLE);

        let mut worst = 0.0f32;
        let mut energy = 0.0f32;
        for (m, &r) in real.iter().enumerate() {
            let p = rot.next();
            // Only the even display samples have a decoder counterpart; the odd
            // ones are the interpolator's and are checked elsewhere.
            if m % DVBT_DISPLAY_OVERSAMPLE != 0 {
                continue;
            }
            let c = iq[m / DVBT_DISPLAY_OVERSAMPLE];
            worst = worst.max((r - (c.re * p.re - c.im * p.im)).abs());
            energy += c.norm_sqr();
        }
        assert!(energy > 0.0, "buffer is empty; nothing was asserted");
        assert!(worst < 1e-3, "projection identity broke by {worst}");
    }
}

/// The interpolated samples must carry real signal, not zeros — a zero-stuffed
/// buffer would still satisfy the identity above while halving the power and
/// putting a full-strength image on screen.
#[test]
fn interpolated_samples_carry_signal() {
    let mut src = make();
    let real = src.next_samples(8192);
    let even: Vec<f32> = real.iter().copied().step_by(2).collect();
    let odd: Vec<f32> = real.iter().copied().skip(1).step_by(2).collect();
    let (re, ro) = (rms(&even), rms(&odd));
    assert!(ro > 0.5 * re, "odd-sample RMS {ro} against even {re}");
    assert!(ro < 2.0 * re, "odd-sample RMS {ro} against even {re}");
}

// ── Buffer sizing ──────────────────────────────────────────────────────────

/// A fixed super-frame count would span 1.44 s at 333 kHz and 0.06 s at 8 MHz,
/// so the buffer targets a *duration* and is capped for memory.
#[test]
fn the_buffer_targets_a_duration_and_is_capped() {
    use orion_sdr_view::source::dvbt::{
        DVBT_BUFFER_TARGET_SECS, DVBT_MAX_BUFFER_SUPER_FRAMES, dvbt_buffer_super_frames,
    };
    for &bw in DvbTBandwidth::ALL {
        for &guard in orion_sdr_view::source::dvbt::DVBT_GUARDS {
            let n = dvbt_buffer_super_frames(guard, bw.fs());
            assert!((1..=DVBT_MAX_BUFFER_SUPER_FRAMES).contains(&n));
            // Either the target is met, or the cap is what stopped it.
            let secs = n as f32 * dvbt_super_frame_samples(guard) as f32 / bw.fs();
            assert!(
                secs >= DVBT_BUFFER_TARGET_SECS || n == DVBT_MAX_BUFFER_SUPER_FRAMES,
                "{} {guard:?}: {secs} s at {n} super-frames",
                bw.label()
            );
        }
    }
    // The narrow modes need one; the broadcast ones ask for more than the cap.
    assert_eq!(
        dvbt_buffer_super_frames(GuardInterval::G1_32, DvbTBandwidth::Bw333kHz.fs()),
        1
    );
    assert_eq!(
        dvbt_buffer_super_frames(GuardInterval::G1_32, DvbTBandwidth::Bw8MHz.fs()),
        DVBT_MAX_BUFFER_SUPER_FRAMES
    );
}

// ── Display level ──────────────────────────────────────────────────────────

/// The display gain is *derived* to hit a target RMS, not fitted — so the burst
/// lands at the same level in every mode, and above the shared signal threshold
/// in all of them.
#[test]
fn the_display_gain_hits_the_target_rms_in_every_mode() {
    let want_rms = 10f32.powf(DVBT_DISPLAY_RMS_DBFS / 20.0);
    for &bw in &[DvbTBandwidth::Bw333kHz, DvbTBandwidth::Bw1MHz] {
        for &constellation in orion_sdr_view::source::dvbt::DVBT_CONSTELLATIONS {
            let mut src = make_with(
                bw,
                DvbTLinkParams {
                    guard: DVBT_DEFAULT_GUARD,
                    constellation,
                    code_rate: DVBT_DEFAULT_CODE_RATE,
                },
                DvbTShaping::off(),
            );
            // A whole buffer: the gain normalises over the render, and a short
            // read lands in the high-crest frame head.
            let n = dvbt_super_frame_samples(DVBT_DEFAULT_GUARD) * DVBT_DISPLAY_OVERSAMPLE;
            let got = rms(&src.next_samples(n));
            let err_db = 20.0 * (got / want_rms).log10();
            assert!(
                err_db.abs() < 1.0,
                "{} {constellation:?}: RMS {got} is {err_db:.2} dB off target {want_rms}",
                bw.label()
            );
            // Unit-scale: above the shared threshold, so nothing needs a
            // per-source one.  See `DVBT_DISPLAY_RMS_DBFS`.
            assert!(
                got > orion_sdr_view::decode::SIGNAL_THRESHOLD,
                "{} {constellation:?}: RMS {got} is under the signal threshold",
                bw.label()
            );
        }
    }
}

// ── Band centre ────────────────────────────────────────────────────────────

#[test]
fn the_centre_is_clamped_to_where_the_whole_band_fits() {
    for &bw in DvbTBandwidth::ALL {
        let fs_d = bw.display_fs();
        let (lo, hi) = dvbt_center_bounds(fs_d);
        assert!(lo < hi, "{}: inverted bounds", bw.label());
        // At either bound the band edge sits exactly on the display edge.
        let half = dvb_t_occupied_bw(bw.fs()) / 2.0;
        assert!((lo - half).abs() < 1.0);
        assert!((hi - (fs_d / 2.0 - half)).abs() < 1.0);
        // The default centre is inside, and out-of-range requests pin.
        assert_eq!(dvbt_clamp_center(-1.0e9, fs_d), lo);
        assert_eq!(dvbt_clamp_center(1.0e9, fs_d), hi);
        assert_eq!(dvbt_clamp_center(f32::NAN, fs_d), dvbt_default_center_hz(fs_d));
        let src = make_with(bw, link(), DvbTShaping::off());
        assert!((lo..=hi).contains(&src.center_hz()));
    }
}

// ── Timing ─────────────────────────────────────────────────────────────────

/// Phase durations are wall-clock, driven by `advance_time`, so they do not
/// scale with the frame rate or with the (heavily non-realtime) playback rate.
#[test]
fn signal_and_gap_phases_are_dt_driven() {
    let mut src = DvbTSource::new(
        2.0,
        1.0,
        DVBT_DEFAULT_CN_DB,
        DvbTBandwidth::Bw1MHz,
        link(),
        DvbTShaping::off(),
        center(DvbTBandwidth::Bw1MHz),
    );
    assert_eq!(src.signal_phase(), Some(true));
    for _ in 0..119 {
        src.advance_time(1.0 / 60.0);
    }
    assert_eq!(src.signal_phase(), Some(true), "still inside 2 s");
    // Two more, not one: 120 additions of 1/60 in f32 land just short of 2.0.
    // The claim under test is that the phase is timed in seconds, not that the
    // accumulator is exact.
    src.advance_time(1.0 / 60.0);
    src.advance_time(1.0 / 60.0);
    assert_eq!(src.signal_phase(), Some(false), "gap begins at 2 s");
    for _ in 0..61 {
        src.advance_time(1.0 / 60.0);
    }
    assert_eq!(src.signal_phase(), Some(true), "gap ends at 1 s");
}

#[test]
fn the_gap_carries_noise_only() {
    let mut src = DvbTSource::new(
        1.0,
        1.0,
        20.0,
        DvbTBandwidth::Bw1MHz,
        link(),
        DvbTShaping::off(),
        center(DvbTBandwidth::Bw1MHz),
    );
    let sig = rms(&src.next_samples(4096));
    src.advance_time(1.5);
    assert_eq!(src.signal_phase(), Some(false));
    let gap = rms(&src.next_samples(4096));
    assert!(gap < sig * 0.5, "gap RMS {gap} against signal {sig}");
    assert!(gap > 0.0, "the gap should still carry noise");
}

// ── apply_params ───────────────────────────────────────────────────────────

/// Only the waveform set re-renders.  A C/N nudge must not rebuild a buffer that
/// costs hundreds of 2048-point IFFTs plus an interpolation pass.
#[test]
fn only_the_waveform_set_rerenders() {
    let mut src = make();
    let before = src.next_samples(1024);
    let apply = |s: &mut DvbTSource, cn: f32, shaping: DvbTShaping, link: DvbTLinkParams| {
        s.apply_params(
            2.0,
            1.0,
            cn,
            DvbTBandwidth::Bw1MHz,
            link,
            shaping,
            center(DvbTBandwidth::Bw1MHz),
        );
    };
    // A C/N change alters the noise, not the buffer: the display gain is the
    // render's own output, so an unchanged gain means no re-render happened.
    let gain = src.display_gain();
    apply(&mut src, 10.0, DvbTShaping::off(), link());
    assert_eq!(src.display_gain(), gain);
    assert_eq!(src.cn_db(), 10.0);

    // A constellation change does re-render, and changes the per-frame payload.
    let payload = src.frame_payload_len();
    apply(
        &mut src,
        10.0,
        DvbTShaping::off(),
        DvbTLinkParams {
            constellation: orion_sdr::modulate::ConstellationOrder::Qam64,
            ..link()
        },
    );
    assert_ne!(src.frame_payload_len(), payload);
    assert!(!before.is_empty());
}

#[test]
fn a_bandwidth_change_moves_both_rates_and_reclamps_the_centre() {
    let mut src = make();
    assert_eq!(src.sample_rate(), DvbTBandwidth::Bw1MHz.display_fs());
    src.apply_params(
        2.0,
        1.0,
        DVBT_DEFAULT_CN_DB,
        DvbTBandwidth::Bw8MHz,
        link(),
        DvbTShaping::off(),
        // The old centre, which is far below the new band's lower bound.
        center(DvbTBandwidth::Bw1MHz),
    );
    assert_eq!(src.sample_rate(), DvbTBandwidth::Bw8MHz.display_fs());
    let (lo, hi) = dvbt_center_bounds(DvbTBandwidth::Bw8MHz.display_fs());
    assert!((lo..=hi).contains(&src.center_hz()));
    assert_eq!(src.center_hz(), lo);
}

// ── Sanity on the complex tap ──────────────────────────────────────────────

/// Odd block sizes must not lose or duplicate a decoder sample: the even/odd
/// phase belongs to the buffer cursor, not to the block boundary, so a block
/// that starts on an odd cursor carries one fewer.
#[test]
fn the_complex_tap_follows_the_cursor_across_odd_blocks() {
    let mut src = make();
    let mut cursor_parity = 0usize;
    for n in [1usize, 2, 3, 4095, 4096, 7] {
        let real = src.next_samples(n);
        assert_eq!(real.len(), n);
        let iq: &[C32] = src.last_samples_iq().expect("complex baseband");
        // Even cursor positions in `cursor_parity..cursor_parity + n`.
        let want = (cursor_parity + n).div_ceil(DVBT_DISPLAY_OVERSAMPLE)
            - cursor_parity.div_ceil(DVBT_DISPLAY_OVERSAMPLE);
        assert_eq!(iq.len(), want, "n = {n}, cursor parity {cursor_parity}");
        cursor_parity = (cursor_parity + n) % DVBT_DISPLAY_OVERSAMPLE;
    }
}
