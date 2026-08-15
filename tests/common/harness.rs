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
use orion_sdr_view::utils::script::{Action, Script, source_mode_by_name};

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
    /// Viewport commands from the most recent pass.  The app's only outward
    /// channel for a capture request, and observable without a renderer.
    viewport_commands: Vec<egui::ViewportCommand>,
    /// Where a `pane` directive writes.  Defaults to the app's own setting.
    pub capture_dir: std::path::PathBuf,
}

impl Harness {
    /// The fixed frame time.  1/60 s because that is what the app is paced for:
    /// at 48 kHz it makes the per-frame sample budget 800, comfortably inside
    /// the 128..4096 clamp, so the narrowband sources run at true wall clock.
    pub const DT: f32 = 1.0 / 60.0;

    pub fn new(cfg: ViewConfig) -> Self {
        let ctx = egui::Context::default();
        let app = ViewApp::new(&ctx, cfg);
        let capture_dir = app.capture_dir().to_path_buf();
        Self {
            ctx,
            app,
            frames: 0,
            viewport_commands: Vec::new(),
            capture_dir,
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
        // A real window size, not egui's 10000 x 10000 fallback for a pass that
        // supplies none.  Matches the driver's `DEFAULT_SIZE`, so a script
        // replayed here and by the driver lays out identically.
        let (w, h) = orion_sdr_view::replay::DEFAULT_SIZE;
        let mut raw = egui::RawInput {
            events,
            modifiers,
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(w, h),
            )),
            ..Default::default()
        };
        let id = raw.viewport_id;
        raw.viewports.entry(id).or_default().native_pixels_per_point =
            Some(orion_sdr_view::replay::DEFAULT_SCALE);
        self.ctx.begin_pass(raw);
        self.app.advance(&self.ctx, Self::DT);
        self.app.handle_keys(&self.ctx);
        let out = self.ctx.end_pass();
        // Keep the viewport commands this pass produced.  They are the app's
        // only outward channel for a screenshot request, and a bare
        // `egui::Context` is enough to observe them — which is what makes the
        // capture path testable with no window, renderer or GPU.
        self.viewport_commands = out
            .viewport_output
            .into_values()
            .flat_map(|v| v.commands)
            .collect();
        self.frames += 1;
    }

    /// The viewport commands the last pass emitted.
    pub fn viewport_commands(&self) -> &[egui::ViewportCommand] {
        &self.viewport_commands
    }

    /// The capture tags requested during the last pass.
    pub fn screenshot_tags(&self) -> Vec<orion_sdr_view::capture::CaptureTag> {
        self.viewport_commands
            .iter()
            .filter_map(|c| match c {
                egui::ViewportCommand::Screenshot(user_data) => user_data
                    .data
                    .as_ref()?
                    .downcast_ref::<orion_sdr_view::capture::CaptureTag>()
                    .copied(),
                _ => None,
            })
            .collect()
    }

    /// The window title the last pass asked for, if it asked.
    pub fn requested_title(&self) -> Option<String> {
        self.viewport_commands.iter().rev().find_map(|c| match c {
            egui::ViewportCommand::Title(t) => Some(t.clone()),
            _ => None,
        })
    }

    /// Deliver a screenshot reply, as eframe's wgpu integration would.
    ///
    /// The real readback pushes `Event::Screenshot` into the *next* pass's raw
    /// input; this is that, with a synthetic image of `colour`.
    pub fn deliver_screenshot(
        &mut self,
        tag: orion_sdr_view::capture::CaptureTag,
        width: usize,
        height: usize,
        colour: egui::Color32,
    ) {
        let image = egui::ColorImage::new([width, height], vec![colour; width * height]);
        self.frame(
            vec![egui::Event::Screenshot {
                viewport_id: egui::ViewportId::ROOT,
                user_data: egui::UserData::new(tag),
                image: std::sync::Arc::new(image),
            }],
            egui::Modifiers::default(),
        );
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
                // Already implemented, and by pressing `I` — which is exactly
                // what the directive means.  The parser refuses a repeat on it,
                // so there is no count to honour here.
                Action::Source { mode } => self.select_source(*mode),
                // Executed, not ignored: a pane's pixels are CPU-side, so the
                // harness can write one exactly as the driver does.
                Action::Pane { pane, label } => {
                    let dir = self.capture_dir.clone();
                    self.app
                        .capture_pane(&dir, *pane, label.as_deref())
                        .unwrap_or_else(|e| panic!("line {}: {e}", step.line));
                }
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
            "locked" => self.app.source_locked() as u8 as f64,
            "fs_hz" => self.app.source_sample_rate() as f64,
            "cofdm_center_hz" => self.app.settings().cofdm_center_hz() as f64,
            _ => return None,
        })
    }

    fn check(&self, line: usize, name: &str, args: &[String]) {
        let first = args
            .first()
            .unwrap_or_else(|| panic!("line {line}: `assert {name}` needs a value"));

        // `source` asserts a **name**, not an index.  An index is a position in
        // `SourceMode::ALL`, so adding or reordering a source changes what the
        // line means without changing the line — and the assertion carries on
        // passing, against a source nobody asked about.  Resolved through the
        // same folding the `source` directive uses, so `assert source am-dsb`
        // and `source AM-DSB` agree on what they name.
        if name == "source" {
            let got = self.app.source_mode();
            // Joined, not just the first word: `assert` splits on whitespace, so
            // a two-word label arrives as two arguments.  The `source` directive
            // joins for the same reason, and the two must accept the same
            // spellings or `assert source Test Tone` would read as `Test`.
            let spelling = args.join(" ");
            let want = source_mode_by_name(&spelling)
                .unwrap_or_else(|| panic!("line {line}: `{spelling}` is not a source"));
            assert_eq!(
                got,
                want,
                "line {line}: source is {}, expected {}",
                got.label(),
                want.label()
            );
            return;
        }

        let got = self
            .probe(name)
            .unwrap_or_else(|| panic!("line {line}: `{name}` is not an assertable property"));
        let want: f64 = first
            .parse()
            .unwrap_or_else(|_| panic!("line {line}: `{first:?}` is not a number"));
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
