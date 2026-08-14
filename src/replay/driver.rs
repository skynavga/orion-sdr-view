// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The headless replay driver: run the app from a script, with no window, no
//! renderer and no GPU, and emit the measurement stream as JSON Lines.
//!
//! ```text
//! orion-sdr-view --headless --script demo.txt --dump run.jsonl
//! ```
//!
//! **Why a measurement dump rather than video.**  Video answers *does it look
//! right*; a dump answers *is it correct*, and does so machine-checkably.  For a
//! DSP tool the second is the more valuable of the two and far the cheaper —
//! there is no offscreen render here, so no headless GPU device and no software
//! rasterizer in CI.
//!
//! It already existed in embryo: `tests/cofdm_link_budget.rs` is an `#[ignore]`d
//! harness that pumps a source through a receiver and prints FER/EVM tables, and
//! is what produced the figures in the 0.0.23 release notes.  This is that
//! pattern promoted from hand-written test code to a mode — any source, any
//! settings, any scripted interaction, one output format.
//!
//! # What makes a run reproducible
//!
//! Four impure reads had to go, and all four are handled by
//! [`ViewApp::new_replay`](crate::app::ViewApp::new_replay) and this loop:
//!
//! 1. **The frame clock.**  `dt` is fixed at [`RunOptions::dt`] rather than
//!    measured, so the sample budget, the source's timeline and the scroll
//!    pacing all repeat.  (Shipped in 0.0.25.)
//! 2. **The decode thread.**  Results arrived when the scheduler got to them.
//!    A replay run decodes inline.
//! 3. **Dropped chunks.**  Both channels `try_send`, which discards under
//!    pressure.  Inline, nothing is ever in flight across a frame boundary —
//!    and the driver asserts the drop count is zero rather than assuming it.
//! 4. **The wall clock.**  CW and PSK31 stamp each burst and FT8 stamps each
//!    decoded frame, so the time of day would land in the dump.  A replay run
//!    uses [`Clock::Scripted`](crate::utils::time::Clock).
//!
//! Both PRNGs were already seeded from fixed constants, so the signal path
//! needed nothing.
//!
//! # What a dump's `t` is, and is not
//!
//! `t` is **scripted** time: frames × `dt`.  It is not a claim about how much
//! signal elapsed.  `dt` drives both the source's phase timer, which has
//! wall-clock semantics, and its sample budget of `dt * fs` — and that budget is
//! clamped to 4096.  At COFDM's 1.92 MHz the clamp binds hard: a `dt` of 1/60 s
//! asks for 32 000 samples and gets 4096, so the waveform advances at about an
//! eighth of the rate its own timer thinks it does.  Every record therefore also
//! carries `samples`, the cumulative count actually consumed, which is the
//! honest measure of signal time.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::app::ViewApp;
use crate::config::ViewConfig;
use crate::decode::DecodeResult;
use crate::utils::script::{Action, Script};

use super::dump::{Dump, Record, sha256_hex};

/// How a replay run ended.
#[derive(Debug)]
pub enum RunError {
    /// The script did not parse.  Carries the offending line.
    Script(crate::utils::script::ScriptError),
    /// A file could not be read or written.
    Io(PathBuf, std::io::Error),
    /// The decoder's sequence counter shows a hole.
    ///
    /// Fatal rather than a warning: a gap breaks a streaming demodulator's
    /// framing, so every frame error after it is the harness's fault and the
    /// dump is not a measurement of the link.
    DroppedChunks(u64),
    /// Neither `--script` nor `--duration` was given, so the run has no end.
    Unbounded,
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Script(e) => write!(f, "script: {e}"),
            RunError::Io(p, e) => write!(f, "{}: {e}", p.display()),
            RunError::DroppedChunks(n) => write!(
                f,
                "{n} decode chunk(s) were dropped; the dump measures the harness, not the link"
            ),
            RunError::Unbounded => {
                write!(f, "a headless run needs --script or --duration to bound it")
            }
        }
    }
}

impl std::error::Error for RunError {}

/// How long a run continues past its last scripted step when nothing names a
/// duration.
///
/// **Without a tail the run would end on the very frame the last action lands
/// on**, so whatever that action was for — a source switch, a retune, a settings
/// change — would never be measured. One second is enough for several COFDM
/// instrument readings and a CW character or two, and it is an *absolute*
/// margin rather than a fraction because what it buys is a fixed amount of
/// decoding, not a proportion of the script.
pub const DEFAULT_TAIL_SECS: f32 = 1.0;

/// What a replay run reports back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSummary {
    pub frames: u64,
    pub samples: u64,
    pub records: u64,
}

/// Knobs for one run.
///
/// The script is carried as **source text, not a path**: the driver does no
/// input I/O, so a test can hand it a string literal and a caller can supply a
/// script from anywhere.  Reading the file is [`run_file`]'s job.
#[derive(Debug, Clone)]
pub struct RunOptions {
    /// The script to replay.  `None` runs the app untouched for `duration`.
    pub script: Option<String>,
    /// Bound in scripted seconds.
    ///
    /// **Overrides the script's own `duration`.**  `None` defers to the script;
    /// if the script names none either, the run lasts until its last step plus
    /// [`DEFAULT_TAIL_SECS`].
    pub duration: Option<f32>,
    /// The fixed frame delta.
    ///
    /// 1/60 s to match the interactive app, which is what makes a scripted
    /// reproduction and a hand-driven one comparable.
    pub dt: f32,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            script: None,
            duration: None,
            dt: 1.0 / 60.0,
        }
    }
}

/// Run headless, writing the dump to `out`.
///
/// The generic sink is what makes the determinism check cheap: two runs into
/// two `Vec<u8>`s compare byte for byte with no filesystem involved.
pub fn run_into<W: Write>(
    cfg: ViewConfig,
    opts: &RunOptions,
    out: W,
) -> Result<RunSummary, RunError> {
    let (script, digest) = match &opts.script {
        Some(src) => (
            Some(Script::parse(src).map_err(RunError::Script)?),
            Some(sha256_hex(src.as_bytes())),
        ),
        None => (None, None),
    };
    // The command line wins over the script, which wins over nothing.
    let duration = opts
        .duration
        .or_else(|| script.as_ref().and_then(|s| s.settings.duration));
    if script.is_none() && duration.is_none() {
        return Err(RunError::Unbounded);
    }
    let mut dump = Dump::new(out);
    drive(cfg, opts, duration, script.as_ref(), digest, &mut dump)
        .map_err(|e| to_run_error(e, Path::new("<dump>")))
}

/// Run headless from a script file, writing the dump to `dump_path`.
///
/// **`dump_path` and `duration` override the script's own `dump` and
/// `duration`.**  Passing `None` for either defers to the script.
///
/// With neither naming a dump the run **writes nothing** — deliberately, since
/// that is still a useful smoke test: a run that panics, fails to parse or drops
/// a chunk fails just the same.  With neither naming a duration the run ends
/// [`DEFAULT_TAIL_SECS`] past the last step.
pub fn run_file(
    cfg: ViewConfig,
    script_path: Option<&Path>,
    dump_path: Option<&Path>,
    duration: Option<f32>,
) -> Result<RunSummary, RunError> {
    let src = match script_path {
        Some(p) => Some(std::fs::read_to_string(p).map_err(|e| RunError::Io(p.to_path_buf(), e))?),
        None => None,
    };
    // Parsed twice — here for the settings and again inside `run_into` — which
    // is cheap and keeps `run_into` a complete entry point rather than one that
    // only works if a caller pre-resolved its arguments.
    let from_script = match &src {
        Some(s) => Script::parse(s).map_err(RunError::Script)?,
        None => Script::default(),
    };
    // A relative path from either place resolves against the working directory,
    // so the directive and the flag mean the same thing given the same string.
    let dump_path = dump_path
        .map(std::path::Path::to_path_buf)
        .or_else(|| from_script.settings.dump.clone());

    let opts = RunOptions {
        script: src,
        duration,
        ..Default::default()
    };
    match dump_path {
        Some(path) => {
            let file = std::fs::File::create(&path).map_err(|e| RunError::Io(path.clone(), e))?;
            run_into(cfg, &opts, std::io::BufWriter::new(file)).map_err(|e| match e {
                // Name the file that could not be written.
                RunError::Io(_, io) => RunError::Io(path, io),
                other => other,
            })
        }
        None => run_into(cfg, &opts, std::io::sink()),
    }
}

/// Lift a driver error into a [`RunError`], attaching the dump path to I/O
/// failures so the diagnostic names the file that could not be written.
fn to_run_error(e: DriveError, path: &Path) -> RunError {
    match e {
        DriveError::Io(e) => RunError::Io(path.to_path_buf(), e),
        DriveError::Dropped(n) => RunError::DroppedChunks(n),
    }
}

enum DriveError {
    Io(std::io::Error),
    Dropped(u64),
}

impl From<std::io::Error> for DriveError {
    fn from(e: std::io::Error) -> Self {
        DriveError::Io(e)
    }
}

/// The run loop.
///
/// Generic over the sink so a test can drive a `Vec<u8>` and compare bytes
/// without touching the filesystem — which is how the determinism check is
/// written.
fn drive<W: Write>(
    cfg: ViewConfig,
    opts: &RunOptions,
    duration: Option<f32>,
    script: Option<&Script>,
    digest: Option<String>,
    dump: &mut Dump<W>,
) -> Result<RunSummary, DriveError> {
    // A bare context: no window, no renderer, no wgpu device.  `begin_pass` and
    // `end_pass` are public, so a complete pass runs against it and the
    // `FullOutput` — tessellated shapes and texture deltas — is dropped.
    let ctx = egui::Context::default();
    let mut app = ViewApp::new_replay(&ctx, cfg);

    // `duration` is already resolved — command line over script over nothing —
    // and a duration shorter than the script still runs every step, because the
    // loop also waits on the step iterator.  So this can cut a script short or
    // extend it into its steady state.  With nothing named at all, the run ends
    // a fixed margin past the last step; see `DEFAULT_TAIL_SECS`.
    let script_end = script
        .and_then(|s| s.steps.last())
        .map_or(0.0, |s| s.t_secs);
    let end_secs = duration.unwrap_or(script_end + DEFAULT_TAIL_SECS);

    dump.write(&Record::Header {
        version: env!("CARGO_PKG_VERSION"),
        source: app.source_mode().label().to_owned(),
        fs_hz: app.source_sample_rate(),
        script_sha256: digest,
    })?;

    let mut steps = script.map(|s| s.steps.as_slice()).unwrap_or(&[]).iter();
    let mut next = steps.next();
    let mut frames: u64 = 0;
    let mut samples: u64 = 0;
    let mut source = app.source_mode();

    // The action currently being delivered, and how many frames of it are left.
    // A repeat count is **frames, not events**: `key_pressed` is a per-pass
    // boolean, so five press events inside one pass register as one and `key I
    // x5` would switch a single source rather than five.
    let mut pending: Option<(Action, usize)> = None;

    loop {
        // `t` is the time at the *start* of the frame about to run, so the
        // bound is `>=`: `--duration 1.0` at 1/60 s runs frames 0..59 and stops
        // with `t == 1.0` having elapsed exactly one second, not 61 frames
        // covering 1.017 s.  A still-pending step keeps the loop alive
        // regardless, so a script is never truncated by its own last step.
        let t = frames as f32 * opts.dt;
        if t >= end_secs && next.is_none() && pending.is_none() {
            break;
        }

        // Pick up the next due step, unless one is still being delivered.
        // Delivery takes precedence over arrival, so a repeat that overruns the
        // next step's time delays it rather than being truncated by it.
        while pending.is_none() {
            let Some(step) = next else { break };
            if step.t_secs > t {
                break;
            }
            next = steps.next();
            match &step.action {
                // Parsed — so a typo in a script is still an error — and then
                // ignored: executing assertions is the test harness's job.
                // One format, two readers.  See `utils::script`.
                Action::Assert { .. } => continue,
                action => pending = Some((action.clone(), step.repeat.max(1))),
            }
        }

        let (events, modifiers) = match &mut pending {
            Some((action, left)) => {
                let pair = (action.events(), action.modifiers());
                *left -= 1;
                if *left == 0 {
                    pending = None;
                }
                pair
            }
            None => (Vec::new(), egui::Modifiers::default()),
        };

        samples += step_once(&ctx, &mut app, opts.dt, events, modifiers);
        frames += 1;
        let t = frames as f32 * opts.dt;

        if app.source_mode() != source {
            source = app.source_mode();
            dump.write(&Record::Source {
                t,
                samples,
                source: source.label().to_owned(),
                fs_hz: app.source_sample_rate(),
            })?;
        }
        emit(dump, &mut app, t, samples)?;
    }

    let dropped = app.dropped_chunks().unwrap_or(0);
    dump.write(&Record::Summary {
        t: frames as f32 * opts.dt,
        frames,
        samples,
        dropped_chunks: dropped,
        records: dump.records(),
    })?;
    dump.flush()?;

    if dropped > 0 {
        return Err(DriveError::Dropped(dropped));
    }
    Ok(RunSummary {
        frames,
        samples,
        records: dump.records(),
    })
}

/// One complete pass, returning the samples the source consumed.
fn step_once(
    ctx: &egui::Context,
    app: &mut ViewApp,
    dt: f32,
    events: Vec<egui::Event>,
    modifiers: egui::Modifiers,
) -> u64 {
    let before = app.samples_consumed();
    ctx.begin_pass(egui::RawInput {
        events,
        modifiers,
        ..Default::default()
    });
    app.advance(ctx, dt);
    // `handle_keys` runs from `draw`, not `advance`; a driver that only advances
    // would process samples and never see a keystroke.
    app.handle_keys(ctx);
    let _ = ctx.end_pass();
    app.samples_consumed() - before
}

/// Write whatever this frame produced.
fn emit<W: Write>(
    dump: &mut Dump<W>,
    app: &mut ViewApp,
    t: f32,
    samples: u64,
) -> Result<(), DriveError> {
    for result in app.take_replay_results() {
        let record = match result {
            DecodeResult::Text(text) => Record::Text { t, samples, text },
            DecodeResult::Info {
                modulation,
                center_hz,
                bw_hz,
                snr_db,
            } => Record::Info {
                t,
                samples,
                modulation,
                center_hz,
                bw_hz,
                snr_db,
            },
            DecodeResult::Instrument(Some(inst)) => Record::Instrument { t, samples, inst },
            DecodeResult::Instrument(None) => Record::InstrumentCleared { t, samples },
            DecodeResult::Gap { decoded } => Record::Gap {
                t,
                samples,
                decoded,
            },
            DecodeResult::NoSignal => Record::NoSignal { t, samples },
        };
        dump.write(&record)?;
    }
    Ok(())
}
