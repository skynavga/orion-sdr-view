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

use crate::app::{SourceMode, ViewApp};
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
    /// A capture could not be written.
    Capture(String),
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
            RunError::Capture(m) => write!(f, "capture: {m}"),
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

/// The logical window size a headless pass lays out in, when nothing says
/// otherwise.
///
/// **A headless pass supplies no `screen_rect` unless one is set**, and egui's
/// fallback is 10000 x 10000 at scale 1 — measured, not guessed.  Nothing
/// consults the layout while the driver only advances and handles keys, so this
/// changes no existing behaviour; it matters the moment anything *draws*, where
/// the fallback would put every layout-dependent path at a width no window has
/// and make a capture 400 MB.
///
/// The interactive window's size, so a scripted reproduction lays out the way a
/// user's does.
pub const DEFAULT_SIZE: (f32, f32) = (1200.0, 800.0 + crate::app::DECODE_BAR_H);

/// Pixels per point, when nothing says otherwise.  1.0 rather than the 2.0 a
/// Retina display reports, so a run's output does not depend on the machine.
pub const DEFAULT_SCALE: f32 = 1.0;

/// The dump path that means **standard output** rather than a file.
///
/// The same spelling `curl -o -`, `tar -f -` and `sort -o -` use, so a reader
/// who has met one has met this.  A literal file called `-` stays reachable as
/// `./-`, which is the escape hatch those tools offer too.
pub const STDOUT_PATH: &str = "-";

/// Whether a dump path names standard output.
///
/// Compared against the whole path, not a suffix: `./-`, `runs/-` and `dash-`
/// are all ordinary files.
pub fn is_stdout(path: &Path) -> bool {
    path.as_os_str() == STDOUT_PATH
}

/// What a replay run reports back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSummary {
    pub frames: u64,
    pub samples: u64,
    pub records: u64,
    /// Files a `pane` directive wrote, in the order taken.
    pub captures: Vec<PathBuf>,
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
    /// Logical window size.  **Overrides the script's own `size`.**
    pub size: Option<(f32, f32)>,
    /// Pixels per point.  **Overrides the script's own `scale`.**
    pub scale: Option<f32>,
    /// Where captures are written.  **Overrides the script's own `capture`.**
    pub capture: Option<PathBuf>,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            script: None,
            duration: None,
            dt: 1.0 / 60.0,
            size: None,
            scale: None,
            capture: None,
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
///
/// A dump path of [`STDOUT_PATH`] writes to standard output.  Resolved here, at
/// the one point the flag and the script's own `dump` have already merged, so
/// the two cannot disagree about what `-` means.
pub fn run_file(
    cfg: ViewConfig,
    script_path: Option<&Path>,
    dump_path: Option<&Path>,
    duration: Option<f32>,
    capture_dir: Option<&Path>,
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
        capture: capture_dir.map(Path::to_path_buf),
        ..Default::default()
    };
    match dump_path {
        // Deliberately *not* wrapped in a `BufWriter`: `Stdout` is line
        // buffered, so `--dump - | jq` gets a record at a time instead of a
        // block at the end.  Nothing else in a headless run writes to stdout —
        // the summary and every diagnostic go to stderr — so the two streams
        // cannot interleave and the dump stays machine-readable.
        Some(path) if is_stdout(&path) => {
            run_into(cfg, &opts, std::io::stdout().lock()).map_err(|e| match e {
                RunError::Io(_, io) => RunError::Io(PathBuf::from("<stdout>"), io),
                other => other,
            })
        }
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
        DriveError::Capture(m) => RunError::Capture(m),
    }
}

enum DriveError {
    Io(std::io::Error),
    Dropped(u64),
    Capture(String),
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

    // `duration` is already resolved — command line over script over nothing —
    // and a duration shorter than the script still runs every step, because the
    // loop also waits on the step iterator.  So this can cut a script short or
    // extend it into its steady state.  With nothing named at all, the run ends
    // a fixed margin past the last step; see `DEFAULT_TAIL_SECS`.
    // Command line over script over default, the same precedence as `duration`.
    let (w, h) = opts
        .size
        .or_else(|| script.and_then(|s| s.settings.size))
        .unwrap_or(DEFAULT_SIZE);
    let scale = opts
        .scale
        .or_else(|| script.and_then(|s| s.settings.scale))
        .unwrap_or(DEFAULT_SCALE);
    let viewport = (w, h, scale);
    let capture_dir = opts
        .capture
        .clone()
        .or_else(|| script.and_then(|s| s.settings.capture.clone()))
        .unwrap_or_else(|| cfg.capture_dir());

    let mut app = ViewApp::new_replay(&ctx, cfg);

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

    // **Decided once, before the loop.**  A script with no `still` never builds
    // a capturer, so it never draws and never tessellates — the cost of the
    // feature is exactly zero rather than merely small.
    let mut capturer = script
        .is_some_and(|s| {
            s.steps
                .iter()
                .any(|st| matches!(st.action, Action::Still { .. }))
        })
        .then(|| StillCapturer {
            textures: crate::capture::Textures::default(),
            dir: capture_dir.clone(),
        });

    let mut captures: Vec<PathBuf> = Vec::new();
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

        // Set by a `still` step consumed below, and acted on inside the frame.
        let mut still_this_frame: Option<Option<String>> = None;

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
                // `source NAME` presses `I` until that source is active, so its
                // bound is the number of sources rather than the step's repeat
                // — which the parser refuses on this directive anyway.
                Action::Source { .. } => {
                    pending = Some((step.action.clone(), SourceMode::ALL.len()));
                }
                // Written here rather than delivered as input: a pane's pixels
                // are already CPU-side, so there is nothing to press and nothing
                // to wait a frame for.
                // Deferred to the frame rather than done here: a still is of
                // what the pass *draws*, so it has to happen inside one.
                Action::Still { label } => {
                    still_this_frame = Some(label.clone());
                    continue;
                }
                Action::Pane { pane, label } => {
                    match app.capture_pane(&capture_dir, *pane, label.as_deref()) {
                        Ok(Some(path)) => captures.push(path),
                        // Not a failure: a script that captures before any
                        // spectrum has been processed has nothing to write. Said
                        // out loud, because a missing file otherwise looks like
                        // a bug in the directive.
                        Ok(None) => eprintln!(
                            "{}",
                            crate::utils::term::notice(
                                crate::utils::term::Level::Warn,
                                &format!(
                                    "line {}: the {} pane has no pixels yet, so nothing \
                                     was written",
                                    step.line,
                                    pane.name()
                                ),
                            )
                        ),
                        Err(e) => {
                            return Err(DriveError::Capture(format!(
                                "line {}: {} pane: {e}",
                                step.line,
                                pane.name()
                            )));
                        }
                    }
                    continue;
                }
                action => pending = Some((action.clone(), step.repeat.max(1))),
            }
        }

        // Stop pressing as soon as the named source is active.  Checked at the
        // top of the frame because `handle_keys` runs inside the pass: a press
        // delivered in frame N is visible here in frame N+1.  The `left`
        // counter below is only a backstop — `SourceMode::next` cycles every
        // source, so this always fires first.
        if let Some((Action::Source { mode }, _)) = &pending
            && app.source_mode() == *mode
        {
            pending = None;
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

        let step = FrameStep {
            dt: opts.dt,
            events,
            modifiers,
            viewport,
            still: still_this_frame,
        };
        let (consumed, captured) = step_once(&ctx, &mut app, step, capturer.as_mut())?;
        samples += consumed;
        captures.extend(captured);
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
        captures,
    })
}

/// One complete pass, returning the samples the source consumed.
/// One frame's worth of instruction for [`step_once`].
struct FrameStep {
    dt: f32,
    events: Vec<egui::Event>,
    modifiers: egui::Modifiers,
    viewport: (f32, f32, f32),
    /// `Some(label)` when this frame is to be captured.
    still: Option<Option<String>>,
}

/// One complete pass, returning the samples consumed and any still written.
fn step_once(
    ctx: &egui::Context,
    app: &mut ViewApp,
    step: FrameStep,
    capturer: Option<&mut StillCapturer>,
) -> Result<(u64, Option<PathBuf>), DriveError> {
    let before = app.samples_consumed();
    ctx.begin_pass(raw_input(step.events, step.modifiers, step.viewport));
    app.advance(ctx, step.dt);
    // `handle_keys` runs from `draw`, not `advance`; a driver that only advances
    // would process samples and never see a keystroke.
    app.handle_keys(ctx);

    // **The only frame that draws.**  On every other one the pass ends with its
    // shapes discarded, exactly as before this feature existed.
    let drawing = step.still.is_some();
    if let (true, Some(cap)) = (drawing, capturer.as_deref()) {
        cap.draw(ctx, app, step.viewport);
    }
    let out = ctx.end_pass();

    let captured = match capturer {
        Some(cap) => {
            // **Applied every frame, not only on capture frames.** Texture
            // uploads are incremental and mostly happen once: the font atlas is
            // built on first use and the pane textures upload when their
            // contents change. Collecting only the capture frame's delta left
            // the rasterizer without the atlas — and since egui draws *solid*
            // shapes from a white texel in that same atlas, the result was a
            // frame with almost everything missing rather than an error.
            cap.textures.apply(&out.textures_delta);
            match step.still {
                Some(label) => Some(cap.capture(ctx, app, label.as_deref(), step.viewport, out)?),
                None => None,
            }
        }
        None => None,
    };
    Ok((app.samples_consumed() - before, captured))
}

/// Draws and rasterizes the frames a `still` directive asks for.
///
/// **Constructed only when a script contains a `still`.**  A run without one
/// never builds this, never draws, and never tessellates — which is what keeps
/// the cost of the feature at zero for every script that does not use it.  The
/// driver has never drawn on an ordinary frame, so that baseline is not a
/// promise being made here, only one being kept.
struct StillCapturer {
    textures: crate::capture::Textures,
    dir: PathBuf,
}

impl StillCapturer {
    /// Draw the UI into the pass.  Must run before `end_pass`.
    ///
    /// `draw_ui` rather than `draw`, because the driver has already handled this
    /// pass's keys and doing it again would toggle every binding twice.
    fn draw(&self, ctx: &egui::Context, app: &mut ViewApp, viewport: (f32, f32, f32)) {
        let (w, h, _) = viewport;
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h));
        let mut ui = egui::Ui::new(
            ctx.clone(),
            egui::Id::new("headless_capture"),
            egui::UiBuilder::new().max_rect(rect),
        );
        app.draw_ui(&mut ui);
    }

    /// Rasterize what the pass produced and write it.
    fn capture(
        &mut self,
        ctx: &egui::Context,
        app: &mut ViewApp,
        label: Option<&str>,
        viewport: (f32, f32, f32),
        out: egui::FullOutput,
    ) -> Result<PathBuf, DriveError> {
        let (w, h, scale) = viewport;
        let primitives = ctx.tessellate(out.shapes, scale);
        let size_px = ((w * scale).round() as u32, (h * scale).round() as u32);
        let raster = crate::capture::rasterize(&primitives, &self.textures, size_px, scale);

        if raster.missing_textures > 0 {
            // Loud, because the failure mode is a *plausible-looking* image
            // rather than an error: a mesh whose texture is missing simply does
            // not draw, and the result is a picture with pieces silently absent.
            eprintln!(
                "{}",
                crate::utils::term::notice(
                    crate::utils::term::Level::Warn,
                    &format!(
                        "capture: {} mesh(es) referenced a texture that was never \
                         uploaded; the image is incomplete",
                        raster.missing_textures
                    ),
                )
            );
        }
        if raster.skipped_callbacks > 0 {
            // Not silent: this app registers no paint callbacks, so if one ever
            // appears the capture would be quietly missing part of the frame.
            eprintln!(
                "{}",
                crate::utils::term::notice(
                    crate::utils::term::Level::Warn,
                    &format!(
                        "capture: {} paint callback(s) could not be rasterized",
                        raster.skipped_callbacks
                    ),
                )
            );
        }

        app.write_still_raster(&self.dir, label, raster.width, raster.height, &raster.rgba)
            .map_err(|e| DriveError::Capture(e.to_string()))
    }
}

/// The pass's raw input, at a stated size and scale.
///
/// `screen_rect` and `native_pixels_per_point` are set explicitly because a
/// headless pass has no window to report them.  See [`DEFAULT_SIZE`].
fn raw_input(
    events: Vec<egui::Event>,
    modifiers: egui::Modifiers,
    (w, h, scale): (f32, f32, f32),
) -> egui::RawInput {
    let mut raw = egui::RawInput {
        events,
        modifiers,
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(w, h),
        )),
        ..Default::default()
    };
    // The scale lives on the viewport, not beside `screen_rect`: egui reads it
    // as `raw_input.viewport().native_pixels_per_point`.
    let id = raw.viewport_id;
    raw.viewports.entry(id).or_default().native_pixels_per_point = Some(scale);
    raw
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
            // Bulk pane pixels, not a reading: a dump is a record of what was
            // *measured*, and the probe's symbols and correction map are drawn
            // rather than reported.  `pane constellation` / `pane correction`
            // capture them as rasters, which is the assertable form.
            DecodeResult::Probe(_) => continue,
        };
        dump.write(&record)?;
    }
    Ok(())
}
