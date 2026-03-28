//! Background decode thread and associated types.
//!
//! The decode thread receives raw f32 sample blocks from the main thread via a
//! bounded channel, checks for signal presence, and sends `DecodeResult` values
//! back. The main thread drains results each frame and updates `DecodeTicker`.

use std::sync::{Arc, Mutex};
use std::sync::mpsc::{Receiver, SyncSender};

use orion_sdr::util::rms;

// ── Public types ──────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DecodeMode {
    Off,
    Bpsk31,
    Qpsk31,
    AmDsb,
    TestTone,
}

#[derive(Clone, Debug)]
pub struct DecodeConfig {
    pub mode:       DecodeMode,
    pub carrier_hz: f32,
    pub fs:         f32,
}

impl DecodeConfig {
    pub fn new(fs: f32) -> Self {
        Self { mode: DecodeMode::Off, carrier_hz: 0.0, fs }
    }
}

#[derive(Clone, Debug)]
pub enum DecodeResult {
    /// New decoded text to append to the ticker.
    Text(String),
    /// Non-text signal — display a one-line summary.
    Info {
        modulation: String,
        center_hz:  f32,
        bw_hz:      f32,
        snr_db:     f32,
    },
    /// No signal detected (below RMS threshold).
    NoSignal,
}

/// Scrolling ticker state maintained on the main thread.
pub struct DecodeTicker {
    /// Accumulated decoded text, ring-capped at `MAX_BUF` chars.
    pub buffer:  String,
    /// Current display offset (char index into `buffer`).
    pub offset:  usize,
    /// Seconds since last scroll step.
    pub scroll_timer: f32,
    /// Last result kind — used to choose display mode.
    pub last_result: DecodeResult,
}

const MAX_BUF: usize = 256;

impl DecodeTicker {
    pub fn new() -> Self {
        Self {
            buffer:       String::new(),
            offset:       0,
            scroll_timer: 0.0,
            last_result:  DecodeResult::NoSignal,
        }
    }

    /// Integrate a new result from the decode thread.
    pub fn push_result(&mut self, r: DecodeResult) {
        if let DecodeResult::Text(ref s) = r {
            self.buffer.push_str(s);
            if self.buffer.len() > MAX_BUF {
                let drop = self.buffer.len() - MAX_BUF;
                self.buffer.drain(..drop);
                self.offset = self.offset.saturating_sub(drop);
            }
        }
        self.last_result = r;
    }

    /// Advance scroll by one char if enough time has elapsed (~8 chars/sec).
    pub fn tick(&mut self, dt: f32) {
        if self.buffer.is_empty() { return; }
        self.scroll_timer += dt;
        const SCROLL_INTERVAL: f32 = 1.0 / 8.0;
        while self.scroll_timer >= SCROLL_INTERVAL {
            self.scroll_timer -= SCROLL_INTERVAL;
            self.offset += 1;
            if self.offset >= self.buffer.len() {
                self.offset = 0;
            }
        }
    }

    /// Return the text to display in the decode bar (scrolled view).
    pub fn display_text(&self) -> &str {
        if self.buffer.is_empty() { return ""; }
        &self.buffer[self.offset.min(self.buffer.len())..]
    }

    /// Flush the buffer and reset scroll (call on source/config change).
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.offset = 0;
        self.scroll_timer = 0.0;
        self.last_result = DecodeResult::NoSignal;
    }
}

// ── Decode worker ─────────────────────────────────────────────────────────────

/// Minimum RMS amplitude to be considered "signal present".
const SIGNAL_THRESHOLD: f32 = 1e-4;

pub struct DecodeWorker {
    config: Arc<Mutex<DecodeConfig>>,
    rx:     Receiver<Vec<f32>>,
    tx:     SyncSender<DecodeResult>,
}

impl DecodeWorker {
    pub fn new(
        config: Arc<Mutex<DecodeConfig>>,
        rx:     Receiver<Vec<f32>>,
        tx:     SyncSender<DecodeResult>,
    ) -> Self {
        Self { config, rx, tx }
    }

    pub fn run(self) {
        loop {
            // Block for up to 100 ms waiting for a sample block.
            let samples = match self.rx.recv_timeout(std::time::Duration::from_millis(100)) {
                Ok(s)  => s,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout)       => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected)  => break,
            };

            let signal_present = rms(&samples) >= SIGNAL_THRESHOLD;
            let result = if signal_present {
                DecodeResult::NoSignal // placeholder until Phase 3/4/5
            } else {
                DecodeResult::NoSignal
            };

            // Non-blocking send — drop if the main thread's result queue is full.
            let _ = self.tx.try_send(result);
        }
    }
}
