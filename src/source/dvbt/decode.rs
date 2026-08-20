// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! DVB-T decode — the Di info line plus the instrumentation provider.
//!
//! There is no text decode.  Spectral characterisation is delegated to
//! [`SpectralState`]; this module adds the RF-level measurements and assembles a
//! [`CofdmInstrument`] on the same cadence, so the Di bar and the `X` panel
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
    CofdmFacts, CofdmInstrument, CofdmRxFacts, ERROR_COUNT_WRAP, ErrorUnit,
};
use crate::decode::spectral::{SpectralState, wb_cn_db};
use crate::decode::{CofdmProbe, DecodeResult, ProbeFrameData};

/// Correction (dB) from the C/N a real-projection spectrum measures to the C/N
/// the receiver actually sees.  **Zero for DVB-T, and not by coincidence — by
/// two cancelling factors of two.**
///
/// COFDM carries `REAL_PROJECTION_CN_OFFSET_DB = 3.01` because its noise is
/// generated at the same rate the display measures at: taking the real part of
/// complex baseband quarters the signal into two mirror lobes while merely
/// halving already-symmetric complex noise, so the estimator reads 3 dB low.
///
/// DVB-T runs its display stream at twice the waveform's rate (see
/// `DVBT_DISPLAY_OVERSAMPLE`), and injects noise per *emitted* sample — so the
/// noise is white over `2·fs_waveform` on the display, while the decoder, which
/// reads every other sample, aliases that same power into `fs_waveform` and sees
/// 3 dB more of it.  Writing both out against the requested ratio:
///
/// ```text
/// measured = [(P_s / 4) / B] / [(P_n / 2) / fs_display] = P_s · fs_display / (2 · B · P_n)
/// actual   =  P_s · fs_waveform / (B · P_n)
/// ratio    =  fs_display / (2 · fs_waveform) = 1
/// ```
///
/// So the display's estimate *is* the decoder's C/N, with nothing to correct.
/// Stated as a named constant rather than omitted, because the next multicarrier
/// source will have to work out which of the two it is.
const REAL_PROJECTION_CN_OFFSET_DB: f32 = 0.0;

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
            |real, fs, carrier_hz| {
                wb_cn_db(real, fs, carrier_hz, bw_hz) + REAL_PROJECTION_CN_OFFSET_DB
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
            self.rx = Some((link, frame_payload_len, DvbTRx::new(link, frame_payload_len)));
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
    /// indicator on screen; it stays `Unavailable` until Phase 4 drives it from
    /// `IBER`, which does move.
    fn rx_facts(&self) -> Option<CofdmRxFacts> {
        let (.., rx) = self.rx.as_ref()?;
        let stats = rx.stats();
        let last = rx.last().unwrap_or_default();
        Some(CofdmRxFacts {
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

    #[allow(clippy::too_many_arguments)]
    fn build(
        &self,
        samples: &[f32],
        carrier_hz: f32,
        bw_hz: f32,
        link: DvbTLinkParams,
        fs_waveform: f32,
        full_scale: f32,
    ) -> CofdmInstrument {
        let peak = samples.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
        CofdmInstrument::from_facts(&CofdmFacts {
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
            cp_len: link.guard.cp_len_2k(),
            // Fixed by the standard at exactly 1512 of 2048 bins — asserted by
            // `dvb_t_2k_plans` upstream, and deliberately not re-derived here:
            // anything computed from `n_fft` would be silently wrong.
            data_carriers: DVB_T_DATA_CARRIERS,
            constellation: constellation_label(link.constellation),
            bits_per_symbol: link.constellation.bits_per_symbol(),
            inner_code_rate: code_rate_fraction(link.code_rate),
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
