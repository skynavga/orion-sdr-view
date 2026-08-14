// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use eframe::egui;

use super::freqview::{FreqMarker, FreqView};
use super::persistence::PersistenceRenderer;
use super::settings::{AmDsbSettings, CwSettings, Psk31Settings, SettingsState, ToneSettings};
use super::spectrogram::SpectrogramDisplay;
use super::spectrum::{RingBuffer, SpectrumProcessor};
use super::waterfall::WaterfallDisplay;
use crate::config::ViewConfig;
use crate::decode::{
    DecodeChunk, DecodeConfig, DecodeResult, DecodeState, DecodeTicker, DecodeWorker,
    SIGNAL_THRESHOLD,
};
use crate::source::SignalSource;
use crate::source::tone::TestSignalGen;
use crate::source::tone::TestToneSource;
use crate::utils::time::Clock;
use crate::utils::timer::LoopTimer;

use super::{
    DECODE_BAR_H, DecodeBarMode, FFT_SIZE, MAX_SAMPLES_PER_FRAME, MIN_SAMPLES_PER_FRAME,
    SAMPLE_RATE, SourceMode, WaterfallMode,
};

/// The three mutually-exclusive overlays.  Only one can be up at a time.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Overlay {
    Help,
    Instrument,
    Settings,
}

// ── ViewApp ───────────────────────────────────────────────────────────────────

pub struct ViewApp {
    pub(super) pane_visible: [bool; 3],
    // Fractional height per pane — stored even when hidden so proportions are
    // remembered when re-shown. Future resize handles will mutate these values.
    pub(super) pane_frac: [f32; 3],
    pub(super) show_help: bool,
    /// COFDM instrumentation panel (`X`).  Mutually exclusive with the help
    /// overlay so the two can never stack.
    pub(super) show_instrument: bool,
    pub(super) mono_font_id: egui::FontId,

    // Active signal source (Box<dyn SignalSource> for easy future extension)
    pub(super) source_mode: SourceMode,
    pub(super) source: Box<dyn SignalSource>,

    // Test tone generator — kept alive so its state (cycling, settings) persists
    // across source switches. TestToneSource borrows it when active.
    pub(super) signal_gen: TestSignalGen,

    pub(super) ring_buf: RingBuffer,
    pub(super) spectrum: SpectrumProcessor,
    pub(super) db_min: f32,
    pub(super) db_max: f32,
    /// Peak-hold line: per-bin max dB, decayed slowly.
    pub(super) peak_hold: Vec<f32>,
    pub(super) peak_hold_visible: bool,

    // Pane 2: persistence density
    pub(super) persistence: PersistenceRenderer,
    pub(super) envelope_visible: bool,

    // Pane 3: waterfall — two presentations, cycled by `W`.
    pub(super) waterfall: WaterfallDisplay,
    pub(super) spectrogram: SpectrogramDisplay,
    pub(super) waterfall_mode: WaterfallMode,

    // Frequency viewport (pan + zoom) — shared across all panes
    pub(super) freq_view: FreqView,

    // Markers
    pub(super) markers: [FreqMarker; 3],
    /// Which bracket marker is selected for keyboard positioning: Some(1)=A, Some(2)=B, None.
    pub(super) active_marker: Option<usize>,

    // Settings popover
    pub(super) settings: SettingsState,

    // When true, source freq/carrier tracks center_hz on every display change.
    pub(super) source_locked: bool,

    // Decode bar (pane 3): cycled by D key (Off / Info-only / Text-only).
    pub(super) decode_bar: DecodeBarMode,
    pub(super) loop_timer: LoopTimer,

    // Decode thread channels and shared config.
    pub(super) decode_config: Arc<Mutex<DecodeConfig>>,
    pub(super) decode_tx: mpsc::SyncSender<DecodeChunk>,
    /// Monotonic counter stamped on each `DecodeChunk`; see its `seq` field.
    decode_seq: u64,
    pub(super) decode_rx: mpsc::Receiver<DecodeResult>,
    /// Present when decoding runs on this thread rather than a worker; see
    /// [`InlineDecoder`].
    inline_decode: Option<InlineDecoder>,
    pub(super) decode_ticker: DecodeTicker,
    /// True if the previous frame's sample block was above SIGNAL_THRESHOLD.
    pub(super) last_block_was_signal: bool,
    /// Wall-clock time of the previous frame, for real-time dt calculation.
    /// Read by the `eframe::App` adapter alone — [`advance`](Self::advance) is
    /// handed its `dt` rather than deriving one.
    pub(super) last_frame_time: std::time::Instant,

    /// Per-frame view-side state for FT8/FT4 (frame counts, pending onset,
    /// cached mode/msg_type).  See [`Ft8ViewState`].
    pub(super) ft8_view: crate::source::ft8::Ft8ViewState,
    /// Display timestamps offset from UTC by this many minutes (0 = UTC).
    pub(super) time_zone_offset_min: i32,
    /// Where the burst and frame timestamps come from.  [`Clock::System`]
    /// interactively; [`Clock::Scripted`] under the replay driver, so a run's
    /// decoded text repeats exactly.
    pub(super) clock: Clock,
    /// Samples the source has produced since startup.
    ///
    /// The honest measure of how much *signal* a run covered, which scripted
    /// time is not: the per-frame budget is `dt * fs` clamped to
    /// [`MAX_SAMPLES_PER_FRAME`], and at COFDM's 1.92 MHz that clamp binds.
    samples_consumed: u64,
}

/// Decoding on the caller's thread instead of a worker.
///
/// Holds the worker's two halves — the chunk receiver and the result sender —
/// plus the state that would otherwise live in [`DecodeWorker::run`]'s frame.
/// [`ViewApp::advance`] sends a chunk and immediately pumps it through, so no
/// chunk is ever in flight across a frame boundary and none can be dropped.
///
/// **This is what makes a dump a measurement of the link.** On the worker
/// thread both channels `try_send`, which silently discards under pressure —
/// correct for a real-time display that must not stall, and wrong for a
/// measurement, where the resulting frame errors would be charged to a
/// perfectly good signal.
struct InlineDecoder {
    state: DecodeState,
    chunks: mpsc::Receiver<DecodeChunk>,
    results: mpsc::SyncSender<DecodeResult>,
    /// Every result this frame, in the order the ticker saw them.
    ///
    /// A tap rather than a second consumer: the dump and the `X` panel read the
    /// *same* stream, so the two cannot disagree about what the receiver
    /// reported.  A separate projection would be a second thing to keep in step.
    tapped: Vec<DecodeResult>,
}

impl ViewApp {
    /// Construct the app against a live [`egui::Context`].
    ///
    /// Takes the context rather than an `eframe::CreationContext` because the
    /// only thing that was ever wanted from the latter is `cc.egui_ctx`, and
    /// `CreationContext` cannot be built outside eframe — which put the whole
    /// app out of reach of `tests/`.  The eframe adapter at the bottom of this
    /// file passes `&cc.egui_ctx`; a test passes `&egui::Context::default()`.
    pub fn new(ctx: &egui::Context, cfg: ViewConfig) -> Self {
        Self::build(ctx, cfg, Clock::System, false)
    }

    /// Construct for a **reproducible** run: decode inline on this thread, and
    /// stamp timestamps from a scripted clock rather than the system one.
    ///
    /// Both departures exist for the same reason.  A worker thread makes result
    /// ordering depend on the scheduler and drops chunks under pressure; the
    /// system clock puts the time of day into decoded text.  Either one alone is
    /// enough to make two runs of the same script differ.
    ///
    /// The interactive app must keep both — a display that stalls on a slow
    /// decode is worse than one that skips a block, and a burst stamped
    /// 2026-01-01 would be a lie on screen.  So this is a second constructor
    /// rather than a change to [`new`](Self::new).
    pub fn new_replay(ctx: &egui::Context, cfg: ViewConfig) -> Self {
        Self::build(ctx, cfg, Clock::scripted(), true)
    }

    fn build(ctx: &egui::Context, cfg: ViewConfig, clock: Clock, inline_decode: bool) -> Self {
        let font_bytes = include_bytes!("../../assets/fonts/DejaVuSansMono.ttf");
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "DejaVuSansMono".to_owned(),
            egui::FontData::from_static(font_bytes).into(),
        );
        fonts
            .families
            .entry(egui::FontFamily::Monospace)
            .or_default()
            .insert(0, "DejaVuSansMono".to_owned());
        ctx.set_fonts(fonts);

        // Coherence clamp: db_min must be strictly less than db_max
        let db_max = cfg.db_max();
        let db_min = cfg.db_min().min(db_max - 1.0);

        let signal_gen = TestSignalGen::new(cfg.freq_hz(), SAMPLE_RATE);
        let source: Box<dyn SignalSource> = Box::new(TestToneSource::new(TestSignalGen::new(
            cfg.freq_hz(),
            SAMPLE_RATE,
        )));

        // Decode setup.
        let decode_config = Arc::new(Mutex::new(DecodeConfig::new(SAMPLE_RATE)));
        // Capacity 256: at 60 fps each block is ~16 ms; 256 slots ≈ 4 s of buffer,
        // enough to absorb a slow psk31_sync pass without dropping gap blocks.
        let (decode_tx, sample_rx) = mpsc::sync_channel::<DecodeChunk>(256);
        // The result channel is generously sized when decoding inline: a single
        // chunk can emit several results, and the whole point of the inline path
        // is that nothing is discarded.  The threaded path keeps 16, which is a
        // deliberate back-pressure limit rather than an estimate.
        let (result_tx, decode_rx) =
            mpsc::sync_channel::<DecodeResult>(if inline_decode { 4096 } else { 16 });
        let inline_decode = if inline_decode {
            Some(InlineDecoder {
                state: DecodeState::new(),
                chunks: sample_rx,
                results: result_tx,
                tapped: Vec::new(),
            })
        } else {
            let worker_cfg = Arc::clone(&decode_config);
            std::thread::spawn(move || DecodeWorker::new(worker_cfg, sample_rx, result_tx).run());
            None
        };

        let mut app = Self {
            decode_seq: 0,
            pane_visible: [true; 3],
            pane_frac: [1.0 / 3.0; 3],
            show_help: false,
            show_instrument: false,
            mono_font_id: egui::FontId::new(14.0, egui::FontFamily::Monospace),

            source_mode: SourceMode::TestTone,
            source,
            signal_gen,

            ring_buf: RingBuffer::new(FFT_SIZE),
            spectrum: SpectrumProcessor::new(FFT_SIZE),
            db_min,
            db_max,
            peak_hold: vec![-120.0; FFT_SIZE / 2 + 1],
            peak_hold_visible: true,

            persistence: PersistenceRenderer::new(FFT_SIZE / 2 + 1, 100),
            envelope_visible: true,

            waterfall: WaterfallDisplay::new(FFT_SIZE / 2 + 1, 512, db_min, db_max),
            spectrogram: {
                let mut s = SpectrogramDisplay::new(FFT_SIZE / 2 + 1, 512, db_min, db_max);
                s.set_time_range(cfg.spec_time_range_secs());
                s
            },
            waterfall_mode: WaterfallMode::Vertical,

            freq_view: FreqView::new(SAMPLE_RATE / 2.0),
            markers: [
                FreqMarker::primary(SAMPLE_RATE / 4.0),
                FreqMarker::bracket_a(10_000.0),
                FreqMarker::bracket_b(14_000.0),
            ],
            active_marker: None,

            settings: SettingsState::from_config(&cfg),

            source_locked: false,

            decode_bar: DecodeBarMode::Off,
            loop_timer: LoopTimer::new(),

            decode_config,
            decode_tx,
            decode_rx,
            inline_decode,
            decode_ticker: DecodeTicker::new(),
            last_block_was_signal: false,
            last_frame_time: std::time::Instant::now(),

            ft8_view: crate::source::ft8::Ft8ViewState::new(),
            time_zone_offset_min: 0,
            clock,
            samples_consumed: 0,
        };
        app.time_zone_offset_min = cfg.time_zone_offset_min();
        // Precedence rule, step 1 of 3: the configured zoom applies at startup.
        // (2) a source's `preferred_span_hz` applies on switch *to* it, and
        // (3) the keyboard applies until the next switch.  So this is a startup
        // default rather than a persistent override — see `DisplayConfig::zoom`.
        app.settings.set_zoom_max(app.freq_view.max_zoom_ratio());
        app.freq_view.set_zoom_ratio(app.settings.zoom_ratio());
        app.sync_decode_config();
        super::source::debug_assert_factory_order(&app.settings);
        app
    }

    /// Loop-timer holdoff for the active source.  Only CW uses holdoff; all
    /// other modes use immediate signal/gap transitions.
    pub(super) fn loop_timer_holdoff_secs(&self) -> f32 {
        if self.source_mode == SourceMode::Cw {
            super::source::cw::holdoff_secs(&self.settings)
        } else {
            0.0
        }
    }

    /// The active source's requested C/N, in dB, for the HUD.
    ///
    /// Each source owns its own row rather than sharing one, so this is a
    /// per-source lookup — but the *unit* is now the same for all of them,
    /// which is what lets it be one trait call instead of a `match`.  Before
    /// the C/N change the HUD showed a raw amplitude whose meaning depended on
    /// which source was active.
    pub(super) fn hud_cn_db(&self) -> f32 {
        super::common::source_mode_factory(self.source_mode).cn_db(&self.settings)
    }

    /// Block-RMS level that counts as signal, for a source that does not report
    /// its own phase.
    ///
    /// One value for every source: they are all unit-scale now.  COFDM used to
    /// need its own (`COFDM_SIGNAL_THRESHOLD` = 0.6) because a fitted gain of
    /// 121.0 put its burst an order of magnitude above the shared level while
    /// its gap noise could climb past it — so the threshold had to sit between
    /// two populations that both moved.  Deriving the display level removed the
    /// reason for it.
    pub(super) fn signal_threshold(&self) -> f32 {
        SIGNAL_THRESHOLD
    }

    /// Hard reset: revert all source-mode settings rows to defaults, then
    /// restart the source.  Call on the R key (when settings popover is
    /// closed) and on `switch_source` — i.e. anything that should snap state
    /// back to defaults.
    ///
    /// Do NOT call this from "apply a setting change" paths (cycle audio,
    /// cycle msg mode, commit message, M/N keys) — `reset_source_rows`
    /// would undo the change you just made.  Use `restart_source` for those.
    pub(super) fn reset_playback(&mut self) {
        self.settings.reset_source_rows();
        self.restart_source();
    }

    /// Soft restart: reconstruct the active source from current settings,
    /// reset the loop timer, flush the decode pipeline.  Settings rows are
    /// NOT touched, so caller-applied row changes persist.  Used by all the
    /// "apply a setting and restart playback" paths.
    pub(super) fn restart_source(&mut self) {
        self.sync_settings();
        if self.source_mode == SourceMode::TestTone {
            self.signal_gen = TestSignalGen::new(self.settings.freq_hz(), SAMPLE_RATE);
        }
        self.source = self.make_source();
        self.apply_source_sample_rate();
        self.loop_timer.reset();
        self.loop_timer.set_holdoff(self.loop_timer_holdoff_secs());
        self.loop_timer
            .set_signal_threshold(self.signal_threshold());
        self.decode_ticker.reset();
        self.last_block_was_signal = false;
        self.spectrogram.clear();
        self.ft8_view.reset();
        while self.decode_rx.try_recv().is_ok() {}
        self.decode_seq = self.decode_seq.wrapping_add(1);
        let _ = self
            .decode_tx
            .try_send(DecodeChunk::real(self.decode_seq, Vec::new()));
    }

    /// Re-derive the sample-rate-dependent display pipeline from the currently
    /// constructed source's `sample_rate()`.  Called after any source
    /// (re)construction.  For the narrowband sources (all 48 kHz) this is a
    /// no-op reproducing today's behavior; a source at a different rate shifts
    /// the Nyquist limit, re-bounds the `Zoom` row, and clears bin-indexed
    /// history so the new frequency scaling isn't mixed with the old.
    ///
    /// **Clearing the history is why the rate is a config key and not a row.**
    /// An arrow-nudged `fs` would wipe the waterfall, persistence and
    /// spectrogram on every keypress.
    ///
    /// The rate now varies *within* a source too, not just between them: COFDM
    /// reads `sources.cofdm.fs_hz`.  Nothing here changes for that — the value
    /// has always come from the constructed source rather than from a table.
    fn apply_source_sample_rate(&mut self) {
        let fs = self.source.sample_rate();
        self.freq_view.set_nyquist(fs / 2.0);
        self.settings.set_zoom_max(self.freq_view.max_zoom_ratio());
        self.waterfall.clear();
        self.persistence.clear();
        self.spectrogram.clear();
        if let Ok(mut cfg) = self.decode_config.lock() {
            cfg.fs = fs;
        }
    }

    /// When source_locked, write center_hz into the active source's freq/carrier
    /// setting rows and call sync_settings() to propagate immediately.
    pub(super) fn lock_source_to_center(&mut self) {
        if !self.source_locked {
            return;
        }
        let hz = FreqView::snap_hz(self.freq_view.center_hz, 10.0);
        super::common::source_mode_factory(self.source_mode).set_carrier_hz(&mut self.settings, hz);
        self.sync_settings();
    }

    /// Switch the active source to `mode`, constructing a new source box.
    pub(super) fn switch_source(&mut self, mode: SourceMode) {
        self.source_mode = mode;
        if mode == SourceMode::Ft8 {
            self.ft8_view.reset_to_defaults();
        }
        self.source = if mode == SourceMode::TestTone {
            // Re-create from signal_gen's current settings, not settings.freq_hz()
            Box::new(TestToneSource::new(TestSignalGen::new(
                self.signal_gen.freq_hz,
                SAMPLE_RATE,
            )))
        } else {
            self.make_source()
        };
        self.settings.set_source_mode(mode as usize);
        // Re-derive the per-source sample rate (Nyquist, decode fs, `Zoom` row
        // bound, cleared history) before reframing, so reframe clamps to the
        // new Nyquist.
        self.apply_source_sample_rate();
        let factory = super::common::source_mode_factory(mode);
        // Before `reset_playback`, because this writes a *display* row and the
        // `sync_settings` inside that call is what propagates it to `db_max` and
        // the waterfall.  Display rows are not what `reset_playback` resets.
        if let Some(ref_db) = factory.preferred_ref_db(&self.settings) {
            self.settings.set_db_max(ref_db);
        }
        self.sync_decode_config();
        self.reset_playback();
        // Framed *after* the row reset, because the band centre is read from
        // this source's rows and `reset_playback` restores them to their
        // configured defaults.  Reading it first would frame the band wherever
        // the row happened to be left on the way in — which was harmless only
        // while the centre was a constant.  `reframe` touches the viewport
        // alone, so it needs no further sync.
        if let (Some(center), Some(span)) = (
            factory.nominal_center_hz(&self.settings),
            factory.preferred_span_hz(&self.settings),
        ) {
            self.freq_view.reframe(center, span);
        }
        // Whatever the reframe (or the absence of one) settled on is what the
        // `Zoom` row must now show.  Precedence step 2: a source's stated span
        // wins on switch *to* it.
        self.settings.set_zoom_ratio(self.freq_view.zoom_ratio());
        // Text mode is only valid for CW/PSK31/FT8; clamp if we switched away.
        let has_text = matches!(mode, SourceMode::Cw | SourceMode::Psk31 | SourceMode::Ft8);
        if !has_text && self.decode_bar == DecodeBarMode::Text {
            self.decode_bar = DecodeBarMode::Info;
        }
    }

    /// Close every overlay but `keep`.  The three are mutually exclusive: only
    /// one can be up at a time, so opening one dismisses the others rather than
    /// stacking them.
    fn close_overlays_except(&mut self, keep: Overlay) {
        self.show_help = keep == Overlay::Help;
        self.show_instrument = keep == Overlay::Instrument;
        self.settings.visible = keep == Overlay::Settings;
    }

    /// Process this pass's keyboard input.  Called from
    /// [`draw`](Self::draw) in the live app, and directly by a harness that
    /// does not draw.
    pub fn handle_keys(&mut self, ctx: &egui::Context) {
        // Settings popover consumes arrow/tab/escape/R keys when visible.
        if self.settings.visible {
            let result = self.settings.handle_keys(ctx);
            if result.source_switched {
                let idx = self
                    .settings
                    .source_mode_idx()
                    .min(SourceMode::ALL.len() - 1);
                let new_mode = SourceMode::ALL[idx];
                if new_mode != self.source_mode {
                    self.switch_source(new_mode);
                }
            }
            if result.am_audio_changed {
                self.reload_builtin_audio();
            }
            if result.wav_load_requested && self.try_load_wav() {
                self.settings.defocus();
            }
            if result.cw_msg_accepted {
                self.apply_cw_message();
            }
            if result.psk31_msg_accepted {
                self.apply_psk31_message();
            }
            if result.ft8_text_accepted {
                self.apply_ft8_free_text();
            }
            self.sync_settings();
            // The `Zoom` row is a live control, so push it into the viewport.
            // Unconditional is safe: the panel consumes the arrow keys while it
            // is open, so the keyboard zoom cannot have moved in the same frame
            // — the two directions of this sync are mutually exclusive by
            // construction rather than by a dirty flag.
            self.freq_view.set_zoom_ratio(self.settings.zoom_ratio());
            // Let global keys (Q, I, M, N) and the other overlay toggles (H, X)
            // work even while settings is open, but not when a text field is
            // actively consuming input.
            //
            // The overlay toggles have to be repeated here because this branch
            // returns before the main key handler runs.  Without them the
            // relationship is asymmetric in a way that reads as a bug: from the
            // instrument panel, `S` swaps to settings, but from settings `X`
            // did nothing at all.
            if !result.text_editing {
                let mut quit = false;
                let mut toggle_source = false;
                let mut cycle_mode = false;
                let mut cycle_audio = false;
                let mut show_instrument = false;
                let mut show_help = false;
                ctx.input(|i| {
                    if i.key_pressed(egui::Key::X) {
                        show_instrument = true;
                    }
                    if i.key_pressed(egui::Key::H) {
                        show_help = true;
                    }
                    if i.key_pressed(egui::Key::Q) {
                        quit = true;
                    }
                    if i.key_pressed(egui::Key::I) {
                        toggle_source = true;
                    }
                    if i.key_pressed(egui::Key::M) {
                        cycle_mode = true;
                    }
                    if i.key_pressed(egui::Key::N) {
                        cycle_audio = true;
                    }
                });
                // Swapping overlays, so the same exclusion the main handler
                // applies: showing one closes the others, settings included.
                if show_instrument {
                    self.show_instrument = true;
                    self.close_overlays_except(Overlay::Instrument);
                    return;
                }
                if show_help {
                    self.show_help = true;
                    self.close_overlays_except(Overlay::Help);
                    return;
                }
                if quit {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                if toggle_source {
                    self.switch_source(self.source_mode.next());
                    self.lock_source_to_center();
                }
                if cycle_mode {
                    self.cycle_source_mode();
                }
                if cycle_audio {
                    self.cycle_source_audio();
                }
            }
            return;
        }

        let mut quit = false;
        let mut toggle_source = false;
        let mut cycle_mode = false;
        let mut cycle_audio = false;
        let mut toggle_lock = false;
        // Frequency pan/zoom deltas to apply after the closure.
        let mut pan_delta: f32 = 0.0;
        let mut zoom_delta: f32 = 0.0; // added to zoom ratio; +0.5 coarse, +0.1 fine
        let mut freq_reset = false;
        let mut center_reset = false; // Z: recenter viewport to mid-band
        let mut db_shift: f32 = 0.0;
        // Marker actions
        let mut place_marker_a = false;
        let mut place_marker_b = false;
        let mut toggle_marker_a = false;
        let mut toggle_marker_b = false;
        let mut cycle_active_marker = false;
        let mut marker_delta: f32 = 0.0;

        ctx.input(|i| {
            if i.key_pressed(egui::Key::Num1) {
                self.pane_visible[0] ^= true;
            }
            if i.key_pressed(egui::Key::Num2) {
                self.pane_visible[1] ^= true;
            }
            if i.key_pressed(egui::Key::Num3) {
                self.pane_visible[2] ^= true;
            }
            if i.key_pressed(egui::Key::I) {
                toggle_source = true;
            }
            if i.key_pressed(egui::Key::C) {
                // Toggle cycling on the persistent generator AND the active
                // source's generator, keeping them in sync.  Don't call
                // reset_playback here — that would reconstruct the active
                // source's TestSignalGen and discard the cycling toggle we
                // just set.  Resetting the loop timer is enough to restart
                // the sig/gap accounting cleanly.
                let now_cycling = !self.signal_gen.cycling;
                if now_cycling {
                    self.signal_gen.start_cycling();
                } else {
                    self.signal_gen.stop_cycling();
                }
                if let Some(tts) = self.source.as_any_mut().downcast_mut::<TestToneSource>() {
                    if now_cycling {
                        tts.signal_gen.start_cycling();
                    } else {
                        tts.signal_gen.stop_cycling();
                    }
                }
                self.loop_timer.reset();
            }
            if i.key_pressed(egui::Key::D) {
                let has_text = matches!(
                    self.source_mode,
                    SourceMode::Cw | SourceMode::Psk31 | SourceMode::Ft8
                );
                self.decode_bar = self.decode_bar.next(has_text);
            }
            if i.key_pressed(egui::Key::E) {
                self.envelope_visible ^= true;
            }
            if i.key_pressed(egui::Key::L) {
                toggle_lock = true;
            }
            if i.key_pressed(egui::Key::M) {
                cycle_mode = true;
            }
            if i.key_pressed(egui::Key::N) {
                cycle_audio = true;
            }
            if i.key_pressed(egui::Key::P) {
                self.peak_hold_visible ^= true;
            }
            if i.key_pressed(egui::Key::S) {
                self.settings.visible ^= true;
                if self.settings.visible {
                    self.close_overlays_except(Overlay::Settings);
                }
            }
            if i.key_pressed(egui::Key::W) {
                self.waterfall_mode = self.waterfall_mode.next();
            }
            if i.key_pressed(egui::Key::H) {
                self.show_help ^= true;
                if self.show_help {
                    self.close_overlays_except(Overlay::Help);
                }
            }
            if i.key_pressed(egui::Key::X) {
                self.show_instrument ^= true;
                if self.show_instrument {
                    self.close_overlays_except(Overlay::Instrument);
                }
            }
            for e in &i.events {
                if let egui::Event::Text(s) = e {
                    match s.as_str() {
                        "?" => {
                            self.show_help ^= true;
                            if self.show_help {
                                self.close_overlays_except(Overlay::Help);
                            }
                        }
                        // Shift+A / Shift+B: snap marker to center and make it active
                        "A" => place_marker_a = true,
                        "B" => place_marker_b = true,
                        // a / b: toggle visibility and select/deselect as active marker
                        "a" => toggle_marker_a = true,
                        "b" => toggle_marker_b = true,
                        _ => {}
                    }
                }
            }
            if i.key_pressed(egui::Key::Tab) {
                cycle_active_marker = true;
            }

            // ── Active marker movement ───────────────────────────────────────
            // Ctrl+←/→: coarse (1/8 span).
            // Alt+←/→ (Option on Mac): very fine — one FFT bin width.
            // (Ctrl+Shift+←/→ is reserved for extra-fine pan.)
            let bin_hz = self.freq_view.nyquist / (FFT_SIZE / 2) as f32;
            if i.modifiers.ctrl && !i.modifiers.shift {
                let step = self.freq_view.span_hz / 8.0;
                if i.key_down(egui::Key::ArrowLeft) {
                    marker_delta -= step;
                }
                if i.key_down(egui::Key::ArrowRight) {
                    marker_delta += step;
                }
            } else if i.modifiers.alt {
                // Use key_pressed (fires once per physical keypress, no auto-repeat)
                // so each press moves exactly one bin.
                if i.key_pressed(egui::Key::ArrowLeft) {
                    marker_delta -= bin_hz;
                }
                if i.key_pressed(egui::Key::ArrowRight) {
                    marker_delta += bin_hz;
                }
            }
            if i.key_pressed(egui::Key::R) && !self.settings.visible {
                self.reset_playback();
            }
            if i.key_pressed(egui::Key::Escape) {
                self.show_help = false;
                self.show_instrument = false;
                self.settings.visible = false;
            }
            if i.key_pressed(egui::Key::Q) {
                quit = true;
            }
            if i.key_pressed(egui::Key::Z) {
                center_reset = true;
            }

            // ── Frequency pan ────────────────────────────────────────────────
            // Left/Right:            coarse pan, span/12 per keypress
            // Shift+Left/Right:      fine pan, 10% of coarse
            // Ctrl+Shift+Left/Right: extra-fine pan, 1% of coarse
            //
            // Steps are span-relative so they scale across sources (a narrowband
            // 24 kHz span and a wideband ~1 MHz span both traverse in a similar
            // number of presses).  `key_pressed` (not `key_down`) makes each step
            // frame-rate independent — OS key-repeat continues it when held.
            // Alt+Left/Right (marker move) and Ctrl+Left/Right without Shift
            // (marker coarse move) are reserved — skip pan for those.
            let ctrl_only = i.modifiers.ctrl && !i.modifiers.shift;
            if !i.modifiers.alt && !ctrl_only {
                let left = i.key_pressed(egui::Key::ArrowLeft);
                let right = i.key_pressed(egui::Key::ArrowRight);
                if left || right {
                    // Zoom in from full span first so panning has room at all.
                    // How far is a trade against how much of the screen the
                    // signal fills — see `PAN_AUTO_ZOOM`.
                    self.freq_view.ensure_pannable();
                    let coarse = self.freq_view.span_hz / 12.0;
                    let step = if i.modifiers.ctrl && i.modifiers.shift {
                        coarse * 0.01 // extra-fine
                    } else if i.modifiers.shift {
                        coarse * 0.1 // fine
                    } else {
                        coarse
                    };
                    if left {
                        pan_delta -= step;
                    }
                    if right {
                        pan_delta += step;
                    }
                }
            }

            // ── Frequency zoom ───────────────────────────────────────────────
            // Up/Down: zoom ±0.5; Shift+Up/Down: fine zoom ±0.1.
            // [ / ]: shift dB reference ±5 dB.
            if i.key_pressed(egui::Key::ArrowUp) {
                if i.modifiers.shift {
                    zoom_delta += 0.1;
                } else {
                    zoom_delta += 0.5;
                }
            }
            if i.key_pressed(egui::Key::ArrowDown) {
                if i.modifiers.shift {
                    zoom_delta -= 0.1;
                } else {
                    zoom_delta -= 0.5;
                }
            }
            for e in &i.events {
                if let egui::Event::Text(s) = e {
                    match s.as_str() {
                        "[" => db_shift -= 5.0,
                        "]" => db_shift += 5.0,
                        _ => {}
                    }
                }
            }
            for e in &i.events {
                if let egui::Event::Text(s) = e
                    && (s == "R" || s == "r")
                {
                    freq_reset = true;
                }
            }
        });

        // Apply pan/zoom/span/reset
        if pan_delta != 0.0 {
            // "signal" pan mode moves the signal/center in the arrow's
            // direction; "spectrum" (default) scrolls the spectrum the other way.
            if self.settings.pan_signal_follows() {
                pan_delta = -pan_delta;
            }
            self.freq_view.pan(pan_delta);
        }
        if zoom_delta.abs() > 0.001 {
            self.freq_view.step_zoom(zoom_delta);
        }
        if center_reset {
            // Z: recenter the viewport to mid-band, keeping the current zoom.
            self.freq_view.center_hz = self.freq_view.nyquist / 2.0;
        }
        if freq_reset {
            self.freq_view.reset();
        }
        // Precedence step 3: the keyboard owns the viewport until the next
        // switch, and the `Zoom` row follows it so the panel is never stale.
        self.settings.set_zoom_ratio(self.freq_view.zoom_ratio());

        if toggle_lock {
            self.source_locked ^= true;
        }

        // Update primary marker to track center
        self.markers[0].hz = self.freq_view.center_hz;

        // If source is locked to marker, sync freq/carrier to center_hz
        self.lock_source_to_center();

        // Shift+A/B: snap to center, enable, make active
        if place_marker_a {
            self.markers[1].hz = self.freq_view.center_hz;
            self.markers[1].enabled = true;
            self.active_marker = Some(1);
        }
        if place_marker_b {
            self.markers[2].hz = self.freq_view.center_hz;
            self.markers[2].enabled = true;
            self.active_marker = Some(2);
        }
        // a/b: toggle visibility; if enabling, make active; if disabling, deselect
        if toggle_marker_a {
            self.markers[1].enabled ^= true;
            self.active_marker = if self.markers[1].enabled {
                Some(1)
            } else {
                None
            };
        }
        if toggle_marker_b {
            self.markers[2].enabled ^= true;
            self.active_marker = if self.markers[2].enabled {
                Some(2)
            } else {
                None
            };
        }
        // Tab: cycle active marker  None → A → B → None (skipping disabled markers)
        if cycle_active_marker {
            self.active_marker = match self.active_marker {
                None => {
                    if self.markers[1].enabled {
                        Some(1)
                    } else if self.markers[2].enabled {
                        Some(2)
                    } else {
                        None
                    }
                }
                Some(1) => {
                    if self.markers[2].enabled {
                        Some(2)
                    } else {
                        None
                    }
                }
                Some(_) => None,
            };
        }
        // Ctrl+arrow: move the active marker
        if marker_delta != 0.0
            && let Some(idx) = self.active_marker
        {
            let nyquist = self.freq_view.nyquist;
            self.markers[idx].hz = (self.markers[idx].hz + marker_delta).clamp(0.0, nyquist);
        }

        if db_shift != 0.0 {
            self.db_min += db_shift;
            self.db_max += db_shift;
            self.waterfall.db_min = self.db_min;
            self.waterfall.db_max = self.db_max;
            self.settings.set_db_min(self.db_min);
            self.settings.set_db_max(self.db_max);
        }

        if toggle_source {
            self.switch_source(self.source_mode.next());
            self.lock_source_to_center();
        }
        if cycle_mode {
            self.cycle_source_mode();
        }
        if cycle_audio {
            self.cycle_source_audio();
        }
        if quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// `M` — cycle the active source's *mode*, where it has one: a modulation
    /// or protocol variant, not a parameter.  PSK31 (BPSK31/QPSK31) and FT8
    /// (FT8/FT4) have one; the other four do not, and the key is inert there.
    ///
    /// **COFDM's occupied bandwidth is deliberately not on this key.**  It was
    /// briefly, and it is the wrong axis: a 7-way occupancy parameter with its
    /// own settings row and its own HUD field, rather than a variant of the
    /// waveform.  The name matters more than usual here because DVB-T already
    /// uses "mode" for something specific — the 2K/8K FFT size — with bandwidth
    /// as a separate axis, and a narrowband DVB-T profile is the next queued
    /// source.  Binding `M` to bandwidth now would leave its real mode knob
    /// nowhere to go.
    ///
    /// **One implementation, called from both key paths.**  `handle_keys` has
    /// two: the settings overlay consumes most input and returns early, so it
    /// repeats the global keys itself.  These were duplicated matches, and they
    /// drifted — a COFDM arm reached the settings-open copy alone, so `M`
    /// cycled the bandwidth while the popover was up and did nothing with it
    /// closed.  The match is exhaustive rather than ending in `_ => {}` so a new
    /// source has to state its answer instead of inheriting silence.
    fn cycle_source_mode(&mut self) {
        match self.source_mode {
            SourceMode::Psk31 => {
                self.settings.cycle_psk31_mode();
                self.restart_source();
            }
            SourceMode::Ft8 => self.cycle_ft8_mode(),
            SourceMode::TestTone | SourceMode::Cw | SourceMode::AmDsb | SourceMode::Cofdm => {}
        }
    }

    /// `N` — cycle the active source's audio or message selection.
    ///
    /// Shared between both key paths for the same reason as
    /// [`cycle_source_mode`](Self::cycle_source_mode).
    fn cycle_source_audio(&mut self) {
        match self.source_mode {
            SourceMode::Cw => {
                self.settings.cycle_cw_msg_mode();
                self.apply_cw_message();
            }
            SourceMode::AmDsb => {
                self.settings.cycle_am_audio();
                self.reload_builtin_audio();
            }
            SourceMode::Psk31 => {
                self.settings.cycle_psk31_msg_mode();
                self.apply_psk31_message();
            }
            SourceMode::Ft8 => self.cycle_ft8_msg_type(),
            // Test Tone and COFDM carry no audio or message.
            SourceMode::TestTone | SourceMode::Cofdm => {}
        }
    }

    // ── Per-frame work ────────────────────────────────────────────────────────

    /// Per-frame state work: feed samples, run the spectrum/decode pipeline,
    /// and refresh GPU textures.  Runs before every `draw` and also when the
    /// window is hidden (so decode keeps flowing).  Splitting this out from
    /// drawing is the eframe 0.34+ idiom; `ctx` is available directly here, so
    /// texture uploads need no clone.
    ///
    /// **`dt` is supplied, not read from the clock.**  Everything downstream —
    /// `advance_time`, the `dt * fs` sample budget, the waterfall scroll pacing,
    /// the decode ticker — is already a pure function of it, so moving the one
    /// `Instant::now()` out to the eframe adapter is what makes a run
    /// reproducible: the same script and config must produce the same samples.
    pub fn advance(&mut self, ctx: &egui::Context, dt: f32) {
        // Advance the source's wall-clock timeline before pulling samples, so
        // time-based playback (e.g. COFDM signal/gap phases) is frame-rate
        // independent.  No-op for sources that don't use it.
        self.source.advance_time(dt);
        // A scripted clock advances on the same `dt`, so a timestamp stamped
        // into decoded text is a function of scripted time alone.
        self.clock.advance(dt);

        // Pace sample consumption to wall-clock: pull `dt * fs` samples this
        // frame (clamped) rather than a fixed count.  This makes every source's
        // seconds-based timing (gaps, Test Tone ramp/pause, …) run at true
        // wall-clock instead of scaling with the frame rate.  The clamp keeps
        // the FFT fresh at high frame rates and bounds a large `dt` (post-stall,
        // or a high-`fs` source like COFDM at 1.92 MHz).
        let budget = (dt * self.source.sample_rate()) as usize;
        let n = budget.clamp(MIN_SAMPLES_PER_FRAME, MAX_SAMPLES_PER_FRAME);

        // Feed new samples and process spectrum before drawing.
        let samples = self.source.next_samples(n);
        self.samples_consumed = self.samples_consumed.wrapping_add(samples.len() as u64);
        // Both representations of this block travel together — see
        // `DecodeChunk`. `last_samples_iq` returns the complex counterpart of
        // the block just emitted, so the decoder and the display cannot end up
        // looking at different samples.
        self.decode_seq = self.decode_seq.wrapping_add(1);
        let _ = self.decode_tx.try_send(DecodeChunk {
            seq: self.decode_seq,
            real: samples.clone(),
            iq: self.source.last_samples_iq().map(<[_]>::to_vec),
            signal: self.source.signal_phase(),
        });
        // Decode it now, if this is a replay run: the chunk goes in and its
        // results come back out before anything else touches the frame, so
        // ordering is exact and nothing can be dropped in between.
        self.pump_inline_decode();
        for s in &samples {
            self.ring_buf.push(*s);
        }
        self.spectrum.process(&self.ring_buf);

        // Main-thread gap detection: compute block RMS and signal gap
        // immediately, bypassing any decode-thread latency.  This ensures
        // the ticker clears to "waiting for signal" synchronously with the
        // audio loop, even if the decode thread is mid-window.
        let block_rms = {
            let sq_sum: f32 = samples.iter().map(|v| v * v).sum();
            (sq_sum / samples.len() as f32).sqrt()
        };
        // Prefer the source's own answer; fall back to the RMS heuristic for
        // sources that do not know (and for anything over the air).
        let block_is_signal = self
            .source
            .signal_phase()
            .unwrap_or(block_rms >= self.signal_threshold());
        self.loop_timer.tick_active(block_is_signal, dt);

        // Track signal onset for timestamp capture.
        let is_ft8_mode = self.source_mode == SourceMode::Ft8;
        let is_cw_mode = self.source_mode == SourceMode::Cw;
        let is_psk31_mode = self.source_mode == SourceMode::Psk31;
        // CW and PSK31 both decode incrementally — frame each burst with
        // matching open/close delimiters so the Dt ticker shows
        // "|| HH:MM:SS.mmm | <text> ||" per burst, mirroring FT8.
        let is_burst_text_mode = is_cw_mode || is_psk31_mode;
        if is_ft8_mode {
            let was_signal = self.last_block_was_signal;
            if block_is_signal && !was_signal {
                // Rising edge: capture onset time for timestamp.
                self.ft8_view.on_signal_rising_edge(self.clock.now());
            }
        }
        if is_burst_text_mode && self.loop_timer.signal_onset {
            let delim = super::source::format_burst_open_delimiter(
                self.clock.now(),
                self.time_zone_offset_min,
            );
            self.push_decode_result(DecodeResult::Text(delim));
        }
        self.last_block_was_signal = block_is_signal;

        // Drain decode results first so Info/Text from the decode thread are
        // processed before any gap state change.
        while let Ok(result) = self.decode_rx.try_recv() {
            if let DecodeResult::Gap { decoded } = result {
                // For FT8/FT4: update frm/err counters; capture timestamp on success.
                if is_ft8_mode {
                    if decoded {
                        self.ft8_view.on_decoded_frame(self.time_zone_offset_min);
                    } else {
                        self.ft8_view.on_failed_frame();
                    }
                }
                self.push_decode_result(DecodeResult::Gap { decoded });
            } else if is_ft8_mode {
                // For FT8/FT4: wrap the decoded frame text as
                // "|| HH:MM:SS.fff | <text> ||" so the leading/trailing "||"
                // clearly demarcate the frame boundaries in the Dt ticker.
                // The onset timestamp is still in ft8_view.pending_onset at Text
                // time (it's taken when the Gap{decoded:true} arrives just after).
                let result = if let DecodeResult::Text(ref s) = result {
                    DecodeResult::Text(
                        self.ft8_view
                            .format_decoded_text(s, self.time_zone_offset_min),
                    )
                } else {
                    result
                };
                self.push_decode_result(result);
            } else {
                self.push_decode_result(result);
            }
        }

        // CW / PSK31 closing delimiter: inject after draining all decode
        // results so the last characters appear before the "||" separator.
        if is_burst_text_mode && self.loop_timer.gap_onset {
            self.push_decode_result(DecodeResult::Text(
                super::source::BURST_CLOSE_DELIMITER.to_owned(),
            ));
        }

        if !self.loop_timer.in_signal && self.decode_bar.is_visible() {
            // Push Gap when the loop timer considers us in a real gap (after
            // any holdoff has expired).  This avoids flooding the ticker with
            // spurious Gap events during CW keying gaps.  Gap clears last_info
            // (so Di shows "waiting for signal") and sets in_gap=true (so Dt
            // injects spaces at the scroll rate).
            self.push_decode_result(DecodeResult::Gap { decoded: false });
        }
        self.decode_ticker.tick(dt);

        // Update peak hold (decay slowly: 0.2 dB/frame, then latch new peaks).
        for (ph, &db) in self
            .peak_hold
            .iter_mut()
            .zip(self.spectrum.fft_out_db.iter())
        {
            *ph = (*ph - 0.2_f32).max(db);
        }

        // Per-frame data advance.  The waterfall paces its own scroll by
        // wall-clock `dt` and uploads only changed rows (ring buffer +
        // set_partial), so it runs every frame.
        self.persistence
            .map
            .accumulate(&self.spectrum.fft_out_db, self.db_min, self.db_max);
        self.persistence.map.decay();
        self.waterfall.push_row(&self.spectrum.fft_out_db, dt);
        self.waterfall.update_texture(ctx);

        // Spectrogram: keep db/time-range/color ramp in sync with the
        // user's current display choices, then push one FFT slice.  A
        // column is committed internally only once enough wall-clock
        // time has elapsed (secs_per_col), which drives the
        // time-dilation factor.
        self.spectrogram.db_min = self.db_min;
        self.spectrogram.db_max = self.db_max;
        self.spectrogram
            .set_time_range(self.settings.spec_time_range_secs());
        // Frequency window = ± half the current viewport span (same extent as
        // the spectrum/waterfall panes), so ↑/↓ zoom scales the spectrogram.
        let spec_center = self.markers[0].hz;
        let spec_delta = self.freq_view.visible_span() / 2.0;
        self.spectrogram.push_spectrum(
            &self.spectrum.fft_out_db,
            dt,
            spec_center,
            spec_delta,
            self.freq_view.nyquist,
        );
        if self.waterfall_mode == WaterfallMode::Horizontal {
            self.spectrogram.update_texture(ctx);
        }

        // Persistence is a 2D histogram that changes everywhere each frame, so
        // it re-uploads the whole (small, 513×100) texture.  Measured flat at
        // ~5.75 ms/frame, which sustains a high frame rate without throttling.
        //
        // **Gating this out of a replay run was tried and reverted.**  The
        // 5.75 ms is the cost with a live GPU; headless there is no device, so
        // `update_texture` only stages a `ColorImage` into egui's texture
        // manager for `end_pass` to discard.  Measured over a 7200-frame run:
        // 36.59 s with the uploads against 36.43 s without — noise.  Not worth a
        // second code path through the per-frame work.
        self.persistence.update_texture(ctx);

        // Drive the loop at display rate regardless of interaction.
        ctx.request_repaint();
    }

    /// Per-frame drawing: keyboard handling plus the HUD, optional decode bar,
    /// and central pane stack.  Panels attach to the passed `ui` via
    /// `Panel::show(ui, ..)` (the eframe 0.34+ replacement for opening panels
    /// on the context directly).
    ///
    /// Note that key handling runs from here, not from [`advance`](Self::advance)
    /// — a harness that only advances will process samples and never see a
    /// keystroke.  Moving it would change its ordering against the settings
    /// overlay, so the harness calls [`handle_keys`](Self::handle_keys) itself.
    pub fn draw(&mut self, ui: &mut egui::Ui) {
        self.handle_keys(ui.ctx());
        self.draw_hud(ui);
        if self.decode_bar.is_visible() {
            egui::Panel::bottom("decode_bar")
                .exact_size(DECODE_BAR_H)
                .show(ui, |ui| {
                    let rect = ui.available_rect_before_wrap();
                    self.draw_decode_bar(ui.painter_at(rect), rect);
                });
        }
        egui::CentralPanel::default().show(ui, |ui| {
            self.draw_panes(ui);
            if self.show_help {
                self.draw_help_overlay(ui);
            }
            if self.show_instrument {
                self.draw_instrument_overlay(ui);
            }
            let mono = self.mono_font_id.clone();
            self.settings.draw(ui, &mono);
        });
    }

    // ── Read accessors ────────────────────────────────────────────────────────
    //
    // The state a test needs to assert on, exposed for reading only.  Kept to
    // accessors rather than public fields so the app keeps sole ownership of
    // its invariants — every defect these exist to catch is a *write* ordering
    // problem, and a test that could write would not reproduce one.

    /// Which source is active.
    pub fn source_mode(&self) -> SourceMode {
        self.source_mode
    }

    /// The settings rows, including the per-source containers reachable through
    /// the `<S>Settings` typed-accessor traits.
    pub fn settings(&self) -> &SettingsState {
        &self.settings
    }

    /// The shared pan/zoom viewport.
    pub fn freq_view(&self) -> &FreqView {
        &self.freq_view
    }

    /// True while the source tracks the viewport centre (the `L` key).
    pub fn source_locked(&self) -> bool {
        self.source_locked
    }

    /// The live source's sample rate, which varies by source and — for COFDM —
    /// by config.
    pub fn source_sample_rate(&self) -> f32 {
        self.source.sample_rate()
    }

    /// Pane 3's vertical waterfall, for the CPU-side pixel assertions.
    pub fn waterfall(&self) -> &WaterfallDisplay {
        &self.waterfall
    }

    /// Pane 3's horizontal spectrogram, for the CPU-side pixel assertions.
    pub fn spectrogram(&self) -> &SpectrogramDisplay {
        &self.spectrogram
    }

    /// The decode ticker — the Di/Dt bar's text, last `Info` and last
    /// instrument reading.  This is what the replay driver dumps, so the dump
    /// and the panel are reading the same values by construction.
    pub fn decode_ticker(&self) -> &DecodeTicker {
        &self.decode_ticker
    }

    /// Chunks the inline decoder's sequence counter says never arrived.
    ///
    /// Zero is the invariant of a replay run and the check that says a dump
    /// measured the link rather than the harness.  `None` when decoding on a
    /// worker thread, where dropping under pressure is the intended behaviour
    /// and the count would only invite a meaningless assertion.
    pub fn dropped_chunks(&self) -> Option<u64> {
        self.inline_decode.as_ref().map(|d| d.state.dropped())
    }

    /// The clock timestamps are stamped from.
    pub fn clock(&self) -> Clock {
        self.clock
    }

    /// Samples the source has produced since startup.  See
    /// [`samples_consumed`](Self::samples_consumed).
    pub fn samples_consumed(&self) -> u64 {
        self.samples_consumed
    }

    /// Take this frame's decode results, in the order the ticker saw them.
    ///
    /// Empty unless the app was built by [`new_replay`](Self::new_replay) —
    /// there is nothing to tap on the threaded path, where the results have
    /// already been consumed by the time anything could ask.
    pub fn take_replay_results(&mut self) -> Vec<DecodeResult> {
        match self.inline_decode.as_mut() {
            Some(inline) => std::mem::take(&mut inline.tapped),
            None => Vec::new(),
        }
    }

    /// Hand a result to the ticker, tapping it for the replay dump on the way.
    ///
    /// **Every path to the ticker goes through here, and that is the point.**
    /// Tapping the decode channel instead would miss the three results the app
    /// itself synthesizes — CW and PSK31's burst-open and burst-close
    /// delimiters, and the loop timer's own gap — and would record FT8's frame
    /// text unformatted, since the timestamp wrapping happens on this side.  A
    /// dump that agrees with the panel has to read what the panel reads.
    fn push_decode_result(&mut self, result: DecodeResult) {
        if let Some(inline) = self.inline_decode.as_mut() {
            inline.tapped.push(result.clone());
        }
        self.decode_ticker.push_result(result);
    }

    /// Run the inline decoder over every chunk waiting for it.
    ///
    /// A no-op unless the app was built by [`new_replay`](Self::new_replay).
    /// The borrow dance is deliberate: `process` takes `&mut self.state` while
    /// the config lives behind the app's `Arc<Mutex<..>>`, so the config is
    /// snapshotted first — the same order the worker thread uses, which is what
    /// keeps a mid-script settings change behaving identically on both paths.
    fn pump_inline_decode(&mut self) {
        let Some(inline) = self.inline_decode.as_mut() else {
            return;
        };
        let cfg = self.decode_config.lock().unwrap().clone();
        while let Ok(chunk) = inline.chunks.try_recv() {
            inline.state.process(&chunk, &cfg, &inline.results);
        }
    }
}

// ── eframe::App ───────────────────────────────────────────────────────────────

/// The whole of the app's eframe coupling: two methods that read the clock and
/// discard the `Frame` both of them are handed.
///
/// `eframe::Frame` has `pub(crate)` fields and so cannot be constructed outside
/// eframe; while these were the *only* entry points, that alone put `ViewApp`
/// out of reach of `tests/`.  Both parameters were already unused, so the
/// inherent methods above simply do not take them.
impl eframe::App for ViewApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Wall-clock delta since last frame.  The single call that made the app
        // non-deterministic, now confined to the adapter.
        let now = std::time::Instant::now();
        let dt = now.duration_since(self.last_frame_time).as_secs_f32();
        self.last_frame_time = now;
        self.advance(ctx, dt);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }
}
