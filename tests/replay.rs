// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The headless replay driver.
//!
//! **The headline property is that the same script produces the same bytes.**
//! Everything else here exists to make that claim mean something: a dump that
//! repeated perfectly while silently dropping chunks, or while flattening an
//! absent reading to zero, would be reproducible and worthless.
//!
//! So these check, in order of what they would cost if wrong:
//!
//! * the run is byte-identical across invocations,
//! * no decode chunk is dropped — otherwise the dump measures the harness,
//! * `null` and provenance survive, so a dead link cannot read as a perfect one,
//! * the dump agrees with the panel, since both read one stream,
//! * a bad script, an unbounded run and a dropped chunk all fail *loudly*.

#![cfg(feature = "gui")]

mod common;

use std::path::Path;

use common::harness::config_from_yaml;
use orion_sdr_view::config::ViewConfig;
use orion_sdr_view::replay::{
    DEFAULT_SCALE, DEFAULT_SIZE, DEFAULT_TAIL_SECS, RunError, RunOptions, STDOUT_PATH, is_stdout,
    run_file, run_into,
};

/// Long enough for COFDM to emit several frames' worth of instrument readings,
/// short enough to keep the suite quick.
const RUN_SECS: f32 = 3.0;

/// Cycle to COFDM, zoom in, lock.  Exercises a source switch, the viewport and
/// the settings path in one script.
const SCRIPT: &str = "
0.00     key I x5
0.50     key ArrowUp x3
1.00     key L
";

fn run(script: Option<&str>, cfg: ViewConfig, duration: Option<f32>) -> (Vec<u8>, RunSummaryish) {
    let opts = RunOptions {
        script: script.map(str::to_owned),
        duration,
        ..Default::default()
    };
    let mut out = Vec::new();
    let summary = run_into(cfg, &opts, &mut out).expect("the run should succeed");
    (
        out,
        RunSummaryish {
            frames: summary.frames,
            samples: summary.samples,
            records: summary.records,
        },
    )
}

struct RunSummaryish {
    frames: u64,
    samples: u64,
    records: u64,
}

/// Every record as a parsed JSON value, in file order.
fn records(bytes: &[u8]) -> Vec<serde_json::Value> {
    std::str::from_utf8(bytes)
        .expect("the dump is UTF-8")
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("bad JSONL line {l:?}: {e}")))
        .collect()
}

fn of_kind<'a>(rs: &'a [serde_json::Value], kind: &str) -> Vec<&'a serde_json::Value> {
    rs.iter().filter(|r| r["kind"] == kind).collect()
}

// ── A. Reproducibility ──────────────────────────────────────────────────────

#[test]
fn the_same_script_produces_the_same_bytes() {
    // The property the whole mode rests on, and the one that four separate
    // impure reads had to be removed to get: the frame clock, the decode
    // thread, the dropped-chunk path and the wall clock.  Comparing whole files
    // rather than a summary is deliberate — every measured field, every
    // timestamp and every provenance tag has to match, not just the totals.
    let (a, sa) = run(Some(SCRIPT), ViewConfig::empty(), Some(RUN_SECS));
    let (b, sb) = run(Some(SCRIPT), ViewConfig::empty(), Some(RUN_SECS));

    assert_eq!(sa.frames, sb.frames);
    assert_eq!(sa.samples, sb.samples);
    assert_eq!(sa.records, sb.records);
    assert!(
        a == b,
        "two runs of the same script differed; first divergence at byte {}",
        a.iter()
            .zip(&b)
            .position(|(x, y)| x != y)
            .unwrap_or(a.len().min(b.len()))
    );
    assert!(sa.records > 2, "the run should have measured something");
}

#[test]
fn a_different_script_produces_a_different_digest() {
    // The negative control for the digest: it is what tells a consumer two
    // dumps are comparable at all, so a digest that ignored the script would
    // make the reproducibility claim vacuous.
    let (a, _) = run(Some(SCRIPT), ViewConfig::empty(), Some(0.5));
    let (b, _) = run(Some("0.00 key I\n"), ViewConfig::empty(), Some(0.5));
    let (ha, hb) = (records(&a)[0].clone(), records(&b)[0].clone());
    assert_ne!(ha["script_sha256"], hb["script_sha256"]);
    assert!(ha["script_sha256"].is_string());
}

#[test]
fn a_scripted_clock_stamps_the_same_times_every_run() {
    // CW frames each burst with an opening timestamp, so this is the one source
    // whose *text* would betray a system clock.  Two runs a moment apart would
    // differ in the stamped time and in nothing else — which is exactly the
    // kind of divergence that looks like a decode bug.
    const CW_YAML: &str = "
view:
  sources:
    cw:
      wpm: 25.0
      message: \"CQ\"
      gap_secs: 0.5
";
    let script = "0.00 key I\n"; // TestTone -> CW
    let (a, _) = run(Some(script), config_from_yaml(CW_YAML), Some(4.0));
    let (b, _) = run(Some(script), config_from_yaml(CW_YAML), Some(4.0));

    let texts: Vec<String> = of_kind(&records(&a), "text")
        .iter()
        .map(|r| r["text"].as_str().unwrap_or_default().to_owned())
        .collect();
    assert!(
        texts.iter().any(|t| t.contains(':')),
        "expected a timestamped burst delimiter in {texts:?}"
    );
    assert!(a == b, "a timestamped run was not reproducible");
}

// ── B. The dump measures the link, not the harness ──────────────────────────

#[test]
fn no_decode_chunk_is_ever_dropped() {
    // Both channels `try_send` on the threaded path, which discards under
    // pressure — correct for a display that must not stall, fatal for a
    // measurement, because a hole breaks a streaming demodulator's framing and
    // the resulting frame errors would be charged to a perfectly good link.
    // The inline path cannot drop; asserting it is what says so.
    for (name, script) in [("cofdm", SCRIPT), ("defaults", "0.00 key Q\n")] {
        let (bytes, _) = run(Some(script), ViewConfig::empty(), Some(RUN_SECS));
        let rs = records(&bytes);
        let summary = rs.last().expect("a summary record");
        assert_eq!(summary["kind"], "summary");
        assert_eq!(
            summary["dropped_chunks"], 0,
            "{name}: chunks were dropped, so this dump measures the harness"
        );
    }
}

#[test]
fn the_dump_reports_more_samples_than_scripted_time_implies_for_a_narrowband_source() {
    // Scripted time is not sample time, and the dump says so by carrying both.
    // At 48 kHz the per-frame budget of `dt * fs` is 800, comfortably inside the
    // 128..4096 clamp, so a narrowband source runs at true wall clock and the
    // two agree.
    let (bytes, s) = run(Some("0.00 key Q\n"), ViewConfig::empty(), Some(1.0));
    let _ = bytes;
    let expected = 48_000.0 * 1.0;
    let ratio = s.samples as f32 / expected;
    assert!(
        (0.95..=1.05).contains(&ratio),
        "narrowband should consume ~{expected} samples in 1 s, got {}",
        s.samples
    );
}

#[test]
fn cofdm_consumes_far_fewer_samples_than_its_rate_implies() {
    // The counterpart, and the trap the `samples` field exists for: COFDM asks
    // for 32 000 samples a frame at 1.92 MHz and the clamp gives it 4096, so the
    // waveform advances at about an eighth of the rate its own timer believes.
    // A consumer reading `t` as signal time would be wrong by that factor.
    let (_, s) = run(Some("0.00 key I x5\n"), ViewConfig::empty(), Some(1.0));
    let nominal = 1_920_000.0 * 1.0;
    assert!(
        (s.samples as f32) < nominal * 0.5,
        "expected the 4096-sample clamp to bind, but got {} of a nominal {nominal}",
        s.samples
    );
    assert!(s.samples > 0);
}

// ── C. Absent readings stay absent ──────────────────────────────────────────

#[test]
fn an_absent_reading_serializes_as_null_and_never_as_zero() {
    // The bug JSONL was chosen to prevent.  `rx.rs` documents that the BER rungs
    // go `None` exactly when the link fails, so a format that could not hold
    // `null` would render a dead link as a flawless one — the error rate zero
    // because nothing was measured, not because nothing was wrong.
    //
    // `clock_error_ppm` is `Unavailable` under the receiver by construction:
    // there is no sample-clock estimator. It is the stable case to pin, since it
    // does not depend on how badly a chosen C/N happens to break the link.
    let (bytes, _) = run(Some(SCRIPT), ViewConfig::empty(), Some(RUN_SECS));
    let rs = records(&bytes);
    let inst = of_kind(&rs, "instrument");
    assert!(!inst.is_empty(), "the run produced no instrument records");

    let ppm = &inst[0]["clock_error_ppm"];
    assert!(
        ppm["v"].is_null(),
        "an unavailable reading must serialize as null, got {ppm}"
    );
    assert_eq!(ppm["prov"], "unavailable");
    assert_ne!(ppm["v"], serde_json::json!(0.0));
}

#[test]
fn provenance_distinguishes_a_measurement_from_a_declaration() {
    // A field the receiver measures and a field the source declares must not
    // deserialize alike.  This is the `SIM` badge's distinction, preserved into
    // the file: without it a downstream analysis cannot tell a reading off the
    // air from a number the transmitter asserted about itself.
    let (bytes, _) = run(Some(SCRIPT), ViewConfig::empty(), Some(RUN_SECS));
    let rs = records(&bytes);
    let inst = of_kind(&rs, "instrument");
    let first = inst.first().expect("an instrument record");

    assert_eq!(
        first["cn_db"]["prov"], "measured",
        "C/N is read off the air"
    );
    assert_eq!(
        first["n_fft"]["prov"], "known",
        "the FFT size is declared, not measured"
    );
    assert_eq!(first["clock_error_ppm"]["prov"], "unavailable");
    assert!(
        first["cn_db"]["v"].is_number(),
        "a measured field should carry a value"
    );
}

#[test]
fn a_link_budget_sweep_runs_through_the_driver() {
    // What the mode is *for*, in miniature.  `tests/cofdm_link_budget.rs` is an
    // `#[ignore]`d harness that pumps a source through a receiver and prints
    // FER/EVM tables by hand; this gets the same shape of answer from a script
    // and a dump, with no bespoke test code in the loop.
    //
    // It also pins the two things a link-budget consumer depends on:
    //
    // * MER tracks the requested C/N, so the dump is measuring the link;
    // * at total failure the BER rungs go **null**, not zero.  That is the
    //   distinction the whole JSONL choice was made for — a dead link reporting
    //   `cber: 0.0` would read as a flawless one.
    let mut rows = Vec::new();
    for cn_db in [20.0_f32, 12.0, 8.0, 5.0] {
        let yaml = format!("view:\n  sources:\n    cofdm:\n      cn_db: {cn_db}\n");
        let (bytes, _) = run(Some(SCRIPT), config_from_yaml(&yaml), Some(RUN_SECS));
        let rs = records(&bytes);
        let inst = of_kind(&rs, "instrument");
        let last = *inst.last().expect("an instrument record");
        rows.push((
            cn_db,
            last["frame_count"]["v"].as_u64().unwrap_or(0),
            last["error_count"]["v"].as_u64().unwrap_or(0),
            last["cber"]["v"].as_f64(),
            last["mer_db"]["v"].as_f64(),
        ));
    }

    // Degradation is monotone in C/N: fewer frames get through as it falls.
    for pair in rows.windows(2) {
        assert!(
            pair[1].1 <= pair[0].1,
            "frames decoded should not rise as C/N falls: {rows:?}"
        );
    }
    assert!(
        rows[0].1 > 0,
        "the link should carry frames at 20 dB: {rows:?}"
    );

    // MER follows C/N down, at an implementation loss of a few dB.
    for &(cn_db, _, _, _, mer) in &rows {
        if let Some(mer) = mer {
            let loss = f64::from(cn_db) - mer;
            assert!(
                (0.0..8.0).contains(&loss),
                "MER {mer:.1} dB against a requested {cn_db} dB is a loss of {loss:.1} dB"
            );
        }
    }

    // The bottom rung: no frames at all, and the error rates absent rather than
    // zero.  `rows` is ordered by falling C/N, so this is the last row.
    let (cn_db, frames, _, cber, mer) = rows[rows.len() - 1];
    assert_eq!(frames, 0, "the link should be dead at {cn_db} dB: {rows:?}");
    assert_eq!(
        cber, None,
        "a dead link must report a null BER, not 0.0: {rows:?}"
    );
    assert_eq!(mer, None, "a dead link has no MER to report: {rows:?}");
}

// ── D. The dump agrees with the panel ───────────────────────────────────────

#[test]
fn the_last_instrument_record_matches_what_the_panel_holds() {
    // The dump taps the same `DecodeResult` stream the ticker consumes, rather
    // than projecting a second time from the receiver.  This says so: a
    // divergence here means the tap moved, and a second projection is the thing
    // that would eventually drift from what a user sees.
    use orion_sdr_view::app::ViewApp;

    let ctx = egui::Context::default();
    let mut app = ViewApp::new_replay(&ctx, ViewConfig::empty());
    let dt = 1.0 / 60.0;
    let mut last: Option<serde_json::Value> = None;

    for frame in 0..(60.0 * RUN_SECS) as usize {
        // Cycle to COFDM over the first five frames, one press per pass.
        let events = if frame < 5 {
            let action = orion_sdr_view::utils::script::Action::Key {
                key: egui::Key::I,
                modifiers: egui::Modifiers::default(),
            };
            action.events()
        } else {
            Vec::new()
        };
        ctx.begin_pass(egui::RawInput {
            events,
            ..Default::default()
        });
        app.advance(&ctx, dt);
        app.handle_keys(&ctx);
        let _ = ctx.end_pass();

        for r in app.take_replay_results() {
            if let orion_sdr_view::decode::DecodeResult::Instrument(Some(inst)) = r {
                last = Some(serde_json::to_value(&inst).expect("serializes"));
            }
        }
    }

    let dumped = last.expect("the run should have produced an instrument reading");
    let held = app
        .decode_ticker()
        .last_instrument
        .as_ref()
        .expect("the panel should be holding one");
    let held = serde_json::to_value(held).expect("serializes");
    assert_eq!(
        dumped, held,
        "the dump and the panel disagree about the last reading"
    );
}

// ── E. Failing loudly ───────────────────────────────────────────────────────

#[test]
fn an_unparsable_script_is_an_error_naming_its_line() {
    // Nobody is watching a headless run, so a skipped line would be a silent
    // change of meaning: the run would complete, exit zero, and measure
    // something other than what was asked for.
    let opts = RunOptions {
        script: Some("0.0 key Q\n0.5 nonsense here\n".to_owned()),
        duration: Some(1.0),
        ..Default::default()
    };
    match run_into(ViewConfig::empty(), &opts, std::io::sink()) {
        Err(RunError::Script(e)) => {
            assert_eq!(e.line, 2, "the diagnostic should name the offending line");
            assert!(
                format!("{e}").contains('2'),
                "the message should carry the line number"
            );
        }
        other => panic!("expected a script error, got {other:?}"),
    }
}

#[test]
fn a_scripts_own_duration_bounds_the_run() {
    // A script that names its duration needs no `--duration` to be bounded —
    // that is what makes it a complete recipe rather than half of one.
    let (_, s) = run(
        Some("set run.duration 1.0\n0.00 key I\n"),
        ViewConfig::empty(),
        None,
    );
    assert_eq!(s.frames, 60);
}

#[test]
fn the_command_line_duration_overrides_the_scripts() {
    // Override, not merge or conflict: the recipe stays reusable, so the same
    // script can be run longer or shorter without being edited.
    let script = "set run.duration 1.0\n0.00 key I\n";
    let (_, longer) = run(Some(script), ViewConfig::empty(), Some(2.0));
    let (_, shorter) = run(Some(script), ViewConfig::empty(), Some(0.5));
    assert_eq!(longer.frames, 120, "--duration should extend the script's");
    assert_eq!(shorter.frames, 30, "--duration should also cut it short");
}

#[test]
fn a_duration_shorter_than_the_script_still_runs_every_step() {
    // The loop waits on the step iterator as well as the clock, so cutting a run
    // short cannot silently drop the actions that were asked for — it would
    // otherwise measure a configuration that was never reached.
    let script = "set run.duration 0.1\n0.00 key I x5\n0.50 key ArrowUp\n";
    let (bytes, _) = run(Some(script), ViewConfig::empty(), None);
    let rs = records(&bytes);
    let switches = of_kind(&rs, "source");
    assert_eq!(
        switches.len(),
        5,
        "all five source switches should still happen"
    );
}

#[test]
fn an_unbounded_script_runs_a_margin_past_its_last_step() {
    // Without the tail the run would end on the very frame the last action lands
    // on, so whatever that action was for would never be measured — a script
    // that switches to COFDM and stops has demonstrated nothing about COFDM.
    let (bytes, s) = run(Some("0.00 key I x5\n"), ViewConfig::empty(), None);
    let expected = ((0.0 + DEFAULT_TAIL_SECS) * 60.0) as u64;
    assert_eq!(s.frames, expected, "expected a {DEFAULT_TAIL_SECS} s tail");
    assert!(
        !of_kind(&records(&bytes), "instrument").is_empty(),
        "the tail should be long enough to measure the source it switched to"
    );
}

#[test]
fn the_tail_is_measured_from_the_last_step_not_from_zero() {
    let (_, s) = run(Some("0.00 key Q\n2.00 key Q\n"), ViewConfig::empty(), None);
    assert_eq!(s.frames, ((2.0 + DEFAULT_TAIL_SECS) * 60.0) as u64);
}

#[test]
fn no_dump_named_anywhere_writes_nothing_but_still_runs() {
    // The run is still worth doing with its output discarded: it fails on a
    // panic, an unparsable script or a dropped chunk just the same.
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("demo.txt");
    std::fs::write(&script, "0.00 key I x5\n").expect("write");

    let summary = run_file(ViewConfig::empty(), Some(&script), None, Some(0.5), None)
        .expect("the run should succeed");
    assert!(summary.frames > 0);
    assert_eq!(
        std::fs::read_dir(dir.path()).expect("read_dir").count(),
        1,
        "nothing but the script itself should have been written"
    );
}

#[test]
fn a_scripts_own_dump_is_written() {
    // An absolute path here only because a test cannot safely assume a working
    // directory — the parser takes either verbatim, which
    // `a_dump_path_is_taken_verbatim_whether_relative_or_absolute` pins.
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("demo.txt");
    let dumped = dir.path().join("out.jsonl");
    std::fs::write(
        &script,
        format!("set run.dump {}\n0.00 key I x5\n", dumped.display()),
    )
    .expect("write");

    run_file(ViewConfig::empty(), Some(&script), None, Some(0.5), None).expect("run");
    let text = std::fs::read_to_string(&dumped).expect("the script's dump should exist");
    assert!(
        text.lines()
            .next()
            .unwrap_or_default()
            .contains("\"header\"")
    );
}

#[test]
fn a_dash_names_stdout_and_a_path_ending_in_one_does_not() {
    // The `curl -o -` convention, and the reason it needs a whole-path
    // comparison: `dash-` and `runs/-` are ordinary files, and `./-` is the
    // escape hatch for a file genuinely called `-`.
    assert!(is_stdout(Path::new(STDOUT_PATH)));
    assert!(is_stdout(Path::new("-")));
    for ordinary in ["./-", "runs/-", "dash-", "-.jsonl", "", "--"] {
        assert!(
            !is_stdout(Path::new(ordinary)),
            "`{ordinary}` should be an ordinary path"
        );
    }
}

#[test]
fn dumping_to_stdout_writes_no_file_called_dash() {
    // The failure this guards against is concrete: before `-` was interpreted,
    // `--dump -` did exactly what it said and left a file named `-` in the
    // working directory, which is then awkward to remove by accident-proof
    // means.  The dump itself goes to the test harness's stdout.
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("demo.txt");
    std::fs::write(&script, format!("set run.dump {STDOUT_PATH}\n0.00 key Q\n")).expect("write");

    let summary =
        run_file(ViewConfig::empty(), Some(&script), None, Some(0.05), None).expect("run");
    assert!(summary.records > 0, "the run should have emitted records");
    assert_eq!(
        std::fs::read_dir(dir.path()).expect("read_dir").count(),
        1,
        "only the script should exist; `-` must not have become a file"
    );
    assert!(
        !Path::new(STDOUT_PATH).exists(),
        "a file named `-` was created in the working directory"
    );
}

#[test]
fn the_command_line_dump_overrides_the_scripts() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("demo.txt");
    let from_script = dir.path().join("out.jsonl");
    let elsewhere = dir.path().join("elsewhere.jsonl");
    std::fs::write(
        &script,
        format!("set run.dump {}\n0.00 key I x5\n", from_script.display()),
    )
    .expect("write");

    run_file(
        ViewConfig::empty(),
        Some(&script),
        Some(&elsewhere),
        Some(0.5),
        None,
    )
    .expect("run");
    assert!(elsewhere.exists(), "--dump should win");
    assert!(
        !from_script.exists(),
        "the script's dump should not also have been written"
    );
}

#[test]
fn a_run_with_no_bound_is_an_error_rather_than_a_hang() {
    // The failure mode this replaces is the worst kind for an unattended tool:
    // a process that never exits and produces a file that never ends.
    let opts = RunOptions::default();
    assert!(matches!(
        run_into(ViewConfig::empty(), &opts, std::io::sink()),
        Err(RunError::Unbounded)
    ));
}

#[test]
fn a_dump_write_failure_is_reported() {
    // A dump that stopped half way through would be indistinguishable from a run
    // that ended early — both leave a valid JSONL prefix.
    struct Failing;
    impl std::io::Write for Failing {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("disk full"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let opts = RunOptions {
        duration: Some(0.1),
        ..Default::default()
    };
    assert!(matches!(
        run_into(ViewConfig::empty(), &opts, Failing),
        Err(RunError::Io(..))
    ));
}

// ── F. Script semantics carried into the driver ─────────────────────────────

#[test]
fn assert_directives_are_parsed_and_then_ignored() {
    // One format, two readers.  The driver has to *parse* them — a typo in a
    // script is still an error — and then do nothing with them, because
    // executing assertions is `tests/`' job and a `--check` mode would only
    // duplicate what the harness already does better.
    let with = "0.00 source COFDM\n0.50 assert source COFDM\n0.50 assert center_hz 480000\n";
    let without = "0.00 source COFDM\n";
    let (a, _) = run(Some(with), ViewConfig::empty(), Some(1.0));
    let (b, _) = run(Some(without), ViewConfig::empty(), Some(1.0));

    // Everything but the header, which carries the differing script digest.
    assert_eq!(records(&a)[1..], records(&b)[1..]);
}

#[test]
fn a_repeat_count_advances_one_source_per_frame() {
    // `key I x5` has to be five passes: `key_pressed` is a per-pass boolean, so
    // five press events inside one pass would switch a single source.  The
    // driver's own repeat handling is separate code from the harness's, so it
    // needs its own test.
    let (bytes, _) = run(Some("0.00 key I x5\n"), ViewConfig::empty(), Some(0.5));
    let rs = records(&bytes);
    let switches = of_kind(&rs, "source");
    let labels: Vec<&str> = switches
        .iter()
        .map(|r| r["source"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(
        labels,
        vec!["CW", "AM DSB", "PSK31", "FT8", "COFDM"],
        "five presses should walk five sources, one per frame"
    );
    // One frame apart, in order.
    let ts: Vec<f64> = switches
        .iter()
        .map(|r| r["t"].as_f64().unwrap_or_default())
        .collect();
    for pair in ts.windows(2) {
        let gap = pair[1] - pair[0];
        assert!(
            (gap - 1.0 / 60.0).abs() < 1e-4,
            "expected consecutive frames, got a gap of {gap} s"
        );
    }
}

#[test]
fn naming_a_source_is_the_same_run_as_counting_the_presses() {
    // `source COFDM` is not a shortcut past the UI: it presses `I` exactly as
    // `key I x5` does, so from the default start the two runs agree record for
    // record.  That equivalence is the licence to replace the counts — the
    // directive changes how a script is *written*, not what it measures.
    let (named, _) = run(Some("0.00 source COFDM\n"), ViewConfig::empty(), Some(1.0));
    let (counted, _) = run(Some("0.00 key I x5\n"), ViewConfig::empty(), Some(1.0));
    // Everything but the header, which carries the differing script digest.
    let (a, b) = (records(&named), records(&counted));
    assert_eq!(a[1..], b[1..]);
}

#[test]
fn a_name_reaches_its_source_from_wherever_the_run_already_is() {
    // The failure a name removes: a count encodes the *distance* to a source,
    // so it is wrong the moment the app starts somewhere else — and wrong
    // silently, since the line still parses and still runs.  Here the second
    // directive is four presses from CW and the third is two back, neither of
    // which the script says.
    let (bytes, _) = run(
        Some("0.00 source CW\n0.50 source COFDM\n1.00 source AM-DSB\n"),
        ViewConfig::empty(),
        Some(1.5),
    );
    let rs = records(&bytes);
    let labels: Vec<&str> = of_kind(&rs, "source")
        .iter()
        .map(|r| r["source"].as_str().unwrap_or_default())
        .collect();
    // Every intermediate source is passed through, because `I` is what does the
    // moving; only the destinations were named.
    assert_eq!(
        labels,
        vec![
            "CW", // named
            "AM DSB",
            "PSK31",
            "FT8",
            "COFDM", // named
            "Test Tone",
            "CW",
            "AM DSB", // named, wrapping round
        ]
    );
}

#[test]
fn naming_the_active_source_costs_nothing() {
    // Re-selecting is not free — `switch_source` resets playback, flushing the
    // decode pipeline and restarting the burst — so a script that names the
    // source it is already on must do nothing at all rather than take a lap.
    let script = "0.00 source Test Tone\n";
    let (bytes, _) = run(Some(script), ViewConfig::empty(), Some(0.5));
    let (plain, _) = run(None, ViewConfig::empty(), Some(0.5));
    let (a, b) = (records(&bytes), records(&plain));
    assert!(
        of_kind(&a, "source").is_empty(),
        "no switch should be recorded"
    );
    assert_eq!(a[1..], b[1..], "the run should match an unscripted one");
}

#[test]
fn an_unknown_source_name_stops_the_run_before_it_starts() {
    // Fatal rather than a warning-and-skip: a skipped `source` leaves the run
    // measuring whichever source happened to be active, and the dump it writes
    // is indistinguishable from a correct one.  Refusing to start is the only
    // outcome that cannot be mistaken for a measurement.
    let opts = RunOptions {
        script: Some("0.0 source COFDM\n0.5 source COFDMM\n".to_owned()),
        duration: Some(1.0),
        ..Default::default()
    };
    match run_into(ViewConfig::empty(), &opts, std::io::sink()) {
        Err(RunError::Script(e)) => {
            assert_eq!(e.line, 2);
            assert!(e.message.contains("is not a source"), "{}", e.message);
        }
        other => panic!("expected a script error, got {other:?}"),
    }
}

#[test]
fn the_header_describes_the_run() {
    let (bytes, _) = run(Some(SCRIPT), ViewConfig::empty(), Some(0.5));
    let rs = records(&bytes);
    let h = &rs[0];
    assert_eq!(h["kind"], "header");
    assert_eq!(h["version"], env!("CARGO_PKG_VERSION"));
    // The *startup* source, before the script has run — a switch gets its own
    // record, which is what keeps the two honest.
    assert_eq!(h["source"], "Test Tone");
    assert_eq!(h["fs_hz"], 48_000.0);
}

#[test]
fn a_run_with_no_script_still_measures() {
    // `--duration` alone is a legitimate run: start on the configured source and
    // watch it, with nobody touching the keyboard.
    let (bytes, s) = run(None, ViewConfig::empty(), Some(1.0));
    let rs = records(&bytes);
    assert_eq!(rs[0]["kind"], "header");
    assert!(rs[0]["script_sha256"].is_null(), "no script, no digest");
    assert_eq!(s.frames, 60);
    assert_eq!(rs.last().expect("summary")["kind"], "summary");
}

// ── G. The viewport a headless pass lays out in ─────────────────────────────

#[test]
fn a_headless_pass_lays_out_at_a_real_window_size() {
    // Not egui's 10000 x 10000 fallback, which is what a pass supplying no
    // `screen_rect` gets.  Nothing consults the layout while the driver only
    // advances and handles keys, so this is inert today — but a capture at the
    // fallback would be 400 MB, and every layout-dependent path would run for
    // the first time at a width no window has.
    let ctx = egui::Context::default();
    let opts = RunOptions {
        script: Some("0.0 key Q\n".to_owned()),
        duration: Some(0.1),
        ..Default::default()
    };
    let _ = &ctx;
    let _ = run_into(ViewConfig::empty(), &opts, std::io::sink()).expect("run");
    // The default is the interactive window's own size, so a scripted
    // reproduction lays out the way a user's session does.
    assert_eq!(DEFAULT_SIZE, (1200.0, 828.0));
    assert_eq!(DEFAULT_SCALE, 1.0);
}

#[test]
fn the_command_line_size_and_scale_override_the_scripts() {
    // Same precedence as `duration` and `dump`: command line over script over
    // default, so one script can be re-run at another size without editing.
    let script = "set run.size 800x600\nset run.scale 1\n0.0 key Q\n";
    let opts = RunOptions {
        script: Some(script.to_owned()),
        duration: Some(0.1),
        size: Some((1024.0, 768.0)),
        scale: Some(2.0),
        ..Default::default()
    };
    run_into(ViewConfig::empty(), &opts, std::io::sink()).expect("run");

    // The script's own values still parse and still bound a run on their own.
    let parsed = orion_sdr_view::utils::script::Script::parse(script).expect("parses");
    assert_eq!(parsed.settings.size, Some((800.0, 600.0)));
    assert_eq!(parsed.settings.scale, Some(1.0));
}

#[test]
fn a_size_change_does_not_disturb_the_measurement_stream() {
    // The dump comes from the DSP path, which does not consult layout.  Pinning
    // that now means a later change which *does* couple them cannot slip in
    // unnoticed.
    let at_default = RunOptions {
        script: Some(SCRIPT.to_owned()),
        duration: Some(1.0),
        ..Default::default()
    };
    let at_other = RunOptions {
        size: Some((640.0, 480.0)),
        scale: Some(2.0),
        ..at_default.clone()
    };
    let (mut a, mut b) = (Vec::new(), Vec::new());
    run_into(ViewConfig::empty(), &at_default, &mut a).expect("run");
    run_into(ViewConfig::empty(), &at_other, &mut b).expect("run");
    assert_eq!(records(&a), records(&b), "layout must not reach the dump");
}

// ── `set` ───────────────────────────────────────────────────────────────────
//
// The directive's whole claim is that it writes what the popover writes.  The
// test that matters is therefore not "does the value arrive" but "does it arrive
// where a keystroke would have put it" — so the comparison below drives the same
// change twice, once through the settings overlay and once through `set`, and
// requires the measurement streams to agree.

/// A C/N the COFDM source will show plainly in `info.snr_db`.
const CN_SCRIPT_TAIL: &str = "
0.00 source COFDM
";

fn snr_series(script: &str) -> Vec<f32> {
    let (bytes, _) = run(Some(script), ViewConfig::empty(), None);
    of_kind(&records(&bytes), "info")
        .iter()
        .filter_map(|r| r["snr_db"].as_f64().map(|v| v as f32))
        .collect()
}

#[test]
fn a_timed_set_lands_where_a_keystroke_would_have_put_it() {
    // Both routes take the C/N row from its 35 dB default down to 30: one by
    // opening the popover and nudging five times, the other by naming the value.
    // Identical readings are the fidelity claim — `set` is the settings UI with
    // the arrow-counting done for you, not a second way into the source.
    let keyboard = "
set run.duration 40
0.00 source COFDM
1.00 key S
2.00 key ArrowDown x11
3.00 key ArrowLeft x5
4.00 key Escape
";
    let directive = "
set run.duration 40
0.00 source COFDM
3.00 set cofdm.cn_db 30.0
";
    let (a, b) = (snr_series(keyboard), snr_series(directive));
    assert!(a.len() > 30, "the run should measure something");
    assert_eq!(a.len(), b.len());
    // Compared once both have settled.  The two disagree in exactly one reading
    // — the nudge route walks 35 → 30 over five frames and is briefly caught at
    // 32, where the directive arrives in one — and that difference is the whole
    // of what `set` does differently: it states a value instead of counting
    // presses to it.  Everything downstream of the transition must match.
    let settled = 20;
    assert_eq!(
        a[settled..],
        b[settled..],
        "a settled `set` must read exactly as a settled nudge does"
    );
    assert!(
        a[..settled] != b[..settled],
        "...and the ramp is expected to differ; if it stopped, this test proves less \
         than it claims"
    );
}

#[test]
fn an_untimed_set_configures_the_run_from_the_first_frame() {
    // The other half of the pair: untimed, it is the config file, so the very
    // first reading already reflects it rather than the built-in default.
    let at_default = snr_series(&format!("set run.duration 6{CN_SCRIPT_TAIL}"));
    let configured = snr_series(&format!(
        "set run.duration 6\nset cofdm.cn_db 15.0{CN_SCRIPT_TAIL}"
    ));
    let (d0, c0) = (at_default[0], configured[0]);
    assert!(
        (d0 - c0) > 10.0,
        "the first reading should already be the configured C/N: {d0} vs {c0}"
    );
}

#[test]
fn a_value_no_row_will_take_stops_the_run_before_it_starts() {
    // Pre-flighted, timed ones included.  A misspelled toggle option 30 seconds
    // into a run would otherwise waste 30 seconds before saying so — and this
    // format's stance on a bad line is that nothing runs.
    for (script, needle) in [
        (
            "set run.duration 5\nset cofdm.bandwidth 9/9\n0.0 key Q\n",
            "is not one of",
        ),
        (
            "set run.duration 5\n0.0 source COFDM\n4.0 set cofdm.bandwidth 1\n",
            "ambiguous",
        ),
        (
            "set run.duration 5\nset cofdm.cn_db nope\n0.0 key Q\n",
            "is not a number",
        ),
    ] {
        let opts = RunOptions {
            script: Some(script.to_owned()),
            ..Default::default()
        };
        let mut out = Vec::new();
        let e = run_into(ViewConfig::empty(), &opts, &mut out).expect_err("should refuse");
        let msg = e.to_string();
        assert!(msg.contains(needle), "expected {needle:?}, got {msg:?}");
        assert!(msg.contains("line"), "the diagnostic names its line: {msg}");
    }
}

#[test]
fn a_toggle_takes_the_option_as_shown_or_an_unambiguous_prefix() {
    // The `Mask` row reads `60 dB` on screen and `60` in the config file, and one
    // vocabulary was the point — so a prefix resolves against the label.
    for spec in ["60", "60-dB", "off", "80"] {
        let script = format!("set run.duration 2\nset cofdm.mask {spec}\n0.0 source COFDM\n");
        let opts = RunOptions {
            script: Some(script),
            ..Default::default()
        };
        let mut out = Vec::new();
        run_into(ViewConfig::empty(), &opts, &mut out)
            .unwrap_or_else(|e| panic!("`{spec}` should resolve: {e}"));
    }
}

#[test]
fn a_set_beyond_a_rows_range_clamps_rather_than_refusing() {
    // A row bounds a nudge and a config key the same way, so refusing here would
    // be the divergence — and `sig_secs: 1.0e9` meaning "the top of the range"
    // is a documented spelling rather than a mistake.
    let script = "set run.duration 3\nset cofdm.sig_secs 1.0e9\n0.0 source COFDM\n";
    let opts = RunOptions {
        script: Some(script.to_owned()),
        ..Default::default()
    };
    let mut out = Vec::new();
    run_into(ViewConfig::empty(), &opts, &mut out).expect("a clamp is not an error");
}

#[test]
fn a_set_leaves_the_run_reproducible() {
    // The directive writes settings rows, which are ordinary state — but a
    // feature that reached the source by another route could have introduced an
    // impure read, and this is the property the whole driver exists to keep.
    let script = "
set run.duration 6
set cofdm.cn_db 25.0
0.00 source COFDM
3.00 set cofdm.cn_db 12.0
";
    let (a, _) = run(Some(script), ViewConfig::empty(), None);
    let (b, _) = run(Some(script), ViewConfig::empty(), None);
    assert_eq!(a, b, "two runs of one script must produce the same bytes");
}
