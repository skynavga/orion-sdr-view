// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! DVB-T decode — the Di info line plus the instrumentation provider.
//!
//! There is no text decode.  Spectral characterisation is delegated to
//! [`SpectralState`]; this module adds the RF-level measurements and assembles a
//! [`OfdmInstrument`] on the same cadence, so the Di bar and the `X` panel
//! advance in lockstep with the `Info` line.
//!
//! **The second provider of the instrument.**  Everything the generic COFDM
//! provider fills, this fills too; what it adds is the rungs that were cut with
//! DVB-T in mind and have had nothing to select them — the packet error unit,
//! the transport-stream lock, and configuration read back from the *recovered
//! TPS word* rather than from the transmit settings.  Those arrive in Phase 4 of
//! the plan; this module's job now is to make the measurements available.

use std::sync::mpsc::SyncSender;

use num_complex::Complex32 as C32;
use orion_sdr::util::rms;
use orion_sdr::waveform::dvb_t::{DVB_T_DATA_CARRIERS, DVB_T_N_FFT, DvbTLinkParams};

use super::rx::DvbTRx;
use super::source::{code_rate_fraction, constellation_label};
use crate::decode::instrument::{
    ERROR_COUNT_WRAP, ErrorUnit, OfdmFacts, OfdmInstrument, OfdmRxFacts,
};
use crate::decode::spectral::{SpectralState, wb_cn_db};
use crate::decode::{CofdmProbe, DecodeResult, ProbeFrameData};

/// Correction (dB) from the C/N a real-projection spectrum measures to the C/N
/// the receiver actually sees, given the display's oversampling factor.
///
/// COFDM carries a constant `REAL_PROJECTION_CN_OFFSET_DB = 3.01` because its
/// noise is generated at the same rate the display measures at: taking the real
/// part of complex baseband quarters the signal into two mirror lobes while
/// merely halving already-symmetric complex noise, so the estimator reads 3 dB
/// low.
///
/// DVB-T's is **not a constant, because its oversampling factor is not one**.
/// Noise is injected per *emitted* sample, so it is white over `fs_display`,
/// while the decoder reads every `L`-th sample and aliases that same power into
/// `fs_waveform`.  Writing both out against the requested ratio, with `σ²` the
/// per-component noise variance and `B` the occupied bandwidth:
///
/// ```text
/// measured = [(P_s / 2) / B] / [2σ² / fs_display] = P_s · fs_display / (4 · B · σ²)
/// actual   =  P_s / (2σ² · B / fs_waveform)       = P_s · fs_waveform / (2 · B · σ²)
/// ratio    =  fs_display / (2 · fs_waveform)      = L / 2
/// ```
///
/// So the spectrum over-reads by `10·log10(L/2)` dB and this subtracts it.  At
/// `L = 2` the two factors of two cancel and the correction is exactly zero,
/// which is what the 1 MHz mode measured before the factor became per-mode — and
/// is why this was a `0.0` constant until the narrow modes needed 4 and 12.
/// Left as arithmetic on the two rates the caller already holds, so it cannot
/// disagree with the factor the source actually rendered at.
fn real_projection_cn_offset_db(fs_display: f32, fs_waveform: f32) -> f32 {
    if !(fs_display.is_finite() && fs_waveform > 0.0) {
        return 0.0;
    }
    -10.0 * (fs_display / (2.0 * fs_waveform)).log10()
}

#[derive(Default)]
pub struct DvbTState {
    pub spectral: SpectralState,
    /// The live receiver, and the link it was built for.
    ///
    /// Rebuilt whenever the link parameters or the per-frame payload length
    /// change, because a demodulator whose numerology differs from the
    /// transmitter's does not fail loudly — it simply never acquires.
    rx: Option<(DvbTLinkParams, usize, DvbTRx)>,
}

impl DvbTState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        self.spectral.reset();
        if let Some((.., rx)) = self.rx.as_mut() {
            rx.reset();
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn process(
        &mut self,
        samples: &[f32],
        is_signal: bool,
        gap_edge: bool,
        carrier_hz: f32,
        bw_hz: f32,
        link: DvbTLinkParams,
        frame_payload_len: usize,
        full_scale: f32,
        iq: Option<&[C32]>,
        fs_display: f32,
        fs_waveform: f32,
        want_probe: bool,
        tx: &SyncSender<DecodeResult>,
    ) {
        self.feed_receiver(link, frame_payload_len, is_signal, iq, want_probe);
        self.emit_probe(want_probe, tx);
        let emitted = self.spectral.process(
            samples,
            is_signal,
            gap_edge,
            "DVB-T",
            carrier_hz,
            fs_display,
            // A wideband band spreads its energy over 1512 data carriers, so the
            // narrowband estimator — one peak bin against the noise floor —
            // measures a single subcarrier and reports a number tens of dB off.
            // Compare the mean power across the occupied window instead.
            //
            // **Read this number as approximate, and here more than anywhere.**
            // `wb_cn_db` takes its noise floor from the median of the bins
            // *outside* the occupied window, and its own doc comment names the
            // limit: at 87.5% occupancy there are barely any such bins and they
            // are all transmit skirt.  DVB-T sits at 83.25% in every mode and
            // cannot be narrowed, so the exclusion zone starves down to its
            // `MIN_NOISE_BINS` floor on every reading.  Measured against a
            // requested 35 dB, the Di line settles around 34 and dips to 23-28
            // perhaps one reading in six.
            //
            // The dips are not noise in the estimate — they are the interleaver
            // flush.  `DVBT_DISPLAY_RMS_DBFS` records why a DVB-T frame opens
            // and closes with near-impulsive symbols; an analysis window landing
            // on one splatters broadband energy across the handful of bins the
            // median is computed from, lifting the apparent floor by several dB.
            // A window that misses them reads the true ratio.
            //
            // The fix is not a better spectral estimator: it is to measure the
            // noise where the signal is, from the receiver's EVM, which is what
            // the instrument's `MER` rung is for.  That was blocked on the
            // orion-sdr 0.0.63 defect `DVBT_MEASURE_ERROR_RATES` documents;
            // 0.0.64 unblocked it, and `evm_db` now arrives on every decoded
            // frame.  Switching the HUD's C/N over to it is Phase 4 work — this
            // estimator stays until the panel that consumes `MER` exists, so the
            // reading has one owner rather than two disagreeing ones.
            |real, fs, carrier_hz| {
                wb_cn_db(real, fs, carrier_hz, bw_hz)
                    + real_projection_cn_offset_db(fs_display, fs_waveform)
            },
            // The occupied bandwidth of a DVB-T band is a fixed property of the
            // mode — `fs · 1705/2048`, with no lever that can move it — not a
            // value to measure.  `spectrum_bw_hz` only searches ±4 kHz around the
            // carrier and would report a sliver of this band.
            |_real, _fs, _carrier_hz, _state| bw_hz,
            tx,
        );

        if gap_edge {
            // Clear the panel rather than leaving it holding numbers from a burst
            // that has ended, and reset the receiver with it: the source rewinds
            // its looping buffer when the signal phase begins, so a receiver
            // still holding the last burst's partial frame would concatenate it
            // onto the front of the next.
            if let Some((.., rx)) = self.rx.as_mut() {
                rx.reset();
            }
            let _ = tx.try_send(DecodeResult::Instrument(None));
            return;
        }
        if !emitted {
            return;
        }

        let inst = self.build(samples, carrier_hz, bw_hz, link, fs_waveform, full_scale);
        let _ = tx.try_send(DecodeResult::Instrument(Some(Box::new(inst))));
    }

    /// Feed the receiver this block's complex baseband, building it (or
    /// rebuilding it after a settings change) as needed.
    ///
    /// Only during the signal phase: the gap carries noise alone, and letting the
    /// demodulator chew on it would accumulate spurious acquisitions against a
    /// burst that has not started.
    fn feed_receiver(
        &mut self,
        link: DvbTLinkParams,
        frame_payload_len: usize,
        is_signal: bool,
        iq: Option<&[C32]>,
        want_probe: bool,
    ) {
        let Some(iq) = iq else {
            // A source with no complex representation cannot be demodulated; the
            // instrument falls back to the simulation.
            self.rx = None;
            return;
        };
        let stale = self
            .rx
            .as_ref()
            .is_none_or(|(l, p, _)| *l != link || *p != frame_payload_len);
        if stale {
            self.rx = Some((
                link,
                frame_payload_len,
                DvbTRx::new(link, frame_payload_len),
            ));
        }
        if is_signal && let Some((.., rx)) = self.rx.as_mut() {
            rx.process(iq, want_probe);
        }
    }

    /// Send whatever the probe collected on this block, if anything.
    ///
    /// On the frame-arrival cadence rather than the instrument's, for the reason
    /// the COFDM provider gives: the `X` panel emits about once per 48 000 signal
    /// samples, which would deliver the constellation in visible lurches.
    fn emit_probe(&self, want_probe: bool, tx: &SyncSender<DecodeResult>) {
        if !want_probe {
            return;
        }
        let Some((.., rx)) = self.rx.as_ref() else {
            return;
        };
        let probe = rx.probe();
        if probe.is_empty() {
            return;
        }
        let frames: Vec<ProbeFrameData> = probe
            .iter()
            .map(|f| ProbeFrameData {
                symbols: f.symbols.to_vec(),
                correction: f.correction.to_vec(),
                constellation: f.meta.constellation,
                // No codeword geometry: DVB-T's inner code is always
                // `ConvCode::DvbK7`, which terminates once per frame and has no
                // block structure to draw boundaries for.  Upstream omits the
                // fields for that reason rather than carrying permanent zeroes;
                // `ProbeFrameData` already documents `(0, 0)` as the
                // convolutional arm's answer, so the pane needs no new case.
                codeword_bits: 0,
                codeword_info_bits: 0,
                decoded: f.meta.decoded,
            })
            .collect();
        let _ = tx.try_send(DecodeResult::Probe(Box::new(CofdmProbe { frames })));
    }

    /// What the receiver has measured, or `None` when none is running.
    ///
    /// **Neither FEC flag is carried.**  Upstream does not expose `inner_fec_ok`
    /// at all — DVB-T's inner code is convolutional, whose per-block convergence
    /// flag is a constant `true` — and `outer_fec_ok` is structurally
    /// `Some(true)` on every frame a caller can see, since a frame whose
    /// Reed–Solomon stage failed is returned as an error and counted in
    /// `failed`.  Wiring either to `fec_lock` would put a permanently-green
    /// indicator on screen, so both stay `None` and `OfdmInstrument::from_facts`
    /// decides the `FEC` row from `inner_ber` against the QEF threshold instead
    /// — the one quantity here that moves with link quality.
    fn rx_facts(&self) -> Option<OfdmRxFacts> {
        let (.., rx) = self.rx.as_ref()?;
        let stats = rx.stats();
        let last = rx.last().unwrap_or_default();
        Some(OfdmRxFacts {
            sync_score: last.sync_score,
            cfo_hz: last.cfo_hz,
            evm_db: last.evm_db,
            channel_ber: last.channel_ber,
            inner_ber: last.inner_ber,
            inner_fec_ok: None,
            outer_fec_ok: None,
            frame_error_rate: stats.frame_error_rate(),
            // Counted, not modelled: the receiver knows exactly how many frames
            // failed, so `err` needs no rate-times-elapsed estimate and cannot
            // disagree with the error rate beside it.
            error_count: (stats.failed % u64::from(ERROR_COUNT_WRAP)) as u32,
            error_count_wrapped: stats.failed >= u64::from(ERROR_COUNT_WRAP),
            frame_count: (stats.decoded % u64::from(ERROR_COUNT_WRAP)) as u32,
            frame_count_wrapped: stats.decoded >= u64::from(ERROR_COUNT_WRAP),
        })
    }

    /// The link the **receiver** reports, falling back to `configured` before a
    /// frame has decoded.
    ///
    /// `code_rate_hp` is the one that matters: the viewer transmits
    /// non-hierarchically, so the low-priority stream does not exist and the HP
    /// rate is the link's rate.
    fn signalled_link(&self, configured: DvbTLinkParams) -> DvbTLinkParams {
        let Some((.., rx)) = self.rx.as_ref() else {
            return configured;
        };
        rx.last()
            .and_then(|f| f.tps)
            .map_or(configured, |tps| DvbTLinkParams {
                guard: tps.guard,
                constellation: tps.constellation,
                code_rate: tps.code_rate_hp,
            })
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        &self,
        samples: &[f32],
        carrier_hz: f32,
        bw_hz: f32,
        link: DvbTLinkParams,
        fs_waveform: f32,
        full_scale: f32,
    ) -> OfdmInstrument {
        let peak = samples.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        // **What arrived, not what was configured.**  The TPS carriers signal
        // the constellation, code rate and guard interval, so once a frame
        // decodes the panel reports the link the receiver actually locked to
        // rather than the settings rows that produced it.  That is the same
        // provenance rule the rest of the panel follows, and TPS is what makes
        // it reachable here without a header block: no other source can report
        // its numerology as *received*.  Before the first frame, and for a
        // source rendering with no receiver, the configured link stands in.
        let signalled = self.signalled_link(link);
        OfdmInstrument::from_facts(&OfdmFacts {
            center_hz: carrier_hz,
            bandwidth_hz: bw_hz,
            level_amp: rms(samples),
            peak_amp: peak,
            // **Not 1.0**, unlike every other source since COFDM's fitted gain
            // went away.  DVB-T's crest factor is 29-33 dB, so a burst whose RMS
            // clears the shared signal threshold necessarily peaks well past
            // unity — see `DvbTSource::full_scale`, which is where this comes
            // from.  Referencing dBFS to 1.0 here would report a permanent
            // overload for a waveform doing exactly what its coding chain makes
            // it do.
            full_scale,
            cn_db: self.spectral.smoothed_snr_db,
            // The **waveform's** rate, not the display's: the symbol rate and
            // guard duration derived from it are properties of the transmission,
            // and the ×2 oversampling exists only so the band fits on screen.
            fs: fs_waveform,
            n_fft: DVB_T_N_FFT,
            cp_len: signalled.guard.cp_len_2k(),
            // Fixed by the standard at exactly 1512 of 2048 bins — asserted by
            // `dvb_t_2k_plans` upstream, and deliberately not re-derived here:
            // anything computed from `n_fft` would be silently wrong.
            data_carriers: DVB_T_DATA_CARRIERS,
            constellation: constellation_label(signalled.constellation),
            bits_per_symbol: signalled.constellation.bits_per_symbol(),
            inner_code_rate: code_rate_fraction(signalled.code_rate),
            rx: self.rx_facts(),
            // Simulation-only counters; with a receiver running they are ignored
            // in favour of the counted ones in `rx_facts`.
            error_count: 0,
            error_count_wrapped: false,
            // DVB-T's unit is the 188-byte transport packet, which is what makes
            // this the first thing to select `Packet` — the field was cut for it
            // and nothing has set it until now.  The `FER`/`PER` labels are the
            // same width, so the grid cannot reflow.
            error_unit: ErrorUnit::Packet,
        })
    }
}
