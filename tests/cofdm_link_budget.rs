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
//! cargo test --release --test cofdm_link_budget -- --ignored --nocapture
//! ```
//!
//! # Driven through the replay driver
//!
//! This used to pump a `CofdmSource` into a `CofdmRx` directly, which meant the
//! project had **two** measurement paths: this one, and the one the Di bar and
//! the `X` panel read.  Two paths can disagree, and if they ever did, the tables
//! here would describe a receiver nobody was looking at.
//!
//! It now runs the real app headless and reads the dump, so every number below
//! is one a user could have seen on the panel.  Three consequences worth
//! knowing:
//!
//! * **It needs the `gui` feature**, since the driver drives `ViewApp`.  The old
//!   `--no-default-features` invocation no longer applies.
//! * **The point is bounded by time, not by frame count.**  The driver runs for
//!   a scripted duration; it has no notion of a COFDM frame and should not grow
//!   one.  [`secs_for`] converts the old frame target into a duration instead,
//!   and the `frames` column reports what was actually accounted — which is the
//!   denominator every other column is read against, so it was always the
//!   honest thing to print rather than a target to hit.
//! * **The source is configured by YAML**, exactly as a user configures it,
//!   rather than by calling the constructor with positional arguments.
//!
//! **Why the frame count is large.**  A run of a few dozen frames cannot
//! resolve a 1% FER — it reads "no errors" and then moves when someone looks
//! harder.  [`TARGET_FRAMES`] is set so the quantisation floor is well under the
//! smallest rate the tables report.

#![cfg(feature = "gui")]

mod common;

use common::harness::config_from_yaml;
use orion_sdr_view::config::ViewConfig;
use orion_sdr_view::replay::{RunOptions, run_into};
use orion_sdr_view::source::{CONTINUOUS_SIG_SECS, CofdmBwFraction};

/// Frames each point aims to account for.  1/150 resolves ~0.7%, finer than any
/// distinction these tables draw.
const TARGET_FRAMES: f32 = 150.0;

/// Samples one frame costs, scaled to full occupancy.
///
/// **Measured across all seven fractions**, which is what says the scaling is
/// real rather than assumed: 53 366 samples/frame at 1/8 down to 8 659 at 7/8,
/// and multiplying each by its fraction gives 6 671 … 7 582 — flat to ±7%.  That
/// is the expected shape, since a frame is a fixed number of bits and a narrow
/// band carries fewer per symbol.  The constant is the top of that range so
/// every fraction clears the target rather than the average missing half of
/// them.
const SAMPLES_PER_FRAME_AT_FULL: f32 = 7_600.0;

/// Samples one scripted second buys, at any bandwidth.
///
/// The per-frame budget is `dt * fs` **clamped to 4096**, and at COFDM's
/// 1.92 MHz the clamp always binds — so this is 60 × 4096 and does not vary with
/// the source's rate at all.  It is the reason a point's duration has to be
/// computed rather than guessed.
const SAMPLES_PER_SEC: f32 = 60.0 * 4096.0;

/// Scripted seconds needed for `fraction` to clear [`TARGET_FRAMES`].
///
/// Seven times longer at 1/8 than at 7/8, which is why a single constant either
/// under-measured the narrow fractions or wasted minutes on the wide ones.
fn secs_for(fraction: CofdmBwFraction) -> f32 {
    TARGET_FRAMES * SAMPLES_PER_FRAME_AT_FULL / fraction.value() / SAMPLES_PER_SEC
}

/// Switch to COFDM and then leave it alone: the measurement is of the waveform,
/// not of the interaction.
const SCRIPT: &str = "0.00 key I x5\n";

struct Point {
    fer: f32,
    evm_db: f32,
    cber: f32,
    iber: f32,
    frames: u64,
}

/// A `ViewConfig` for one (fraction, C/N) point.
///
/// **`sig_secs` asks for a continuous burst**, so no point can end in a gap: a
/// gap resets the receiver and restarts the sequence numbering, and the frame
/// accounting would charge the restart as losses.
///
/// This used to be impossible.  The old harness handed `CofdmSource::new` a
/// `sig_secs` of 1.0e6 and got a burst silently clamped to 99.99 s — a
/// configuration no user could reach, and one that truncated every point past
/// 100 s without saying so.  That is how the first calibration of
/// [`SAMPLES_PER_FRAME_AT_FULL`] came out 7× too high.  `CONTINUOUS_SIG_SECS`
/// removed the clamp, so the plain reading of a large `sig_secs` is now the
/// behaviour, and the `instrumentcleared` assertion below is the belt to its
/// braces.
fn config(fraction: CofdmBwFraction, cn_db: f32) -> ViewConfig {
    config_from_yaml(&format!(
        "view:
  sources:
    cofdm:
      bandwidth: \"{}\"
      cn_db: {cn_db}
      sig_secs: {CONTINUOUS_SIG_SECS}
",
        fraction.label()
    ))
}

/// Run one point and reduce its dump to a row.
///
/// EVM, CBER and IBER are sampled once per *newly decoded frame* — the dump
/// carries an instrument record per decode chunk, and `frame_count` is what
/// distinguishes a new frame from a re-report of the last one.  Averaging over
/// raw records instead would weight a frame by how many chunks it spanned.
fn measure(fraction: CofdmBwFraction, cn_db: f32) -> Point {
    let opts = RunOptions {
        script: Some(SCRIPT.to_owned()),
        duration: Some(secs_for(fraction)),
        ..Default::default()
    };
    let mut out = Vec::new();
    let summary =
        run_into(config(fraction, cn_db), &opts, &mut out).expect("the run should finish");
    assert!(
        summary.records > 1,
        "{fraction:?} at {cn_db} dB dumped nothing"
    );

    let mut last: Option<serde_json::Value> = None;
    let mut seen_frames = 0u64;
    let mut cleared = 0u32;
    let (mut evm, mut cber, mut iber) = (Mean::new(), Mean::new(), Mean::new());

    for line in std::str::from_utf8(&out).expect("UTF-8").lines() {
        let r: serde_json::Value = serde_json::from_str(line).expect("valid JSONL");
        // A gap edge clears the panel *and* resets the receiver, so a point that
        // saw one is reporting the tail of its run rather than all of it.  This
        // is the failure `secs_for` guards against; catching it here as well
        // means a future change to the burst length cannot reintroduce it
        // quietly.
        if r["kind"] == "instrumentcleared" {
            cleared += 1;
        }
        if r["kind"] != "instrument" {
            continue;
        }
        let frames = r["frame_count"]["v"].as_u64().unwrap_or(0);
        if frames > seen_frames {
            seen_frames = frames;
            // MER is exactly -EVM in dB and both come from one reading, so this
            // is the same figure the old harness printed, not a conversion.
            evm.push(r["mer_db"]["v"].as_f64().map(|v| -v));
            cber.push(r["cber"]["v"].as_f64());
            iber.push(r["iber"]["v"].as_f64());
        }
        last = Some(r);
    }

    assert_eq!(
        cleared,
        0,
        "{} at {cn_db} dB hit {cleared} gap(s) despite a continuous burst; the \
         receiver reset mid-point and the row would report only the frames \
         after the last one",
        fraction.label()
    );

    let last = last.expect("the run should have produced an instrument reading");
    let decoded = last["frame_count"]["v"].as_u64().unwrap_or(0);
    let bad = last["error_count"]["v"].as_u64().unwrap_or(0);
    Point {
        // Cumulative over the burst, straight off the panel's own `FER`.
        fer: last["error_rate"]["v"].as_f64().unwrap_or(f64::NAN) as f32,
        evm_db: evm.mean(),
        cber: cber.mean(),
        iber: iber.mean(),
        frames: decoded + bad,
    }
}

/// A running mean that ignores absent readings.
///
/// `None` is not zero — the BER rungs go absent exactly when the link fails, so
/// folding them in as zeros would pull a broken point's average *down* and make
/// it look better the worse it got.
struct Mean {
    sum: f64,
    n: u64,
}

impl Mean {
    fn new() -> Self {
        Self { sum: 0.0, n: 0 }
    }
    fn push(&mut self, v: Option<f64>) {
        if let Some(v) = v {
            self.sum += v;
            self.n += 1;
        }
    }
    fn mean(&self) -> f32 {
        if self.n == 0 {
            f32::NAN
        } else {
            (self.sum / self.n as f64) as f32
        }
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
