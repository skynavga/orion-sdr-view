// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! A timed key script: the shared input format for driving the app without a
//! human at the keyboard.
//!
//! ```text
//! duration 30                    # run settings: no time, at most one each
//! dump     run.jsonl
//!
//! # t(s)   directive
//! 0.00     source COFDM          # select a source by name
//! 0.50     key L                 # lock the source to the viewport centre
//! 0.75     key shift+ArrowRight
//! 0.80     text a                # markers arrive as Text, not Key
//! 1.00     assert center_hz 520000
//! ```
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
//! # Run settings
//!
//! `duration` and `dump` take **no time**, because they configure the run rather
//! than happen during it.  That is also what makes them unambiguous to parse: a
//! line beginning with one of those two words is a setting, and anything else
//! must begin with a time exactly as before, so no existing diagnostic changes.
//!
//! They exist so a script can be a **self-contained recipe** — one file that
//! says what to press, how long for, and where the answer goes.  The command
//! line overrides either, which is what keeps that recipe reusable: the same
//! script can be run longer, or dumped somewhere else, without editing it.
//!
//! A `dump` path may be absolute or relative, and a relative one resolves
//! **against the viewer's working directory** — the same as `--dump` and as any
//! other path a shell hands a program.  The directive is a default for the flag,
//! so the two must mean the same thing given the same string.  That includes
//! `-`, which means standard output in both; see
//! [`STDOUT_PATH`](crate::replay::STDOUT_PATH).

use std::fmt;
use std::path::PathBuf;

use crate::app::SourceMode;

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
            // None delivers input: two write a file, the third is checked.
            Self::Still { .. } | Self::Pane { .. } | Self::Assert { .. } => Vec::new(),
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

/// Fold a source name to its comparison form: ASCII letters and digits only,
/// lowercased.  Spacing and punctuation carry no meaning in these labels, so
/// two spellings that differ only there are the same name.
fn fold_name(s: &str) -> String {
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
                Line::Setting(set) => set.apply(&mut settings, line)?,
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

/// One non-blank line: either a timed step or a run setting.
enum Line {
    Step(Step),
    Setting(Setting),
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

/// Words that begin a run-setting line rather than a time.
///
/// Dispatching on the *first word* rather than on "does it parse as a number"
/// is deliberate: a mistyped time like `0.O5 key Q` still reports "not a time in
/// seconds" instead of "not a directive", so no existing diagnostic gets worse.
const SETTING_VERBS: [&str; 5] = ["capture", "dump", "duration", "scale", "size"];

fn parse_line(text: &str, line: usize) -> Result<Line, ScriptError> {
    let first = text.split_whitespace().next().unwrap_or_default();
    if SETTING_VERBS.contains(&first) {
        return parse_setting(text, line).map(Line::Setting);
    }
    parse_step(text, line).map(Line::Step)
}

fn parse_setting(text: &str, line: usize) -> Result<Setting, ScriptError> {
    let err = |message: String| ScriptError { line, message };
    let mut words = text.split_whitespace();
    let verb = words.next().unwrap_or_default();
    let rest: Vec<&str> = words.collect();
    let [arg] = rest[..] else {
        return Err(err(format!(
            "`{verb}` takes exactly one argument, got {}",
            rest.len()
        )));
    };
    match verb {
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
        other => Err(err(format!("`{other}` is not a run setting"))),
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
        err("expected `key`, `source`, `still`, `pane`, `text` or `assert`".to_owned())
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
                 (expected `key`, `source`, `still`, `pane`, `text` or `assert`)"
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
