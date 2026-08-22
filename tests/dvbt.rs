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
    DVBT_DISPLAY_RMS_DBFS, DVBT_MIN_DISPLAY_OVERSAMPLE, DVBT_NARROW_SPAN_HZ,
    DVBT_PREFERRED_FLOOR_DB, DVBT_PREFERRED_REF_DB, DVBT_SYMBOLS_PER_FRAME, DvbTBandwidth,
    DvbTShaping, DvbTSource, MAX_CN_DB, SignalSource, dvbt_center_bounds, dvbt_clamp_center,
    dvbt_default_center_hz, dvbt_frame_payload_bytes, dvbt_preferred_ref_db,
    dvbt_super_frame_samples,
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
    dvbt_default_center_hz(bw)
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
    assert_eq!(
        src.sample_rate(),
        src.waveform_fs() * bw.display_oversample() as f32
    );
    assert!(src.as_any_mut().downcast_mut::<DvbTSource>().is_some());
}

/// The reason the display rate is not the waveform's: a DVB-T band is 83% of its
/// own sample rate, so at 1× it cannot fit the one-sided `0..fs/2` span the
/// viewer draws, at any centre frequency.  This is the arithmetic that forced
/// [`DVBT_MIN_DISPLAY_OVERSAMPLE`], and it is asserted rather than commented
/// because the plan this source was built from got it wrong.
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
    let sps = (DVB_T_N_FFT + DVBT_DEFAULT_GUARD.cp_len_2k()) * bw.display_oversample();
    let b = src.next_samples(sps * DVBT_SYMBOLS_PER_FRAME);
    let r = rms(&b);
    let crest = |s: usize| {
        20.0 * (b[s * sps..(s + 1) * sps]
            .iter()
            .fold(0.0f32, |m, x| m.max(x.abs()))
            / r)
            .log10()
    };

    assert!(
        crest(0) > 25.0,
        "the fill should peak: symbol 0 at {}",
        crest(0)
    );
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

/// The display width is fixed across a bandwidth *group*, and each mode
/// oversamples by the smallest integer that reaches it.
///
/// **This is the table `DvbTBandwidth::display_oversample` writes down**, and
/// nothing else re-derives it — a factor, a rate and a span that disagreed would
/// each look individually reasonable.  Four claims, in the order they constrain
/// each other:
///
/// 1. Every factor clears [`DVBT_MIN_DISPLAY_OVERSAMPLE`], or the band folds.
/// 2. Every mode's display Nyquist reaches its group's span, or the frame would
///    ask for more spectrum than the stream carries and `set_display_span` would
///    quietly clamp it back to a per-mode width — the exact failure the fixed
///    span exists to remove.
/// 3. No smaller factor would do, so the memory the oversampling costs is the
///    least that buys the span.  At 333k the factor is 12 and the rendered
///    buffer is ~55 MB; 13 would be 60 MB for nothing.
/// 4. The widest mode of each group fills 7/8 of the span — the margin COFDM's
///    widest bandwidth setting leaves, which is what "the two wideband sources
///    read alike at their extremes" means in numbers.
#[test]
fn the_oversample_reaches_the_span() {
    for &bw in DvbTBandwidth::ALL {
        let l = bw.display_oversample();
        let span = bw.display_span_hz();
        assert!(
            l >= DVBT_MIN_DISPLAY_OVERSAMPLE,
            "{}: factor {l} would fold the band",
            bw.label()
        );
        assert!(
            bw.display_nyquist_hz() >= span,
            "{}: Nyquist {} cannot reach span {span}",
            bw.label(),
            bw.display_nyquist_hz()
        );
        let smaller = (l - 1) as f32 * bw.fs() / 2.0;
        assert!(
            smaller < span,
            "{}: factor {} would have done, so {l} is wasteful",
            bw.label(),
            l - 1
        );
    }
    // The widest mode of each group, against COFDM's widest fraction.
    for bw in [DvbTBandwidth::Bw2MHz, DvbTBandwidth::Bw8MHz] {
        let fill = bw.occupied_hz() / bw.display_span_hz();
        assert!(
            (fill - 7.0 / 8.0).abs() < 0.01,
            "{}: fills {fill:.4} of the span, wanted 7/8",
            bw.label()
        );
    }
}

/// A group shares one width, and the two groups do not share one with each
/// other.
///
/// The negative half is the point: a single span across all six is what the
/// arithmetic refuses.  The three broadcast rates are exactly 6:7:8, so equal
/// display *rates* would need factors of 28, 24 and 21 — a 200 MS/s stream — and
/// no span the narrowband modes can reach also holds an 8 MHz band.
#[test]
fn a_bandwidth_group_shares_one_display_width() {
    let narrow = [
        DvbTBandwidth::Bw333kHz,
        DvbTBandwidth::Bw1MHz,
        DvbTBandwidth::Bw2MHz,
    ];
    let broadcast = [
        DvbTBandwidth::Bw6MHz,
        DvbTBandwidth::Bw7MHz,
        DvbTBandwidth::Bw8MHz,
    ];
    for group in [narrow.as_slice(), broadcast.as_slice()] {
        let want = group[0].display_span_hz();
        for &bw in group {
            assert_eq!(
                bw.display_span_hz(),
                want,
                "{} breaks its group's width",
                bw.label()
            );
        }
    }
    assert_eq!(narrow[0].display_span_hz(), DVBT_NARROW_SPAN_HZ);
    assert!(
        broadcast[0].display_span_hz() > narrow[0].display_span_hz(),
        "the broadcast group must be the wider one"
    );
    // And the widths are what makes a mode change legible: at a fixed span the
    // band's share of the window is the mode's own width, which is the one thing
    // a per-mode span could not show.
    for bw in narrow {
        let fill = bw.occupied_hz() / bw.display_span_hz();
        assert!(
            (fill - bw.occupied_hz() / DVBT_NARROW_SPAN_HZ).abs() < 1e-6,
            "{}: fill should be nothing but width / span",
            bw.label()
        );
    }
}

/// The spectrum reference leaves the same headroom at every bandwidth.
///
/// **The regression this exists for**: the reference was a constant, calibrated
/// when every mode oversampled by two.  Once the factor became per-mode the
/// trace climbed with it — the burst's *total* power is normalised the same way
/// everywhere, but the spectrum draws it per bin and the display's resolution
/// bandwidth is fixed, so a mode whose carriers are `L/2` times closer together
/// puts `L/2` times as many of them in each bin.  At 333k that is 7.8 dB, and
/// the peak-hold ran off the top of the pane while every other mode looked fine.
///
/// Read the way the pane reads: 4096-sample blocks, a peak-hold across them for
/// the clipping check and a linear average for the level.  A single window will
/// not do — `power_spectrum` uses at most 4096 samples, which is 0.97 of a symbol
/// at 2M but 0.16 of one at 333k, so one reading there is really a question about
/// where in a frame head the cursor happened to be.
#[test]
fn the_reference_leaves_headroom_at_every_bandwidth() {
    use orion_sdr_view::decode::power_spectrum;

    /// Symbols to average over — enough that the interleaver flush is a
    /// minority of the window at every mode rather than all of it.
    const SYMBOLS: usize = 16;
    const BLOCK: usize = 4096;

    let mut margins = Vec::new();
    for &bw in DvbTBandwidth::ALL {
        let mut src = make_with(bw, link(), DvbTShaping::off());
        let sym = (DVB_T_N_FFT + DVBT_DEFAULT_GUARD.cp_len_2k()) * bw.display_oversample();
        let blocks = (SYMBOLS * sym).div_ceil(BLOCK);

        let mut peak: Vec<f32> = Vec::new();
        let mut sum: Vec<f32> = Vec::new();
        let mut bin_hz = 0.0;
        for _ in 0..blocks {
            let real = src.next_samples(BLOCK);
            let (db, bin) = power_spectrum(&real, src.sample_rate());
            bin_hz = bin;
            if peak.is_empty() {
                peak = vec![f32::NEG_INFINITY; db.len()];
                sum = vec![0.0; db.len()];
            }
            for (i, &d) in db.iter().enumerate() {
                peak[i] = peak[i].max(d);
                sum[i] += 10f32.powf(d / 10.0);
            }
        }

        // Nothing clips: the peak-hold, which is what the pane draws in orange,
        // stays under the top of the scale.
        let ref_db = dvbt_preferred_ref_db(bw);
        let top = peak.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        assert!(
            top < ref_db,
            "{}: peak-hold {top:.1} dB is above the reference {ref_db:.1}",
            bw.label()
        );

        // And the mean in-band bin is what carries the scaling law across modes.
        let half = (bw.occupied_hz() / 2.0 / bin_hz) as usize;
        let mid = (src.center_hz() / bin_hz) as usize;
        let band = &sum[mid - half..=mid + half];
        let mean = 10.0 * (band.iter().sum::<f32>() / (band.len() * blocks) as f32).log10();
        margins.push((bw, ref_db - mean));
    }

    let (lo, hi) = (
        margins
            .iter()
            .map(|(_, m)| *m)
            .fold(f32::INFINITY, f32::min),
        margins
            .iter()
            .map(|(_, m)| *m)
            .fold(f32::NEG_INFINITY, f32::max),
    );
    assert!(
        hi - lo < 1.5,
        "the six modes should read alike; margins spread {lo:.1}..{hi:.1} dB: {:?}",
        margins
            .iter()
            .map(|(bw, m)| (bw.label(), format!("{m:.1}")))
            .collect::<Vec<_>>()
    );

    // And the correction really is the factor, not a table of fitted numbers.
    for &bw in DvbTBandwidth::ALL {
        let want = DVBT_PREFERRED_REF_DB
            + 10.0 * (bw.display_oversample() as f32 / DVBT_MIN_DISPLAY_OVERSAMPLE as f32).log10();
        assert!(
            (dvbt_preferred_ref_db(bw) - want).abs() < 1e-4,
            "{}",
            bw.label()
        );
    }
    // The floor does not move with it: the noise is flat across the whole
    // Nyquist and its total power is pinned to the signal's by the C/N
    // reference, so its per-bin level is the same in every mode.
    assert_eq!(DVBT_PREFERRED_FLOOR_DB, -90.0);
}

// ── xL interpolation ───────────────────────────────────────────────────────

/// The exact-interpolation property, which is what lets the decoder read the
/// waveform's own samples out of the display buffer instead of the source
/// keeping two.
///
/// **Run at every bandwidth**, because the oversampling factor is per-mode and
/// runs 2, 3, 4, 12.  At `L = 2` this is the familiar half-band identity; the
/// generalisation — a sinc cut at `1/(2L)` has zeros at every non-zero multiple
/// of `L` — is what the other three depend on, and nothing else in the suite
/// would notice if a phase branch were mis-indexed at `L = 3` while `L = 2` kept
/// working.
///
/// Asserted through the source rather than against the private interpolator: the
/// claim that matters is about `last_samples_iq`, and the identity below is what
/// the receiver depends on.
#[test]
fn every_l_th_display_sample_is_the_waveforms_own() {
    for &bw in DvbTBandwidth::ALL {
        let up = bw.display_oversample();
        let mut src = DvbTSource::new(60.0, 1.0, 30.0, bw, link(), DvbTShaping::off(), center(bw));
        // An independent rotator run over the *display* stream, exactly as the
        // source's is — so this checks the projection rather than re-deriving the
        // oscillator's phase convention.
        let mut rot = Rotator::new(src.center_hz(), src.sample_rate());

        // A whole number of output periods, so every block starts on phase 0 and
        // the expected count is exact rather than cursor-dependent — that is
        // `the_complex_tap_follows_the_cursor_across_odd_blocks`' job.
        let n = 4096 / up * up;
        for _ in 0..4 {
            let real = src.next_samples(n);
            let iq = src.last_samples_iq().expect("complex baseband").to_vec();
            assert_eq!(iq.len(), n / up, "{}", bw.label());

            let mut worst = 0.0f32;
            let mut energy = 0.0f32;
            for (m, &r) in real.iter().enumerate() {
                let p = rot.next();
                // Only every L-th display sample has a decoder counterpart; the
                // rest are the interpolator's and are checked below.
                if !m.is_multiple_of(up) {
                    continue;
                }
                let c = iq[m / up];
                worst = worst.max((r - (c.re * p.re - c.im * p.im)).abs());
                energy += c.norm_sqr();
            }
            assert!(energy > 0.0, "{}: buffer is empty", bw.label());
            assert!(
                worst < 1e-3,
                "{}: projection identity broke by {worst}",
                bw.label()
            );
        }
    }
}

/// The interpolated samples must carry real signal, not zeros — a zero-stuffed
/// buffer would still satisfy the identity above while dividing the power by `L`
/// and putting full-strength images on screen.
///
/// Checked per phase, so a factor with several interpolated phases cannot hide a
/// dead one behind live neighbours: at `L = 12` eleven of every twelve samples
/// come from the filter and a single broken branch would move the total RMS by
/// under half a dB.
#[test]
fn interpolated_samples_carry_signal() {
    for &bw in DvbTBandwidth::ALL {
        let up = bw.display_oversample();
        let mut src = make_with(bw, link(), DvbTShaping::off());
        let real = src.next_samples(8192 * up);
        let reference = rms(&real.iter().copied().step_by(up).collect::<Vec<_>>());
        for p in 1..up {
            let phase: Vec<f32> = real.iter().copied().skip(p).step_by(up).collect();
            let r = rms(&phase);
            assert!(
                r > 0.5 * reference && r < 2.0 * reference,
                "{} phase {p}: RMS {r} against phase 0's {reference}",
                bw.label()
            );
        }
    }
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
            let n = dvbt_super_frame_samples(DVBT_DEFAULT_GUARD) * bw.display_oversample();
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
        let nyquist = bw.display_nyquist_hz();
        let (lo, hi) = dvbt_center_bounds(bw);
        assert!(lo < hi, "{}: inverted bounds", bw.label());
        // At either bound the band edge sits exactly on the display edge.  The
        // bound is the *physical* one — where the real projection would fold —
        // not the framed window, so the band can be tuned into the oversampling
        // headroom rather than being pinned to what is on screen.
        let half = dvb_t_occupied_bw(bw.fs()) / 2.0;
        assert!((lo - half).abs() < 1.0);
        assert!((hi - (nyquist - half)).abs() < 1.0);
        // The default centre is inside, and out-of-range requests pin.
        assert_eq!(dvbt_clamp_center(-1.0e9, bw), lo);
        assert_eq!(dvbt_clamp_center(1.0e9, bw), hi);
        assert_eq!(
            dvbt_clamp_center(f32::NAN, bw),
            dvbt_default_center_hz(bw).clamp(lo, hi)
        );
        let src = make_with(bw, link(), DvbTShaping::off());
        assert!((lo..=hi).contains(&src.center_hz()));
    }
}

/// The config's default centre and the source's must be the same number.
///
/// They are computed by different code from the same intent — "mid-display" —
/// and the two rates in play make that easy to get wrong.  It was: the config
/// accessor derived its bounds from the *waveform's* rate, so its default landed
/// below the real lower bound, the source clamped it up, and the band drew hard
/// against the left edge of the display with nothing on screen to say so.
#[test]
fn the_configured_default_centre_is_mid_display() {
    use orion_sdr_view::config::ViewConfig;
    let cfg = ViewConfig::empty();
    let bw = DvbTBandwidth::Bw1MHz; // the config default
    let want = dvbt_default_center_hz(bw);
    assert!(
        (cfg.dvbt_center_hz() - want).abs() < 1.0,
        "config centre {} vs mid-display {want}",
        cfg.dvbt_center_hz()
    );
    // And it survives the source, which clamps independently.
    let src = make_with(bw, link(), DvbTShaping::off());
    assert!((src.center_hz() - want).abs() < 1.0);
    // Which means the band is symmetric in the *framed* window: both edges land
    // the same distance inside the span the viewer opens at.
    let span = bw.display_span_hz();
    let half = bw.occupied_hz() / 2.0;
    let (lo_edge, hi_edge) = (src.center_hz() - half, src.center_hz() + half);
    assert!(
        lo_edge > 0.0,
        "lower band edge {lo_edge} is off the display"
    );
    assert!(
        (lo_edge - (span - hi_edge)).abs() < 1.0,
        "band is not symmetric in the window: {lo_edge} vs {}",
        span - hi_edge
    );
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
    let (lo, hi) = dvbt_center_bounds(DvbTBandwidth::Bw8MHz);
    assert!((lo..=hi).contains(&src.center_hz()));
    assert_eq!(src.center_hz(), lo);
}

// ── Sanity on the complex tap ──────────────────────────────────────────────

/// Odd block sizes must not lose or duplicate a decoder sample: the phase
/// belongs to the buffer cursor, not to the block boundary, so a block that
/// starts mid-phase carries one fewer.
///
/// Run at 333k as well as the default, because the two have oversampling factors
/// of 12 and 4: a block of 7 samples straddles several phase boundaries at one
/// and less than a single output period at the other.
#[test]
fn the_complex_tap_follows_the_cursor_across_odd_blocks() {
    for bw in [DvbTBandwidth::Bw1MHz, DvbTBandwidth::Bw333kHz] {
        let up = bw.display_oversample();
        let mut src = make_with(bw, link(), DvbTShaping::off());
        let mut cursor = 0usize;
        for n in [1usize, 2, 3, 4095, 4096, 7] {
            let real = src.next_samples(n);
            assert_eq!(real.len(), n);
            let iq: &[C32] = src.last_samples_iq().expect("complex baseband");
            // Cursor positions that are multiples of `up` in `cursor..cursor + n`.
            let want = (cursor + n).div_ceil(up) - cursor.div_ceil(up);
            assert_eq!(iq.len(), want, "{} n = {n}, cursor {cursor}", bw.label());
            cursor = (cursor + n) % up;
        }
    }
}
