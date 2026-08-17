// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Background decode thread and associated types.
//!
//! The decode thread receives raw f32 sample blocks from the main thread via a
//! bounded channel and dispatches based on mode:
//!
//! PSK31 (BPSK31 / QPSK31): accumulates samples while the carrier is present,
//! then decodes the entire transmission once when the gap arrives.  This avoids
//! duplicate output and mid-stream cold-starts that the old rolling-window
//! approach produced.
//!
//! AM DSB / Test Tone: uses a fixed rolling window for spectral analysis.
//!
//! CW: character-timed text decode using a pre-computed schedule from the known
//! message and WPM, plus spectral analysis for the Di bar.
//!
//! FT8 / FT4: streaming accumulate+decode via `Ft8StreamDecoder`.
//!
//! The main thread drains results each frame and updates `DecodeTicker`.

use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, Mutex};

use orion_sdr::util::SIGNAL_THRESHOLD;
use orion_sdr::util::rms;

use num_complex::Complex32 as C32;
use orion_sdr::demodulate::BitOutcome;
use orion_sdr::modulate::ConstellationOrder;

use crate::decode::instrument::CofdmInstrument;
use crate::source::cofdm::CofdmShaping;
use crate::source::{amdsb, cofdm, cw, ft8, psk31, tone};

/// One block of samples on its way to the decode worker.
///
/// Carries both representations of the **same** block. A demodulator needs an
/// analytic signal — the real projection carries a conjugate image that makes
/// the Schmidl & Cox carrier estimate a constant rather than a measurement (see
/// [`crate::source::cofdm::rx`]) — while the display wants the real projection
/// it has always drawn. Sending them together is what keeps them the same
/// samples; two independent streams would drift and nothing would catch it.
///
/// `iq` is `None` for the real-valued sources, which have no complex form and
/// no demodulator that wants one.
pub struct DecodeChunk {
    /// Monotonic block counter, so the worker can tell that the sample stream
    /// it is demodulating has a hole in it.
    ///
    /// Chunks are sent with `try_send` and **dropped when the channel is
    /// full** — fine for the spectral analysers, which treat each block as an
    /// independent window, but not for a streaming demodulator, whose framing
    /// spans blocks. A dropped block would surface as frame errors on a
    /// perfectly good link: the viewer's own hiccup, reported as the signal's
    /// fault.
    pub seq: u64,
    pub real: Vec<f32>,
    pub iq: Option<Vec<C32>>,
    /// The source's own transmitting/silent state, when it knows it — see
    /// [`SignalSource::signal_phase`](crate::source::SignalSource::signal_phase).
    /// `None` falls back to the block-RMS threshold.
    pub signal: Option<bool>,
}

impl DecodeChunk {
    /// A real-only block, for sources with no complex representation.
    pub fn real(seq: u64, real: Vec<f32>) -> Self {
        Self {
            seq,
            real,
            iq: None,
            signal: None,
        }
    }
}

/// One frame's probe data, owned so it can cross the decode channel.
///
/// The upstream `ProbedFrame` borrows the receiver's reusable buffers and so
/// cannot outlive the `feed_probed` that filled it — which is the point of that
/// design.  Crossing a thread boundary needs ownership, so the decode worker
/// copies out what the pane will draw, inside the borrow, and sends this.
#[derive(Clone, Debug)]
pub struct ProbeFrameData {
    /// The equalizer's output, in demap order.
    pub symbols: Vec<C32>,
    /// Per-coded-bit outcomes.  **Empty when [`decoded`](Self::decoded) is
    /// false** — no ground truth, which is not the same as no errors.
    pub correction: Vec<BitOutcome>,
    /// The constellation the symbols were demapped against, recovered from the
    /// frame header rather than read off the transmit config.
    pub constellation: ConstellationOrder,
    /// The inner code's `n` and `k`, for the map's codeword geometry.  Both `0`
    /// when the code has no block structure (the convolutional arm).
    pub codeword_bits: usize,
    pub codeword_info_bits: usize,
    /// Whether the payload verified.  `false` ⇒ symbols only.
    pub decoded: bool,
}

/// One decode chunk's worth of probe frames.
#[derive(Clone, Debug, Default)]
pub struct CofdmProbe {
    pub frames: Vec<ProbeFrameData>,
}

// ── Public types ──────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DecodeMode {
    Off,
    Bpsk31,
    Qpsk31,
    AmDsb,
    TestTone,
    Cw,
    /// FT8 full-frame accumulate+decode (Phase 2).
    Ft8,
    /// FT4 full-frame accumulate+decode (Phase 2).
    Ft4,
    /// Wideband COFDM — info-only spectral analysis (no text decode).
    Cofdm,
}

#[derive(Clone, Debug)]
pub struct DecodeConfig {
    pub mode: DecodeMode,
    pub carrier_hz: f32,
    pub fs: f32,
    /// COFDM occupied bandwidth (Hz), reported directly in the Di bar since the
    /// narrowband `spectrum_bw_hz` estimator cannot measure a wideband band.
    pub cofdm_bw_hz: f32,
    /// Effective COFDM edge guard and DC occupancy.  The instrumentation reads
    /// the data-carrier count off the plan these produce rather than deriving
    /// it from `n_fft`, which would be wrong for any profile whose active
    /// carriers are a small fraction of the FFT (DVB-T 2K: 1512 of 2048).
    /// The *effective* transmit shaping.
    ///
    /// The receiver builds its numerology from this through the same
    /// `cofdm_link_config` the modulator uses, so the two ends cannot drift: a
    /// demodulator differing by one field does not fail loudly, it simply never
    /// acquires, which is indistinguishable from a dead signal.
    pub cofdm_shaping: CofdmShaping,
    /// Whether to run the receiver's diagnostic probe — the equalized symbols
    /// and per-coded-bit correction map pane 3's constellation mode draws.
    ///
    /// **Driven by display state, not by settings**, which makes it the only
    /// field here that is: the pane being on is the whole reason to pay for it.
    /// Off, upstream's plain `feed` is called and nothing is computed, allocated
    /// or sent; the gate is the choice of method rather than a branch inside it.
    pub cofdm_probe: bool,
    /// Block RMS at or above which the source counts as transmitting.  Must
    /// match the main thread's `LoopTimer` threshold, or the two disagree about
    /// where a burst ends: the decode side keeps emitting into a gap the loop
    /// timer has already declared.
    pub signal_threshold: f32,
    // CW-specific fields for character-timed text decode.
    pub cw_message: String,
    pub cw_wpm: f32,
    pub cw_dash_weight: f32,
    pub cw_char_space: f32,
    pub cw_word_space: f32,
    pub cw_msg_repeat: usize,
}

impl DecodeConfig {
    pub fn new(fs: f32) -> Self {
        Self {
            mode: DecodeMode::Off,
            carrier_hz: 0.0,
            fs,
            cofdm_bw_hz: 0.0,
            cofdm_shaping: CofdmShaping::derived(crate::source::cofdm::COFDM_DEFAULT_BW_FRACTION),
            cofdm_probe: false,
            signal_threshold: SIGNAL_THRESHOLD,
            cw_message: String::new(),
            cw_wpm: 0.0,
            cw_dash_weight: 3.0,
            cw_char_space: 3.0,
            cw_word_space: 7.0,
            cw_msg_repeat: 1,
        }
    }
}

#[derive(Clone, Debug)]
pub enum DecodeResult {
    /// New decoded text to append to the ticker.
    Text(String),
    /// Non-text signal — display a one-line summary.
    Info {
        modulation: String,
        center_hz: f32,
        bw_hz: f32,
        snr_db: f32,
    },
    /// COFDM instrumentation, for the Di bar's prioritised line and the `X`
    /// panel.  A new variant rather than a widening of `Info`: widening would
    /// touch all eight `Info` construction sites across psk31/cw/ft8/spectral
    /// for no benefit, while this touches none of them.
    ///
    /// `None` **clears** the panel at a gap edge, so it falls back to em-dashes
    /// rather than holding numbers from a burst that has ended.
    Instrument(Option<Box<CofdmInstrument>>),
    /// Equalized symbols and the per-bit correction map for the frames that
    /// completed in one chunk — pane 3's constellation mode.
    ///
    /// A new variant rather than a widening of `Instrument`, on the precedent
    /// that variant's own doc comment sets.  It also runs on a **different
    /// cadence**: the instrument emits about once per 48 000 signal samples
    /// (~9 Hz), which would deliver the constellation in visible lurches and
    /// batch 17–57 map rows at a time.  This one emits whenever frames arrive,
    /// 8–51 Hz, which is the rate the data is produced at.
    ///
    /// Only sent while the pane is asking for it — see
    /// [`DecodeConfig::cofdm_probe`].
    Probe(Box<CofdmProbe>),
    /// No signal detected or carrier not found.
    NoSignal,
    /// Definite signal gap — bypasses hold timer.
    /// `decoded`: for FT8/FT4, true if at least one CRC-pass frame was found at
    /// this gap edge; always false for other sources (ignored by the main thread).
    Gap { decoded: bool },
}

// ── DecodeTicker ──────────────────────────────────────────────────────────────

/// Minimum seconds to hold an Info result before replacing it.
const INFO_HOLD_SECS: f32 = 3.0;
/// Scroll speed in pixels per second.
/// 36 px/s at 12 pt monospace (~7.2 px/char) ≈ 5 chars/s.
const SCROLL_PX_PER_SEC: f32 = 36.0;
/// Approximate character width at 12 pt monospace.
const CHAR_W: f32 = 7.2;
/// Max visible text buffer length (chars).
const MAX_BUF: usize = 512;

/// Scrolling ticker state maintained on the main thread.
///
/// Decoded text is queued in `pending`.  `tick()` advances a smooth pixel
/// offset; when it crosses a character-width boundary, the next character is
/// popped from `pending` to `visible`.  The renderer shifts the visible text
/// by the sub-character pixel fraction for jitter-free animation.
pub struct DecodeTicker {
    /// Characters waiting to be displayed, in order.
    pending: std::collections::VecDeque<char>,
    /// Characters already shown on screen (right-aligned, newest at right).
    pub visible: String,
    /// Accumulated sub-character pixel offset (0.0 .. CHAR_W).
    /// When this reaches CHAR_W, a new character is popped from pending.
    pub sub_px: f32,
    /// Currently displayed result.
    pub last_result: DecodeResult,
    /// Seconds the current result has been displayed.
    hold_elapsed: f32,
    /// Most recent Info result, retained independently of `last_result` so the
    /// Di bar can show signal data even while a Text hold is in effect.
    pub last_info: Option<DecodeResult>,
    /// Most recent COFDM instrumentation, retained on the same terms as
    /// `last_info`.  The `X` panel re-reads this every frame, so it is live
    /// rather than a snapshot: opening it freezes nothing and closing it loses
    /// nothing.  Cleared alongside `last_info` on a gap.
    pub last_instrument: Option<Box<CofdmInstrument>>,
    /// Probe frames delivered since the main thread last drained them.
    ///
    /// **Drained rather than held**, unlike `last_instrument`: the panes
    /// *accumulate* what arrives (a density map and a scrolling ring), so a
    /// batch read twice would double-count it and a batch never read would
    /// leave a hole in the scroll.
    pub pending_probe: Vec<ProbeFrameData>,
    /// True while in a signal gap — drives SPACE injection in `tick()`.
    pub in_gap: bool,
}

impl Default for DecodeTicker {
    fn default() -> Self {
        Self::new()
    }
}

impl DecodeTicker {
    pub fn new() -> Self {
        Self {
            pending: std::collections::VecDeque::new(),
            visible: String::new(),
            sub_px: 0.0,
            last_result: DecodeResult::NoSignal,
            hold_elapsed: 0.0,
            last_info: None,
            last_instrument: None,
            pending_probe: Vec::new(),
            in_gap: false,
        }
    }

    /// Integrate a new result from the decode thread.
    ///
    /// - `Text`: characters are queued in `pending` for gradual reveal.
    /// - `Info`: updates `last_info` (for Di bar); replaces `last_result` after hold.
    /// - `NoSignal` / `Gap`: transitions to no-signal state (Gap bypasses hold).
    /// - `FtGap`: consumed by the main thread before reaching here; treated as Gap if it arrives.
    pub fn push_result(&mut self, r: DecodeResult) {
        match &r {
            DecodeResult::Text(s) => {
                self.in_gap = false;
                for c in s.chars() {
                    self.pending.push_back(c);
                }
                if !matches!(self.last_result, DecodeResult::Text(_)) {
                    self.last_result = r;
                    self.hold_elapsed = 0.0;
                }
            }
            DecodeResult::Info { .. } => {
                self.last_info = Some(r.clone());
                let hold = match self.last_result {
                    DecodeResult::Text(_) => 0.0,
                    DecodeResult::Info { .. } => INFO_HOLD_SECS,
                    // Neither `Instrument` nor `Probe` ever becomes
                    // `last_result` (they do not participate in the hold), so
                    // they impose none.
                    DecodeResult::Instrument(_)
                    | DecodeResult::Probe(_)
                    | DecodeResult::NoSignal
                    | DecodeResult::Gap { .. } => 0.0,
                };
                if self.hold_elapsed >= hold {
                    self.last_result = r;
                    self.hold_elapsed = 0.0;
                }
            }
            DecodeResult::NoSignal => {
                let hold = match self.last_result {
                    DecodeResult::Text(_) => 0.0,
                    DecodeResult::Info { .. } => INFO_HOLD_SECS,
                    // Neither `Instrument` nor `Probe` ever becomes
                    // `last_result` (they do not participate in the hold), so
                    // they impose none.
                    DecodeResult::Instrument(_)
                    | DecodeResult::Probe(_)
                    | DecodeResult::NoSignal
                    | DecodeResult::Gap { .. } => 0.0,
                };
                if self.hold_elapsed >= hold {
                    self.last_result = r;
                    self.hold_elapsed = 0.0;
                }
            }
            DecodeResult::Probe(p) => {
                // Appended, not replaced: two chunks can complete frames
                // between one main-thread drain and the next, and dropping the
                // older batch would tear a hole in the correction map's scroll.
                self.pending_probe.extend(p.frames.iter().cloned());
            }
            DecodeResult::Instrument(inst) => {
                // Does not participate in the Info hold: the instrument feeds
                // the panel and the COFDM Di line directly, both of which read
                // it every frame.  `None` is the gap-edge clear.
                self.last_instrument = inst.clone();
            }
            DecodeResult::Gap { .. } => {
                self.last_result = DecodeResult::NoSignal;
                self.hold_elapsed = 0.0;
                self.last_info = None;
                self.last_instrument = None;
                self.pending_probe.clear();
                self.in_gap = true;
            }
        }
    }

    /// Advance the ticker.  Call once per frame with frame delta time.
    ///
    /// Smoothly advances pixel offset; pops characters from `pending` to
    /// `visible` when crossing character-width boundaries.  During gaps,
    /// injects SPACE characters at the same rate.
    pub fn tick(&mut self, dt: f32) {
        self.hold_elapsed += dt;

        // Only scroll if there's something to show or inject.
        let has_work = !self.pending.is_empty() || (self.in_gap && !self.visible.is_empty());
        if !has_work {
            return;
        }

        self.sub_px += SCROLL_PX_PER_SEC * dt;

        // Pop characters when crossing each CHAR_W boundary.
        while self.sub_px >= CHAR_W {
            self.sub_px -= CHAR_W;
            if let Some(c) = self.pending.pop_front() {
                self.visible.push(c);
            } else if self.in_gap {
                self.visible.push(' ');
            }
        }

        // Cap visible buffer length.
        if self.visible.len() > MAX_BUF {
            let drop = self.visible.len() - MAX_BUF;
            self.visible.drain(..drop);
        }
    }

    /// Flush the buffer and reset scroll (call on source/config change).
    pub fn reset(&mut self) {
        self.pending.clear();
        self.visible.clear();
        self.sub_px = 0.0;
        self.hold_elapsed = 0.0;
        self.last_result = DecodeResult::NoSignal;
        self.last_info = None;
        self.last_instrument = None;
        self.pending_probe.clear();
        self.in_gap = false;
    }
}

// ── Decode worker ─────────────────────────────────────────────────────────────

/// Fixed window size (samples) for spectral analysis (AM DSB, CW, Test Tone).
/// 4096 samples at 48 kHz = ~85 ms; bin resolution = 11.7 Hz.
pub const SPECTRUM_WINDOW_SAMPLES: usize = 4096;

/// Everything the decode worker carries between chunks.
///
/// Split out of [`DecodeWorker::run`], which held all of it as locals above a
/// `recv_timeout` loop and so had no per-chunk entry point.  The threaded worker
/// is now a thin adapter over [`DecodeState::process`], and the headless replay
/// driver calls the same method inline — one decode path, driven two ways.
///
/// **Running it inline is not merely convenient, it is what makes a measurement
/// reproducible.**  On the thread, results arrive when the scheduler gets to
/// them and both channels `try_send`, so a full channel silently discards.  That
/// is the right trade for a real-time display and the wrong one for a dump,
/// which would otherwise be measuring the viewer's frame pacing.
pub struct DecodeState {
    last_mode: DecodeMode,
    last_carrier: f32,
    was_signal: bool,
    last_seq: Option<u64>,
    /// Chunks the sequence counter says never arrived.
    ///
    /// Zero is the invariant a synchronous run must hold, and asserting it is
    /// the cheapest proof that a dump measures the link rather than the harness
    /// — a hole breaks a streaming demodulator's framing, so it would surface as
    /// frame errors on a perfectly good signal.
    dropped: u64,
    psk31: psk31::Psk31State,
    cw: cw::CwState,
    amdsb: amdsb::AmDsbState,
    testtone: tone::ToneState,
    ft8: ft8::Ft8State,
    cofdm: cofdm::CofdmState,
}

impl Default for DecodeState {
    fn default() -> Self {
        Self::new()
    }
}

impl DecodeState {
    pub fn new() -> Self {
        Self {
            last_mode: DecodeMode::Off,
            last_carrier: 0.0,
            was_signal: false,
            last_seq: None,
            dropped: 0,
            psk31: psk31::Psk31State::new(),
            cw: cw::CwState::new(),
            amdsb: amdsb::AmDsbState::new(),
            testtone: tone::ToneState::new(),
            ft8: ft8::Ft8State::new(),
            cofdm: cofdm::CofdmState::new(),
        }
    }

    /// How many chunks the sequence counter says went missing.  See [`dropped`].
    ///
    /// [`dropped`]: Self::dropped
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Reset every per-mode decoder.  Called on a flush chunk and on any config
    /// change that invalidates accumulated samples.
    fn reset_all(&mut self) {
        self.psk31.reset();
        self.cw.reset();
        self.amdsb.reset();
        self.testtone.reset();
        self.ft8.reset();
        self.cofdm.reset();
        self.was_signal = false;
    }

    /// Decode one chunk, emitting any results through `tx`.
    ///
    /// `cfg` is passed by reference rather than read from the shared
    /// `Arc<Mutex<..>>` here, so the threaded and inline callers cannot diverge
    /// on *which* fields they sample or on when they sample them — the lock
    /// scope belongs to the caller, the field set to this method.
    pub fn process(
        &mut self,
        chunk: &DecodeChunk,
        cfg: &DecodeConfig,
        tx: &SyncSender<DecodeResult>,
    ) {
        let samples = &chunk.real;

        // A hole in the sample stream breaks a streaming demodulator's framing,
        // so start its accounting over rather than charging the link for samples
        // the viewer dropped. See `DecodeChunk::seq`.
        if let Some(prev) = self.last_seq
            && chunk.seq != prev.wrapping_add(1)
        {
            self.dropped += chunk.seq.wrapping_sub(prev).wrapping_sub(1);
            self.cofdm.reset();
        }
        self.last_seq = Some(chunk.seq);

        // CW decodes against a known message and schedule, so its state carries
        // config rather than reading it per call.
        if cfg.mode == DecodeMode::Cw {
            self.cw.message.clone_from(&cfg.cw_message);
            self.cw.wpm = cfg.cw_wpm;
            self.cw.dash_weight = cfg.cw_dash_weight;
            self.cw.char_space = cfg.cw_char_space;
            self.cw.word_space = cfg.cw_word_space;
            self.cw.msg_repeat = cfg.cw_msg_repeat;
        }
        let (mode, carrier_hz, fs) = (cfg.mode, cfg.carrier_hz, cfg.fs);

        // Empty vec is a flush signal (sent by main thread on source reset).
        if samples.is_empty() {
            self.reset_all();
            self.last_mode = mode;
            self.last_carrier = carrier_hz;
            return;
        }

        // Flush accumulated buffer on config change.
        if mode != self.last_mode || (carrier_hz - self.last_carrier).abs() > 0.5 {
            self.reset_all();
            self.last_mode = mode;
            self.last_carrier = carrier_hz;
        }

        let is_signal = chunk
            .signal
            .unwrap_or_else(|| rms(samples) >= cfg.signal_threshold);
        let gap_edge = !is_signal && self.was_signal;
        self.was_signal = is_signal;

        match mode {
            DecodeMode::Bpsk31 | DecodeMode::Qpsk31 => {
                self.psk31
                    .process(samples, is_signal, gap_edge, mode, carrier_hz, fs, tx);
            }
            DecodeMode::Cw => {
                self.cw
                    .process(samples, is_signal, gap_edge, carrier_hz, fs, tx);
            }
            DecodeMode::AmDsb => {
                self.amdsb
                    .process(samples, is_signal, gap_edge, carrier_hz, fs, tx);
            }
            DecodeMode::TestTone => {
                self.testtone
                    .process(samples, is_signal, gap_edge, carrier_hz, fs, tx);
            }
            DecodeMode::Ft8 | DecodeMode::Ft4 => {
                self.ft8
                    .process(samples, is_signal, gap_edge, mode, carrier_hz, fs, tx);
            }
            DecodeMode::Cofdm => {
                self.cofdm.process(
                    samples,
                    is_signal,
                    gap_edge,
                    carrier_hz,
                    cfg.cofdm_bw_hz,
                    cfg.cofdm_shaping,
                    chunk.iq.as_deref(),
                    fs,
                    cfg.cofdm_probe,
                    tx,
                );
            }
            DecodeMode::Off => {}
        }
    }
}

pub struct DecodeWorker {
    config: Arc<Mutex<DecodeConfig>>,
    rx: Receiver<DecodeChunk>,
    tx: SyncSender<DecodeResult>,
}

impl DecodeWorker {
    pub fn new(
        config: Arc<Mutex<DecodeConfig>>,
        rx: Receiver<DecodeChunk>,
        tx: SyncSender<DecodeResult>,
    ) -> Self {
        Self { config, rx, tx }
    }

    /// Receive chunks until the sender hangs up, decoding each one.
    ///
    /// The whole body is [`DecodeState::process`]; what is left here is the
    /// thread's own concerns — the timeout, the disconnect, and holding the
    /// config lock across exactly one chunk.
    pub fn run(self) {
        let mut state = DecodeState::new();
        loop {
            let chunk = match self.rx.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok(c) => c,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            };
            // Snapshot rather than hold the guard across `process`: decoding a
            // chunk is unbounded work, and the writer is the UI thread.
            let cfg = self.config.lock().unwrap().clone();
            state.process(&chunk, &cfg, &self.tx);
        }
    }
}
