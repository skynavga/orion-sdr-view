// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! COFDM receiver — the streaming demodulator and its frame accounting.
//!
//! This is what turns the instrumentation panel's simulated block into
//! measurement.  It consumes the source's **complex baseband** via
//! [`SignalSource::last_samples_iq`](crate::source::SignalSource::last_samples_iq)
//! and runs orion-sdr's [`OfdmFrameStreamDemod`] over it.
//!
//! **Why not decode the real stream the display consumes.**  That was tried
//! first, on the reasoning that deriving the decoder's input from the display's
//! samples keeps the two honest about each other.  It does not survive
//! measurement, for a reason that is structural rather than incidental.
//!
//! The real projection carries a conjugate image.  Mixing it back down to
//! baseband leaves that image at full power, and for a real input `s` the
//! Schmidl & Cox correlation collapses:
//!
//! ```text
//! r[n]              = s[n] * exp(-j*w0*n)
//! r[n+L]*conj(r[n]) = s[n]*s[n+L] * exp(-j*w0*L)
//! ```
//!
//! Every term carries the *same* phase, set by the mixer frequency and the lag
//! alone — so `arg` of the sum, and with it the estimated carrier frequency
//! offset, is a **constant independent of the actual offset**.  At `w0 = fs/4`
//! and `L = 64` that constant is zero.  Measured: the estimate came back as the
//! same -0.0134 Hz for true offsets of 0, 50, 200 and 1000 Hz, and only the
//! zero-offset case decoded.  The synthetic source happens to have no offset, so
//! a receiver built this way looks perfect and reports a `Δf` row that is
//! incapable of ever reading anything else.
//!
//! Low-passing the image away restores observability, but measured a +23.4 Hz
//! bias at the 1/4 fraction — independent of tap count from 11 to 81, and not
//! caused by rotator pairing or filter startup, both of which were eliminated.
//! Since the receiver holds its channel estimate across the frame
//! (`TrainingSymbolHold`, no residual-CFO tracking), 23.4 Hz integrates to about
//! 123 degrees of constellation rotation over one 14.7 ms frame: the header
//! decodes and the payload does not, which is exactly the observed
//! `CrcMismatch`.
//!
//! | Front end | estimate at true 0 / 200 Hz | decode |
//! | --- | --- | --- |
//! | complex baseband | 0.00 / 200.00 Hz | EVM -140 dB |
//! | mix only | -0.013 / -0.013 Hz | only at true zero |
//! | mix + low-pass | 23.4 / 212 Hz | payload fails |
//!
//! So the source hands over complex baseband, which it has anyway.  The
//! display's real samples remain the projection of the *same impaired samples*
//! — `real[k] == re(iq[k] * exp(j*2*pi*f0*k/fs))`, asserted in the tests — so
//! decoder and display still cannot disagree about what was transmitted.  A
//! genuinely real-valued source (a recorded IF, a soundcard) would need a proper
//! analytic front end (Hilbert, or a complex band-pass), designed against a
//! *nonzero* offset; a naive mixer and low-pass is not it.

use num_complex::Complex32 as C32;
use orion_sdr::demodulate::{OfdmFrameStreamDemod, OfdmRxProbe, RxFrame};
use orion_sdr::modulate::McsTable;

use super::source::{COFDM_BUFFER_FRAMES, CofdmShaping, cofdm_link_config};

/// One frame's measured diagnostics, flattened out of orion-sdr's `RxFrame`
/// into the shape the instrument wants.
///
/// Every field is `Option` because upstream reports "the stage that would
/// produce this did not run" as `None` rather than as a sentinel — and for the
/// two BER rungs that distinction is load-bearing: they are measured by
/// re-encoding a **decoded** frame, so they go `None` exactly when the link
/// fails.  A rising error rate that suddenly stops reporting is the signal that
/// the link has given up, and rendering that as `0.0` would invert its meaning.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OfdmRxFacts {
    pub sync_score: Option<f32>,
    pub cfo_hz: Option<f32>,
    pub evm_db: Option<f32>,
    pub channel_ber: Option<f32>,
    pub inner_ber: Option<f32>,
    pub inner_fec_ok: Option<bool>,
    pub outer_fec_ok: Option<bool>,
    // No `delay_spread_us`, deliberately.  The plan expects `Δt` and the
    // echo-within-guard verdict to come from the per-bin channel estimate, whose
    // inverse transform is the power delay profile — but band-limiting alone
    // makes that profile a Dirichlet kernel rather than a delta, so a
    // **perfectly flat** channel measures a large spread that depends only on
    // the occupancy.  Measured across the bandwidth fractions: 8.03 µs at 1/8
    // falling monotonically to 3.03 µs at 7/8, with no channel involved at all.
    //
    // Calibrating that floor out (subtracting the flat-channel reference in
    // quadrature, peak-centred and circular) shrank it but did not remove it,
    // and the residual still swamps real echoes: against an injected two-ray
    // channel the statistic read 6.88 µs flat, 6.65 µs with an echo at 4 samples
    // — *lower* than flat — and only rose usefully at 16 samples (11.27 µs).
    // A reading that moves the wrong way for small echoes is worse than no
    // reading, so `Δt` stays `Unavailable` (an em-dash) until a metric is
    // validated against known injected echoes rather than assumed.
}

/// Running frame accounting across a burst.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CofdmRxStats {
    /// Frames that decoded.
    pub decoded: u64,
    /// Frames the demodulator reported as failed.
    pub failed: u64,
    /// Frames that vanished **with nothing reported** — a sequence gap the
    /// failure count does not already explain.
    ///
    /// **This is not a refinement; it is the only way a silent drop is
    /// visible.**  Before orion-sdr 0.0.59 the streaming receiver discarded
    /// frames whenever its sync search ranked a later preamble ahead of an
    /// earlier one (measured: 6 of 8 lost at `Noise amp` 0.05, with zero errors
    /// reported), and decode-failure counting alone called that a perfect link.
    ///
    /// **Failures are excluded, not merely distinguished.**  A frame that fails
    /// to decode is skipped, so the next good frame's `sequence_num` is two
    /// ahead and the raw gap counts that same frame again — which double-counted
    /// every error (measured: `failed` and `lost` identical at every noise level
    /// from 0.53 to 1.00, so the panel showed twice the true count).  Only
    /// gap-minus-failures is a loss.
    pub lost: u64,
}

impl CofdmRxStats {
    /// Frames that should have arrived.
    pub fn expected(&self) -> u64 {
        self.decoded + self.failed + self.lost
    }

    /// Frame error rate over the burst so far, or `None` before any frame has
    /// been accounted for.
    pub fn frame_error_rate(&self) -> Option<f32> {
        let n = self.expected();
        (n > 0).then(|| (self.failed + self.lost) as f32 / n as f32)
    }
}

/// The COFDM receiver: streaming demodulator plus frame accounting.
pub struct CofdmRx {
    demod: OfdmFrameStreamDemod,
    /// Reusable diagnostic buffers for the constellation / correction pane.
    ///
    /// Held here rather than allocated per call — that is the whole point of
    /// upstream's caller-owned design: a probed frame is ~2600 complex symbols
    /// and ~5100 outcome bytes at 8–51 frames per second, and capacity is
    /// retained across calls so a steady stream does not reallocate.
    ///
    /// It costs nothing while the pane is closed: [`process`](Self::process)
    /// calls plain `feed` then, and upstream's gate is the choice of method
    /// rather than a flag, so there is not even a branch to pay.
    probe: OfdmRxProbe,
    stats: CofdmRxStats,
    last: Option<OfdmRxFacts>,
    last_seq: Option<u32>,
    /// Decode failures since the last accepted frame, so the sequence gap that
    /// follows them is not charged a second time — see [`CofdmRxStats::lost`].
    failed_since_accept: u64,
}

impl CofdmRx {
    /// Builds a receiver matching `shaping` at `fs`.
    ///
    /// The numerology comes from [`cofdm_link_config`], the *same* builder the
    /// modulator uses.  A receiver whose config differs from the transmitter's
    /// by one field does not fail loudly — it simply never acquires, which is
    /// indistinguishable from a dead signal.
    pub fn new(shaping: &CofdmShaping, fs: f32) -> Self {
        let (cfg, preamble) = cofdm_link_config(shaping, fs);
        let demod = OfdmFrameStreamDemod::new(cfg, McsTable::default_ladder(), preamble)
            // True CBER/IBER, measured by re-encoding each CRC-verified frame.
            // Off by default upstream because it costs one encode per frame;
            // measured here at under 2% of decode, so there is no reason to gate
            // it behind a setting.
            //
            // `with_channel_estimate` stays OFF: it costs an n_fft-sized
            // allocation per frame and nothing consumes it yet — see the note on
            // delay spread below.
            .with_error_rates(true);
        Self {
            demod,
            probe: OfdmRxProbe::new(),
            stats: CofdmRxStats::default(),
            last: None,
            last_seq: None,
            failed_since_accept: 0,
        }
    }

    pub fn stats(&self) -> CofdmRxStats {
        self.stats
    }

    pub fn last(&self) -> Option<OfdmRxFacts> {
        self.last
    }

    /// Clears all accumulated state — call at a gap edge, so one burst's frame
    /// accounting is never attributed to the next.
    pub fn reset(&mut self) {
        self.demod.clear();
        self.probe.clear();
        self.stats = CofdmRxStats::default();
        self.last = None;
        self.last_seq = None;
        self.failed_since_accept = 0;
    }

    /// The probe filled by the most recent [`process`](Self::process) call, or
    /// an empty one when probing is off.
    ///
    /// Read it *inside* the borrow — a `ProbedFrame` cannot outlive the call
    /// that filled it, and the next `process` clears and refills.
    pub fn probe(&self) -> &OfdmRxProbe {
        &self.probe
    }

    /// Feeds one block of complex baseband — the source's
    /// [`last_samples_iq`](crate::source::SignalSource::last_samples_iq) — and
    /// folds any completed frames into the running stats and the last-frame
    /// snapshot.
    ///
    /// `want_probe` selects the entry point rather than setting a flag: with it
    /// off nothing is computed for the pane at all.
    pub fn process(&mut self, iq: &[C32], want_probe: bool) {
        let results = if want_probe {
            self.demod.feed_probed(iq, &mut self.probe)
        } else {
            self.demod.feed(iq)
        };
        for result in results {
            match result {
                Ok(frame) => self.accept(&frame),
                Err(_) => {
                    self.stats.failed += 1;
                    self.failed_since_accept += 1;
                }
            }
        }
    }

    fn accept(&mut self, frame: &RxFrame) {
        self.stats.decoded += 1;
        self.count_gap(frame.packet.metadata.sequence_num);

        let d = &frame.diagnostics;
        self.last = Some(OfdmRxFacts {
            sync_score: d.sync_score,
            cfo_hz: d.cfo_hz,
            evm_db: d.evm_db,
            channel_ber: d.channel_ber,
            inner_ber: d.inner_ber,
            inner_fec_ok: d.inner_fec_ok,
            outer_fec_ok: d.outer_fec_ok,
        });
    }

    /// Counts frames missing between the last accepted `sequence_num` and this
    /// one.
    ///
    /// The source emits `COFDM_BUFFER_FRAMES` frames numbered `0..N` and then
    /// loops the buffer, so the sequence wraps and a *decrease* is normal.
    ///
    /// **Done in signed arithmetic, deliberately.** The obvious
    /// `seq.wrapping_sub(last).wrapping_sub(1) % N` is wrong: `wrapping_sub`
    /// reduces modulo 2^32, and that only commutes with `% N` when `N` divides
    /// 2^32. It does not for the shipped `N` of 40 — `2^32 mod 40 == 16` — so
    /// every pass through the buffer's 39 -> 0 seam invented **16** lost frames.
    /// Measured before this was fixed: 46 frames decoded and 16 reported lost,
    /// out of a stream that only contained 46. The narrow bandwidth fractions
    /// hid it, because their longer frames never reached the wrap inside a test.
    ///
    /// The cost of the wrap-around is that a loss of `N` or more frames aliases
    /// to a smaller number, which is the right trade when the alternative is
    /// reporting a routine loop as a catastrophic gap.
    fn count_gap(&mut self, seq: u32) {
        let n = COFDM_BUFFER_FRAMES as i64;
        if let Some(last) = self.last_seq
            && n > 0
        {
            let gap = (i64::from(seq) - i64::from(last) - 1).rem_euclid(n) as u64;
            // Frames already reported as failures account for part of the gap.
            self.stats.lost += gap.saturating_sub(self.failed_since_accept);
        }
        self.failed_since_accept = 0;
        self.last_seq = Some(seq);
    }
}
