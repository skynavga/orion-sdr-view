mod config;
mod decode;
mod freqview;
mod persistence;
mod settings;
mod signal;
mod source;
mod spectrum;
mod waterfall;

use std::sync::{Arc, Mutex};
use std::sync::mpsc;

use clap::Parser;
use config::ViewConfig;
use decode::{DecodeConfig, DecodeMode, DecodeResult, DecodeTicker, DecodeWorker, SIGNAL_THRESHOLD};
use eframe::egui;
use freqview::{FreqMarker, FreqView};
use persistence::PersistenceRenderer;
use settings::SettingsState;
use signal::TestSignalGen;
use source::{AmDsbSource, BuiltinAudio, Psk31Mode, Psk31Source, SignalSource, TestToneSource, load_builtin};
use spectrum::{RingBuffer, SpectrumProcessor};
use waterfall::WaterfallDisplay;

#[derive(Parser)]
#[command(name = "orion-sdr-view", about = "SDR spectrum viewer")]
struct Cli {
    /// Path to a YAML configuration file
    #[arg(long, value_name = "FILE")]
    config: Option<std::path::PathBuf>,
}

const PANE_BG: [egui::Color32; 3] = [
    egui::Color32::from_rgb(10, 10, 20),
    egui::Color32::from_rgb(20, 50, 40),
    egui::Color32::from_rgb(40, 30, 60),
];

const FFT_SIZE: usize = 1024;
const SAMPLE_RATE: f32 = 48_000.0;
// Number of new samples fed per frame, targeting ~60 fps.
const SAMPLES_PER_FRAME: usize = (SAMPLE_RATE / 60.0) as usize;
// Fixed pixel height of the decode bar (does not participate in pane proportions).
const DECODE_BAR_H: f32 = 28.0;

// ── Loop timer ────────────────────────────────────────────────────────────────

/// Tracks signal/gap phase timing and loop iteration count for the decode bar.
struct LoopTimer {
    in_signal:  bool,
    phase_secs: f32,
    loop_count: u32,
}

impl LoopTimer {
    fn new() -> Self { Self { in_signal: false, phase_secs: 0.0, loop_count: 0 } }

    fn reset(&mut self) { self.in_signal = false; self.phase_secs = 0.0; self.loop_count = 0; }

    /// Call once per frame with the measured block RMS and the frame duration.
    fn tick(&mut self, rms: f32, dt: f32) {
        let active = rms >= SIGNAL_THRESHOLD;
        if active != self.in_signal {
            // Transition: gap→signal increments loop count.
            if active { self.loop_count = (self.loop_count + 1) % 1000; }
            self.in_signal  = active;
            self.phase_secs = 0.0;
        } else {
            self.phase_secs += dt;
        }
    }

    /// Formatted string: "sig  12.34s loop 007" or "gap   2.00s loop 007".
    fn label(&self) -> String {
        let kind = if self.in_signal { "sig" } else { "gap" };
        format!("{kind} {:6.2}s loop {:03}", self.phase_secs, self.loop_count)
    }
}

// ── Decode bar mode ───────────────────────────────────────────────────────────

/// Three-state decode bar: off → info-only → text-only → off (cycles with D).
#[derive(Clone, Copy, PartialEq, Eq)]
enum DecodeBarMode {
    /// Bar hidden.
    Off,
    /// Bar visible; shows only signal info (modulation, freq, BW, SNR).
    Info,
    /// Bar visible; shows only decoded text ticker.
    Text,
}

impl DecodeBarMode {
    /// Cycle to the next mode.  `has_text` gates whether Text mode is reachable:
    /// non-text sources (Test Tone, AM DSB) skip straight from Info back to Off.
    fn next(self, has_text: bool) -> Self {
        match self {
            Self::Off  => Self::Info,
            Self::Info => if has_text { Self::Text } else { Self::Off },
            Self::Text => Self::Off,
        }
    }
    fn is_visible(self) -> bool { self != Self::Off }
}

// ── Source mode ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum SourceMode {
    TestTone,
    AmDsb,
    Psk31,
}

impl SourceMode {
    const ALL: &'static [SourceMode] = &[SourceMode::TestTone, SourceMode::AmDsb, SourceMode::Psk31];

    fn label(self) -> &'static str {
        match self {
            SourceMode::TestTone => "Test Tone",
            SourceMode::AmDsb => "AM DSB",
            SourceMode::Psk31 => "PSK31",
        }
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|&m| m == self).unwrap_or(0)
    }

    fn next(self) -> Self {
        let idx = (self.index() + 1) % Self::ALL.len();
        Self::ALL[idx]
    }
}

// ── ViewApp ───────────────────────────────────────────────────────────────────

struct ViewApp {
    pane_visible: [bool; 3],
    // Fractional height per pane — stored even when hidden so proportions are
    // remembered when re-shown. Future resize handles will mutate these values.
    pane_frac: [f32; 3],
    show_help: bool,
    mono_font_id: egui::FontId,

    // Active signal source (Box<dyn SignalSource> for easy future extension)
    source_mode: SourceMode,
    source: Box<dyn SignalSource>,

    // Test tone generator — kept alive so its state (cycling, settings) persists
    // across source switches. TestToneSource borrows it when active.
    signal_gen: TestSignalGen,

    ring_buf: RingBuffer,
    spectrum: SpectrumProcessor,
    db_min: f32,
    db_max: f32,
    /// Peak-hold line: per-bin max dB, decayed slowly.
    peak_hold: Vec<f32>,
    peak_hold_visible: bool,

    // Pane 2: persistence density
    persistence: PersistenceRenderer,
    envelope_visible: bool,

    // Pane 3: waterfall
    waterfall: WaterfallDisplay,

    // Frequency viewport (pan + zoom) — shared across all panes
    freq_view: FreqView,

    // Markers
    markers: [FreqMarker; 3],
    /// Which bracket marker is selected for keyboard positioning: Some(1)=A, Some(2)=B, None.
    active_marker: Option<usize>,

    // Settings popover
    settings: SettingsState,

    // When true, source freq/carrier tracks center_hz on every display change.
    source_locked: bool,

    // Decode bar (pane 3): cycled by D key (Off / Info-only / Text-only).
    decode_bar: DecodeBarMode,
    loop_timer: LoopTimer,

    // Decode thread channels and shared config.
    decode_config: Arc<Mutex<DecodeConfig>>,
    decode_tx:     mpsc::SyncSender<Vec<f32>>,
    decode_rx:     mpsc::Receiver<DecodeResult>,
    decode_ticker: DecodeTicker,
    /// True if the previous frame's sample block was above SIGNAL_THRESHOLD.
    last_block_was_signal: bool,
    /// Wall-clock time of the previous frame, for real-time dt calculation.
    last_frame_time: std::time::Instant,
}

impl ViewApp {
    fn new(cc: &eframe::CreationContext<'_>, cfg: ViewConfig) -> Self {
        let font_bytes = include_bytes!("../assets/fonts/DejaVuSansMono.ttf");
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
        let source: Box<dyn SignalSource> =
            Box::new(TestToneSource::new(TestSignalGen::new(cfg.freq_hz(), SAMPLE_RATE)));

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
        };
        app.sync_decode_config();
        app
    }

    /// Build a fresh AmDsbSource from current settings values.
    fn make_am_source(&self) -> AmDsbSource {
        let (audio, audio_rate) = if self.settings.am_audio_is_custom() {
            // Custom with no path yet — start silent; audio loaded on WAV entry
            (Vec::new(), SAMPLE_RATE)
        } else {
            let builtin = BuiltinAudio::ALL[self.settings.am_audio_idx().min(BuiltinAudio::ALL.len() - 1)];
            load_builtin(builtin)
        };
        AmDsbSource::new(
            audio,
            audio_rate,
            self.settings.am_carrier_hz(),
            self.settings.am_mod_index(),
            self.settings.am_loop_gap_secs(),
            self.settings.am_noise_amp(),
            self.settings.am_msg_repeat(),
            SAMPLE_RATE,
        )
    }

    /// When source_locked, write center_hz into the active source's freq/carrier
    /// setting rows and call sync_settings() to propagate immediately.
    fn lock_source_to_center(&mut self) {
        if !self.source_locked { return; }
        let hz = FreqView::snap_hz(self.freq_view.center_hz, 10.0);
        match self.source_mode {
            SourceMode::TestTone => self.settings.set_freq_hz(hz),
            SourceMode::AmDsb    => self.settings.set_am_carrier_hz(hz),
            SourceMode::Psk31    => self.settings.set_psk31_carrier_hz(hz),
        }
        self.sync_settings();
    }

    /// Build a fresh Psk31Source from current settings values.
    fn make_psk31_source(&self) -> Psk31Source {
        let mode = match self.settings.psk31_mode_str() {
            "QPSK31" => Psk31Mode::Qpsk31,
            _        => Psk31Mode::Bpsk31,
        };
        Psk31Source::new(
            self.settings.psk31_carrier_hz(),
            self.settings.psk31_loop_gap_secs(),
            self.settings.psk31_noise_amp(),
            mode,
            self.settings.psk31_message().to_owned(),
            self.settings.psk31_msg_repeat(),
            SAMPLE_RATE,
        )
    }

    /// Full reset: restart source, reset timers, flush decode pipeline.
    /// Call on R key, mode/message/audio cycle — anything that changes the signal.
    fn reset_playback(&mut self) {
        self.source.restart();
        self.loop_timer.reset();
        self.decode_ticker.reset();
        self.last_block_was_signal = false;
        while self.decode_rx.try_recv().is_ok() {}
        let _ = self.decode_tx.try_send(Vec::new());
    }

    /// Switch the active source to `mode`, constructing a new source box.
    fn switch_source(&mut self, mode: SourceMode) {
        self.source_mode = mode;
        self.source = match mode {
            SourceMode::TestTone => {
                // Re-create from signal_gen's current settings
                Box::new(TestToneSource::new(TestSignalGen::new(
                    self.signal_gen.freq_hz,
                    SAMPLE_RATE,
                )))
            }
            SourceMode::AmDsb  => Box::new(self.make_am_source()),
            SourceMode::Psk31  => Box::new(self.make_psk31_source()),
        };
        self.settings.set_source_mode(mode as usize);
        self.sync_decode_config();
        self.reset_playback();
        // Text mode is only valid for PSK31; clamp if we switched away.
        if mode != SourceMode::Psk31 && self.decode_bar == DecodeBarMode::Text {
            self.decode_bar = DecodeBarMode::Info;
        }
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        // Settings popover consumes arrow/tab/escape/R keys when visible.
        if self.settings.visible {
            let result = self.settings.handle_keys(ctx);
            if result.source_switched {
                let idx = self.settings.source_mode_idx().min(SourceMode::ALL.len() - 1);
                let new_mode = SourceMode::ALL[idx];
                if new_mode != self.source_mode {
                    self.switch_source(new_mode);
                }
            }
            if result.am_audio_changed {
                self.reload_builtin_audio();
            }
            if result.wav_load_requested {
                self.try_load_wav();
            }
            if result.psk31_msg_accepted {
                self.apply_psk31_message();
            }
            self.sync_settings();
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
        let mut zoom_delta: f32 = 0.0;  // added to zoom ratio; +0.5 coarse, +0.1 fine
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
            if i.key_pressed(egui::Key::Num1) { self.pane_visible[0] ^= true; }
            if i.key_pressed(egui::Key::Num2) { self.pane_visible[1] ^= true; }
            if i.key_pressed(egui::Key::Num3) { self.pane_visible[2] ^= true; }
            if i.key_pressed(egui::Key::I) { toggle_source = true; }
            if i.key_pressed(egui::Key::C) {
                if self.signal_gen.cycling {
                    self.signal_gen.stop_cycling();
                } else {
                    self.signal_gen.start_cycling();
                }
                // Propagate to active TestToneSource if applicable
                if let Some(tts) = self.source.as_any_mut().downcast_mut::<TestToneSource>() {
                    if tts.signal_gen.cycling {
                        tts.signal_gen.stop_cycling();
                    } else {
                        tts.signal_gen.start_cycling();
                    }
                }
                self.reset_playback();
            }
            if i.key_pressed(egui::Key::D) {
                let has_text = self.source_mode == SourceMode::Psk31;
                self.decode_bar = self.decode_bar.next(has_text);
            }
            if i.key_pressed(egui::Key::E) { self.envelope_visible ^= true; }
            if i.key_pressed(egui::Key::L) { toggle_lock = true; }
            if i.key_pressed(egui::Key::M) { cycle_mode = true; }
            if i.key_pressed(egui::Key::N) { cycle_audio = true; }
            if i.key_pressed(egui::Key::P) { self.peak_hold_visible ^= true; }
            if i.key_pressed(egui::Key::S) { self.settings.visible ^= true; }
            if i.key_pressed(egui::Key::H) { self.show_help ^= true; }
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
            if i.key_pressed(egui::Key::Tab) { cycle_active_marker = true; }

            // ── Active marker movement ───────────────────────────────────────
            // Ctrl+←/→: coarse (1/8 span).
            // Alt+←/→ (Option on Mac): very fine — one FFT bin width.
            // (Ctrl+Shift+←/→ is reserved for extra-fine pan.)
            let bin_hz = self.freq_view.nyquist / (FFT_SIZE / 2) as f32;
            if i.modifiers.ctrl && !i.modifiers.shift {
                let step = self.freq_view.span_hz / 8.0;
                if i.key_down(egui::Key::ArrowLeft)  { marker_delta -= step; }
                if i.key_down(egui::Key::ArrowRight) { marker_delta += step; }
            } else if i.modifiers.alt {
                // Use key_pressed (fires once per physical keypress, no auto-repeat)
                // so each press moves exactly one bin.
                if i.key_pressed(egui::Key::ArrowLeft)  { marker_delta -= bin_hz; }
                if i.key_pressed(egui::Key::ArrowRight) { marker_delta += bin_hz; }
            }
            if i.key_pressed(egui::Key::R) && !self.settings.visible {
                self.reset_playback();
            }
            if i.key_pressed(egui::Key::Escape) {
                self.show_help = false;
                self.settings.visible = false;
            }
            if i.key_pressed(egui::Key::Q) { quit = true; }

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
                    let left  = i.key_pressed(egui::Key::ArrowLeft);
                    let right = i.key_pressed(egui::Key::ArrowRight);
                    let arrow = left || right;
                    if arrow && self.freq_view.span_hz >= self.freq_view.nyquist {
                        self.freq_view.step_zoom(0.1);
                    }
                    if left  { pan_delta -= 10.0; }
                    if right { pan_delta += 10.0; }
                    if arrow { snap_pan_grid = 10.0; }
                } else if !i.modifiers.ctrl {
                    if i.modifiers.shift {
                        // Fine pan: snap to 100 Hz. Zoom in first if at full span.
                        let arrow = i.key_pressed(egui::Key::ArrowLeft) || i.key_pressed(egui::Key::ArrowRight);
                        if arrow && self.freq_view.span_hz >= self.freq_view.nyquist {
                            self.freq_view.step_zoom(0.1);
                        }
                        if i.key_pressed(egui::Key::ArrowLeft)  { pan_delta -= 100.0; }
                        if i.key_pressed(egui::Key::ArrowRight) { pan_delta += 100.0; }
                        if arrow { snap_pan_grid = 100.0; }
                    } else {
                        let arrow = i.key_down(egui::Key::ArrowLeft) || i.key_down(egui::Key::ArrowRight);
                        if arrow && self.freq_view.span_hz >= self.freq_view.nyquist {
                            self.freq_view.step_zoom(0.1);
                        }
                        let pan_step = self.freq_view.span_hz / 8.0;
                        if i.key_down(egui::Key::ArrowLeft)  { pan_delta -= pan_step; }
                        if i.key_down(egui::Key::ArrowRight) { pan_delta += pan_step; }
                    }
                }
            }

            // ── Frequency zoom ───────────────────────────────────────────────
            // Up/Down: zoom ±0.5; Shift+Up/Down: fine zoom ±0.1.
            // [ / ]: shift dB reference ±5 dB.
            if i.key_pressed(egui::Key::ArrowUp) {
                if i.modifiers.shift { zoom_delta += 0.1; } else { zoom_delta += 0.5; }
            }
            if i.key_pressed(egui::Key::ArrowDown) {
                if i.modifiers.shift { zoom_delta -= 0.1; } else { zoom_delta -= 0.5; }
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
                if let egui::Event::Text(s) = e {
                    if s == "R" || s == "r" { freq_reset = true; }
                }
            }
        });

        // Apply pan/zoom/span/reset
        if pan_delta != 0.0 {
            self.freq_view.pan(pan_delta);
            if snap_pan_grid > 0.0 {
                self.freq_view.center_hz = FreqView::snap_hz(self.freq_view.center_hz, snap_pan_grid);
            }
        }
        if zoom_delta.abs() > 0.001 { self.freq_view.step_zoom(zoom_delta); }
        if freq_reset { self.freq_view.reset(); }

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
            self.active_marker = if self.markers[1].enabled { Some(1) } else { None };
        }
        if toggle_marker_b {
            self.markers[2].enabled ^= true;
            self.active_marker = if self.markers[2].enabled { Some(2) } else { None };
        }
        // Tab: cycle active marker  None → A → B → None (skipping disabled markers)
        if cycle_active_marker {
            self.active_marker = match self.active_marker {
                None => {
                    if self.markers[1].enabled { Some(1) }
                    else if self.markers[2].enabled { Some(2) }
                    else { None }
                }
                Some(1) => {
                    if self.markers[2].enabled { Some(2) } else { None }
                }
                Some(_) => None,
            };
        }
        // Ctrl+arrow: move the active marker
        if marker_delta != 0.0 {
            if let Some(idx) = self.active_marker {
                let nyquist = self.freq_view.nyquist;
                self.markers[idx].hz = (self.markers[idx].hz + marker_delta).clamp(0.0, nyquist);
            }
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
                _ => {}
            }
        }
        if cycle_audio {
            match self.source_mode {
                SourceMode::AmDsb => {
                    self.settings.cycle_am_audio();
                    self.reload_builtin_audio();
                }
                SourceMode::Psk31 => {
                    self.settings.cycle_psk31_msg_mode();
                    self.apply_psk31_message();
                }
                _ => {}
            }
        }
        if quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// Reload the built-in audio buffer into the active AmDsbSource after the
    /// AM audio toggle changes (Morse ↔ Voice). No-op if source is not AM DSB
    /// or if Custom is selected (user WAV takes precedence).
    fn reload_builtin_audio(&mut self) {
        if self.source_mode != SourceMode::AmDsb {
            return;
        }
        if self.settings.am_audio_is_custom() {
            return;
        }
        let audio_idx = self.settings.am_audio_idx();
        self.settings.reset_am_repeat_for_audio(audio_idx);
        let builtin = BuiltinAudio::ALL[audio_idx.min(BuiltinAudio::ALL.len() - 1)];
        let (audio, rate) = load_builtin(builtin);
        if let Some(am) = self.source.as_any_mut().downcast_mut::<AmDsbSource>() {
            am.set_audio(audio, rate);
            am.msg_repeat = self.settings.am_msg_repeat();
        }
        self.reset_playback();
    }

    /// Attempt to load the WAV path from settings into the AM DSB source.
    fn try_load_wav(&mut self) {
        let path_str = self.settings.wav_path().to_owned();
        if path_str.is_empty() {
            self.settings.set_wav_status(false);
            return;
        }
        match source::load_wav_file(std::path::Path::new(&path_str)) {
            Ok((audio, rate)) => {
                if self.source_mode == SourceMode::AmDsb {
                    if let Some(am) = self.source.as_any_mut().downcast_mut::<AmDsbSource>() {
                        am.set_audio(audio, rate);
                    }
                }
                self.settings.set_wav_status(true);
                self.reset_playback();
            }
            Err(_) => {
                self.settings.set_wav_status(false);
            }
        }
    }

    /// Push current settings values into live signal/display state.
    fn sync_settings(&mut self) {
        self.db_min = self.settings.db_min();
        self.db_max = self.settings.db_max();
        self.waterfall.db_min = self.settings.db_min();
        self.waterfall.db_max = self.settings.db_max();
        self.signal_gen.freq_hz = self.settings.freq_hz();
        self.signal_gen.noise_amp = self.settings.noise_amp();
        self.signal_gen.amp_max = self.settings.amp_max();
        self.signal_gen.ramp_secs = self.settings.ramp_secs();
        self.signal_gen.pause_secs = self.settings.pause_secs();

        // Propagate test-tone settings into the active source if applicable
        if let Some(tts) = self.source.as_any_mut().downcast_mut::<TestToneSource>() {
            tts.signal_gen.freq_hz = self.settings.freq_hz();
            tts.signal_gen.noise_amp = self.settings.noise_amp();
            tts.signal_gen.amp_max = self.settings.amp_max();
            tts.signal_gen.ramp_secs = self.settings.ramp_secs();
            tts.signal_gen.pause_secs = self.settings.pause_secs();
        }

        // Propagate AM DSB settings into the active source if applicable
        if let Some(am) = self.source.as_any_mut().downcast_mut::<AmDsbSource>() {
            let carrier_changed = (am.carrier_hz - self.settings.am_carrier_hz()).abs() > 0.5;
            let index_changed = (am.mod_index - self.settings.am_mod_index()).abs() > 0.001;
            am.carrier_hz = self.settings.am_carrier_hz();
            am.mod_index = self.settings.am_mod_index();
            if carrier_changed || index_changed {
                am.rebuild_mod();
            }
            let gap_changed = (am.loop_gap_secs - self.settings.am_loop_gap_secs()).abs() > 0.01;
            if gap_changed {
                am.loop_gap_secs = self.settings.am_loop_gap_secs();
                am.update_loop_gap();
            }
            am.noise_amp = self.settings.am_noise_amp();
            am.msg_repeat = self.settings.am_msg_repeat().max(1);
        }

        if let Some(psk31) = self.source.as_any_mut().downcast_mut::<Psk31Source>() {
            let new_mode = match self.settings.psk31_mode_str() {
                "QPSK31" => Psk31Mode::Qpsk31,
                _        => Psk31Mode::Bpsk31,
            };
            let new_repeat      = self.settings.psk31_msg_repeat();
            let carrier_changed = (psk31.carrier_hz - self.settings.psk31_carrier_hz()).abs() > 0.01;
            let mode_changed    = psk31.mode != new_mode;
            let repeat_changed  = psk31.msg_repeat != new_repeat;
            psk31.carrier_hz    = self.settings.psk31_carrier_hz();
            psk31.noise_amp     = self.settings.psk31_noise_amp();
            psk31.loop_gap_secs = self.settings.psk31_loop_gap_secs();
            psk31.mode          = new_mode;
            psk31.msg_repeat    = new_repeat.max(1);
            // message is NOT synced here — it is applied only when the user
            // explicitly accepts the text edit via Enter (see apply_psk31_message).
            if carrier_changed || mode_changed || repeat_changed { psk31.render(); }
            psk31.update_loop_gap();
        }
        self.sync_decode_config();
    }

    /// Apply the committed PSK31 message and repeat count to the live source and
    /// re-render.  Called only when the user explicitly accepts the message edit.
    fn apply_psk31_message(&mut self) {
        if let Some(psk31) = self.source.as_any_mut().downcast_mut::<Psk31Source>() {
            psk31.message = self.settings.psk31_message().to_owned();
            psk31.render();
        }
        self.reset_playback();
    }

    /// Update the shared DecodeConfig to match the current source mode and carrier.
    fn sync_decode_config(&mut self) {
        let mode = match self.source_mode {
            SourceMode::Psk31 => match self.settings.psk31_mode_str() {
                "QPSK31" => DecodeMode::Qpsk31,
                _        => DecodeMode::Bpsk31,
            },
            SourceMode::AmDsb    => DecodeMode::AmDsb,
            SourceMode::TestTone => DecodeMode::TestTone,
        };
        let carrier_hz = match self.source_mode {
            SourceMode::Psk31    => self.settings.psk31_carrier_hz(),
            SourceMode::AmDsb    => self.settings.am_carrier_hz(),
            SourceMode::TestTone => self.settings.freq_hz(),
        };
        if let Ok(mut cfg) = self.decode_config.lock() {
            cfg.mode       = mode;
            cfg.carrier_hz = carrier_hz;
        }
    }

    fn draw_hud(&self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("hud").show(ctx, |ui| {
            // Build the status string first so we can centre it.
            let center  = self.freq_view.center_hz;
            let span    = self.freq_view.span_hz;
            let zoom_ratio = self.freq_view.zoom_ratio();
            let center_str = if center >= 1000.0 {
                format!("{:.2}kHz", center / 1000.0)
            } else {
                format!("{:.0}Hz", center)
            };
            let span_str = if span >= 1000.0 {
                format!("{:.1}kHz", span / 1000.0)
            } else {
                format!("{:.0}Hz", span)
            };
            let zoom_str = if (zoom_ratio - 1.0).abs() < 0.01 {
                "1×".to_owned()
            } else {
                format!("{:.1}×", zoom_ratio)
            };
            // Format a single marker frequency label, bracketed if active.
            let fmt_hz = |hz: f32| -> String {
                if hz >= 1000.0 { format!("{:.2}kHz", hz / 1000.0) }
                else            { format!("{:.0}Hz", hz) }
            };
            let m_a = &self.markers[1];
            let m_b = &self.markers[2];
            let mut marker_str = String::new();
            if m_a.enabled {
                let tag = if self.active_marker == Some(1) { "[A]" } else { "A" };
                marker_str.push_str(&format!("  {} {}", tag, fmt_hz(m_a.hz)));
            }
            if m_b.enabled {
                let tag = if self.active_marker == Some(2) { "[B]" } else { "B" };
                marker_str.push_str(&format!("  {} {}", tag, fmt_hz(m_b.hz)));
            }
            if m_a.enabled && m_b.enabled {
                let diff = m_b.hz - m_a.hz;
                marker_str.push_str(&format!("  B-A {}", fmt_hz(diff.abs())));
            }
            // Active mode flags: only show user-togglable states.
            // C = amplitude cycling (Test Tone only; AM DSB loops structurally, not flagged).
            let cycling = self.source_mode == SourceMode::TestTone && self.signal_gen.cycling;
            let modes: String = {
                let mut flags = Vec::new();
                if cycling                            { flags.push("C"); }
                if self.decode_bar == DecodeBarMode::Info  { flags.push("Di"); }
                if self.decode_bar == DecodeBarMode::Text  { flags.push("Dt"); }
                if self.envelope_visible              { flags.push("E"); }
                if self.source_locked      { flags.push("L"); }
                if self.peak_hold_visible  { flags.push("P"); }
                if flags.is_empty() { String::new() }
                else { format!(" ({})", flags.join(",")) }
            };
            let submode_str: String = match self.source_mode {
                SourceMode::AmDsb => match self.settings.am_audio_str() {
                    "Voice"  => "  aud v".to_owned(),
                    "Custom" => "  aud c".to_owned(),
                    _        => "  aud m".to_owned(),
                },
                SourceMode::Psk31 => {
                    let mode_ch = match self.settings.psk31_mode_str() {
                        "QPSK31" => "q",
                        _        => "b",
                    };
                    let msg_ch = match self.settings.psk31_msg_mode_str() {
                        "Custom" => "c",
                        _        => "n",
                    };
                    format!("  mode {mode_ch}  msg {msg_ch}")
                }
                _ => String::new(),
            };
            let status = format!(
                "{}{}{}  ctr {}  span {}  zoom {}  ref {:.0}dB{}",
                self.source_mode.label(), modes, submode_str, center_str, span_str, zoom_str, self.db_max, marker_str
            );

            // Three-section bar: left (title/hints) | centre (status) | —
            // The status is painted via the raw painter at the panel's horizontal
            // midpoint so it aligns with the primary marker in the panes below.
            let panel_rect = ui.max_rect();
            let mid_x = panel_rect.center().x;
            let mid_y = panel_rect.center().y;

            // Paint all three sections via the raw painter so none can overlap.
            let right_x = panel_rect.right() - 6.0;
            ui.painter().text(
                egui::pos2(mid_x, mid_y),
                egui::Align2::CENTER_CENTER,
                &status,
                self.mono_font_id.clone(),
                egui::Color32::from_rgb(0, 200, 255),
            );
            ui.painter().text(
                egui::pos2(right_x, mid_y),
                egui::Align2::RIGHT_CENTER,
                "? help",
                self.mono_font_id.clone(),
                egui::Color32::GRAY,
            );

            ui.horizontal(|ui| {
                // ── Left: title only ──────────────────────────────────────
                ui.label(
                    egui::RichText::new("orion-sdr-view")
                        .font(self.mono_font_id.clone())
                        .strong(),
                );
            });
        });
    }

    fn draw_panes(&self, ui: &mut egui::Ui) {
        let visible_count = self.pane_visible.iter().filter(|&&v| v).count();

        let avail = ui.available_rect_before_wrap();
        let pane_total_h = avail.height();

        if visible_count > 0 {
            let total_frac: f32 = self
                .pane_visible
                .iter()
                .zip(self.pane_frac.iter())
                .map(|(&vis, &f)| if vis { f } else { 0.0 })
                .sum();

            let mut y = avail.top();
            for i in 0..3 {
                if !self.pane_visible[i] {
                    continue;
                }
                let h = (self.pane_frac[i] / total_frac) * pane_total_h;
                let rect = egui::Rect::from_min_size(
                    egui::pos2(avail.left(), y),
                    egui::vec2(avail.width(), h),
                );
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, 0.0, PANE_BG[i]);
                match i {
                    0 => self.draw_spectrum(&painter, rect),
                    1 => self.draw_persistence_pane(&painter, rect),
                    _ => self.draw_waterfall_pane(&painter, rect),
                }
                y += h;
            }
        }

    }

    fn draw_decode_bar(&self, painter: egui::Painter, rect: egui::Rect) {
        const BAR_BG:    egui::Color32 = egui::Color32::from_rgb(15, 15, 30);
        const LABEL_COL: egui::Color32 = egui::Color32::from_rgb(80, 100, 140);
        const TEXT_COL:  egui::Color32 = egui::Color32::from_rgb(200, 200, 200);
        const DIM_COL:   egui::Color32 = egui::Color32::from_rgb(100, 100, 100);

        painter.rect_filled(rect, 0.0, BAR_BG);

        let font   = egui::FontId::new(12.0, egui::FontFamily::Monospace);
        let text_y = rect.center().y;

        // Dim mode label on the left: "DEC·i" = info, "DEC·t" = text.
        let dec_label = match self.decode_bar {
            DecodeBarMode::Info => "DEC\u{b7}i",
            DecodeBarMode::Text => "DEC\u{b7}t",
            DecodeBarMode::Off  => "DEC",
        };
        painter.text(
            egui::pos2(rect.left() + 6.0, text_y),
            egui::Align2::LEFT_CENTER,
            dec_label,
            font.clone(),
            LABEL_COL,
        );

        let label_w = painter.layout_no_wrap(dec_label.to_owned(), font.clone(), LABEL_COL).size().x;
        let content_x = rect.left() + 6.0 + label_w + 12.0; // 12 px right margin

        // Measure loop timer label so we know where the scrolling text must stop.
        let timer_label = self.loop_timer.label();
        let timer_w = painter.layout_no_wrap(timer_label.clone(), font.clone(), TEXT_COL).size().x;
        let em_w    = painter.layout_no_wrap("M".to_owned(),       font.clone(), TEXT_COL).size().x;
        // Right-aligned loop timer: "sig  12.34s loop 007" / "gap   2.00s loop 007"
        let timer_x = rect.right() - 6.0;
        let timer_left = timer_x - timer_w;
        // Right boundary for the scrolling text region: one 'M'-width gap before the timer.
        let scroll_right = timer_left - em_w;

        // Build the static (non-scrolling) text for Di mode and fallback states.
        let info_str = |modulation: &str, center_hz: f32, bw_hz: f32, snr_db: f32| -> String {
            let bw_str = if bw_hz.is_nan() || bw_hz <= 0.0 {
                "\u{2014}".to_owned()
            } else if bw_hz >= 1000.0 {
                format!("{:.1}kHz", bw_hz / 1000.0)
            } else {
                format!("{:.0}Hz", bw_hz)
            };
            format!("{modulation}  ctr {:.1}kHz  bw {bw_str}  snr {snr_db:.1}dB",
                center_hz / 1000.0)
        };

        // Determine whether Dt mode has content to render.
        let has_dt_text = self.decode_bar == DecodeBarMode::Text
            && !self.decode_ticker.visible.is_empty();

        if has_dt_text {
            // ── Smooth-scrolling ticker (Dt mode) ─────────────────────────
            // Text is right-aligned at scroll_right.  sub_px shifts the text
            // smoothly leftward as the next character slides in; when sub_px
            // crosses a character width, a new character appears at the right.
            let sub_px = self.decode_ticker.sub_px;
            let text_x = scroll_right + (em_w - sub_px);

            let clip_rect = egui::Rect::from_min_max(
                egui::pos2(content_x, rect.min.y),
                egui::pos2(scroll_right, rect.max.y),
            );
            let clipped = painter.with_clip_rect(clip_rect);
            clipped.text(
                egui::pos2(text_x, text_y),
                egui::Align2::RIGHT_CENTER,
                &self.decode_ticker.visible,
                font.clone(),
                TEXT_COL,
            );
        } else {
            // ── Static left-aligned text (Di mode or waiting) ────────────────
            let (text, color) = if self.decode_bar == DecodeBarMode::Info {
                // Di mode: show last_info if available.
                if let Some(DecodeResult::Info { modulation, center_hz, bw_hz, snr_db })
                    = &self.decode_ticker.last_info
                {
                    (info_str(modulation, *center_hz, *bw_hz, *snr_db), TEXT_COL)
                } else {
                    ("waiting for signal\u{2026}".to_owned(), DIM_COL)
                }
            } else {
                // Dt mode with no visible text yet: just show waiting.
                ("waiting for signal\u{2026}".to_owned(), DIM_COL)
            };
            painter.text(
                egui::pos2(content_x, text_y),
                egui::Align2::LEFT_CENTER,
                text,
                font.clone(),
                color,
            );
        }

        painter.text(
            egui::pos2(timer_x, text_y),
            egui::Align2::RIGHT_CENTER,
            timer_label,
            font,
            TEXT_COL,
        );
    }

    fn draw_spectrum(&self, painter: &egui::Painter, rect: egui::Rect) {
        let bins = &self.spectrum.fft_out_db;
        let n = bins.len();
        if n < 2 {
            return;
        }

        let lo = self.freq_view.lo();
        let hi = self.freq_view.hi();
        let nyquist = self.freq_view.nyquist;

        // bin index → Hz
        let bin_hz = |b: usize| b as f32 * nyquist / (n - 1) as f32;

        // Hz → X pixel (within visible window)
        let x_for_hz = |hz: f32| {
            rect.left() + self.freq_view.hz_to_x_norm(hz) * rect.width()
        };
        let y_for_db = |db: f32| {
            let t = (db - self.db_min) / (self.db_max - self.db_min);
            rect.bottom() - t.clamp(0.0, 1.0) * rect.height()
        };

        // ── Horizontal dB grid lines ──────────────────────────────────────
        let grid_stroke = egui::Stroke::new(0.5, egui::Color32::from_gray(45));
        let label_font = egui::FontId::new(10.0, egui::FontFamily::Monospace);
        let mut db = (self.db_min / 10.0).ceil() * 10.0;
        while db <= self.db_max {
            let y = y_for_db(db);
            painter.hline(rect.x_range(), y, grid_stroke);
            painter.text(
                egui::pos2(rect.left() + 4.0, y - 2.0),
                egui::Align2::LEFT_BOTTOM,
                format!("{:.0}dB", db),
                label_font.clone(),
                egui::Color32::from_gray(110),
            );
            db += 10.0;
        }

        // ── Vertical frequency grid lines + labels ────────────────────────
        // Choose a nice grid step based on visible span.
        let span = hi - lo;
        let raw_step = span / 5.0;
        let magnitude = 10f32.powf(raw_step.log10().floor());
        let norm = raw_step / magnitude;
        let nice = if norm < 1.5 { 1.0 } else if norm < 3.5 { 2.0 } else if norm < 7.5 { 5.0 } else { 10.0 };
        let grid_hz = nice * magnitude;

        let first_grid = (lo / grid_hz).ceil() * grid_hz;
        let mut hz = first_grid;
        // dB labels occupy ~50 px at the bottom-left; keep freq labels clear of them.
        let db_label_clearance = 52.0_f32;
        while hz <= hi + 0.5 {
            let x = x_for_hz(hz);
            if x >= rect.left() - 1.0 && x <= rect.right() + 1.0 {
                painter.vline(x, rect.y_range(), grid_stroke);
                let label = if hz >= 1000.0 {
                    format!("{:.1}k", hz / 1000.0)
                } else {
                    format!("{:.0}", hz)
                };
                // Skip freq label if it would overlap the bottom-left dB label area.
                let label_x = x + 3.0;
                if label_x >= rect.left() + db_label_clearance {
                    painter.text(
                        egui::pos2(label_x, rect.bottom() - 14.0),
                        egui::Align2::LEFT_BOTTOM,
                        label,
                        label_font.clone(),
                        egui::Color32::from_gray(110),
                    );
                }
            }
            hz += grid_hz;
        }

        // ── Spectrum line (visible bins only) ─────────────────────────────
        let mut points: Vec<egui::Pos2> = Vec::new();
        for b in 0..n {
            let hz = bin_hz(b);
            if hz < lo || hz > hi {
                if !points.is_empty() {
                    painter.line(
                        std::mem::take(&mut points),
                        egui::Stroke::new(1.5, egui::Color32::from_rgb(0, 220, 180)),
                    );
                }
                continue;
            }
            points.push(egui::pos2(x_for_hz(hz), y_for_db(bins[b])));
        }
        if !points.is_empty() {
            painter.line(points, egui::Stroke::new(1.5, egui::Color32::from_rgb(0, 220, 180)));
        }

        // ── Peak hold line ────────────────────────────────────────────────
        if self.peak_hold_visible {
            let mut ph_points: Vec<egui::Pos2> = Vec::new();
            for b in 0..n {
                let hz = bin_hz(b);
                if hz < lo || hz > hi {
                    if !ph_points.is_empty() {
                        painter.line(
                            std::mem::take(&mut ph_points),
                            egui::Stroke::new(1.0, egui::Color32::from_rgba_premultiplied(255, 180, 0, 180)),
                        );
                    }
                    continue;
                }
                ph_points.push(egui::pos2(x_for_hz(hz), y_for_db(self.peak_hold[b])));
            }
            if !ph_points.is_empty() {
                painter.line(
                    ph_points,
                    egui::Stroke::new(1.0, egui::Color32::from_rgba_premultiplied(255, 180, 0, 180)),
                );
            }
        }

        // ── Frequency markers ─────────────────────────────────────────────
        self.draw_freq_markers(painter, rect, &label_font);
    }

    /// Draw vertical marker lines into any pane rect.
    fn draw_freq_markers(&self, painter: &egui::Painter, rect: egui::Rect, label_font: &egui::FontId) {
        let lo = self.freq_view.lo();
        let hi = self.freq_view.hi();

        // Approximate pixel width of a bracket marker label ("A 12.34k" or "[A 12.34k]").
        // Used to detect overlap when assigning label rows.
        let label_width = 72.0_f32;
        // Each row tracks a list of occupied [left, right] intervals.
        let mut row_intervals: [Vec<(f32, f32)>; 3] = [vec![], vec![], vec![]];

        // Pre-reserve the primary marker's label interval on row 0.
        let primary_x = rect.center().x;
        row_intervals[0].push((primary_x + 3.0, primary_x + 3.0 + label_width));

        // Collect (idx, x, row, label, color, line_width) for each visible marker.
        struct Entry {
            x: f32,
            row: usize,
            label: String,
            color: egui::Color32,
            line_width: f32,
        }
        let mut entries: Vec<Entry> = Vec::new();

        // Primary marker first (always row 0, drawn at pane center).
        {
            let m = &self.markers[0];
            if m.enabled {
                let x = rect.center().x;
                let hz_label = if m.hz >= 1000.0 {
                    format!("{} {:.2}k", m.label(), m.hz / 1000.0)
                } else {
                    format!("{} {:.0}", m.label(), m.hz)
                };
                entries.push(Entry { x, row: 0, label: hz_label,
                    color: m.color(), line_width: 1.0 });
            }
        }

        // Bracket markers: assign rows greedily starting from 0.
        for (idx, m) in self.markers[1..].iter().enumerate() {
            let idx = idx + 1; // real index into self.markers
            if !m.enabled { continue; }
            if m.hz < lo || m.hz > hi { continue; }
            let x = rect.left() + self.freq_view.hz_to_x_norm(m.hz) * rect.width();
            let is_active = self.active_marker == Some(idx);
            let color = if is_active { egui::Color32::WHITE } else { m.color() };
            let line_width = if is_active { 1.5 } else { 1.0 };

            let hz_label = if m.hz >= 1000.0 {
                format!("{} {:.2}k", m.label(), m.hz / 1000.0)
            } else {
                format!("{} {:.0}", m.label(), m.hz)
            };
            let display_label = if is_active { format!("[{}]", hz_label) } else { hz_label };

            // Find the lowest row where this label fits without overlapping any
            // already-placed label interval on that row.
            let label_x = x + 3.0;
            let label_right = label_x + label_width;
            let row = (0..3)
                .find(|&r| row_intervals[r].iter().all(|&(lo, hi)| label_right <= lo || label_x >= hi))
                .unwrap_or(2);
            row_intervals[row].push((label_x, label_right));

            entries.push(Entry { x, row, label: display_label, color, line_width });
        }

        // Draw all markers.
        let dash_len = 8.0;
        let gap_len  = 5.0;
        for e in &entries {
            // Dashed vertical line
            let stroke = egui::Stroke::new(e.line_width, e.color);
            let mut y = rect.top();
            let mut paint = true;
            while y < rect.bottom() {
                let seg_len = if paint { dash_len } else { gap_len };
                let y_end = (y + seg_len).min(rect.bottom());
                if paint {
                    painter.line_segment(
                        [egui::pos2(e.x, y), egui::pos2(e.x, y_end)],
                        stroke,
                    );
                }
                y = y_end;
                paint = !paint;
            }
            // Label
            let label_y = rect.top() + 3.0 + e.row as f32 * 14.0;
            painter.text(
                egui::pos2(e.x + 3.0, label_y),
                egui::Align2::LEFT_TOP,
                &e.label,
                label_font.clone(),
                e.color,
            );
        }
    }

    /// Draw persistence pane with freq zoom UV and markers.
    fn draw_persistence_pane(&self, painter: &egui::Painter, rect: egui::Rect) {
        // Draw with UV cropped to visible frequency range
        let lo_uv = self.freq_view.lo() / self.freq_view.nyquist;
        let hi_uv = self.freq_view.hi() / self.freq_view.nyquist;

        if let Some(tex) = self.persistence.texture_handle() {
            let uv = egui::Rect::from_min_max(
                egui::pos2(lo_uv, 0.0),
                egui::pos2(hi_uv, 1.0),
            );
            painter.image(tex.id(), rect, uv, egui::Color32::WHITE);
        } else {
            self.persistence.draw(painter, rect, self.envelope_visible);
            return;
        }

        if self.envelope_visible {
            self.persistence.draw_envelope_cropped(painter, rect, lo_uv, hi_uv);
        }

        let label_font = egui::FontId::new(10.0, egui::FontFamily::Monospace);
        self.draw_freq_markers(painter, rect, &label_font);
    }

    /// Draw waterfall pane with freq zoom UV and markers.
    fn draw_waterfall_pane(&self, painter: &egui::Painter, rect: egui::Rect) {
        let lo_uv = self.freq_view.lo() / self.freq_view.nyquist;
        let hi_uv = self.freq_view.hi() / self.freq_view.nyquist;

        if let Some(tex) = self.waterfall.texture_handle() {
            let uv = egui::Rect::from_min_max(
                egui::pos2(lo_uv, 0.0),
                egui::pos2(hi_uv, 1.0),
            );
            painter.image(tex.id(), rect, uv, egui::Color32::WHITE);
        } else {
            self.waterfall.draw(painter, rect);
            return;
        }

        let label_font = egui::FontId::new(10.0, egui::FontFamily::Monospace);
        self.draw_freq_markers(painter, rect, &label_font);
    }

    fn draw_help_overlay(&self, ui: &mut egui::Ui) {
        let screen = ui.ctx().content_rect();
        let overlay_rect = egui::Rect::from_center_size(
            screen.center(),
            egui::vec2(580.0, 470.0),
        );
        let painter = ui.painter();
        painter.rect_filled(
            overlay_rect,
            8.0,
            egui::Color32::from_rgba_premultiplied(0, 0, 0, 220),
        );
        painter.rect_stroke(
            overlay_rect,
            8.0,
            egui::Stroke::new(1.0, egui::Color32::GRAY),
            egui::StrokeKind::Outside,
        );

        // Each entry: kind 0=title, 1=section, 2=entry(key, description)
        // Entry strings use "\t" to split key column from description column.
        let lines: &[(&str, u8)] = &[
            ("Keyboard shortcuts", 0),
            ("Panes & Sources", 1),
            ("1 / 2 / 3\ttoggle Spectrum / Persistence / Waterfall", 2),
            ("I / M / N\tselect next source / mode / audio or message", 2),
            ("C / E / P\tcycle amplitude  |  envelope  |  peak hold", 2),
            ("L\tlock source freq/carrier to display center", 2),
            ("D\tcycle decode bar: off → info → text → off", 2),
            ("Frequency Pan / Zoom", 1),
            ("← / →\tpan left / right", 2),
            ("Shift+← / →\tfine pan, snap 100 Hz", 2),
            ("Ctrl+Shift+← / →\textra-fine pan, snap 10 Hz", 2),
            ("↑ / ↓ | Shift+↑ / ↓\tzoom | fine zoom (in / out)", 2),
            ("[ / ]\tref level ±5 dB", 2),
            ("R\treset to full view (0 – Nyquist)", 2),
            ("Markers", 1),
            ("A / B (shift)\tplace marker A / B at center, select it", 2),
            ("a / b\ttoggle marker A / B; select when enabling", 2),
            ("Tab\tcycle active marker: A → B → none", 2),
            ("Ctrl+← / →\tmove active marker (coarse)", 2),
            ("Alt+← / →\tmove active marker (one bin)", 2),
            ("Display", 1),
            ("S\topen/close settings popover", 2),
            ("? or H\ttoggle this help overlay", 2),
            ("Escape\tdismiss overlays", 2),
            ("Q\tquit", 2),
        ];

        // Fixed column positions. The key column uses monospace 12pt (~7.2 px/char).
        // Longest key is "Ctrl+Shift+← / →" (17 display chars) → ~122 px + 16 gutter.
        let col_x  = overlay_rect.left() + 28.0;
        let desc_x = col_x + 148.0;

        let mut y = overlay_rect.top() + 14.0;
        for (text, kind) in lines {
            let (size, color, dy) = match kind {
                0 => (15.0, egui::Color32::WHITE, 26.0),
                1 => (11.0, egui::Color32::from_rgb(120, 180, 255), 20.0),
                _ => (12.0, egui::Color32::from_gray(220), 18.0),
            };
            let font = egui::FontId::new(size, egui::FontFamily::Monospace);
            if *kind == 2 {
                let mut parts = text.splitn(2, '\t');
                let key  = parts.next().unwrap_or("");
                let desc = parts.next().unwrap_or("");
                painter.text(egui::pos2(col_x,  y), egui::Align2::LEFT_TOP, key,  font.clone(), color);
                painter.text(egui::pos2(desc_x, y), egui::Align2::LEFT_TOP, desc, font,         color);
            } else {
                let x = overlay_rect.left() + 20.0;
                painter.text(egui::pos2(x, y), egui::Align2::LEFT_TOP, *text, font, color);
            }
            y += dy;
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
        self.last_block_was_signal = block_is_signal;

        // Drain decode results first so Info/Text from the decode thread are
        // processed before any gap state change.
        while let Ok(result) = self.decode_rx.try_recv() {
            self.decode_ticker.push_result(result);
        }

        if !block_is_signal && self.decode_bar.is_visible() {
            // Push Gap every silent frame so that late-arriving Info/Text from
            // the decode thread's batch decode cannot pin the Di bar to stale
            // data.  The decode thread no longer sends Gap, so this is the sole
            // source; it keeps last_result=NoSignal throughout the gap.
            // Gap also sets in_gap=true in the ticker, which drives SPACE
            // injection during tick() at the nominal character scroll rate.
            self.decode_ticker.push_result(DecodeResult::Gap);
        }
        self.decode_ticker.tick(dt);

        // Update peak hold (decay slowly: 0.2 dB/frame, then latch new peaks).
        for (ph, &db) in self.peak_hold.iter_mut().zip(self.spectrum.fft_out_db.iter()) {
            *ph = (*ph - 0.2_f32).max(db);
        }

        self.persistence.map.accumulate(&self.spectrum.fft_out_db, self.db_min, self.db_max);
        self.persistence.map.decay();
        self.persistence.update_texture(ctx);
        self.waterfall.push_row(&self.spectrum.fft_out_db);
        self.waterfall.update_texture(ctx);

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

fn main() -> eframe::Result<()> {
    let cli = Cli::parse();
    let cfg = ViewConfig::load(cli.config);

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("orion-sdr-view")
            .with_inner_size([1200.0, 800.0 + DECODE_BAR_H]),
        ..Default::default()
    };
    eframe::run_native(
        "orion-sdr-view",
        options,
        Box::new(|cc| Ok(Box::new(ViewApp::new(cc, cfg)))),
    )
}
