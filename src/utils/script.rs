// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! A timed key script: the shared input format for driving the app without a
//! human at the keyboard.
//!
//! ```text
//! set run.duration 30            # untimed: configures the run
//! set run.dump     run.jsonl
//! set cofdm.cn_db  10            # untimed: an app setting, before frame 0
//!
//! # t(s)   directive
//! 0.00     source COFDM          # select a source by name
//! 0.50     key L                 # lock the source to the viewport centre
//! 0.75     key shift+ArrowRight
//! 0.80     text a                # markers arrive as Text, not Key
//! 1.00     set cofdm.cn_db 5     # ...and timed, as an edit during the run
//! 1.00     assert center_hz 520000
//! ```
//!
//! **One rule tells the two apart**: a line beginning with `set` is untimed,
//! and every other line begins with a time.  That replaced a list of reserved
//! words — `duration`, `dump`, `capture`, `size`, `scale` — which each had to be
//! kept from colliding with anything else the format might want to say.
//!
//! **One format, two readers.**  A test harness replays the `key`/`text`
//! directives and *executes* the `assert` ones; the headless replay driver
//! replays and *ignores* them.  Defining it once is what makes a bug report and
//! a regression test the same artifact — a reproduction recipe can be dropped
//! into `tests/` unchanged, and a failing test can be replayed interactively.
//!
//! Times are **absolute seconds**, not deltas: with the `dt` injected into
//! [`ViewApp::advance`](crate::app::ViewApp::advance) a driver steps exactly to
//! each boundary, so "at t = 0.75 s" is exact rather than approximate.
//!
//! # Naming a source
//!
//! `source COFDM` selects a source by name.  It is not a shorthand for a
//! different mechanism — it presses `I` exactly as `key I` does, with **the
//! count worked out at run time** rather than written down.
//!
//! That is the whole of it: `key I x5` encodes the *distance* from wherever the
//! app happens to be to the source you meant, so adding a source, reordering
//! the list, or starting from a different one retargets every such line at once.
//! It fails silently, too — the line still parses and still runs, just onto the
//! wrong source, and a dump is perfectly happy to record a measurement of the
//! wrong thing.  A name cannot go stale that way: it either resolves or the
//! script does not parse.
//!
//! Names are case- and punctuation-insensitive, so `AM DSB`, `AM-DSB`, `AM_DSB`
//! and `amdsb` are one source.  See [`source_mode_by_name`].
//!
//! # `set`
//!
//! One directive, three scopes, and the scope is what says which kind of thing
//! is being written:
//!
//! - **`run.`** — how the run is conducted: `duration`, `dump`, `capture`,
//!   `size`, `scale`.  Never timed, because they configure the run rather than
//!   happen during it, and **the command line overrides every one of them**.
//!   That is what keeps a recipe reusable: the same script can be run longer, or
//!   dumped somewhere else, without being edited.
//! - **`display.`** and **a source name** — the app's own settings rows, in the
//!   config file's spelling.  `set cofdm.cn_db 10` and a `cn_db: 10` under
//!   `sources.cofdm` say the same thing in the same words, and a source may be
//!   named as the config writes it or as the HUD shows it, since the two fold
//!   alike.
//!
//! An app setting may be written either way round, and the two are not the same
//! statement.  **Untimed it is a configuration**: applied before the first
//! frame, moving the row's default as well as its value, so an `R` reset returns
//! to it — exactly what `--config` does.  **Timed it is an interaction**:
//! applied at that instant, moving the value only, so a reset discards it like
//! any other edit.  A `run.` key timed is a parse error; there is no instant at
//! which "where the dump goes" could take effect.
//!
//! A `dump` or `capture` path may be absolute or relative, and a relative one
//! resolves **against the viewer's working directory** — the same as `--dump`
//! and as any other path a shell hands a program.  The directive is a default
//! for the flag, so the two must mean the same thing given the same string.
//! That includes `-`, which means standard output in both; see
//! [`STDOUT_PATH`](crate::replay::STDOUT_PATH).
//!
//! # What `set` deliberately cannot reach
//!
//! **A row, not a config field.**  A `set` writes the settings row a popover
//! edit writes and is read back by the same accessors, so it cannot reach a
//! state no user could — the failure a harness that wrote past the UI produced
//! once already, silently measuring a configuration nobody could select.
//!
//! Two consequences worth stating, because both look like omissions:
//! `cofdm.fs_hz` is a config key with no row, so it stays a `--config` key; and
//! the rows with no config key — AM-DSB's audio selection, the message-mode
//! toggles — are reached with `key` and `text` instead.  Between the two halves
//! every row is reachable, each exactly once.

use std::fmt;
use std::path::PathBuf;

use crate::app::SourceMode;
use crate::app::settings::SetTarget;

/// A parse failure, carrying the 1-based source line so the diagnostic can name
/// it.  A headless run must fail loudly on an unparsable script rather than
/// silently skipping the line — nobody is watching it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ScriptError {}

/// What one directive does.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Synthesize a key press and its matching release.
    Key {
        key: egui::Key,
        modifiers: egui::Modifiers,
    },
    /// Synthesize text input.
    ///
    /// Not redundant with `Key`: the app reads `?`, `a`/`b`, `A`/`B` and `[`/`]`
    /// out of [`egui::Event::Text`], so a key-only format could not reach the
    /// marker or dB-reference bindings at all.
    Text { text: String },
    /// Select a source by name.
    ///
    /// **This is `key I` with the repeat count deferred to run time.**  It
    /// delivers the same press, and the reader keeps delivering it until
    /// `mode` is active — at most [`SourceMode::ALL`]`.len()` times, since
    /// [`SourceMode::next`] cycles every source.  So it drives the same key
    /// path a user does; the only thing it removes is the need to know how far
    /// away the source is, which is the part a script cannot know and the part
    /// that goes stale.
    Source { mode: SourceMode },
    /// Capture the whole window to the capture directory.
    ///
    /// Unlike [`Pane`](Self::Pane), this is everything the viewer draws — HUD,
    /// decode bar, overlays and all — which means the frame has to be *drawn*,
    /// and drawn frames are the only expensive ones in a headless run.  A
    /// script that never asks for one pays nothing.
    Still {
        /// Appended to the filename, so a script taking several is readable.
        label: Option<String>,
    },
    /// Write one pane's raster to the capture directory.
    ///
    /// **Not a screenshot.**  The waterfall, spectrogram and persistence panes
    /// each keep their pixels CPU-side, so this needs no renderer and no GPU —
    /// it is the DSP's own output, without the HUD, the spectrum plot or any
    /// chrome around it.  A cheaper and more directly assertable thing than a
    /// picture of the window, and a different question.
    Pane {
        pane: Pane,
        /// Appended to the filename, so a script taking several is readable.
        label: Option<String>,
    },
    /// Write one of the app's settings rows, mid-run.
    ///
    /// **An interaction, not a configuration.**  It lands where a popover edit
    /// lands and moves the row's value only, so an `R` reset discards it exactly
    /// as it discards a nudge.  The untimed spelling of the same line is the
    /// configuration — see the module docs.
    ///
    /// What it buys that no other directive can: a quantity swept *during* a
    /// run.  Walking `cofdm.cn_db` down through the FEC cliff is one run and one
    /// dump, where before it was one run per point and a script apiece.
    Set { target: SetTarget, value: String },
    /// A property for the *test harness* to check.  The replay driver parses it
    /// — so a typo is still an error — and then ignores it.
    Assert { name: String, args: Vec<String> },
}

/// A pane that keeps a CPU-side raster.
///
/// The spectrum pane is absent deliberately: it is a line plot drawn straight to
/// a painter, with no pixel buffer to hand over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Waterfall,
    Spectrogram,
    Persistence,
    /// Pane 3's decoder-mode left half: the equalizer's output, stamped as
    /// hollow circles over a density map.
    Constellation,
    /// Pane 3's decoder-mode right half: the per-coded-bit correction map,
    /// scrolling by codeword.
    Correction,
}

impl Pane {
    pub const ALL: &'static [Pane] = &[
        Pane::Waterfall,
        Pane::Spectrogram,
        Pane::Persistence,
        Pane::Constellation,
        Pane::Correction,
    ];

    /// The name a script writes, and the suffix a filename carries.
    pub fn name(self) -> &'static str {
        match self {
            Self::Waterfall => "waterfall",
            Self::Spectrogram => "spectrogram",
            Self::Persistence => "persistence",
            Self::Constellation => "constellation",
            Self::Correction => "correction",
        }
    }

    /// Resolve a name written in a script, folded like a source name.
    pub fn by_name(s: &str) -> Option<Self> {
        let want = fold_name(s);
        (!want.is_empty())
            .then(|| {
                Self::ALL
                    .iter()
                    .copied()
                    .find(|p| fold_name(p.name()) == want)
            })
            .flatten()
    }
}

impl Action {
    /// The raw events this action delivers in **one** pass.
    ///
    /// A key is pressed and released within the same pass so it cannot stick
    /// down across later frames.  The cost is that bindings reading
    /// `key_down` rather than `key_pressed` — the Ctrl+←/→ coarse marker move is
    /// the only one — see the key up again in the same pass and so do not fire.
    pub fn events(&self) -> Vec<egui::Event> {
        match self {
            Self::Key { key, modifiers } => key_events(*key, *modifiers),
            // One `I` press, the same as `key I`.  How many of them a selection
            // needs is the reader's business, not the event's.
            Self::Source { .. } => key_events(egui::Key::I, egui::Modifiers::default()),
            Self::Text { text } => vec![egui::Event::Text(text.clone())],
            // None delivers input: two write a file, one is checked, and `set`
            // writes the row the keyboard would have nudged to.
            Self::Still { .. } | Self::Pane { .. } | Self::Set { .. } | Self::Assert { .. } => {
                Vec::new()
            }
        }
    }

    /// The modifier state to put on the pass's [`egui::RawInput`].
    ///
    /// Separate from the events because `InputState::modifiers` — which is what
    /// `handle_keys` reads for shift/ctrl/alt — is taken from `RawInput`, not
    /// from the key event.  Setting only one of the two would give a script a
    /// `shift+` that the app never sees.
    pub fn modifiers(&self) -> egui::Modifiers {
        match self {
            Self::Key { modifiers, .. } => *modifiers,
            _ => egui::Modifiers::default(),
        }
    }
}

/// Parse an optional trailing label, which becomes part of a filename.
///
/// Restricted to what survives a filesystem and a shell unquoted: ASCII letters,
/// digits, `-` and `_`.  A label that needed quoting would make the very
/// artifact it names awkward to handle, and a rejected label is a parse error
/// rather than a silently mangled filename.
fn parse_label(rest: &[&str], line: usize) -> Result<Option<String>, ScriptError> {
    let [label] = rest[..] else {
        if rest.is_empty() {
            return Ok(None);
        }
        return Err(ScriptError {
            line,
            message: format!(
                "a label is one whitespace-free word, got {} — try `{}`",
                rest.len(),
                rest.join("-")
            ),
        });
    };
    let ok = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    if !label.chars().all(ok) {
        return Err(ScriptError {
            line,
            message: format!(
                "label `{label}` may use only letters, digits, `-` and `_`; \
                 it becomes part of a filename"
            ),
        });
    }
    Ok(Some(label.to_owned()))
}

/// One key's press and release, for delivery within a single pass.
fn key_events(key: egui::Key, modifiers: egui::Modifiers) -> Vec<egui::Event> {
    vec![
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        },
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers,
        },
    ]
}

/// Resolve a source name as written in a script.
///
/// Case- and punctuation-insensitive: `AM DSB`, `AM-DSB`, `AM_DSB` and `amdsb`
/// all name the same source.  The labels these are matched against are the ones
/// the HUD shows, so what a script writes is what a user reads on screen — and
/// a source added to [`SourceMode::ALL`] becomes nameable with no edit here.
pub fn source_mode_by_name(name: &str) -> Option<SourceMode> {
    let want = fold_name(name);
    if want.is_empty() {
        return None;
    }
    SourceMode::ALL
        .iter()
        .copied()
        .find(|m| fold_name(m.label()) == want)
}

/// Whether two names are the same once folded.  An empty fold matches nothing,
/// so punctuation alone is not a name.
pub fn names_match(a: &str, b: &str) -> bool {
    let a = fold_name(a);
    !a.is_empty() && a == fold_name(b)
}

/// Fold a source name to its comparison form: ASCII letters and digits only,
/// lowercased.  Spacing and punctuation carry no meaning in these labels, so
/// two spellings that differ only there are the same name.
///
/// It is also what makes the config file's source keys and the HUD's labels one
/// vocabulary rather than two: `am_dsb` and `AM DSB` fold alike, so a `set`
/// naming a source and a `source` directive naming one accept the same word.
pub fn fold_name(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// One directive at one instant.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    /// Absolute time from the start of the run, in seconds.
    pub t_secs: f32,
    /// How many **consecutive frames** the action occupies (the `xN` suffix).
    ///
    /// Frames, not events: `key_pressed` is a per-pass boolean, so five press
    /// events in one pass are indistinguishable from one.  `key I x5` therefore
    /// has to be five passes to switch five sources.
    pub repeat: usize,
    pub action: Action,
    /// 1-based source line, for diagnostics.
    pub line: usize,
}

/// Run settings a script may carry, each optional and each at most once.
///
/// Both are **defaults, not commands**: the driver's caller overrides either, so
/// a script that names its own duration and dump can still be re-run for longer
/// or written elsewhere without being edited.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScriptSettings {
    /// Where to write the dump.  Absolute or relative; a relative path resolves
    /// against the working directory, exactly as `--dump` does, and `-` means
    /// standard output in both.
    pub dump: Option<PathBuf>,
    /// How long to run, in scripted seconds.
    pub duration: Option<f32>,
    /// Logical window size, in points.
    ///
    /// A headless pass supplies no `screen_rect` unless something sets one, and
    /// egui's fallback is 10000 x 10000 — a size no window has.  Nothing
    /// consults the layout while the driver only advances and handles keys, but
    /// anything that *draws* needs a real one.
    pub size: Option<(f32, f32)>,
    /// Pixels per point.  2.0 is a Retina-class display.
    pub scale: Option<f32>,
    /// Where captures are written.  Absolute or relative, exactly as `dump` is,
    /// and overridden by `--capture`.
    pub capture: Option<PathBuf>,
    /// Untimed `set`s on the app's settings, in source order.
    ///
    /// A `Vec` rather than an `Option` per key, because several are the norm and
    /// order matters where rows are coupled — COFDM's edge guard re-seeds from
    /// the bandwidth toggle, so a `set` of both means the later one wins on the
    /// guard, exactly as two nudges in that order would.
    pub sets: Vec<ScriptSet>,
}

/// A parsed script: run settings plus steps in ascending time order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Script {
    pub settings: ScriptSettings,
    pub steps: Vec<Step>,
}

impl Script {
    /// Parse a script, failing on the first bad line.
    pub fn parse(src: &str) -> Result<Self, ScriptError> {
        let mut steps = Vec::new();
        let mut settings = ScriptSettings::default();
        for (i, raw) in src.lines().enumerate() {
            let line = i + 1;
            let text = strip_comment(raw);
            if text.trim().is_empty() {
                continue;
            }
            match parse_line(text, line)? {
                Line::Step(s) => steps.push(s),
                Line::Run(set) => set.apply(&mut settings, line)?,
                Line::Set(s) => {
                    // The same rule the run settings follow: naming one key
                    // twice means the author believed one of them, and picking
                    // silently is only noticed after a run has answered wrongly.
                    if let Some(prev) = settings.sets.iter().find(|p| p.target == s.target) {
                        return Err(ScriptError {
                            line,
                            message: format!(
                                "`{}` is set more than once (already on line {})",
                                s.target, prev.line
                            ),
                        });
                    }
                    settings.sets.push(s);
                }
            }
        }
        // Sort by time so a script may be written out of order, but keep equal
        // times in source order — two directives at the same instant are meant
        // to happen in the order they were written.
        steps.sort_by(|a, b| a.t_secs.total_cmp(&b.t_secs));
        Ok(Self { settings, steps })
    }

    /// Steps due in the half-open window `[from, to)`.
    ///
    /// Half-open so that stepping a run as `[0, dt)`, `[dt, 2·dt)`, … delivers
    /// every step exactly once, whatever `dt` is.
    pub fn steps_in(&self, from: f32, to: f32) -> impl Iterator<Item = &Step> {
        self.steps
            .iter()
            .filter(move |s| s.t_secs >= from && s.t_secs < to)
    }

    /// Time of the last step, or 0.0 for an empty script.  A driver still has to
    /// run past this to let the last action's repeats and the frame it lands on
    /// take effect.
    pub fn duration_secs(&self) -> f32 {
        self.steps.last().map_or(0.0, |s| s.t_secs)
    }
}

/// Drop a trailing `#` comment.  Text directives therefore cannot contain `#`,
/// which no binding in this app needs.
fn strip_comment(line: &str) -> &str {
    match line.find('#') {
        Some(i) => &line[..i],
        None => line,
    }
}

/// One non-blank line: a timed step, a run setting, or an untimed app setting.
enum Line {
    Step(Step),
    Run(Setting),
    Set(ScriptSet),
}

/// An untimed `set` on the app's own settings, applied before the first frame.
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptSet {
    pub target: SetTarget,
    pub value: String,
    /// 1-based source line, so a value the target refuses names its line.
    pub line: usize,
}

/// A parsed run-setting line, before it is folded into [`ScriptSettings`].
enum Setting {
    Dump(PathBuf),
    Duration(f32),
    Size(f32, f32),
    Scale(f32),
    Capture(PathBuf),
}

impl Setting {
    /// Fold into `settings`, refusing a second occurrence.
    ///
    /// A repeat is an error rather than last-wins: two `duration` lines mean the
    /// author believed one of them, and silently picking the other is the kind
    /// of thing that is only noticed after a run has produced the wrong answer.
    fn apply(self, settings: &mut ScriptSettings, line: usize) -> Result<(), ScriptError> {
        let dup = |what: &str| ScriptError {
            line,
            message: format!("`{what}` is given more than once"),
        };
        match self {
            Setting::Dump(p) => {
                if settings.dump.is_some() {
                    return Err(dup("dump"));
                }
                settings.dump = Some(p);
            }
            Setting::Duration(d) => {
                if settings.duration.is_some() {
                    return Err(dup("duration"));
                }
                settings.duration = Some(d);
            }
            Setting::Size(w, h) => {
                if settings.size.is_some() {
                    return Err(dup("size"));
                }
                settings.size = Some((w, h));
            }
            Setting::Scale(s) => {
                if settings.scale.is_some() {
                    return Err(dup("scale"));
                }
                settings.scale = Some(s);
            }
            Setting::Capture(p) => {
                if settings.capture.is_some() {
                    return Err(dup("capture"));
                }
                settings.capture = Some(p);
            }
        }
        Ok(())
    }
}

/// The verb that begins every untimed line.
///
/// Dispatching on the *first word* rather than on "does it parse as a number"
/// is deliberate: a mistyped time like `0.O5 key Q` still reports "not a time in
/// seconds" instead of "not a directive", so no existing diagnostic gets worse.
const SET_VERB: &str = "set";

/// The scope naming the run itself rather than anything the app draws.
const RUN_SCOPE: &str = "run";

/// Keys under [`RUN_SCOPE`].  Listed for the diagnostic; the match below is
/// what resolves them.
const RUN_KEYS: [&str; 5] = ["capture", "dump", "duration", "scale", "size"];

fn parse_line(text: &str, line: usize) -> Result<Line, ScriptError> {
    let first = text.split_whitespace().next().unwrap_or_default();
    if first == SET_VERB {
        let (spec, value) = split_set(text.split_whitespace().skip(1), line)?;
        return match parse_scope(&spec, line)? {
            Scope::Run(key) => parse_run_setting(&key, &value, line).map(Line::Run),
            Scope::App(target) => Ok(Line::Set(ScriptSet {
                target,
                value,
                line,
            })),
        };
    }
    parse_step(text, line).map(Line::Step)
}

/// What a `set` key path names.
enum Scope {
    /// A run setting, carrying the key after `run.`.
    Run(String),
    /// An app settings row, already resolved.
    App(SetTarget),
}

/// Split `set`'s two arguments, which are always a key path and one value.
///
/// One whitespace-free value, the same restriction `text` carries and for the
/// same reason: a quoted argument would need an escaping grammar this format
/// does not have, and refusing is better than silently taking the first word.
fn split_set<'a>(
    words: impl Iterator<Item = &'a str>,
    line: usize,
) -> Result<(String, String), ScriptError> {
    let rest: Vec<&str> = words.collect();
    let [spec, value] = rest[..] else {
        return Err(ScriptError {
            line,
            message: format!(
                "`set` takes a key path and one whitespace-free value, got {} \
                 — e.g. `set cofdm.cn_db 10`",
                rest.len()
            ),
        });
    };
    Ok((spec.to_owned(), value.to_owned()))
}

/// Resolve a `set` key path to the run or to a settings row.
fn parse_scope(spec: &str, line: usize) -> Result<Scope, ScriptError> {
    if let Some(key) = spec.strip_prefix(concat!("run", ".")) {
        return Ok(Scope::Run(key.to_owned()));
    }
    // Everything else is the app's own settings, which own their key tables —
    // so a source added later becomes settable with no edit here.
    SetTarget::resolve(spec)
        .map(Scope::App)
        .map_err(|message| ScriptError { line, message })
}

fn parse_run_setting(key: &str, arg: &str, line: usize) -> Result<Setting, ScriptError> {
    let err = |message: String| ScriptError { line, message };
    match key {
        "dump" => Ok(Setting::Dump(PathBuf::from(arg))),
        "capture" => Ok(Setting::Capture(PathBuf::from(arg))),
        "duration" => {
            let secs: f32 = arg
                .parse()
                .map_err(|_| err(format!("`{arg}` is not a duration in seconds")))?;
            if !secs.is_finite() || secs <= 0.0 {
                return Err(err(format!(
                    "duration `{arg}` must be finite and greater than 0"
                )));
            }
            Ok(Setting::Duration(secs))
        }
        "size" => parse_size(arg)
            .map(|(w, h)| Setting::Size(w, h))
            .ok_or_else(|| {
                err(format!(
                    "`{arg}` is not a size; write it as WIDTHxHEIGHT in points, e.g. 1200x828"
                ))
            }),
        "scale" => {
            let s: f32 = arg
                .parse()
                .map_err(|_| err(format!("`{arg}` is not a scale factor")))?;
            if !s.is_finite() || !(0.1..=8.0).contains(&s) {
                return Err(err(format!("scale `{arg}` must be between 0.1 and 8.0")));
            }
            Ok(Setting::Scale(s))
        }
        other => Err(err(format!(
            "`{other}` is not a run setting (expected one of: {})",
            RUN_KEYS.join(", ")
        ))),
    }
}

/// Parse `WIDTHxHEIGHT` in points.
///
/// `x` rather than a comma or a space: it is how every other tool spells a
/// window size, and it keeps the value a single whitespace-free argument like
/// every other setting's.
fn parse_size(s: &str) -> Option<(f32, f32)> {
    let (w, h) = s.split_once(['x', 'X'])?;
    let (w, h): (f32, f32) = (w.trim().parse().ok()?, h.trim().parse().ok()?);
    // An upper bound as well as a lower one: the whole reason this setting
    // exists is that egui's 10000 x 10000 fallback is not a window, and a
    // capture at that size would be 400 MB.
    let ok = |v: f32| v.is_finite() && (16.0..=8192.0).contains(&v);
    (ok(w) && ok(h)).then_some((w, h))
}

fn parse_step(text: &str, line: usize) -> Result<Step, ScriptError> {
    let err = |message: String| ScriptError { line, message };
    let mut words = text.split_whitespace();

    let t_word = words
        .next()
        .ok_or_else(|| err("expected a time".to_owned()))?;
    let t_secs: f32 = t_word
        .parse()
        .map_err(|_| err(format!("`{t_word}` is not a time in seconds")))?;
    if !t_secs.is_finite() || t_secs < 0.0 {
        return Err(err(format!("time `{t_word}` must be finite and >= 0")));
    }

    let verb = words.next().ok_or_else(|| {
        err("expected `key`, `source`, `still`, `pane`, `text`, `set` or `assert`".to_owned())
    })?;
    let rest: Vec<&str> = words.collect();

    // A trailing `xN` repeat count applies to key and text alike.
    let (rest, repeat) = match rest.split_last() {
        Some((last, head)) if is_repeat(last) => (head.to_vec(), parse_repeat(last, line)?),
        _ => (rest, 1),
    };

    let action = match verb {
        "key" => {
            let [spec] = rest[..] else {
                return Err(err(format!(
                    "`key` takes one `[mod+]Name` argument, got {}",
                    rest.len()
                )));
            };
            let (key, modifiers) = parse_key_spec(spec, line)?;
            Action::Key { key, modifiers }
        }
        "source" => {
            // A repeat is refused rather than ignored: `source COFDM x5` reads
            // as "five presses", but the count is exactly what this directive
            // exists to stop anyone writing, and honouring it would re-select
            // the same source five times — five playback resets, silently.
            if repeat != 1 {
                return Err(err(
                    "`source` takes no repeat count; it presses `I` until the named source is active"
                        .to_owned(),
                ));
            }
            if rest.is_empty() {
                return Err(err("`source` needs a source name".to_owned()));
            }
            // Joined, so a two-word label may be written the way it is shown:
            // `source AM DSB` and `source AM-DSB` are the same line.
            let name = rest.join(" ");
            let mode = source_mode_by_name(&name).ok_or_else(|| {
                let names: Vec<&str> = SourceMode::ALL.iter().map(|m| m.label()).collect();
                err(format!(
                    "`{name}` is not a source (expected one of: {})",
                    names.join(", ")
                ))
            })?;
            Action::Source { mode }
        }
        "still" => {
            if repeat != 1 {
                return Err(err(
                    "`still` takes no repeat count; capturing the same frame twice \
                     would only overwrite it"
                        .to_owned(),
                ));
            }
            Action::Still {
                label: parse_label(&rest, line)?,
            }
        }
        "pane" => {
            if repeat != 1 {
                return Err(err(
                    "`pane` takes no repeat count; writing the same raster twice \
                     would only overwrite it"
                        .to_owned(),
                ));
            }
            let Some((name, rest)) = rest.split_first() else {
                let names: Vec<&str> = Pane::ALL.iter().map(|p| p.name()).collect();
                return Err(err(format!(
                    "`pane` needs a pane name (one of: {})",
                    names.join(", ")
                )));
            };
            let pane = Pane::by_name(name).ok_or_else(|| {
                let names: Vec<&str> = Pane::ALL.iter().map(|p| p.name()).collect();
                err(format!(
                    "`{name}` is not a pane (expected one of: {})",
                    names.join(", ")
                ))
            })?;
            let label = parse_label(rest, line)?;
            Action::Pane { pane, label }
        }
        "text" => {
            let [literal] = rest[..] else {
                return Err(err(format!(
                    "`text` takes one whitespace-free argument, got {}",
                    rest.len()
                )));
            };
            Action::Text {
                text: literal.to_owned(),
            }
        }
        SET_VERB => {
            if repeat != 1 {
                return Err(err(
                    "`set` takes no repeat count; writing the same value twice \
                     changes nothing the first did not"
                        .to_owned(),
                ));
            }
            let (spec, value) = split_set(rest.iter().copied(), line)?;
            match parse_scope(&spec, line)? {
                Scope::App(target) => Action::Set { target, value },
                // Not "unknown key": the key is real, and saying so is what
                // stops the reader hunting for a typo that is not there.
                Scope::Run(key) => {
                    return Err(err(format!(
                        "`{RUN_SCOPE}.{key}` takes no time; it configures the run \
                         rather than happens during it"
                    )));
                }
            }
        }
        "assert" => {
            let Some((name, args)) = rest.split_first() else {
                return Err(err("`assert` needs a property name".to_owned()));
            };
            Action::Assert {
                name: (*name).to_owned(),
                args: args.iter().map(|s| (*s).to_owned()).collect(),
            }
        }
        other => {
            return Err(err(format!(
                "`{other}` is not a directive \
                 (expected `key`, `source`, `still`, `pane`, `text`, `set` or `assert`)"
            )));
        }
    };

    Ok(Step {
        t_secs,
        repeat,
        action,
        line,
    })
}

fn is_repeat(word: &str) -> bool {
    word.strip_prefix('x')
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

fn parse_repeat(word: &str, line: usize) -> Result<usize, ScriptError> {
    let n: usize = word[1..].parse().map_err(|_| ScriptError {
        line,
        message: format!("`{word}` is not a repeat count"),
    })?;
    if n == 0 {
        return Err(ScriptError {
            line,
            message: "a repeat count of 0 does nothing; drop the line instead".to_owned(),
        });
    }
    Ok(n)
}

/// Parse `shift+ctrl+ArrowUp` into a key and its modifier state.
///
/// Modifiers are spelled out rather than punctuated because the app binds
/// several combinations and `^`/`⌥` shorthand reads badly in a plain-text file.
/// `command` is deliberately accepted and mapped to egui's platform-dependent
/// `command` flag, which is Cmd on macOS and Ctrl elsewhere — the same
/// distinction that keeps the capture keys unmodified.
fn parse_key_spec(spec: &str, line: usize) -> Result<(egui::Key, egui::Modifiers), ScriptError> {
    let mut modifiers = egui::Modifiers::default();
    let mut parts = spec.split('+').peekable();
    let mut name = None;
    while let Some(part) = parts.next() {
        let last = parts.peek().is_none();
        // A lone `+` key would arrive as an empty part; treat the final part as
        // the key name whatever it looks like.
        if last {
            name = Some(part);
            break;
        }
        match part.to_ascii_lowercase().as_str() {
            "shift" => modifiers.shift = true,
            "ctrl" | "control" => modifiers.ctrl = true,
            "alt" | "option" => modifiers.alt = true,
            "cmd" | "command" => modifiers.command = true,
            other => {
                return Err(ScriptError {
                    line,
                    message: format!("`{other}` is not a modifier"),
                });
            }
        }
    }
    let name = name.filter(|n| !n.is_empty()).ok_or_else(|| ScriptError {
        line,
        message: format!("`{spec}` names no key"),
    })?;
    let key = egui::Key::from_name(name).ok_or_else(|| ScriptError {
        line,
        message: format!("`{name}` is not an egui key name"),
    })?;
    Ok((key, modifiers))
}
