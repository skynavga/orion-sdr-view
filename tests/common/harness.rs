// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Headless harness: drive [`ViewApp`] through complete egui passes with **no
//! renderer, no window and no wgpu device**.
//!
//! `egui::Context::begin_pass`/`end_pass` are public, so a full pass runs
//! against a bare `Context`; the `FullOutput` (tessellated shapes and texture
//! deltas) is simply dropped.  What survives is all the state the app computed
//! on the way — which is what every UI-layer defect this project has produced
//! has been made of.
//!
//! Two things a harness has to know about this app:
//!
//! * **`handle_keys` runs from `draw`, not `advance`.**  Advancing alone
//!   processes samples and never sees a keystroke, so [`Harness::frame`] calls
//!   it explicitly.  Moving the call into `advance` would change its ordering
//!   against the settings overlay, which is load-bearing.
//! * **`dt` is injected.**  Every frame here is exactly [`Harness::DT`], so two
//!   runs of the same script produce the same samples — the property the
//!   determinism test rests on.
//!
//! Drawing is deliberately not exercised.  It is unnecessary for state
//! assertions and would drag in fonts, layout and a screen rect; see the
//! "Tier 2" discussion in `headless-app-testing.md`.

use orion_sdr_view::app::ViewApp;
use orion_sdr_view::config::ViewConfig;
use orion_sdr_view::utils::script::{Action, Script};

/// Wrapper matching the `view:` key, so a test can build a [`ViewConfig`] from
/// an inline YAML fixture the same way the real loader does.
#[derive(serde::Deserialize)]
struct TestFile {
    view: ViewConfig,
}

/// A [`ViewConfig`] parsed from a YAML fragment rooted at `view:`.
pub fn config_from_yaml(yaml: &str) -> ViewConfig {
    serde_yaml::from_str::<TestFile>(yaml)
        .expect("fixture parses")
        .view
}

pub struct Harness {
    pub ctx: egui::Context,
    pub app: ViewApp,
    frames: usize,
}

impl Harness {
    /// The fixed frame time.  1/60 s because that is what the app is paced for:
    /// at 48 kHz it makes the per-frame sample budget 800, comfortably inside
    /// the 128..4096 clamp, so the narrowband sources run at true wall clock.
    pub const DT: f32 = 1.0 / 60.0;

    pub fn new(cfg: ViewConfig) -> Self {
        let ctx = egui::Context::default();
        let app = ViewApp::new(&ctx, cfg);
        Self {
            ctx,
            app,
            frames: 0,
        }
    }

    /// A harness on built-in defaults.
    ///
    /// [`ViewConfig::empty`] rather than `load(None)`, which would pick up a
    /// `.orionsdr.yaml` in the working directory and make the result depend on
    /// where the test was run from.
    pub fn with_defaults() -> Self {
        Self::new(ViewConfig::empty())
    }

    /// A harness on an inline YAML fixture rooted at `view:`.
    pub fn from_yaml(yaml: &str) -> Self {
        Self::new(config_from_yaml(yaml))
    }

    /// Scripted time at the *start* of the next frame.  Computed from the frame
    /// count rather than accumulated, so it does not drift.
    pub fn t(&self) -> f32 {
        self.frames as f32 * Self::DT
    }

    /// One complete pass: deliver `events`, advance by `DT`, run key handling.
    pub fn frame(&mut self, events: Vec<egui::Event>, modifiers: egui::Modifiers) {
        self.ctx.begin_pass(egui::RawInput {
            events,
            modifiers,
            ..Default::default()
        });
        self.app.advance(&self.ctx, Self::DT);
        self.app.handle_keys(&self.ctx);
        let _ = self.ctx.end_pass();
        self.frames += 1;
    }

    /// `n` frames with no input.
    pub fn idle(&mut self, n: usize) {
        for _ in 0..n {
            self.frame(Vec::new(), egui::Modifiers::default());
        }
    }

    /// Press and release one key, in one frame.
    pub fn key(&mut self, key: egui::Key) {
        self.key_mod(key, egui::Modifiers::default());
    }

    /// Press and release one key with modifiers held, in one frame.
    pub fn key_mod(&mut self, key: egui::Key, modifiers: egui::Modifiers) {
        let action = Action::Key { key, modifiers };
        self.frame(action.events(), modifiers);
    }

    /// Press one key `n` times.
    ///
    /// One frame per press: `key_pressed` is a per-pass boolean, so `n` press
    /// events inside a single pass would register as one.
    pub fn key_n(&mut self, key: egui::Key, n: usize) {
        for _ in 0..n {
            self.key(key);
        }
    }

    /// Deliver a text event, in one frame.  The marker (`a`/`b`/`A`/`B`), help
    /// (`?`) and dB-reference (`[`/`]`) bindings are only reachable this way.
    pub fn text(&mut self, s: &str) {
        self.frame(
            vec![egui::Event::Text(s.to_owned())],
            egui::Modifiers::default(),
        );
    }

    /// Switch to `mode` by pressing `I` until it is active, rather than by
    /// calling `switch_source` — the key path is what the tests are about.
    pub fn select_source(&mut self, mode: orion_sdr_view::app::SourceMode) {
        for _ in 0..orion_sdr_view::app::SourceMode::ALL.len() {
            if self.app.source_mode() == mode {
                return;
            }
            self.key(egui::Key::I);
        }
        assert_eq!(
            self.app.source_mode(),
            mode,
            "never reached {}",
            mode.label()
        );
    }

    /// Replay a script, executing its `assert` directives.
    ///
    /// Panics naming the offending source line, so a failure points at the
    /// script rather than at the harness.
    pub fn run_script(&mut self, src: &str) {
        let script = Script::parse(src).unwrap_or_else(|e| panic!("script does not parse: {e}"));
        for step in &script.steps {
            while self.t() < step.t_secs {
                self.idle(1);
            }
            match &step.action {
                Action::Assert { name, args } => self.check(step.line, name, args),
                action => {
                    for _ in 0..step.repeat {
                        self.frame(action.events(), action.modifiers());
                    }
                }
            }
        }
    }

    /// The value of a named property, for `assert` directives.
    ///
    /// Deliberately small: a script asserts on what a *user* can see, so the
    /// list mirrors the HUD and the settings panel rather than exposing
    /// internals.
    fn probe(&self, name: &str) -> Option<f64> {
        use orion_sdr_view::app::settings::CofdmSettings;
        Some(match name {
            "center_hz" => self.app.freq_view().center_hz as f64,
            "span_hz" => self.app.freq_view().span_hz as f64,
            "zoom" => self.app.freq_view().zoom_ratio() as f64,
            "zoom_row" => self.app.settings().zoom_ratio() as f64,
            "source" => self.app.source_mode().index() as f64,
            "locked" => self.app.source_locked() as u8 as f64,
            "fs_hz" => self.app.source_sample_rate() as f64,
            "cofdm_center_hz" => self.app.settings().cofdm_center_hz() as f64,
            _ => return None,
        })
    }

    fn check(&self, line: usize, name: &str, args: &[String]) {
        let got = self
            .probe(name)
            .unwrap_or_else(|| panic!("line {line}: `{name}` is not an assertable property"));
        let want: f64 = args
            .first()
            .unwrap_or_else(|| panic!("line {line}: `assert {name}` needs a value"))
            .parse()
            .unwrap_or_else(|_| panic!("line {line}: `{:?}` is not a number", args[0]));
        let tol: f64 = match args.get(1) {
            Some(t) => t
                .parse()
                .unwrap_or_else(|_| panic!("line {line}: `{t}` is not a tolerance")),
            None => want.abs().max(1.0) * 1e-6,
        };
        assert!(
            (got - want).abs() <= tol,
            "line {line}: {name} is {got}, expected {want} (±{tol})"
        );
    }
}
