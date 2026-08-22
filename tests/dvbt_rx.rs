// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Integration tests for the DVB-T receiver: the complex-baseband tap,
//! acquisition across the mode matrix, frame accounting, and the measured
//! diagnostics the instrumentation panel reads.
//!
//! **Noise is scaled to the signal, never stated as an amplitude.**  A DVB-T
//! frame's mean sample power is ~4.4e-4 at unit modulator scale, so an absolute
//! noise amplitude of 0.1 is 225× the signal and every frame dies in *TPS*
//! decode long before the payload degrades interestingly.  The source's C/N is a
//! ratio, so these tests are right by construction — but anything reaching past
//! it to an amplitude would hit that wall.

use orion_sdr::modulate::ConstellationOrder;
use orion_sdr::waveform::dvb_t::{DvbTLinkParams, GuardInterval};
use orion_sdr_view::source::dvbt::{DVBT_CODE_RATES, DVBT_CONSTELLATIONS, DVBT_GUARDS};
use orion_sdr_view::source::{
    DVBT_DEFAULT_CN_DB, DVBT_DEFAULT_CODE_RATE, DVBT_DEFAULT_CONSTELLATION, DVBT_DEFAULT_GUARD,
    DvbTBandwidth, DvbTRx, DvbTShaping, DvbTSource, MAX_CN_DB, SignalSource,
    dvbt_default_center_hz, dvbt_super_frame_samples,
};

/// The viewer's real per-render-frame block size, so the tests exercise the same
/// feed granularity the decode thread sees rather than one big buffer.
const BLOCK: usize = 4096;

/// A C/N high enough that the injected noise is negligible.  There is no "off" —
/// a ratio has no infinite value.
const CLEAN_CN_DB: f32 = MAX_CN_DB;

fn link_of(
    guard: GuardInterval,
    constellation: ConstellationOrder,
    code_rate: orion_sdr::fec::PunctureRate,
) -> DvbTLinkParams {
    DvbTLinkParams {
        guard,
        constellation,
        code_rate,
    }
}

fn default_link() -> DvbTLinkParams {
    link_of(
        DVBT_DEFAULT_GUARD,
        DVBT_DEFAULT_CONSTELLATION,
        DVBT_DEFAULT_CODE_RATE,
    )
}

fn source_with(bw: DvbTBandwidth, link: DvbTLinkParams, cn_db: f32) -> DvbTSource {
    DvbTSource::new(
        60.0,
        1.0,
        cn_db,
        bw,
        link,
        DvbTShaping::off(),
        dvbt_default_center_hz(bw),
    )
}

/// Pulls `n` display samples in `BLOCK`-sized bites, exactly as the app does:
/// take the real block for the display, then hand the decoder its complex
/// counterpart.
fn pump(src: &mut DvbTSource, rx: &mut DvbTRx, n: usize) {
    let mut taken = 0;
    while taken < n {
        let want = BLOCK.min(n - taken);
        let _display = src.next_samples(want);
        let iq = src
            .last_samples_iq()
            .expect("DVB-T must offer complex baseband")
            .to_vec();
        rx.process(&iq, false);
        taken += want;
    }
}

/// Display samples spanning `n` super-frames at `guard`, in `bw`'s mode.
///
/// The mode matters: the display oversampling factor runs from 2 to 12 across
/// the six bandwidths, so a count derived from a fixed factor would pump a
/// twelfth of the intended signal at 333k and report an acquisition failure that
/// was really a short read.
fn samples_for(bw: DvbTBandwidth, guard: GuardInterval, n: usize) -> usize {
    n * dvbt_super_frame_samples(guard) * bw.display_oversample()
}

// ── Acquisition ────────────────────────────────────────────────────────────

/// The whole chain, end to end: a rendered super-frame is acquired without a
/// preamble, its TPS word read, and its payload recovered through RS + Viterbi.
#[test]
fn a_clean_link_decodes_every_frame() {
    let bw = DvbTBandwidth::Bw1MHz;
    let link = default_link();
    let mut src = source_with(bw, link, CLEAN_CN_DB);
    let mut rx = DvbTRx::new(link, src.frame_payload_len());
    pump(&mut src, &mut rx, samples_for(bw, link.guard, 2));
    let stats = rx.stats();
    assert!(
        stats.decoded >= 6,
        "expected most of 8 frames, got {stats:?}"
    );
    assert_eq!(stats.failed, 0, "clean link should not fail a frame");
}

/// Every guard / constellation / code-rate combination must acquire.  A
/// receiver whose numerology differs from the transmitter's by one field does
/// not fail loudly — it never acquires, which looks identical to a dead signal.
#[test]
fn every_mode_acquires() {
    let bw = DvbTBandwidth::Bw1MHz;
    for &guard in DVBT_GUARDS {
        for &constellation in DVBT_CONSTELLATIONS {
            for &code_rate in DVBT_CODE_RATES {
                let link = link_of(guard, constellation, code_rate);
                let mut src = source_with(bw, link, CLEAN_CN_DB);
                let mut rx = DvbTRx::new(link, src.frame_payload_len());
                pump(&mut src, &mut rx, samples_for(bw, guard, 1));
                let stats = rx.stats();
                assert!(
                    stats.decoded >= 3,
                    "{guard:?} {constellation:?} {code_rate:?}: {stats:?}"
                );
                assert_eq!(
                    stats.failed, 0,
                    "{guard:?} {constellation:?} {code_rate:?}: {stats:?}"
                );
            }
        }
    }
}

/// Every bandwidth mode too — the 24× rate span is the axis the buffer sizing
/// keys off, so a mode that renders too little to acquire from would show up
/// here and nowhere else.
#[test]
fn every_bandwidth_acquires() {
    let link = default_link();
    for &bw in DvbTBandwidth::ALL {
        let mut src = source_with(bw, link, CLEAN_CN_DB);
        let mut rx = DvbTRx::new(link, src.frame_payload_len());
        pump(&mut src, &mut rx, samples_for(bw, link.guard, 1));
        let stats = rx.stats();
        assert!(stats.decoded >= 3, "{}: {stats:?}", bw.label());
        assert_eq!(stats.failed, 0, "{}: {stats:?}", bw.label());
    }
}

/// The default C/N is a *display* choice — the floor has to be visible — so it
/// must sit well clear of the FEC cliff, at every mode.
#[test]
fn the_default_cn_decodes_cleanly_at_every_mode() {
    let bw = DvbTBandwidth::Bw1MHz;
    for &constellation in DVBT_CONSTELLATIONS {
        for &code_rate in DVBT_CODE_RATES {
            let link = link_of(DVBT_DEFAULT_GUARD, constellation, code_rate);
            let mut src = source_with(bw, link, DVBT_DEFAULT_CN_DB);
            let mut rx = DvbTRx::new(link, src.frame_payload_len());
            pump(&mut src, &mut rx, samples_for(bw, link.guard, 1));
            let stats = rx.stats();
            assert_eq!(
                stats.failed, 0,
                "{constellation:?} {code_rate:?} at {DVBT_DEFAULT_CN_DB} dB: {stats:?}"
            );
            assert!(
                stats.decoded >= 3,
                "{constellation:?} {code_rate:?}: {stats:?}"
            );
        }
    }
}

// ── Shaping ────────────────────────────────────────────────────────────────

/// The default shaping must be transparent at **every** mode, not just the
/// default one — a user switching to 64-QAM should not lose the link to a
/// setting they did not touch.
///
/// This is what forced `DVBT_DEFAULT_TAPER` to `Off`: with COFDM's 1/4 taper the
/// same sweep loses 16-QAM from r5/6 up and 64-QAM from r2/3 up.  The baseband
/// mask, which is what the default shaping now consists of, costs nothing
/// anywhere.
#[test]
fn the_default_shaping_is_transparent_at_every_mode() {
    let bw = DvbTBandwidth::Bw1MHz;
    for &constellation in DVBT_CONSTELLATIONS {
        for &code_rate in DVBT_CODE_RATES {
            let link = link_of(DVBT_DEFAULT_GUARD, constellation, code_rate);
            let mut src = DvbTSource::new(
                60.0,
                1.0,
                CLEAN_CN_DB,
                bw,
                link,
                DvbTShaping::default_enabled(),
                dvbt_default_center_hz(bw),
            );
            let mut rx = DvbTRx::new(link, src.frame_payload_len());
            pump(&mut src, &mut rx, samples_for(bw, link.guard, 1));
            let stats = rx.stats();
            assert_eq!(
                stats.failed, 0,
                "shaped {constellation:?} {code_rate:?}: {stats:?}"
            );
            assert!(
                stats.decoded >= 3,
                "shaped {constellation:?} {code_rate:?}: {stats:?}"
            );
            assert_eq!(
                stats.corrected_bytes, 0,
                "shaped {constellation:?} {code_rate:?} should need no repair"
            );
        }
    }
}

/// The symbol taper's price, pinned rather than described.
///
/// `DvbTFrameMod` windows each symbol independently instead of overlap-adding
/// consecutive ones, so the taper eats guard the cyclic prefix needs.  It is
/// free at QPSK and fatal above it — which is exactly the demonstration the row
/// exists for, and exactly why it is not the default.  If a future orion-sdr
/// overlap-adds, this test is what will notice.
#[test]
fn the_taper_costs_the_dense_constellations() {
    use orion_sdr_view::source::dvbt::{DvbTMask, DvbTTaper};
    let bw = DvbTBandwidth::Bw1MHz;
    let shaping = DvbTShaping {
        enabled: true,
        taper: DvbTTaper::Quarter,
        mask: DvbTMask::Off,
    };
    let decoded = |constellation, code_rate| {
        let link = link_of(DVBT_DEFAULT_GUARD, constellation, code_rate);
        let mut src = DvbTSource::new(
            60.0,
            1.0,
            CLEAN_CN_DB,
            bw,
            link,
            shaping,
            dvbt_default_center_hz(bw),
        );
        let mut rx = DvbTRx::new(link, src.frame_payload_len());
        pump(&mut src, &mut rx, samples_for(bw, link.guard, 1));
        rx.stats().decoded
    };
    assert!(
        decoded(ConstellationOrder::Qpsk, orion_sdr::fec::PunctureRate::R7_8) >= 3,
        "QPSK absorbs the taper"
    );
    assert_eq!(
        decoded(
            ConstellationOrder::Qam64,
            orion_sdr::fec::PunctureRate::R3_4
        ),
        0,
        "64-QAM does not — if this now decodes, the taper became transparent \
         and DVBT_DEFAULT_TAPER should be revisited"
    );
}

// ── Diagnostics ────────────────────────────────────────────────────────────

/// The measured ladder, and specifically the rung the panel drives its error
/// metrics from.
///
/// **`rs_corrected_bytes` must read zero on a clean link**, and filling the
/// frame is what guarantees it.  A sparse payload makes the receiver decode a
/// prefix, and the Forney(12,17) tail then draws on codewords the prefix never
/// covers, so Reed–Solomon quietly repairs the shortfall with no channel
/// involved at all.  A panel driving its error count off a rung with a nonzero
/// floor would show a permanently damaged link.
///
/// Re-measured on orion-sdr 0.0.64 across the 15 constellation/rate pairs: a
/// frame-filling payload reads **0** at all fifteen either way, while a 184-byte
/// payload reads **1** at all fifteen with the error-rate gate off and **0** at
/// all fifteen with it on.  The gate moves it because `want_truth` sizes the
/// decode from `dvb_t_frame_fill` rather than from the caller's `payload_len`,
/// so a measured receiver decodes the whole frame and never sees the prefix
/// artifact.  Filling the frame makes the two agree, which is the point: a rung
/// whose value depends on whether anyone is looking at it is not a measurement.
#[test]
fn a_clean_link_measures_a_clean_ladder() {
    let bw = DvbTBandwidth::Bw1MHz;
    let link = default_link();
    let mut src = source_with(bw, link, CLEAN_CN_DB);
    let mut rx = DvbTRx::new(link, src.frame_payload_len());
    pump(&mut src, &mut rx, samples_for(bw, link.guard, 1));

    let f = rx.last().expect("a frame should have decoded");
    assert_eq!(
        f.rs_corrected_bytes,
        Some(0),
        "the outer code should have nothing to repair on a clean link"
    );
    assert_eq!(rx.stats().corrected_bytes, 0);

    let score = f
        .sync_score
        .expect("guard-interval acquisition reports a score");
    assert!(
        (0.0..=1.0).contains(&score) && score > 0.5,
        "sync score {score}"
    );

    // The TPS word is what arrived, and it must match what was transmitted.
    let tps = f.tps.expect("TPS is decoded before the payload");
    assert_eq!(tps.constellation, link.constellation);
    assert_eq!(tps.code_rate_hp, link.code_rate);
    assert_eq!(tps.guard, link.guard);
    assert!(tps.frame_number <= 3);

    // The measured rungs, live since orion-sdr 0.0.64.  Zero rather than small:
    // a near-zero BER on a noiseless link would mean the truth reference is
    // misaligned with what was transmitted, which is exactly the failure 0.0.63
    // had.
    assert_eq!(f.channel_ber, Some(0.0), "CBER on a noiseless link");
    assert_eq!(f.inner_ber, Some(0.0), "IBER on a noiseless link");
    let evm = f.evm_db.expect("EVM shares the BER gate and is now on");
    assert!(evm < -25.0, "EVM {evm} dB should be clean at max C/N");
}

/// Asking for the measured rungs must not change whether a frame decodes, at
/// **any** mode — the regression pin for the defect that kept
/// `DVBT_MEASURE_ERROR_RATES` off through orion-sdr 0.0.63.
///
/// Through 0.0.63 `DvbTFrameMod` stuffed null packets until the coded stream
/// *met or exceeded* the frame capacity and transmitted only what fit, while
/// `DvbTFrameDemod` reconstructed the same packet count and asked `decode_chain`
/// for the full coded length.  The discarded remainder is the tail of the coded
/// stream, so what happened next depended on how big it was, and the whole
/// matrix has to be walked to see it:
///
/// - Ten modes overran by exactly the K=7 code's coded tail (8-12 bits), losing
///   nothing that carries information, and decoded correctly.  That is why the
///   suite was green.
/// - Four failed outright — QPSK r3/4 (1 096 bits), QPSK r7/8 (1 406), 16-QAM
///   r7/8 (940), 64-QAM r3/4 (1 096).
/// - **64-QAM r7/8 (474) decoded and reported `inner_ber` 5.0e-5 with five bytes
///   of Reed–Solomon repair, on a noiseless link.**  It is the sentinel: a fix
///   that merely clamps the plan to the LLR length turns the four failures green
///   and leaves this one quietly wrong.
///
/// So the assertion is exact zero, not "small", and it is made at every one of
/// the fifteen constellation/rate pairs.
#[test]
fn the_error_rate_gate_never_changes_the_decode() {
    use orion_sdr::demodulate::DvbTFrameDemod;
    use orion_sdr::modulate::DvbTFrameMod;
    use orion_sdr::waveform::dvb_t::DvbTFrameParams;

    let payload: Vec<u8> = (0..184).map(|i| (i % 251) as u8).collect();
    for &constellation in DVBT_CONSTELLATIONS {
        for &code_rate in DVBT_CODE_RATES {
            let link = link_of(DVBT_DEFAULT_GUARD, constellation, code_rate);
            let params = DvbTFrameParams {
                link,
                frame_number: 0,
                cell_id: 0x0A,
            };
            let mode = format!("{constellation:?} {code_rate:?}");
            let frame = DvbTFrameMod::new(params).modulate(&payload);

            let plain = DvbTFrameDemod::new(params).decode(&frame.iq, frame.n_symbols, 184);
            assert!(plain.is_ok(), "ungated decode at {mode}: {plain:?}");

            let measured = DvbTFrameDemod::new(params).with_error_rates(true).decode(
                &frame.iq,
                frame.n_symbols,
                184,
            );
            assert!(
                measured.is_ok(),
                "with_error_rates(true) must not break a noiseless decode at {mode}: {measured:?}"
            );
            let d = measured.unwrap().diagnostics;
            assert_eq!(d.channel_ber, Some(0.0), "CBER at {mode}");
            assert_eq!(d.inner_ber, Some(0.0), "IBER at {mode}");
            assert_eq!(d.rs_corrected_bytes, Some(0), "RS repair at {mode}");
        }
    }
}

/// Frame accounting has no `lost` counter, and this is why: the stream demod
/// reports `Ok` or `Err` for every frame whose samples are fully present, so
/// `decoded + failed` is the whole population.  A `count_gap` built by analogy
/// with COFDM's would key off a TPS frame number that wraps every four frames.
#[test]
fn frame_accounting_covers_every_arrival() {
    let bw = DvbTBandwidth::Bw1MHz;
    let link = default_link();
    let mut src = source_with(bw, link, CLEAN_CN_DB);
    let mut rx = DvbTRx::new(link, src.frame_payload_len());
    let n_super = 2;
    pump(&mut src, &mut rx, samples_for(bw, link.guard, n_super));
    let stats = rx.stats();
    // Four frames per super-frame, less at most one lost to the leading
    // guard-interval search window.
    let want = 4 * n_super as u64;
    assert!(
        stats.expected() >= want - 1 && stats.expected() <= want,
        "expected ~{want} frames, accounted {stats:?}"
    );
    assert_eq!(stats.frame_error_rate(), Some(0.0));
}

/// A gap edge must clear the receiver, or the partial frame left in its buffer
/// is concatenated onto the front of the next burst.
#[test]
fn reset_clears_the_accounting_and_the_buffer() {
    let bw = DvbTBandwidth::Bw1MHz;
    let link = default_link();
    let mut src = source_with(bw, link, CLEAN_CN_DB);
    let mut rx = DvbTRx::new(link, src.frame_payload_len());
    pump(&mut src, &mut rx, samples_for(bw, link.guard, 1));
    assert!(rx.stats().decoded > 0);
    rx.reset();
    assert_eq!(rx.stats().decoded, 0);
    assert_eq!(rx.stats().failed, 0);
    assert!(rx.last().is_none());
    // And it still works afterwards.
    pump(&mut src, &mut rx, samples_for(bw, link.guard, 1));
    assert!(rx.stats().decoded > 0);
}
