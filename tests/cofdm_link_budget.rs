// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Link-budget measurement harness for the COFDM source.
//!
//! **Not an assertion suite** — a reproducible way to produce the FER and EVM
//! tables that the impairment plan and `CHANGELOG` quote.  Marked `#[ignore]`
//! so it does not run in CI: it pumps tens of millions of samples through a
//! full receiver, which is minutes of work for numbers that only need
//! remeasuring when the waveform or the impairment model changes.
//!
//! Run with:
//!
//! ```text
//! cargo test --no-default-features --release --test cofdm_link_budget -- --ignored --nocapture
//! ```
//!
//! **Why the frame count is large.**  A run of a few dozen frames cannot
//! resolve a 1% FER — it reads "no errors" and then moves when someone looks
//! harder.  `FRAMES` is set so the quantisation floor is well under the
//! smallest rate the tables report.

use orion_sdr_view::source::{
    COFDM_DEFAULT_FS, CofdmBwFraction, CofdmRx, CofdmShaping, CofdmSource, SignalSource,
    cofdm_default_center_hz,
};

/// The band centre these tests use unless they are specifically moving it.
fn center() -> f32 {
    cofdm_default_center_hz(COFDM_DEFAULT_FS)
}

const BLOCK: usize = 4096;
/// Frames accounted per measurement point.  1/150 resolves ~0.7%, which is
/// finer than any distinction these tables draw, and the narrow fractions cost
/// 56k samples per frame — so this is where resolution stops being free.
const FRAMES: u64 = 150;
/// Give up on a point rather than spin if the receiver never acquires.  Past
/// the cliff it does not, and an unbounded point would dominate the run.
const MAX_SAMPLES: usize = 20_000_000;

struct Point {
    fer: f32,
    evm_db: f32,
    cber: f32,
    iber: f32,
    frames: u64,
}

/// Pump one (fraction, C/N) point until `FRAMES` frames are accounted for.
///
/// EVM, CBER and IBER are sampled once per *newly decoded* frame rather than
/// once per block, so a slow block cadence cannot weight one frame more than
/// another.
fn measure(fraction: CofdmBwFraction, cn_db: f32) -> Point {
    let shaping = CofdmShaping::default_for(fraction);
    let effective = shaping.effective(fraction, center(), COFDM_DEFAULT_FS);
    // A signal phase long enough that the burst never ends: a gap would reset
    // the receiver mid-measurement and restart the sequence numbering.
    let mut src = CofdmSource::new(
        1.0e6,
        1.0,
        cn_db,
        fraction,
        shaping,
        center(),
        COFDM_DEFAULT_FS,
    );
    let mut rx = CofdmRx::new(&effective, COFDM_DEFAULT_FS);

    let (mut evm_sum, mut cber_sum, mut iber_sum) = (0.0f64, 0.0f64, 0.0f64);
    let (mut evm_n, mut cber_n, mut iber_n) = (0u64, 0u64, 0u64);
    let mut seen_decoded = 0u64;
    let mut taken = 0usize;

    while rx.stats().expected() < FRAMES && taken < MAX_SAMPLES {
        let _display = src.next_samples(BLOCK);
        let iq = src.last_samples_iq().expect("complex baseband").to_vec();
        rx.process(&iq);
        taken += BLOCK;

        let decoded = rx.stats().decoded;
        if decoded > seen_decoded {
            seen_decoded = decoded;
            if let Some(f) = rx.last() {
                if let Some(v) = f.evm_db {
                    evm_sum += v as f64;
                    evm_n += 1;
                }
                if let Some(v) = f.channel_ber {
                    cber_sum += v as f64;
                    cber_n += 1;
                }
                if let Some(v) = f.inner_ber {
                    iber_sum += v as f64;
                    iber_n += 1;
                }
            }
        }
    }

    let stats = rx.stats();
    let expected = stats.expected().max(1);
    let mean = |sum: f64, n: u64| {
        if n == 0 {
            f32::NAN
        } else {
            (sum / n as f64) as f32
        }
    };
    Point {
        fer: (stats.failed + stats.lost) as f32 / expected as f32,
        evm_db: mean(evm_sum, evm_n),
        cber: mean(cber_sum, cber_n),
        iber: mean(iber_sum, iber_n),
        frames: stats.expected(),
    }
}

/// EVM across every bandwidth fraction at one C/N.
///
/// The property this pins: per-carrier SNR is *flat* across fractions, because
/// rendered signal power tracks occupied bandwidth and in-band noise power
/// tracks it too, so the ratio cancels.  It held under the old absolute-amplitude
/// impairment and must still hold now that the knob is a ratio.
#[test]
#[ignore = "measurement harness; run explicitly with --ignored"]
fn evm_across_fractions() {
    const CN_DB: f32 = 25.0;
    println!("\n## EVM across bandwidth fractions at C/N {CN_DB:.0} dB\n");
    println!("| Fraction | EVM (dB) | CBER | IBER | frames |");
    println!("| --- | --- | --- | --- | --- |");
    let mut lo = f32::MAX;
    let mut hi = f32::MIN;
    for &fr in CofdmBwFraction::ALL {
        let p = measure(fr, CN_DB);
        lo = lo.min(p.evm_db);
        hi = hi.max(p.evm_db);
        println!(
            "| {} | {:.1} | {:.2e} | {:.2e} | {} |",
            fr.label(),
            p.evm_db,
            p.cber,
            p.iber,
            p.frames
        );
    }
    println!("\nspread: {:.1} dB\n", hi - lo);
}

/// FER against C/N, per bandwidth fraction.
#[test]
#[ignore = "measurement harness; run explicitly with --ignored"]
fn fer_against_cn() {
    const SWEEP: &[f32] = &[25.0, 20.0, 17.0, 14.0, 11.0];
    const FRACTIONS: &[CofdmBwFraction] = &[
        CofdmBwFraction::OneEighth,
        CofdmBwFraction::OneQuarter,
        CofdmBwFraction::OneHalf,
        CofdmBwFraction::SevenEighths,
    ];
    println!("\n## FER against C/N\n");
    print!("| Fraction |");
    for cn in SWEEP {
        print!(" {cn:.0} dB |");
    }
    println!();
    print!("| --- |");
    for _ in SWEEP {
        print!(" --- |");
    }
    println!();
    for &fr in FRACTIONS {
        print!("| {} |", fr.label());
        for &cn in SWEEP {
            print!(" {:.3} |", measure(fr, cn).fer);
        }
        println!();
    }
    println!();
}
