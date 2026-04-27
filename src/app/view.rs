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
use crate::decode::{DecodeConfig, DecodeResult, DecodeTicker, DecodeWorker, SIGNAL_THRESHOLD};
use crate::source::SignalSource;
use crate::source::tone::TestSignalGen;
use crate::source::tone::TestToneSource;
use crate::utils::timer::LoopTimer;

use super::{
    DECODE_BAR_H, DecodeBarMode, FFT_SIZE, SAMPLE_RATE, SAMPLES_PER_FRAME, SourceMode,
    WaterfallMode,
};

// ── ViewApp ───────────────────────────────────────────────────────────────────

pub(crate) struct ViewApp {
    pub(super) pane_visible: [bool; 3],
    // Fractional height per pane — stored even when hidden so proportions are
    // remembered when re-shown. Future resize handles will mutate these values.
    pub(super) pane_frac: [f32; 3],
    pub(super) show_help: bool,
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
    pub(super) decode_tx: mpsc::SyncSender<Vec<f32>>,
    pub(super) decode_rx: mpsc::Receiver<DecodeResult>,
    pub(super) decode_ticker: DecodeTicker,
    /// True if the previous frame's sample block was above SIGNAL_THRESHOLD.
    pub(super) last_block_was_signal: bool,
    /// Wall-clock time of the previous frame, for real-time dt calculation.
    pub(super) last_frame_time: std::time::Instant,

    /// Per-frame view-side state for FT8/FT4 (frame counts, pending onset,
    /// cached mode/msg_type).  See [`Ft8ViewState`].
    pub(super) ft8_view: crate::source::ft8::Ft8ViewState,
    /// Display timestamps offset from UTC by this many minutes (0 = UTC).
    pub(super) time_zone_offset_min: i32,
}

impl ViewApp {
    pub(crate) fn new(cc: &eframe::CreationContext<'_>, cfg: ViewConfig) -> Self {
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
        cc.egui_ctx.set_fonts(fonts);

        // Coherence clamp: db_min must be strictly less than db_max
        let db_max = cfg.db_max();
        let db_min = cfg.db_min().min(db_max - 1.0);

        let signal_gen = TestSignalGen::new(cfg.freq_hz(), SAMPLE_RATE);
        let source: Box<dyn SignalSource> = Box::new(TestToneSource::new(TestSignalGen::new(
            cfg.freq_hz(),
            SAMPLE_RATE,
        )));

        // Decode thread setup.
        let decode_config = Arc::new(Mutex::new(DecodeConfig::new(SAMPLE_RATE)));
        // Capacity 256: at 60 fps each block is ~16 ms; 256 slots ≈ 4 s of buffer,
        // enough to absorb a slow psk31_sync pass without dropping gap blocks.
        let (decode_tx, sample_rx) = mpsc::sync_channel::<Vec<f32>>(256);
        let (result_tx, decode_rx) = mpsc::sync_channel::<DecodeResult>(16);
        {
            let worker_cfg = Arc::clone(&decode_config);
            std::thread::spawn(move || DecodeWorker::new(worker_cfg, sample_rx, result_tx).run());
        }

        let mut app = Self {
            pane_visible: [true; 3],
            pane_frac: [1.0 / 3.0; 3],
            show_help: false,
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
                let mut s = SpectrogramDisplay::new(256, 512, db_min, db_max);
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
            decode_ticker: DecodeTicker::new(),
            last_block_was_signal: false,
            last_frame_time: std::time::Instant::now(),

            ft8_view: crate::source::ft8::Ft8ViewState::new(),
            time_zone_offset_min: 0,
        };
        app.time_zone_offset_min = cfg.time_zone_offset_min();
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

    /// Full reset: restart source, reset timers, flush decode pipeline.
    /// Call on R key, mode/message/audio cycle — anything that changes the signal.
    pub(super) fn reset_playback(&mut self) {
        self.settings.reset_source_rows();
        self.sync_settings();
        if self.source_mode == SourceMode::TestTone {
            self.signal_gen = TestSignalGen::new(self.settings.freq_hz(), SAMPLE_RATE);
        }
        self.source = self.make_source();
        self.loop_timer.reset();
        self.loop_timer.set_holdoff(self.loop_timer_holdoff_secs());
        self.decode_ticker.reset();
        self.last_block_was_signal = false;
        self.spectrogram.clear();
        self.ft8_view.reset();
        while self.decode_rx.try_recv().is_ok() {}
        let _ = self.decode_tx.try_send(Vec::new());
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
        self.sync_decode_config();
        self.reset_playback();
        // Text mode is only valid for CW/PSK31/FT8; clamp if we switched away.
        let has_text = matches!(mode, SourceMode::Cw | SourceMode::Psk31 | SourceMode::Ft8);
        if !has_text && self.decode_bar == DecodeBarMode::Text {
            self.decode_bar = DecodeBarMode::Info;
        }
    }

    pub(super) fn handle_keys(&mut self, ctx: &egui::Context) {
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
            // Let global keys (Q, M, N) work even while settings is open,
            // but not when a text field is actively consuming input.
            if !result.text_editing {
                let mut quit = false;
                let mut toggle_source = false;
                let mut cycle_mode = false;
                let mut cycle_audio = false;
                ctx.input(|i| {
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
                if quit {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                if toggle_source {
                    self.switch_source(self.source_mode.next());
                    self.lock_source_to_center();
                }
                if cycle_mode {
                    match self.source_mode {
                        SourceMode::Psk31 => {
                            self.settings.cycle_psk31_mode();
                            self.sync_settings();
                            self.reset_playback();
                        }
                        SourceMode::Ft8 => {
                            self.cycle_ft8_mode();
                        }
                        _ => {}
                    }
                }
                if cycle_audio {
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
                        SourceMode::Ft8 => {
                            self.cycle_ft8_msg_type();
                        }
                        _ => {}
                    }
                }
            }
            return;
        }

        let mut quit = false;
        let mut toggle_source = false;
        let mut cycle_mode = false;
        let mut cycle_audio = false;
        let mut toggle_lock = false;
        // When non-zero, snap center_hz to this grid after applying pan_delta.
        let mut snap_pan_grid: f32 = 0.0;
        // Frequency pan/zoom deltas to apply after the closure.
        let mut pan_delta: f32 = 0.0;
        let mut zoom_delta: f32 = 0.0; // added to zoom ratio; +0.5 coarse, +0.1 fine
        let mut freq_reset = false;
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
            }
            if i.key_pressed(egui::Key::W) {
                self.waterfall_mode = self.waterfall_mode.next();
            }
            if i.key_pressed(egui::Key::H) {
                self.show_help ^= true;
            }
            for e in &i.events {
                if let egui::Event::Text(s) = e {
                    match s.as_str() {
                        "?" => self.show_help ^= true,
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
                self.settings.visible = false;
            }
            if i.key_pressed(egui::Key::Q) {
                quit = true;
            }

            // ── Frequency pan ────────────────────────────────────────────────
            // Left/Right:             coarse pan (span/8, auto-repeat)
            // Shift+Left/Right:       fine pan, snap to nearest 100 Hz (auto-repeat)
            // Ctrl+Shift+Left/Right:  extra-fine pan:
            //   key_pressed (first hit) → snap to nearest 10 Hz
            //   key_down (held)         → snap to nearest 100 Hz
            // Alt+Left/Right reserved for marker movement — skip pan when alt held.
            if !i.modifiers.alt {
                if i.modifiers.ctrl && i.modifiers.shift {
                    // Extra-fine pan: 10 Hz per keypress.
                    let left = i.key_pressed(egui::Key::ArrowLeft);
                    let right = i.key_pressed(egui::Key::ArrowRight);
                    let arrow = left || right;
                    if arrow && self.freq_view.span_hz >= self.freq_view.nyquist {
                        self.freq_view.step_zoom(0.1);
                    }
                    if left {
                        pan_delta -= 10.0;
                    }
                    if right {
                        pan_delta += 10.0;
                    }
                    if arrow {
                        snap_pan_grid = 10.0;
                    }
                } else if !i.modifiers.ctrl {
                    if i.modifiers.shift {
                        // Fine pan: snap to 100 Hz. Zoom in first if at full span.
                        let arrow = i.key_pressed(egui::Key::ArrowLeft)
                            || i.key_pressed(egui::Key::ArrowRight);
                        if arrow && self.freq_view.span_hz >= self.freq_view.nyquist {
                            self.freq_view.step_zoom(0.1);
                        }
                        if i.key_pressed(egui::Key::ArrowLeft) {
                            pan_delta -= 100.0;
                        }
                        if i.key_pressed(egui::Key::ArrowRight) {
                            pan_delta += 100.0;
                        }
                        if arrow {
                            snap_pan_grid = 100.0;
                        }
                    } else {
                        let arrow =
                            i.key_down(egui::Key::ArrowLeft) || i.key_down(egui::Key::ArrowRight);
                        if arrow && self.freq_view.span_hz >= self.freq_view.nyquist {
                            self.freq_view.step_zoom(0.1);
                        }
                        let pan_step = self.freq_view.span_hz / 8.0;
                        if i.key_down(egui::Key::ArrowLeft) {
                            pan_delta -= pan_step;
                        }
                        if i.key_down(egui::Key::ArrowRight) {
                            pan_delta += pan_step;
                        }
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
            self.freq_view.pan(pan_delta);
            if snap_pan_grid > 0.0 {
                self.freq_view.center_hz =
                    FreqView::snap_hz(self.freq_view.center_hz, snap_pan_grid);
            }
        }
        if zoom_delta.abs() > 0.001 {
            self.freq_view.step_zoom(zoom_delta);
        }
        if freq_reset {
            self.freq_view.reset();
        }

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
            match self.source_mode {
                SourceMode::Psk31 => {
                    self.settings.cycle_psk31_mode();
                    self.sync_settings();
                    self.reset_playback();
                }
                SourceMode::Ft8 => {
                    self.cycle_ft8_mode();
                }
                _ => {}
            }
        }
        if cycle_audio {
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
                SourceMode::Ft8 => {
                    self.cycle_ft8_msg_type();
                }
                _ => {}
            }
        }
        if quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

// ── eframe::App ───────────────────────────────────────────────────────────────

impl eframe::App for ViewApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Wall-clock delta since last frame.
        let now = std::time::Instant::now();
        let dt = now.duration_since(self.last_frame_time).as_secs_f32();
        self.last_frame_time = now;

        // Feed new samples and process spectrum before drawing.
        let samples = self.source.next_samples(SAMPLES_PER_FRAME);
        // Non-blocking send to decode thread; drop if channel is full.
        let _ = self.decode_tx.try_send(samples.clone());
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
        self.loop_timer.tick(block_rms, dt);

        let block_is_signal = block_rms >= SIGNAL_THRESHOLD;

        // Track signal onset for timestamp capture.
        let is_ft8_mode = self.source_mode == SourceMode::Ft8;
        let is_cw_mode = self.source_mode == SourceMode::Cw;
        if is_ft8_mode {
            let was_signal = self.last_block_was_signal;
            if block_is_signal && !was_signal {
                // Rising edge: capture onset time for timestamp.
                self.ft8_view.on_signal_rising_edge();
            }
        }
        // CW uses holdoff-aware onset from the loop timer.
        if is_cw_mode && self.loop_timer.signal_onset {
            let delim = super::source::cw::format_open_delimiter(
                std::time::SystemTime::now(),
                self.time_zone_offset_min,
            );
            self.decode_ticker.push_result(DecodeResult::Text(delim));
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
                self.decode_ticker
                    .push_result(DecodeResult::Gap { decoded });
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
                self.decode_ticker.push_result(result);
            } else {
                self.decode_ticker.push_result(result);
            }
        }

        // CW closing delimiter: inject after draining all decode results so
        // the last characters appear before the "||" separator.
        if is_cw_mode && self.loop_timer.gap_onset {
            self.decode_ticker
                .push_result(DecodeResult::Text(" ||".to_owned()));
        }

        if !self.loop_timer.in_signal && self.decode_bar.is_visible() {
            // Push Gap when the loop timer considers us in a real gap (after
            // any holdoff has expired).  This avoids flooding the ticker with
            // spurious Gap events during CW keying gaps.  Gap clears last_info
            // (so Di shows "waiting for signal") and sets in_gap=true (so Dt
            // injects spaces at the scroll rate).
            self.decode_ticker
                .push_result(DecodeResult::Gap { decoded: false });
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

        self.persistence
            .map
            .accumulate(&self.spectrum.fft_out_db, self.db_min, self.db_max);
        self.persistence.map.decay();
        self.persistence.update_texture(ctx);
        self.waterfall.push_row(&self.spectrum.fft_out_db);
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
        let spec_center = self.markers[0].hz;
        let spec_delta = self.settings.spec_freq_delta_hz();
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

        self.handle_keys(ctx);
        self.draw_hud(ctx);
        if self.decode_bar.is_visible() {
            egui::TopBottomPanel::bottom("decode_bar")
                .exact_height(DECODE_BAR_H)
                .show(ctx, |ui| {
                    let rect = ui.available_rect_before_wrap();
                    self.draw_decode_bar(ui.painter_at(rect), rect);
                });
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            self.draw_panes(ui);
            if self.show_help {
                self.draw_help_overlay(ui);
            }
            let mono = self.mono_font_id.clone();
            self.settings.draw(ui, &mono);
        });

        ctx.request_repaint();
    }
}
