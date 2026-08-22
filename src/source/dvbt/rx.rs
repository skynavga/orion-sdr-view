// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! DVB-T receiver — the streaming demodulator and its frame accounting.
//!
//! Consumes the source's **complex baseband** via
//! [`SignalSource::last_samples_iq`](crate::source::SignalSource::last_samples_iq)
//! and runs orion-sdr's [`DvbTFrameStreamDemod`] over it.  The reason it is not
//! given the real stream the display consumes is the one
//! [`crate::source::cofdm::rx`] documents at length: mixing a real projection
//! back to baseband leaves the conjugate image at full power, and the frequency
//! estimator built on it reports a constant rather than a measurement.  DVB-T's
//! guard-interval acquisition is a correlation over the same kind of product and
//! degrades the same way.
//!
//! **Two things are genuinely different from COFDM here.**
//!
//! *There is no sequence number.*  A DVB-T frame carries a TPS `frame_number` in
//! `0..=3` — a position within the super-frame, not a monotonic counter — so
//! [`CofdmRx::count_gap`](crate::source::cofdm::CofdmRx)'s modular arithmetic has
//! no counterpart.  Reaching for it by analogy would produce a plausible number
//! that aliases every four frames.  It is also unnecessary: the stream demod
//! returns `Ok` or `Err` for *every* frame whose samples are fully present (see
//! its `try_one_frame`, which consumes past a failure rather than re-locking it),
//! so `decoded + failed` accounts for the whole burst and there is no silent-drop
//! channel for a `lost` counter to expose.
//!
//! *The frame geometry is told, not discovered.*  A DVB-T signal is preamble-less
//! and its frames are fixed-size, so the receiver is constructed with the symbol
//! count and per-frame payload length rather than reading them from a header.
//! Those must match what the modulator produced or the recovered payload is
//! silently truncated — which is why both come from
//! [`DvbTSource::frame_payload_len`](super::DvbTSource::frame_payload_len) and
//! [`DVBT_SYMBOLS_PER_FRAME`] rather than being restated here.

use num_complex::Complex32 as C32;
use orion_sdr::demodulate::{DvbTFrameStreamDemod, DvbTRxFrame, DvbTRxProbe};
use orion_sdr::waveform::dvb_t::{DvbTFrameParams, DvbTLinkParams};
use orion_sdr::waveform::dvb_t_tps::TpsWord;

use super::source::{DVBT_CELL_ID, DVBT_RX_WINDOW_BACKOFF, DVBT_SYMBOLS_PER_FRAME};

/// Whether to ask upstream for the measured CBER / IBER / EVM rungs.
///
/// **On since orion-sdr 0.0.64.**  It bought the three rungs and the pane-3
/// constellation together, because `feed_probed` sets the same internal
/// `want_truth` flag: probing and measuring are one switch upstream, not two.
///
/// What it costs is a whole-frame re-encode per decoded frame — the truth
/// reference the BERs are measured against.  That is the reason this source
/// fills its frames rather than copying COFDM's fixed 184 bytes (see
/// [`dvbt_frame_payload_bytes`](super::dvbt_frame_payload_bytes)): the re-encode
/// is taken on the whole frame whatever the payload, so a 184-byte payload would
/// pay ~5× the FEC work for a 2% fill.  Kept as a named constant so the cost can
/// be dropped without unpicking the call sites.
///
/// **It was `false` through 0.0.63**, where turning it on made a DVB-T frame
/// fail to decode outright at QPSK r3/4, QPSK r7/8, 16-QAM r7/8 and 64-QAM r3/4,
/// and — worse — decode with a *wrong* `inner_ber` at 64-QAM r7/8.  The
/// modulator stuffed null packets until the coded stream met or exceeded the
/// frame capacity, transmitted only what fit, and the receiver rebuilt the same
/// packet count and asked `decode_chain` for the full coded length; the
/// remainder was decoded from bits that were never sent.  0.0.64 states the rule
/// once, in `dvb_t_frame_fill`, and both directions call it: the frame carries
/// the largest packet count that *fits*, and the leftover carriers repeat the
/// coded stream's head where nothing decodes or compares.  Pinned downstream by
/// `error_rates_break_decoding_upstream` in `tests/dvbt_rx.rs`.
const DVBT_MEASURE_ERROR_RATES: bool = true;

/// One frame's measured diagnostics, flattened out of orion-sdr's
/// [`DvbTRxDiagnostics`](orion_sdr::demodulate::DvbTRxDiagnostics) plus the
/// recovered TPS word.
///
/// Every field is `Option` because upstream reports "the stage that would
/// produce this did not run" as `None` rather than as a sentinel — and for the
/// BER rungs that distinction is load-bearing: they are measured by re-encoding
/// a *decoded* frame, so they go `None` exactly when the link fails.
///
/// **`outer_fec_ok` is deliberately absent**, though upstream carries it.  DVB-T
/// has no CRC, so `ChainOutcome::is_valid` consults the Reed–Solomon result and
/// a frame whose outer code failed is returned as `Err` and never reaches here —
/// making the flag structurally `Some(true)` on every frame a caller can see.
/// Wiring it to a panel lock would put a permanently-green indicator on screen
/// that no link condition could move.  `inner_fec_ok` is not exposed upstream at
/// all, for the same reason applied to the convolutional inner code.  The two
/// rungs that actually move with link quality are
/// [`inner_ber`](Self::inner_ber) and
/// [`rs_corrected_bytes`](Self::rs_corrected_bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DvbTRxFacts {
    pub sync_score: Option<f32>,
    pub cfo_hz: Option<f32>,
    pub evm_db: Option<f32>,
    pub channel_ber: Option<f32>,
    pub inner_ber: Option<f32>,
    /// Bytes the outer Reed–Solomon decoder repaired across this frame.
    ///
    /// The one rung that degrades *gracefully*: a pass/fail flag saturates, but
    /// a rising correction count is a link approaching the cliff while still
    /// delivering every byte.  It reads zero on a clean link only because the
    /// source fills its frames — see
    /// [`dvbt_frame_payload_bytes`](super::dvbt_frame_payload_bytes).
    pub rs_corrected_bytes: Option<u32>,
    /// The transmission parameters read off the TPS carriers — **what arrived**,
    /// not what was configured.  This is what lets the instrument report the
    /// constellation, guard and code rate as `Known` without a header block.
    pub tps: Option<TpsWord>,
}

/// Running frame accounting across a burst.
///
/// No `lost` field, unlike [`CofdmRxStats`](crate::source::cofdm::CofdmRxStats):
/// see the module header.  Every fully-arrived frame is reported one way or the
/// other, so `decoded + failed` is the whole population.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DvbTRxStats {
    /// Frames that decoded.
    pub decoded: u64,
    /// Frames the demodulator reported as failed — TPS undecodable, or the
    /// Reed–Solomon stage uncorrectable.
    pub failed: u64,
    /// Bytes the outer code repaired across every decoded frame in this burst.
    /// Accumulated here rather than in the instrument, which is rebuilt from
    /// scratch on every emit.
    pub corrected_bytes: u64,
}

impl DvbTRxStats {
    /// Frames that should have arrived.
    pub fn expected(&self) -> u64 {
        self.decoded + self.failed
    }

    /// Frame error rate over the burst so far, or `None` before any frame has
    /// been accounted for.
    pub fn frame_error_rate(&self) -> Option<f32> {
        let n = self.expected();
        (n > 0).then(|| self.failed as f32 / n as f32)
    }
}

/// The DVB-T receiver: streaming demodulator plus frame accounting.
pub struct DvbTRx {
    demod: DvbTFrameStreamDemod,
    /// Reusable diagnostic buffers for the constellation / correction pane.
    /// Held here rather than allocated per call — a probed 68-symbol frame is
    /// ~103k complex symbols and its correction map larger still, and capacity
    /// is retained across calls so a steady stream does not reallocate.
    probe: DvbTRxProbe,
    stats: DvbTRxStats,
    last: Option<DvbTRxFacts>,
}

impl DvbTRx {
    /// Builds a receiver for a link carrying `frame_payload_len` TS payload
    /// bytes per 68-symbol frame.
    ///
    /// The `frame_number` and `cell_id` in the constructed params are the
    /// receiver's *cold-start assumption*, not a filter: the demodulator uses
    /// `link` for the numerology and reports whatever TPS word actually arrived,
    /// so one receiver decodes all four frames of a super-frame.
    pub fn new(link: DvbTLinkParams, frame_payload_len: usize) -> Self {
        let params = DvbTFrameParams {
            link,
            frame_number: 0,
            cell_id: (DVBT_CELL_ID >> 8) as u8,
        };
        let demod = DvbTFrameStreamDemod::new(params, DVBT_SYMBOLS_PER_FRAME, frame_payload_len)
            // Zero, and deliberately so — see `DVBT_RX_WINDOW_BACKOFF`, where the
            // measurement is.  Set explicitly rather than left to the default so
            // that the two ends are visibly configured from one constant: a
            // receiver whose window differs from what the transmitter's shaping
            // assumes does not fail loudly, it just decodes worse.
            .with_rx_window_backoff(DVBT_RX_WINDOW_BACKOFF)
            // True CBER/IBER, and EVM which shares the gate.  See
            // `DVBT_MEASURE_ERROR_RATES` for what the re-encode costs.
            .with_error_rates(DVBT_MEASURE_ERROR_RATES);
        Self {
            demod,
            probe: DvbTRxProbe::new(),
            stats: DvbTRxStats::default(),
            last: None,
        }
    }

    pub fn stats(&self) -> DvbTRxStats {
        self.stats
    }

    pub fn last(&self) -> Option<DvbTRxFacts> {
        self.last
    }

    /// Clears all accumulated state — call at a gap edge, so one burst's frame
    /// accounting is never attributed to the next, and the partial frame left in
    /// the demodulator's buffer is not concatenated onto the front of the next.
    pub fn reset(&mut self) {
        self.demod.clear();
        self.probe.clear();
        self.stats = DvbTRxStats::default();
        self.last = None;
    }

    /// The probe filled by the most recent [`process`](Self::process) call, or
    /// an empty one when probing is off.
    ///
    /// Read it *inside* the borrow — a probed frame cannot outlive the call that
    /// filled it, and the next `process` clears and refills.
    pub fn probe(&self) -> &DvbTRxProbe {
        &self.probe
    }

    /// Feeds one block of complex baseband and folds any completed frames into
    /// the running stats and the last-frame snapshot.
    ///
    /// `want_probe` selects the entry point rather than setting a flag: with it
    /// off nothing is computed for the pane at all.
    ///
    /// It is additionally gated on [`DVBT_MEASURE_ERROR_RATES`], because
    /// `feed_probed` sets the same upstream `want_truth` flag that
    /// `with_error_rates` does.  The two are one switch upstream, so probing
    /// while the rungs are off would silently pay for a truth re-encode whose
    /// results nothing reads.
    pub fn process(&mut self, iq: &[C32], want_probe: bool) {
        let want_probe = want_probe && DVBT_MEASURE_ERROR_RATES;
        let results = if want_probe {
            self.demod.feed_probed(iq, &mut self.probe)
        } else {
            self.demod.feed(iq)
        };
        for result in results {
            match result {
                Ok(frame) => self.accept(&frame),
                Err(_) => self.stats.failed += 1,
            }
        }
    }

    fn accept(&mut self, frame: &DvbTRxFrame) {
        self.stats.decoded += 1;
        let d = &frame.diagnostics;
        self.stats.corrected_bytes += u64::from(d.rs_corrected_bytes.unwrap_or(0));
        self.last = Some(DvbTRxFacts {
            sync_score: d.sync_score,
            cfo_hz: d.cfo_hz,
            evm_db: d.evm_db,
            channel_ber: d.channel_ber,
            inner_ber: d.inner_ber,
            rs_corrected_bytes: d.rs_corrected_bytes,
            tps: Some(frame.tps),
        });
    }
}
