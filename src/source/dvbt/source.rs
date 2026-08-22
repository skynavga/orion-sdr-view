// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use num_complex::Complex32 as C32;
use orion_sdr::demodulate::CodecCache;
use orion_sdr::dsp::{Rotator, kaiser_lowpass_taps, kaiser_num_taps};
use orion_sdr::fec::{ConvCode, CrcKind, InnerFec, InterleaverKind, PunctureRate};
use orion_sdr::modulate::ofdm_frame::block_plan;
use orion_sdr::modulate::{
    ConstellationOrder, DVB_T_FRAMES_PER_SUPER_FRAME, DvbTFrameMod, DvbTSuperFrameMod,
    DvbTSuperFrameParams,
};
use orion_sdr::multicarrier::TxLowpass;
use orion_sdr::waveform::dvb_t::{
    DVB_T_DATA_CARRIERS, DVB_T_FRAME_OUTER, DVB_T_FRAME_OUTER_IL, DVB_T_KMAX, DVB_T_N_FFT,
    DvbTLinkParams, GuardInterval, dvb_t_fs_for_bandwidth, dvb_t_occupied_bw,
    is_dvb_t_constellation,
};
use orion_sdr::waveform::dvb_t_tps::TPS_SYMBOLS_PER_FRAME;
use orion_sdr::waveform::dvb_t_ts::{TS_PACKET_LEN, TS_PAYLOAD_LEN};

use crate::source::{
    CnNoise, CnReference, NoiseDomain, SignalSource, is_continuous_sig, mean_power_c,
};

// ── DVB-T constants ───────────────────────────────────────────────────────────
//
// DVB-T is a *conformant* wideband source: unlike the synthetic COFDM source,
// almost every number below is fixed by ETSI EN 300 744 rather than chosen.  The
// 2K structure is 2048 bins, 1705 active carriers, exactly 1512 of them data,
// and 68 symbols per frame — none of it configurable, at any bandwidth.
//
// **Bandwidth is the sample rate, and nothing else.**  Upstream's `docs/dvb.md`:
// narrowband scaling "changes only the sample rate: `occupied_BW = fs ·
// 1705/2048`… The 2K structure is unchanged."  So the six bandwidth modes below
// are six `fs` values over one numerology — there is no carrier-count lever to
// pull, which is what makes DVB-T's settings so much smaller than COFDM's.
//
// **Modulation is at baseband, upconversion is ours.**  `DvbTSuperFrameMod` has
// no `rf_hz` concept at all (its `OfdmConfig`'s `fs` "only affects timing/CFO
// units"), so the source holds one continuous `Rotator` and upconverts at read
// time — the arrangement `CofdmSource` arrived at, and for the same reason: a
// `TxLowpass` is centred on DC and would delete a stream already sitting at the
// band centre.

/// OFDM symbols in one DVB-T frame (§4.4).  Not a choice — the TPS word is 68
/// bits, one per symbol, so the frame length *is* the signalling block length.
pub const DVBT_SYMBOLS_PER_FRAME: usize = TPS_SYMBOLS_PER_FRAME;

/// Smallest display oversampling factor that keeps the band from folding.
///
/// **This is the one place DVB-T cannot follow COFDM, and it is forced by the
/// standard.**  DVB-T occupies `1705/2048` — 83.25% — of its own sample rate.
/// The viewer displays the *real projection* of a source's stream over the
/// one-sided span `0..fs/2`, and a band 0.83·fs wide does not fit in a window
/// 0.5·fs wide at any centre frequency: the upper edge folds back over the lower
/// one and the spectrum on screen is an alias of itself.  COFDM never meets this
/// because its widest bandwidth fraction is 7/8 of Nyquist, i.e. 0.44·fs.
///
/// So the source runs its display stream at `L · fs`, which is what a real
/// panadapter does anyway — a tuner watching a 1 MHz DATV channel samples well
/// above 1.2 MS/s.  Two is the smallest `L` that works: `0.8325·fs <
/// fs_display/2` requires `fs_display > 1.665·fs`.  The interpolation that
/// produces the extra samples is exact on the ones the decoder reads at every
/// `L` (see [`DvbTSource::render`]).
///
/// **`L` is not the same for every bandwidth**, and that is what
/// [`DvbTBandwidth::display_oversample`] is for: the display *width* is held
/// fixed across a group of modes, so the narrow modes oversample harder to reach
/// it.  This constant is the floor each of them is checked against.
pub const DVBT_MIN_DISPLAY_OVERSAMPLE: usize = 2;

/// Display width (Hz) shared by the three narrowband (amateur DATV) modes.
///
/// **A fixed axis across a group of modes, which is what a panadapter has and
/// what `fs`-derived spans do not.**  Tying the displayed width to each mode's
/// own rate rescaled the frequency axis on every `Bandwidth` press — 333k, 1M
/// and 2M each drew their band at the same 83% of a *different* window, so the
/// one thing a bandwidth toggle exists to show, that these are different widths,
/// was the one thing invisible.  Held fixed, 333k occupies 14.5% of the window,
/// 1M occupies 43.5%, and 2M occupies 87.0%.
///
/// 2.3 MHz is chosen so the widest mode in the group fills **7/8** of it — the
/// margin COFDM's widest bandwidth setting leaves, so the two wideband sources
/// read alike at their respective extremes.  A round number rather than the
/// exact `2 MHz · 8/7`, because it is a display choice and 2300.0 kHz reads
/// better on the axis than 2285.7 does.
pub const DVBT_NARROW_SPAN_HZ: f32 = 2_300_000.0;

/// Display width (Hz) shared by the three broadcast modes.  Exactly 4×
/// [`DVBT_NARROW_SPAN_HZ`], and chosen on the same rule: 8M fills 7/8 of it.
pub const DVBT_BROADCAST_SPAN_HZ: f32 = 4.0 * DVBT_NARROW_SPAN_HZ;

/// Kaiser stop-band target for the ×2 interpolator, in dB.
///
/// The images this suppresses land immediately outside the occupied band, where
/// the noise floor sits ~35 dB below the signal at the default C/N — so 60 dB
/// puts them well under anything the display resolves, and buying more costs
/// taps on every rendered sample.
const DVBT_INTERP_STOPBAND_DB: f32 = 60.0;

/// Receiver FFT-window back-off, in samples.  **Zero, measured rather than
/// assumed — and this is where the plan this source was built from was wrong.**
///
/// The back-off exists to make a TX symbol taper transparent: it slides the FFT
/// window earlier so the tapered samples fall in guard the receiver discards.
/// Upstream's `docs/dvb.md` tabulates the price in scattered-pilot interpolation
/// error — 17% at b = 32 ("free"), 28% at 42 (~1 dB), 62% at 64, 100% at 85
/// ([`DVB_T_MAX_RX_WINDOW_BACKOFF`], the aliasing ceiling) — and the plan took
/// b = 32 from it.
///
/// That table does not hold for the dense constellations.  Measured here on a
/// noiseless link, frames decoded of 3 with the outer code's corrections in
/// parentheses:
///
/// | b | QPSK r7/8 | 64-QAM r1/2 | 64-QAM r7/8 |
/// | ---: | --- | --- | --- |
/// | 0 | 3/3 (0) | 3/3 (0) | 3/3 (0) |
/// | 8 | 3/3 (0) | 3/3 (0) | **0/3** |
/// | 16 | 3/3 (0) | 3/3 (30) | 0/3 |
/// | 24 | 3/3 (0) | 3/3 (195) | 0/3 |
/// | 32 | **0/3** | 3/3 (360) | 0/3 |
///
/// Identical at G1/32 and G1/4, which confirms the cap is the pilot grid and not
/// the cyclic prefix.  Zero is the only value clean at every mode, so the taper
/// — the one lever that needs a back-off — is off by default instead.  See
/// [`DVBT_DEFAULT_TAPER`].
///
/// [`DVB_T_MAX_RX_WINDOW_BACKOFF`]: orion_sdr::waveform::dvb_t::DVB_T_MAX_RX_WINDOW_BACKOFF
pub const DVBT_RX_WINDOW_BACKOFF: usize = 0;

/// Guard samples available to TX spectral shaping: `roll_off + group_delay ≤
/// DVBT_SHAPING_SLACK`.
///
/// **Half the shortest cyclic prefix**, which is a different bound from COFDM's
/// `min(cp_len − b, b)` — that formula is about a backed-off window, and this
/// receiver does not back off ([`DVBT_RX_WINDOW_BACKOFF`]).  What it expresses
/// instead is ordinary OFDM dispersion: a symmetric mask of `2·group_delay + 1`
/// taps spreads energy `±group_delay`, and a channel whose impulse response fits
/// inside the cyclic prefix is absorbed by the equalizer.  G1/32's 64 samples is
/// the tight case, so the budget is 32 at every guard interval — a constant
/// where COFDM's is derived, for the opposite reason.
pub const DVBT_SHAPING_SLACK: usize = 32;

/// Target duration of the looping signal buffer, in seconds of native signal.
///
/// **A duration, not a super-frame count, and that is forced.**  COFDM renders a
/// fixed 40 frames because its frame length scales with its (fixed) rate.  A
/// DVB-T super-frame is a fixed *sample* count while `fs` varies 24× across the
/// bandwidth modes, so the same count spans 1.44 s at 333 kHz and 0.06 s at
/// 8 MHz — a buffer that is either a wasteful 40 MB or an audibly short loop
/// depending only on a bandwidth toggle.
pub const DVBT_BUFFER_TARGET_SECS: f32 = 0.25;

/// Ceiling on the rendered buffer, in super-frames.
///
/// Two rather than COFDM's forty, because a DVB-T super-frame is four 68-symbol
/// frames of 2048-point IFFTs and the display buffer holds
/// [`DvbTBandwidth::display_oversample`] samples for each.
///
/// **The oversampling factor is what makes this cost money**, and it is largest
/// exactly where the cap does *not* bind.  Measured at G1/4, the worst case for
/// each mode:
///
/// | mode | L | super-frames | buffer | render |
/// | --- | ---: | ---: | ---: | ---: |
/// | 333k | 12 | 1 | 66.8 MB | 117 ms |
/// | 1M | 4 | 1 | 22.3 MB | 53 ms |
/// | 2M | 2 | 1 | 11.1 MB | 36 ms |
/// | 6M | 3 | 2 | 33.4 MB | 87 ms |
/// | 7M | 3 | 2 | 33.4 MB | 87 ms |
/// | 8M | 2 | 2 | 22.3 MB | 71 ms |
///
/// 333k is the ceiling on both counts and cannot be reduced by this constant: it
/// already renders the minimum of one super-frame, which at its rate is 1.44 s
/// of signal against a 0.25 s target.  The floor is structural — a super-frame
/// is the unit `DvbTSuperFrameMod` produces, and its four frames are what cycle
/// the TPS frame number.
///
/// The cap itself binds only at the broadcast widths, where it delivers 0.145 s
/// of native signal against the target — and that is fine, because playback is
/// heavily non-realtime.  The app consumes at most `MAX_SAMPLES_PER_FRAME`
/// (4096) per frame, so even the shortest buffer here takes several seconds of
/// wall clock to play through once.
pub const DVBT_MAX_BUFFER_SUPER_FRAMES: usize = 2;

/// Target signal-phase RMS for the rendered burst, in dBFS.
///
/// **The RMS is normalised; the peaks are not, and cannot be.**  COFDM's
/// -15 dBFS was chosen to leave its 10-12 dB of crest factor inside full scale.
/// That reasoning does not survive here: measured across the whole mode matrix,
/// a DVB-T burst's crest factor is **29.5 to 32.8 dB** — see
/// `the_crest_factor_is_the_interleaver_flush` in `tests/dvbt.rs`, which pins
/// both the number and its cause.
///
/// The cause is not the carrier count.  It is that `DvbTFrameMod` runs a fresh
/// `encode_chain` per frame, so the Forney(12,17) outer interleaver both **fills
/// and drains inside every frame**: at each end ~2244 bytes of its output are
/// branch registers that are largely empty, which the convolutional coder turns
/// into a near-constant bit pattern, which maps to a near-constant
/// frequency-domain vector, which IFFTs to an impulse.  Measured per symbol at
/// 333 kHz QPSK r3/4: 29, 28, 27, 24, 23, 19, 16 dB across symbols 0-6, a flat
/// 10-13 dB — ordinary OFDM — across symbols 7-60, then 15, 20, 22, 25, 26, 28,
/// 29 dB climbing back out across 61-67.  A real DVB-T modulator carries
/// interleaver state across frames (§4.7's byte-continuous stream), which
/// upstream explicitly defers.
///
/// So a level that put the *peaks* inside full scale would put the RMS at
/// -35 dBFS, below the shared [`SIGNAL_THRESHOLD`] of 0.1 — reintroducing
/// exactly the per-source threshold COFDM's doc comment celebrates having
/// removed.  -18 dBFS keeps the burst unit-scale (0.126, comfortably over the
/// threshold) and the source reports its true peak swing as
/// [`DvbTSource::full_scale`] instead, so dBFS is referenced to what this
/// waveform actually reaches rather than to an assumption it violates.
///
/// [`SIGNAL_THRESHOLD`]: orion_sdr::util::SIGNAL_THRESHOLD
pub const DVBT_DISPLAY_RMS_DBFS: f32 = -18.0;

/// Display reference level (dBFS, spectrum-scale top) at the **minimum**
/// oversampling factor.  Use [`dvbt_preferred_ref_db`], which lifts this by the
/// factor the mode actually renders at.
///
/// COFDM sits 21 dB below its burst RMS.  DVB-T's power spreads over 1705 active
/// carriers filling 83% of the display span at `L = 2`, against COFDM's 64
/// filling 25% at the default fraction, so the same total RMS lands several dB
/// lower per bin.
///
/// **Set against a rendered capture, not derived.**  The analytic estimate came
/// out at -44, which put the peak-hold trace hard against the top gridline;
/// measured, the live trace's per-bin mean sits near -50 dBFS, so -41 leaves the
/// same ~9 dB of headroom COFDM's picture has and the two sources read alike.
///
/// Stated rather than inherited: `SourceFactory::preferred_ref_db` defaults to
/// the shared `Defaults::DB_MAX`, and a source declaring no preference would
/// draw its spectrum against whatever the last wideband source set.
pub const DVBT_PREFERRED_REF_DB: f32 = -41.0;

/// Display reference level (dBFS) for `bandwidth` — the base level lifted by
/// `10·log10(L/2)`.
///
/// **The reference cannot be a constant once the oversampling factor varies**,
/// and this is the correction that was missing when it started to: at 333k the
/// peak-hold trace ran off the top of the spectrum pane.
///
/// The burst's *total* power is normalised to [`DVBT_DISPLAY_RMS_DBFS`] in every
/// mode, but the spectrum draws power **per bin**, and the number of bins the
/// band covers is `1.665/L` of the FFT — 83.25% at `L = 2` and 13.9% at `L = 12`.
/// The same total spread over a sixth as many bins sits 7.8 dB higher in each of
/// them.  So the trace climbs with the factor, by exactly the ratio this
/// restates, and the scale has to climb with it or the narrow modes clip.
///
/// **Only the top moves.**  The noise floor is flat across the whole Nyquist and
/// its total power is pinned by the C/N reference to a fixed multiple of the
/// signal's, so its per-bin level is the same in every mode —
/// [`DVBT_PREFERRED_FLOOR_DB`] stays put and the window simply grows taller at
/// the narrow modes, from 49 dB to 57.  That is the honest picture: a
/// concentrated band against an unchanged noise floor genuinely has more
/// dynamic range to show.
pub fn dvbt_preferred_ref_db(bandwidth: DvbTBandwidth) -> f32 {
    let l = bandwidth.display_oversample() as f32;
    DVBT_PREFERRED_REF_DB + 10.0 * (l / DVBT_MIN_DISPLAY_OVERSAMPLE as f32).log10()
}

/// Display floor (dBFS, spectrum-scale bottom) preferred by DVB-T.
///
/// **The one source that has to state a floor, and for the same reason it has to
/// state a low reference.**  The shared floor is -80 dBFS, which suits a signal
/// whose per-bin level sits near -42 — COFDM's, with its power concentrated into
/// a quarter of the display span.  DVB-T's spreads over 83% of it, so the same
/// burst RMS lands ~10 dB lower per bin (measured: -52 dBFS mean, -48 peak-hold),
/// and the noise floor at the default 35 dB C/N lands near **-88 dBFS** — eight
/// dB below the bottom of the scale.
///
/// The failure that hides behind is worth stating plainly, because it is the
/// same shape as the one the centre bug had: nothing looks broken.  The band
/// draws correctly, the C/N reads correctly, and the out-of-band region simply
/// sits pinned at the floor — which is what a *noiseless* channel looks like.
/// An operator who reached for the `C/N` row to see the noise move would find it
/// did not.
///
/// -90 rather than -88, so the noise floor sits just clear of the bottom the way
/// COFDM's does (-78 against -80) rather than riding on it.  That makes the scale
/// 49 dB tall against COFDM's 44 — both comfortably inside what the spectrum,
/// persistence and waterfall panes share.
pub const DVBT_PREFERRED_FLOOR_DB: f32 = -90.0;

/// Default signal-burst duration, in **wall-clock seconds**.
pub const DVBT_DEFAULT_SIG_SECS: f32 = 10.0;
/// Default silence gap between bursts, in **wall-clock seconds**.
pub const DVBT_DEFAULT_GAP_SECS: f32 = 2.0;

/// Default C/N (dB).
///
/// Chosen the way COFDM's 35 dB was — high enough that every mode decodes
/// cleanly, low enough that the out-of-band noise floor the shaping rows act
/// against is on screen.  DVB-T's occupancy is 83% of its own rate, so the
/// spreading factor between the occupied band and the noise is under 1 dB where
/// COFDM's is 9: the same requested ratio puts the floor much closer to the
/// signal here, and a lower number would sit on the FEC cliff rather than below
/// it.
pub const DVBT_DEFAULT_CN_DB: f32 = 35.0;

/// Cell identifier signalled on the TPS carriers (§4.6.2.10).  Arbitrary — the
/// viewer transmits one cell — but non-zero so a receiver reading it back is
/// demonstrably reading *something*.
pub const DVBT_CELL_ID: u16 = 0x0A73;

// ── Bandwidth mode ──────────────────────────────────────────────────────────

/// A DVB-T channel bandwidth.  The three narrowband (amateur DATV) modes
/// `orion-sdr` names in [`NbBandwidth`], plus the three broadcast widths.
///
/// **The mode is nothing but a sample rate.**  `fs = BW · 2048/1705`, and the 2K
/// structure above it is identical in every mode — same 1705 active carriers,
/// same 1512 data carriers, same 68-symbol frame.  That is why this is one
/// source with a bandwidth row rather than a "narrowband DVB-T" source and a
/// "broadcast DVB-T" source.
///
/// [`NbBandwidth`]: orion_sdr::waveform::dvb_t::NbBandwidth
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DvbTBandwidth {
    Bw333kHz,
    Bw1MHz,
    Bw2MHz,
    Bw6MHz,
    Bw7MHz,
    Bw8MHz,
}

impl DvbTBandwidth {
    /// All variants in display order (matches the settings toggle options).
    pub const ALL: &'static [DvbTBandwidth] = &[
        DvbTBandwidth::Bw333kHz,
        DvbTBandwidth::Bw1MHz,
        DvbTBandwidth::Bw2MHz,
        DvbTBandwidth::Bw6MHz,
        DvbTBandwidth::Bw7MHz,
        DvbTBandwidth::Bw8MHz,
    ];

    /// Nominal occupied RF bandwidth (Hz).
    pub const fn occupied_hz(self) -> f32 {
        match self {
            DvbTBandwidth::Bw333kHz => 333_000.0,
            DvbTBandwidth::Bw1MHz => 1_000_000.0,
            DvbTBandwidth::Bw2MHz => 2_000_000.0,
            DvbTBandwidth::Bw6MHz => 6_000_000.0,
            DvbTBandwidth::Bw7MHz => 7_000_000.0,
            DvbTBandwidth::Bw8MHz => 8_000_000.0,
        }
    }

    /// The **waveform's** native sample rate (S/s): `occupied_hz · 2048/1705`.
    /// This is the rate the modulator, the receiver and the C/N reference all
    /// speak; it is not what the viewer displays at.
    pub fn fs(self) -> f32 {
        dvb_t_fs_for_bandwidth(self.occupied_hz())
    }

    /// Display oversampling factor for this mode.
    ///
    /// Not a constant, and not a free choice: it is the smallest integer that
    /// makes the display Nyquist reach this mode's group span
    /// ([`display_span_hz`](Self::display_span_hz)), floored at
    /// [`DVBT_MIN_DISPLAY_OVERSAMPLE`] so the band cannot fold.  Written out
    /// rather than computed, because the six answers are fixed by the standard's
    /// own rate table and a reader deserves to see them:
    ///
    /// | mode | `fs` (S/s) | L | display Nyquist | span | fill |
    /// | --- | ---: | ---: | ---: | ---: | ---: |
    /// | 333k | 399 991 | 12 | 2 399 944 | 2 300 000 | 14.5% |
    /// | 1M | 1 201 173 | 4 | 2 402 346 | 2 300 000 | 43.5% |
    /// | 2M | 2 402 346 | 2 | 2 402 346 | 2 300 000 | 87.0% |
    /// | 6M | 7 207 038 | 3 | 10 810 557 | 9 200 000 | 65.2% |
    /// | 7M | 8 408 211 | 3 | 12 612 317 | 9 200 000 | 76.1% |
    /// | 8M | 9 609 384 | 2 | 9 609 384 | 9 200 000 | 87.0% |
    ///
    /// `the_oversample_reaches_the_span` in `tests/dvbt.rs` re-derives every row
    /// and fails if a span, a rate or a factor moves out from under the others.
    ///
    /// **Why the groups cannot share one span.**  The three broadcast rates are
    /// exactly 6:7:8, so equalising their display *rates* would need factors of
    /// 28, 24 and 21 — a 200 MS/s stream.  The narrowband three are luckier
    /// (12/4/2 land within 0.1% of each other) but they cannot reach the
    /// broadcast widths either.  Two groups is what integer oversampling allows.
    pub const fn display_oversample(self) -> usize {
        match self {
            DvbTBandwidth::Bw333kHz => 12,
            DvbTBandwidth::Bw1MHz => 4,
            DvbTBandwidth::Bw2MHz => 2,
            DvbTBandwidth::Bw6MHz => 3,
            DvbTBandwidth::Bw7MHz => 3,
            DvbTBandwidth::Bw8MHz => 2,
        }
    }

    /// The width the viewer frames at zoom 1× — fixed across the mode's group,
    /// so stepping the `Bandwidth` toggle changes the band's width on screen
    /// rather than the axis under it.
    ///
    /// Always at or below the display Nyquist; the difference is headroom that
    /// is sampled but never framed.  See [`DVBT_NARROW_SPAN_HZ`].
    pub const fn display_span_hz(self) -> f32 {
        match self {
            DvbTBandwidth::Bw333kHz | DvbTBandwidth::Bw1MHz | DvbTBandwidth::Bw2MHz => {
                DVBT_NARROW_SPAN_HZ
            }
            DvbTBandwidth::Bw6MHz | DvbTBandwidth::Bw7MHz | DvbTBandwidth::Bw8MHz => {
                DVBT_BROADCAST_SPAN_HZ
            }
        }
    }

    /// The rate the viewer displays at: [`display_oversample`](Self::display_oversample)
    /// times the waveform's own, so the 83%-occupied band fits the one-sided
    /// span with room for the group's fixed width.
    pub fn display_fs(self) -> f32 {
        self.fs() * self.display_oversample() as f32
    }

    /// One-sided span of the display stream — all of it, framed or not.
    pub fn display_nyquist_hz(self) -> f32 {
        self.display_fs() / 2.0
    }

    /// Short label for the HUD / settings toggle.
    pub fn label(self) -> &'static str {
        match self {
            DvbTBandwidth::Bw333kHz => "333k",
            DvbTBandwidth::Bw1MHz => "1M",
            DvbTBandwidth::Bw2MHz => "2M",
            DvbTBandwidth::Bw6MHz => "6M",
            DvbTBandwidth::Bw7MHz => "7M",
            DvbTBandwidth::Bw8MHz => "8M",
        }
    }
}

/// Default bandwidth on startup / reset: the common general-purpose amateur
/// DATV channel, and the width upstream's own framing centres on.
pub const DVBT_DEFAULT_BANDWIDTH: DvbTBandwidth = DvbTBandwidth::Bw1MHz;

// ── Guard / constellation / code rate ───────────────────────────────────────
//
// These three are upstream's own enums, used directly rather than wrapped.  A
// viewer-side copy would need a conversion in both directions and could drift
// from the library's set by one variant without anything failing — and all a
// wrapper would add is the `ALL` slice and the `label`, which are free functions
// here instead.

/// The guard intervals, in display order.
pub const DVBT_GUARDS: &[GuardInterval] = &[
    GuardInterval::G1_32,
    GuardInterval::G1_16,
    GuardInterval::G1_8,
    GuardInterval::G1_4,
];

/// Default guard interval: 1/32, the shortest prefix and so the highest data
/// rate.  The viewer's channel is a wire, not a multipath one.
pub const DVBT_DEFAULT_GUARD: GuardInterval = GuardInterval::G1_32;

/// Short label for a guard interval.
pub fn guard_label(guard: GuardInterval) -> &'static str {
    match guard {
        GuardInterval::G1_32 => "1/32",
        GuardInterval::G1_16 => "1/16",
        GuardInterval::G1_8 => "1/8",
        GuardInterval::G1_4 => "1/4",
    }
}

/// The DVB-T constellations, in display order.
///
/// All three of §4.3.5, not the two the convenience MCS ladder carries:
/// `DvbTFrameParams::inner()` builds the inner code from `code_rate` directly
/// rather than indexing `dvb_t_mcs_table()`, and `TpsWord` encodes the full
/// Table 11/12 sets — so the ladder is a convenience, not a constraint.
pub const DVBT_CONSTELLATIONS: &[ConstellationOrder] = &[
    ConstellationOrder::Qpsk,
    ConstellationOrder::Qam16,
    ConstellationOrder::Qam64,
];

/// Default constellation: QPSK, the most robust of the three.
pub const DVBT_DEFAULT_CONSTELLATION: ConstellationOrder = ConstellationOrder::Qpsk;

/// Short label for a DVB-T constellation.
pub fn constellation_label(order: ConstellationOrder) -> &'static str {
    match order {
        ConstellationOrder::Bpsk => "BPSK",
        ConstellationOrder::Qpsk => "QPSK",
        ConstellationOrder::Qam16 => "QAM16",
        ConstellationOrder::Qam64 => "QAM64",
        ConstellationOrder::Qam256 => "QAM256",
    }
}

/// The inner code rates, in display order (§4.3.3, Table 2).
pub const DVBT_CODE_RATES: &[PunctureRate] = &[
    PunctureRate::R1_2,
    PunctureRate::R2_3,
    PunctureRate::R3_4,
    PunctureRate::R5_6,
    PunctureRate::R7_8,
];

/// Default inner code rate: 3/4, the middle of the ladder and the usual amateur
/// DATV choice.
pub const DVBT_DEFAULT_CODE_RATE: PunctureRate = PunctureRate::R3_4;

/// Short label for an inner code rate.
pub fn code_rate_label(rate: PunctureRate) -> &'static str {
    match rate {
        PunctureRate::R1_2 => "1/2",
        PunctureRate::R2_3 => "2/3",
        PunctureRate::R3_4 => "3/4",
        PunctureRate::R5_6 => "5/6",
        PunctureRate::R7_8 => "7/8",
    }
}

/// The inner code rate as `(k, n)`, for the instrument's `CR` readout and the
/// bit rate derived from it.  The outer RS(204,188) is deliberately not folded
/// in — `CR` advertises the inner code alone, as it does for COFDM.
pub fn code_rate_fraction(rate: PunctureRate) -> (usize, usize) {
    match rate {
        PunctureRate::R1_2 => (1, 2),
        PunctureRate::R2_3 => (2, 3),
        PunctureRate::R3_4 => (3, 4),
        PunctureRate::R5_6 => (5, 6),
        PunctureRate::R7_8 => (7, 8),
    }
}

/// The inner FEC for a link: K=7 punctured convolutional at its code rate.
/// Mirrors `DvbTFrameParams::inner()`, which needs a frame number this does not.
pub fn dvbt_inner_fec(link: DvbTLinkParams) -> InnerFec {
    InnerFec::Convolutional {
        rate: link.code_rate,
        code: ConvCode::DvbK7,
    }
}

// ── Spectral shaping ────────────────────────────────────────────────────────

/// Symbol-window roll-off, as a fraction of the **shaping budget** — not of the
/// guard, as COFDM's is.
///
/// DVB-T's budget is [`DVBT_SHAPING_SLACK`] (32 samples) at every guard
/// interval, so a fraction of `cp_len` would mean four different things across
/// the guard rows while the budget stayed the same, and 3/8 of G1/4's 512-sample
/// prefix would overrun it six times over.
///
/// **The taper is not RX-transparent here, at any setting, and the cost is
/// measured.**  `DvbTFrameMod` windows each symbol independently rather than
/// overlap-adding consecutive ones, so the taper attenuates guard samples the
/// cyclic prefix needs; upstream is explicit that it is transparent only when
/// paired with a receiver window back-off, and
/// [`DVBT_RX_WINDOW_BACKOFF`] shows there is no back-off the dense
/// constellations can afford.  Measured on a noiseless link, frames decoded of
/// 3 at G1/32 with no back-off:
///
/// | taper | QPSK r7/8 | 16-QAM r5/6 | 64-QAM r3/4 |
/// | --- | --- | --- | --- |
/// | off | 3/3 | 3/3 | 3/3 |
/// | 1/8 | 3/3 | 3/3, 6 corrected | **0/3** |
/// | 1/4 | 3/3 | **0/3** | 0/3 |
/// | 3/8 | 3/3 | 0/3 | 0/3 |
///
/// The baseband mask, by contrast, is transparent at every depth and every
/// constellation — which is why it is the lever that is on by default and this
/// one is not.  The row stays because the cost *is* the demonstration: it shows
/// what near-skirt shaping charges a dense constellation, which is the sort of
/// thing this viewer exists to make visible.
///
/// Capped at 3/8 for the reason COFDM caps there: a taper consuming the whole
/// budget leaves zero group delay for the mask, which would silently drop the
/// mask while the settings row still named a stop-band depth.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DvbTTaper {
    Off,
    Eighth,
    Quarter,
    ThreeEighths,
}

impl DvbTTaper {
    /// All variants in display order (matches the settings toggle options).
    pub const ALL: &'static [DvbTTaper] = &[
        DvbTTaper::Off,
        DvbTTaper::Eighth,
        DvbTTaper::Quarter,
        DvbTTaper::ThreeEighths,
    ];

    /// Raised-cosine taper length per symbol edge, in samples.
    pub fn roll_off(self) -> usize {
        match self {
            DvbTTaper::Off => 0,
            DvbTTaper::Eighth => DVBT_SHAPING_SLACK / 8,
            DvbTTaper::Quarter => DVBT_SHAPING_SLACK / 4,
            DvbTTaper::ThreeEighths => 3 * DVBT_SHAPING_SLACK / 8,
        }
    }

    /// Short label for the HUD / settings toggle.
    pub fn label(self) -> &'static str {
        match self {
            DvbTTaper::Off => "off",
            DvbTTaper::Eighth => "1/8",
            DvbTTaper::Quarter => "1/4",
            DvbTTaper::ThreeEighths => "3/8",
        }
    }
}

/// Baseband spectral-mask stop-band depth.
///
/// The deeper options are offered even though the budget cannot always reach
/// them: [`DvbTShaping::mask_filter`] clamps the filter to what the guard
/// leaves, and a clamped filter is a *shallower* mask rather than a broken one.
/// Documented rather than hidden, because the alternative — dropping options the
/// budget cannot fund — would make this row's content depend on the taper row
/// above it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DvbTMask {
    Off,
    Db40,
    Db60,
    Db80,
}

impl DvbTMask {
    /// All variants in display order (matches the settings toggle options).
    pub const ALL: &'static [DvbTMask] = &[
        DvbTMask::Off,
        DvbTMask::Db40,
        DvbTMask::Db60,
        DvbTMask::Db80,
    ];

    /// Kaiser stop-band attenuation target, or `None` when the mask is off.
    pub fn stopband_db(self) -> Option<f32> {
        match self {
            DvbTMask::Off => None,
            DvbTMask::Db40 => Some(40.0),
            DvbTMask::Db60 => Some(60.0),
            DvbTMask::Db80 => Some(80.0),
        }
    }

    /// Short label for the HUD / settings toggle.
    pub fn label(self) -> &'static str {
        match self {
            DvbTMask::Off => "off",
            DvbTMask::Db40 => "40 dB",
            DvbTMask::Db60 => "60 dB",
            DvbTMask::Db80 => "80 dB",
        }
    }

    /// The label as the top HUD renders it, with the unit closed up.
    ///
    /// The HUD separates fields by a single space and attaches units to their
    /// numbers (`c/n 35dB`, `ref -38dB`), so a value carrying an internal space
    /// would read as two fields.  The settings toggle and the config keep the
    /// spaced form, which is what a row of options wants.
    pub fn hud_label(self) -> &'static str {
        match self {
            DvbTMask::Off => "off",
            DvbTMask::Db40 => "40dB",
            DvbTMask::Db60 => "60dB",
            DvbTMask::Db80 => "80dB",
        }
    }
}

/// Default taper: **off**, unlike COFDM's 1/4.
///
/// Not a timidity — it is the only setting under which all fifteen
/// constellation/rate pairs decode.  See [`DvbTTaper`] for the measured table.
pub const DVBT_DEFAULT_TAPER: DvbTTaper = DvbTTaper::Off;
/// Default mask: 60 dB, matching COFDM.  Free at every mode, and the lever that
/// upstream describes as *exceeding* the symbol-windowing ceiling anyway.
pub const DVBT_DEFAULT_MASK: DvbTMask = DvbTMask::Db60;
/// Shaping is on by default — meaning the mask alone, per the two above.
pub const DVBT_DEFAULT_SHAPING_ENABLED: bool = true;

/// The out-of-band spectral-shaping parameter set.
///
/// **Two levers, where COFDM has three.**  There is no edge-carrier guard here:
/// DVB-T's extreme active carriers (`±852`) are mandatory continual pilots, so
/// the occupied band is not negotiable and the only shaping available is the
/// symbol taper (acts on the near skirt) and the baseband mask (acts far out).
/// That also means the C/N reference bandwidth is a constant of the mode rather
/// than something the shaping can move.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DvbTShaping {
    pub enabled: bool,
    pub taper: DvbTTaper,
    pub mask: DvbTMask,
}

impl DvbTShaping {
    /// The shaping-disabled configuration: no taper, no mask.
    pub fn off() -> Self {
        Self {
            enabled: false,
            taper: DvbTTaper::Off,
            mask: DvbTMask::Off,
        }
    }

    /// The enabled defaults.
    pub fn default_enabled() -> Self {
        Self {
            enabled: DVBT_DEFAULT_SHAPING_ENABLED,
            taper: DVBT_DEFAULT_TAPER,
            mask: DVBT_DEFAULT_MASK,
        }
    }

    /// What is actually rendered: this set, or [`off`](Self::off) when shaping
    /// is disabled.  One resolver, so the renderer and the receiver cannot
    /// disagree about whether a taper is present.
    ///
    /// Simpler than COFDM's `effective`, which also had to clamp an edge guard
    /// against the band centre.  DVB-T's band width is fixed, so the centre
    /// constrains only the centre.
    pub fn effective(&self) -> Self {
        if self.enabled { *self } else { Self::off() }
    }

    /// Symbol-window roll-off in samples, after the enable flag.
    pub fn roll_off(&self) -> usize {
        self.effective().taper.roll_off()
    }

    /// The mask filter for the fixed DVB-T 2K band edge, or `None` when the mask
    /// is off.
    ///
    /// `taps_for_null_band` answers only the spectral constraint — the shortest
    /// filter whose transition reaches the stop band inside the 343-bin null
    /// band.  The guard budget is the other one, so the length is clamped to
    /// `2·(slack − roll_off) + 1` and `for_null_band` then centres its
    /// transition instead of pushing it against the band edge.
    pub fn mask_filter(&self) -> Option<TxLowpass> {
        let eff = self.effective();
        let stopband_db = eff.mask.stopband_db()?;
        // Cannot underflow: `DvbTTaper` caps `roll_off` below the slack.
        let max_delay = DVBT_SHAPING_SLACK.checked_sub(eff.taper.roll_off())?;
        let taps = TxLowpass::taps_for_null_band(DVB_T_N_FFT, DVB_T_KMAX / 2, stopband_db)
            .min(2 * max_delay + 1);
        (taps >= 3).then(|| DvbTFrameMod::tx_lowpass_for_2k(taps, stopband_db))
    }
}

// ── Frame geometry ──────────────────────────────────────────────────────────

/// Bits one DVB-T frame's data carriers hold, at `link`'s constellation:
/// `68 · 1512 · v`.
pub fn dvbt_frame_capacity_bits(link: DvbTLinkParams) -> usize {
    DVBT_SYMBOLS_PER_FRAME * DVB_T_DATA_CARRIERS * link.constellation.bits_per_symbol()
}

/// TS payload bytes one frame carries when the payload **fills** it: the largest
/// whole number of 188-byte TS packets whose coded stream still fits in 68
/// symbols, times the 187 payload bytes each carries.
///
/// **Filling the frame is the load-bearing decision here, and copying COFDM's
/// fixed 184 bytes would have been wrong.**  A DVB-T frame is 68 symbols
/// whatever the payload, so `DvbTFrameMod` null-packet-stuffs a short one to
/// fill it (§4.4).  A receiver told `payload_len = 184` then decodes a *prefix* —
/// 39 180 of 205 632 coded bits at QPSK — while every measured rung is taken on
/// the whole frame regardless, so the diagnostics cost ~5× the FEC work for a
/// payload that is 2% of what was transmitted.  Against a frame-filling payload
/// the same diagnostics are ~5%.  It is also the honest waveform: a real DATV
/// transmitter fills its frames.
///
/// **Why the *largest that fits* rather than the smallest that overflows.**  The
/// modulator sizes the frame from the payload — `n_symbols = max(payload_syms,
/// 68)` — so one packet too many pushes the frame to 69 symbols and the signal
/// stops being conformant DVB-T.  At QPSK r1/2 the boundary is sharp: 51 packets
/// code to 202 380 bits (fits), 52 to 205 644 (overflows 205 632 by twelve
/// bits).  So the payload is 51 packets and the modulator stuffs exactly one
/// null packet — a 98% fill, in a 68-symbol frame.
///
/// Computed through the library's own [`block_plan`] rather than re-derived, for
/// the reason `cofdm_data_carriers`' doc comment gives about carrier counts: an
/// arithmetic restatement of a coding chain drifts silently, and this one has a
/// Forney interleaver's flush delay and a convolutional tail in it.
pub fn dvbt_frame_payload_bytes(link: DvbTLinkParams) -> usize {
    dvbt_frame_payload_packets(link) * TS_PAYLOAD_LEN
}

/// The packet count behind [`dvbt_frame_payload_bytes`].
fn dvbt_frame_payload_packets(link: DvbTLinkParams) -> usize {
    let capacity = dvbt_frame_capacity_bits(link);
    let cache = CodecCache::new();
    let inner = dvbt_inner_fec(link);
    let coded = |n: usize| {
        block_plan(
            n * TS_PACKET_LEN,
            CrcKind::None,
            DVB_T_FRAME_OUTER,
            inner,
            DVB_T_FRAME_OUTER_IL,
            InterleaverKind::None,
            &cache,
        )
        .coded_bits
    };
    // Coded length is monotone in the packet count, so double until it overflows
    // and then bisect.  A linear scan would run to 319 `block_plan` calls at
    // 64-QAM r7/8; this runs about twenty at every mode.
    if coded(1) > capacity {
        return 1;
    }
    let mut lo = 1usize;
    while coded(lo * 2) <= capacity {
        lo *= 2;
    }
    let mut hi = lo * 2; // coded(lo) <= capacity < coded(hi)
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if coded(mid) <= capacity {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

/// Samples in one super-frame at `guard`, at the **waveform's** rate:
/// `4 · 68 · (2048 + cp_len)`.
pub fn dvbt_super_frame_samples(guard: GuardInterval) -> usize {
    DVB_T_FRAMES_PER_SUPER_FRAME * DVBT_SYMBOLS_PER_FRAME * (DVB_T_N_FFT + guard.cp_len_2k())
}

/// Super-frames to render for a buffer covering [`DVBT_BUFFER_TARGET_SECS`] at
/// the waveform's rate `fs`, clamped to `1..=`[`DVBT_MAX_BUFFER_SUPER_FRAMES`].
///
/// One super-frame carries 333 kHz through 2 MHz (its 1.44-0.24 s already meets
/// or nearly meets the target); the broadcast widths ask for more than the cap
/// allows and get the cap.
pub fn dvbt_buffer_super_frames(guard: GuardInterval, fs: f32) -> usize {
    let per = dvbt_super_frame_samples(guard) as f32;
    if !(fs.is_finite() && fs > 0.0) || per <= 0.0 {
        return 1;
    }
    let want = (DVBT_BUFFER_TARGET_SECS * fs / per).ceil();
    if !want.is_finite() || want < 1.0 {
        return 1;
    }
    (want as usize).min(DVBT_MAX_BUFFER_SUPER_FRAMES)
}

// ── Band centre ─────────────────────────────────────────────────────────────

/// Legal range for the band centre (Hz) in `bandwidth`'s mode.
///
/// The occupied band is `dvb_t_occupied_bw(fs)` wide — 83% of the waveform's own
/// rate — and is symmetric about the upconversion frequency, so the centre must
/// keep both edges inside `0..display_nyquist`.  That is the *physical* bound:
/// past it the real projection folds.  It is deliberately wider than the framed
/// window, so the band can be tuned into the oversampling headroom rather than
/// being pinned to the part of the spectrum that happens to be on screen.
///
/// **These take the mode, not a rate**, and that is a fix rather than a
/// convenience: DVB-T carries two rates that differ by a per-mode factor, and
/// every earlier signature here took "a rate" and left the caller to pick.  One
/// caller picked wrong and drew the band hard against the left edge of the
/// display with every number self-consistent.  A `DvbTBandwidth` cannot be
/// passed wrongly.
///
/// Returns a degenerate midpoint pair if the band cannot fit at all, which
/// [`DVBT_MIN_DISPLAY_OVERSAMPLE`] rules out but which keeps the range
/// non-inverted for a pathological rate.
pub fn dvbt_center_bounds(bandwidth: DvbTBandwidth) -> (f32, f32) {
    let half = dvb_t_occupied_bw(bandwidth.fs()) / 2.0;
    let nyquist = bandwidth.display_nyquist_hz();
    let (lo, hi) = (half, nyquist - half);
    if lo <= hi {
        (lo, hi)
    } else {
        (nyquist / 2.0, nyquist / 2.0)
    }
}

/// Default band centre (Hz): the middle of the **framed** window, not of the
/// display's full Nyquist.
///
/// The two are the same thing only when the oversampling lands exactly on the
/// group span, which it does at 2M and 8M and nowhere else.  Centring on the
/// Nyquist instead would put the band off to one side of a window it more than
/// fits in — at 333k, 100 kHz above the frame's own centre.
pub fn dvbt_default_center_hz(bandwidth: DvbTBandwidth) -> f32 {
    bandwidth.display_span_hz() / 2.0
}

/// A requested band centre, clamped to [`dvbt_center_bounds`].  A non-finite
/// request falls back to the default centre rather than poisoning the rotator.
pub fn dvbt_clamp_center(center_hz: f32, bandwidth: DvbTBandwidth) -> f32 {
    let (lo, hi) = dvbt_center_bounds(bandwidth);
    if center_hz.is_finite() {
        center_hz.clamp(lo, hi)
    } else {
        dvbt_default_center_hz(bandwidth).clamp(lo, hi)
    }
}

// ── HUD helper ──────────────────────────────────────────────────────────────

/// Submode line for the top HUD, e.g. " 1M 1/32 QPSK 3/4 shp 1/4·60dB".
pub fn hud_submode_str(
    bandwidth: DvbTBandwidth,
    link: DvbTLinkParams,
    shaping: &DvbTShaping,
) -> String {
    let mut s = format!(
        " {} {} {} {}",
        bandwidth.label(),
        guard_label(link.guard),
        constellation_label(link.constellation),
        code_rate_label(link.code_rate),
    );
    if shaping.enabled {
        s.push_str(&format!(
            " shp {}·{}",
            shaping.taper.label(),
            shaping.mask.hud_label()
        ));
    }
    s
}

// ── DvbTSource ───────────────────────────────────────────────────────────────

/// Conformant DVB-T 2K signal source (ETSI EN 300 744).
///
/// Pre-renders a looping buffer of back-to-back super-frames — four 68-symbol
/// frames each, TPS-signalled, preamble-less — via [`DvbTSuperFrameMod`],
/// interpolates it to the display rate, and emits the real part of the
/// upconverted IQ.  Playback alternates a `sig_secs` signal phase with a
/// `gap_secs` silence phase, timed by wall-clock `dt` exactly as
/// [`CofdmSource`](crate::source::CofdmSource) is, so a "Gap 2 s" setting yields
/// a ~2 s pause regardless of frame rate.
pub struct DvbTSource {
    pub sig_secs: f32,
    pub gap_secs: f32,
    noise: CnNoise,
    bandwidth: DvbTBandwidth,
    link: DvbTLinkParams,
    shaping: DvbTShaping,
    /// Band centre (Hz) the baseband buffer is upconverted to, in display-rate
    /// terms.
    center_hz: f32,
    /// Looping signal buffer, **complex baseband, noise-free, at the display
    /// rate**.  Stored pre-upconversion because the receiver needs an analytic
    /// signal; see [`crate::source::dvbt::rx`].
    ///
    /// Its even-indexed samples are exactly the waveform's own — the
    /// interpolator is a half-band design, so it passes them through unchanged
    /// (see [`render`](Self::render)).  That is what lets the decoder read the
    /// native-rate stream straight out of the display buffer.
    iq: Vec<C32>,
    /// Wrapping read cursor into `iq` during the signal phase.  `iq.len()` is
    /// always even, so the even/odd phase survives the wrap.
    pos: usize,
    /// Upconversion oscillator at the display rate, advanced once per emitted
    /// sample and never reset mid-run, so there is no phase step at a block,
    /// loop or phase boundary.
    rot: Rotator,
    /// Impaired complex baseband at the **waveform's** rate for the block most
    /// recently returned by `next_samples` — the even-indexed samples of it.
    last_iq: Vec<C32>,
    /// Scalar `render` applied to reach [`DVBT_DISPLAY_RMS_DBFS`].
    display_gain: f32,
    /// Largest complex magnitude in the rendered buffer — the amplitude this
    /// source counts as 0 dBFS.  See [`full_scale`](Self::full_scale).
    full_scale: f32,
    /// TS payload bytes each frame carries — the frame-filling size for the
    /// current link, which the receiver must be told.
    frame_payload_len: usize,
    /// True during the signal phase, false during the silence gap.
    in_signal: bool,
    /// Wall-clock seconds elapsed in the current phase.
    phase_secs: f32,
    rng: u64,
}

impl DvbTSource {
    pub fn new(
        sig_secs: f32,
        gap_secs: f32,
        cn_db: f32,
        bandwidth: DvbTBandwidth,
        link: DvbTLinkParams,
        shaping: DvbTShaping,
        center_hz: f32,
    ) -> Self {
        debug_assert!(
            is_dvb_t_constellation(link.constellation),
            "DVB-T carries QPSK, 16-QAM or 64-QAM only"
        );
        let fs_display = bandwidth.display_fs();
        let center_hz = dvbt_clamp_center(center_hz, bandwidth);
        let mut src = Self {
            sig_secs,
            gap_secs,
            noise: CnNoise::new(cn_db, cn_reference(0.0, 0.0, bandwidth.fs())),
            bandwidth,
            link,
            shaping,
            center_hz,
            iq: Vec::new(),
            pos: 0,
            rot: Rotator::new(center_hz, fs_display),
            last_iq: Vec::new(),
            display_gain: 1.0,
            full_scale: 1.0,
            frame_payload_len: 0,
            in_signal: true,
            phase_secs: 0.0,
            rng: 0x2545_f491_4f6c_dd1d,
        };
        src.render();
        src
    }

    /// Band centre (Hz), after clamping to [`dvbt_center_bounds`].
    pub fn center_hz(&self) -> f32 {
        self.center_hz
    }

    /// The channel bandwidth mode.
    pub fn bandwidth(&self) -> DvbTBandwidth {
        self.bandwidth
    }

    /// The waveform's own sample rate (Hz) — what the receiver and the C/N
    /// reference speak, half of what [`sample_rate`](SignalSource::sample_rate)
    /// reports.
    pub fn waveform_fs(&self) -> f32 {
        self.bandwidth.fs()
    }

    /// The link's guard / constellation / code rate.
    pub fn link(&self) -> DvbTLinkParams {
        self.link
    }

    /// The shaping actually rendered.
    pub fn effective_shaping(&self) -> DvbTShaping {
        self.shaping.effective()
    }

    /// Requested carrier-to-noise ratio, in dB.
    pub fn cn_db(&self) -> f32 {
        self.noise.cn_db()
    }

    /// Per-component standard deviation of the injected complex noise.
    pub fn noise_sigma(&self) -> f32 {
        self.noise.sigma()
    }

    /// The display scalar `render` derived for the current configuration.
    pub fn display_gain(&self) -> f32 {
        self.display_gain
    }

    /// The amplitude that counts as 0 dBFS for this source: the largest complex
    /// magnitude in the rendered buffer, which bounds the real projection at
    /// every rotator phase.
    ///
    /// **Not 1.0, and this is the source that needs the distinction back.**
    /// Every other source is unit-scale, so `CofdmFacts::full_scale` has been
    /// 1.0 since COFDM's fitted gain was replaced by a derived one.  DVB-T's
    /// crest factor is 29-33 dB (see [`DVBT_DISPLAY_RMS_DBFS`]), so an RMS
    /// normalised to any level a unit-scale threshold would accept puts the
    /// peaks well past 1.0 — and referencing dBFS to 1.0 would then report a
    /// permanent overload for a waveform that is behaving exactly as its coding
    /// chain makes it behave.
    ///
    /// Referenced here instead, `lvl` reads the burst's crest factor as a
    /// negative dBFS, `peak` reads ~0, and `overload` regains a meaning: it
    /// fires when the *impaired* stream exceeds the clean buffer's swing, which
    /// is something noise can genuinely do at a low C/N.
    pub fn full_scale(&self) -> f32 {
        self.full_scale
    }

    /// Length of the rendered display buffer, in complex samples.  Exposed for
    /// the tests that pin the memory the oversampling factor costs.
    pub fn buffer_len(&self) -> usize {
        self.iq.len()
    }

    /// TS payload bytes per frame in the rendered buffer.  The receiver is built
    /// with this: a stream demodulator trims each frame's recovered payload to a
    /// length it is told, so a mismatch silently truncates.
    pub fn frame_payload_len(&self) -> usize {
        self.frame_payload_len
    }

    /// True while in the signal phase (exposed for tests / decode gating).
    pub fn in_signal(&self) -> bool {
        self.in_signal
    }

    /// Occupied RF bandwidth (Hz) of the transmitted band: `fs · 1705/2048`
    /// against the *waveform's* rate.
    pub fn occupied_bw_hz(&self) -> f32 {
        dvb_t_occupied_bw(self.waveform_fs())
    }

    /// Render the looping buffer: enough back-to-back super-frames to cover
    /// [`DVBT_BUFFER_TARGET_SECS`], interpolated to the display rate and
    /// normalised to the display target.
    ///
    /// **Stops at complex baseband.**  Noise and upconversion both happen at
    /// read time, so the real projection the display consumes and the complex
    /// baseband the decoder consumes come from one impaired sample each.
    fn render(&mut self) {
        let shaping = self.effective_shaping();
        let params = DvbTSuperFrameParams {
            link: self.link,
            cell_id: DVBT_CELL_ID,
        };
        let mut modu = DvbTSuperFrameMod::new(params);
        let roll_off = shaping.roll_off();
        if roll_off > 0 {
            modu = modu.with_symbol_window(roll_off);
        }
        if let Some(mask) = shaping.mask_filter() {
            // Not `TxLowpass::fits_guard`, which tests `min(cp_len − b, b)` — the
            // budget of a *backed-off* window, and this receiver does not back
            // off.  The bound that applies is the one `DVBT_SHAPING_SLACK`
            // states: the mask's spread plus the taper must sit inside the
            // shortest cyclic prefix, so the equalizer absorbs it as ordinary
            // channel dispersion.
            debug_assert!(
                roll_off + mask.group_delay() <= DVBT_SHAPING_SLACK,
                "shaping overran the guard budget"
            );
            modu = modu.with_tx_lowpass(mask);
        }

        // Each frame carries the frame-filling payload, so the super-frame is
        // handed four times that — the driver splits it into four contiguous
        // parts, one per frame.  See `dvbt_frame_payload_bytes` for why the
        // frame is filled rather than sparsely loaded.
        self.frame_payload_len = dvbt_frame_payload_bytes(self.link);
        let super_frame_payload = self.frame_payload_len * DVB_T_FRAMES_PER_SUPER_FRAME;
        let n_super = dvbt_buffer_super_frames(self.link.guard, self.waveform_fs());

        // Each super-frame carries a fresh deterministic pseudo-random payload so
        // the spectrum stays fully populated and the loop point is not obvious.
        // The mask is applied per super-frame by the modulator, which filters
        // across its own three interior frame seams; the seams *between*
        // super-frames are symbol boundaries like any other.
        let mut native: Vec<C32> =
            Vec::with_capacity(n_super * dvbt_super_frame_samples(self.link.guard));
        for _ in 0..n_super {
            let payload = self.build_payload(super_frame_payload);
            native.extend_from_slice(&modu.modulate(&payload).iq);
        }

        // Up to the display rate.  See `DVBT_MIN_DISPLAY_OVERSAMPLE` for why any
        // of this is needed — at the waveform's own rate the band is wider than
        // the one-sided display span and folds over itself — and
        // `DvbTBandwidth::display_oversample` for why the factor is per-mode.
        let up = self.bandwidth.display_oversample();
        let mut iq = interpolate(&native, up);

        // Display scaling, derived from what was actually rendered.  The target
        // is a real-projection RMS of `DVBT_DISPLAY_RMS_DBFS`, so `RMS_real =
        // sqrt(P_complex / 2)` is the quantity to normalise, and it is applied
        // once across the whole buffer so every symbol is scaled alike.
        let unit_rms = (mean_power_c(&iq) / 2.0).sqrt();
        self.display_gain = if unit_rms > 0.0 {
            display_target_rms() / unit_rms
        } else {
            1.0
        };
        for c in &mut iq {
            *c *= self.display_gain;
        }
        // The swing this waveform actually reaches, for `full_scale`.  Taken on
        // the complex magnitude rather than the real projection, so it bounds
        // the output at every rotator phase rather than at the one this render
        // happened to see.
        self.full_scale = iq
            .iter()
            .fold(0.0f32, |m, c| m.max(c.norm_sqr()))
            .sqrt()
            .max(f32::MIN_POSITIVE);

        // The C/N reference: the mean power of the samples the **decoder** reads,
        // which is the even-indexed subsequence, at the **waveform's** rate.
        //
        // Whole-buffer rather than a payload window, and here that is simply
        // correct rather than an approximation: COFDM must exclude its Schmidl &
        // Cox preamble because the preamble is deliberately hotter than the
        // payload, and a DVB-T frame is preamble-less.  The boosted pilots (±4/3,
        // so 16/9 power, on ~204 of 1705 carriers) lift the mean by ~0.3 dB
        // uniformly at every bandwidth — a constant offset, not a cross-mode
        // tilt, which is what a ratio exists to remove.
        //
        // **The rate is the waveform's, not the display's.**  Noise is injected
        // per emitted sample, so its power lands in `fs_display`; the decoder
        // then reads every other sample, which aliases that same power into
        // `fs_waveform`.  Referencing to the waveform's rate is what makes the
        // requested C/N the one the *decoder* experiences — the number that
        // decides whether frames decode.  See `dvbt::decode` for the display
        // side, where the two factors of two cancel exactly.
        let signal_power = mean_power_at(&iq, up);
        let fs_wave = self.waveform_fs();
        self.noise.set_reference(cn_reference(
            signal_power,
            dvb_t_occupied_bw(fs_wave),
            fs_wave,
        ));

        self.iq = iq;
        self.pos = 0;
    }

    /// Build a deterministic pseudo-random TS payload of `len` bytes.
    fn build_payload(&mut self, len: usize) -> Vec<u8> {
        (0..len).map(|_| (self.next_u64() & 0xff) as u8).collect()
    }

    /// Apply fresh parameters.
    ///
    /// The re-render condition is the *waveform* set — bandwidth, link
    /// parameters and effective shaping.  `sig_secs` / `gap_secs` are wall-clock
    /// phase durations applied live and `cn_db` is arithmetic on the cached
    /// reference, so neither rebuilds a buffer that costs FEC encoding, hundreds
    /// of 2048-point IFFTs and an interpolation pass.
    ///
    /// **The centre is not on the re-render path, unlike COFDM's.**  There the
    /// centre clamps the edge guard and so can change the carrier plan; here the
    /// occupied band is fixed at 1705/2048 of the waveform's rate, so a retune
    /// moves the rotator and nothing else.  Bandwidth *is* on it, and moves the
    /// rate with it — which re-clamps the centre and rebuilds the rotator too.
    // One over the threshold, and grouping would not help: `DvbTLinkParams`
    // already collapses the three waveform knobs that belong together, and the
    // rest are independent settings rows that reach here from different places.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_params(
        &mut self,
        sig_secs: f32,
        gap_secs: f32,
        cn_db: f32,
        bandwidth: DvbTBandwidth,
        link: DvbTLinkParams,
        shaping: DvbTShaping,
        center_hz: f32,
    ) {
        let fs_display = bandwidth.display_fs();
        let center_hz = dvbt_clamp_center(center_hz, bandwidth);
        let rerender = bandwidth != self.bandwidth
            || link != self.link
            || shaping.effective() != self.effective_shaping();
        let retuned = center_hz != self.center_hz || bandwidth != self.bandwidth;
        self.sig_secs = sig_secs;
        self.gap_secs = gap_secs;
        self.bandwidth = bandwidth;
        self.link = link;
        self.shaping = shaping;
        self.center_hz = center_hz;
        if retuned {
            self.rot = Rotator::new(center_hz, fs_display);
        }
        if rerender {
            self.render();
        }
        // After any re-render, so the C/N is derived against the geometry the new
        // buffer implies rather than the outgoing one's.
        self.noise.set_cn_db(cn_db);
    }

    fn next_u64(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }
}

impl SignalSource for DvbTSource {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn restart(&mut self) {
        self.pos = 0;
        self.in_signal = true;
        self.phase_secs = 0.0;
    }

    /// Advance the signal/gap phase timer by `dt` seconds and flip the phase when
    /// it reaches the current phase's duration.  Frame-rate independent.
    fn advance_time(&mut self, dt_secs: f32) {
        self.phase_secs += dt_secs;
        if self.in_signal && is_continuous_sig(self.sig_secs) {
            return;
        }
        let limit = if self.in_signal {
            self.sig_secs
        } else {
            self.gap_secs
        };
        if self.phase_secs >= limit {
            self.phase_secs = 0.0;
            self.in_signal = !self.in_signal;
            if self.in_signal {
                self.pos = 0;
            }
        }
    }

    /// Emits `n` real samples at the **display** rate, and records the
    /// waveform-rate complex baseband underneath them for
    /// [`last_samples_iq`](SignalSource::last_samples_iq).
    ///
    /// Each sample is impaired **once**, at baseband, and the real output is the
    /// projection of that same impaired sample:
    ///
    /// ```text
    /// iq[m]   = buffer[pos] + noise[m]
    /// real[m] = re(iq[m] * exp(j*2*pi*f0*m/fs_display))
    /// ```
    ///
    /// with the decoder handed the subsequence at even `pos`, which the half-band
    /// interpolator guarantees are the waveform's own samples.  So
    /// `real[2k] == re(iq_decoder[k] * exp(j*2*pi*f0*2k/fs_display))` holds by
    /// construction — the identity the tests assert, and the reason the decoder
    /// and the display cannot disagree about what was transmitted.
    ///
    /// Noising the real stream and handing the decoder the clean render buffer
    /// would report `CBER`/`IBER` of exactly zero at every C/N while the spectrum
    /// on screen was visibly noisy.
    fn next_samples(&mut self, n: usize) -> Vec<f32> {
        let up = self.bandwidth.display_oversample();
        let mut out = Vec::with_capacity(n);
        self.last_iq.clear();
        self.last_iq.reserve(n.div_ceil(up));
        let len = self.iq.len();
        let live = self.in_signal && len > 0;
        for _ in 0..n {
            // Silence gap (or empty buffer): noise only.  The cursor still
            // advances so the even/odd phase — and with it which samples the
            // decoder is handed — does not depend on the phase history.
            let at_wave_sample = self.pos.is_multiple_of(up);
            let mut c = if live {
                self.iq[self.pos]
            } else {
                C32::default()
            };
            if len > 0 {
                self.pos = (self.pos + 1) % len; // loop the content buffer
            }
            c.re += self.noise.next();
            c.im += self.noise.next();
            let r = self.rot.next();
            out.push(c.re * r.re - c.im * r.im);
            if at_wave_sample {
                self.last_iq.push(c);
            }
        }
        out
    }

    fn last_samples_iq(&self) -> Option<&[C32]> {
        Some(&self.last_iq)
    }

    fn signal_phase(&self) -> Option<bool> {
        Some(self.in_signal)
    }

    /// The **display** rate — [`DvbTBandwidth::display_oversample`] times the
    /// waveform's, so the band fits the one-sided span with room for the group's
    /// fixed width.  `ViewApp::apply_source_sample_rate` re-derives the display
    /// Nyquist from this.
    fn sample_rate(&self) -> f32 {
        self.bandwidth.display_fs()
    }
}

// ── ×2 interpolation ────────────────────────────────────────────────────────

/// Interpolates `native` to `up` times its rate, exactly preserving the input
/// samples at output indices that are multiples of `up`.
///
/// **The "exactly" is the point, and it does not depend on `up` being two.**  A
/// windowed sinc with its −6 dB cutoff at `1/(2·up)` of the output rate has
/// `sinc(n/up) = 0` at every non-zero multiple of `up`, whatever the window — so
/// scaling the prototype to unit centre tap makes `y[k·up] == x[k]`
/// bit-for-bit.  At `up = 2` that is the familiar half-band identity; the same
/// argument carries to 3, 4 and 12, which is what lets the six bandwidth modes
/// oversample by different factors while the decoder still reads the waveform's
/// own samples straight out of the display buffer.
///
/// Phase 0 is a copy and costs nothing; the other `up − 1` phases each use their
/// own `1/up` share of the prototype, so the work per *output* sample is
/// `n_taps / up` regardless of `up`.  The transition band runs from the
/// outermost active carrier (`±852/2048` of the input rate) to its first image,
/// which fixes the tap count through [`kaiser_num_taps`].
fn interpolate(native: &[C32], up: usize) -> Vec<C32> {
    debug_assert!(up >= 1, "oversampling factor must be positive");
    let mut out = vec![C32::default(); native.len() * up];
    if native.is_empty() || up == 1 {
        out.copy_from_slice(native);
        return out;
    }
    for (k, c) in native.iter().enumerate() {
        out[k * up] = *c;
    }

    // Passband edge is the outermost active carrier as a fraction of the OUTPUT
    // rate; the first image starts at `1/up` minus the same, so the transition
    // is what is left between them and is centred on the `1/(2·up)` cutoff.
    let up_f = up as f32;
    let pass_norm = (DVB_T_KMAX as f32 / 2.0) / (DVB_T_N_FFT as f32 * up_f);
    let cutoff = 0.5 / up_f;
    let trans_norm = 1.0 / up_f - 2.0 * pass_norm;
    let n_taps = kaiser_num_taps(trans_norm, DVBT_INTERP_STOPBAND_DB);
    let proto = kaiser_lowpass_taps(n_taps, cutoff, DVBT_INTERP_STOPBAND_DB);
    let mid = proto.len() / 2;
    // Unit centre tap, so phase-0 outputs pass through unscaled and the rest
    // carry the same passband gain.
    let scale = if proto[mid].abs() > f32::EPSILON {
        1.0 / proto[mid]
    } else {
        1.0
    };

    // One branch per non-zero phase, as (input offset, tap) pairs: output
    // `k·up + p` reads `x[k - r]` through `proto[mid + p + r·up]`.
    let branch = |p: usize| -> Vec<(isize, f32)> {
        let mid = mid as isize;
        (0..proto.len() as isize)
            .filter(|t| (t - mid - p as isize).rem_euclid(up as isize) == 0)
            .map(|t| {
                (
                    (t - mid - p as isize) / up as isize,
                    proto[t as usize] * scale,
                )
            })
            .collect()
    };
    for p in 1..up {
        let taps = branch(p);
        let (r_min, r_max) = (
            taps.iter().map(|(r, _)| *r).min().unwrap_or(0),
            taps.iter().map(|(r, _)| *r).max().unwrap_or(0),
        );
        // Interior first, without bounds checks: the buffer runs to millions of
        // samples and this is the only loop in `render` proportional to both
        // length and tap count.
        let lo = r_max.max(0) as usize;
        let hi = native.len().saturating_sub(r_min.unsigned_abs());
        for k in lo..hi {
            let mut acc = C32::default();
            for (r, tap) in &taps {
                acc += native[(k as isize - r) as usize] * *tap;
            }
            out[k * up + p] = acc;
        }
        // Edges, where the taps reach outside the buffer (treated as zero).  The
        // loop point is a discontinuity anyway; this makes the first and last few
        // samples slightly soft rather than wrong.
        for k in (0..lo).chain(hi..native.len()) {
            let mut acc = C32::default();
            for (r, tap) in &taps {
                let idx = k as isize - r;
                if idx >= 0 && (idx as usize) < native.len() {
                    acc += native[idx as usize] * *tap;
                }
            }
            out[k * up + p] = acc;
        }
    }
    out
}

// ── Derived display level and C/N geometry ──────────────────────────────────

/// Target real-projection RMS implied by [`DVBT_DISPLAY_RMS_DBFS`].
fn display_target_rms() -> f32 {
    10f32.powf(DVBT_DISPLAY_RMS_DBFS / 20.0)
}

/// Mean power of every `up`-th sample — the waveform's own, and so the ones the
/// decoder reads.
fn mean_power_at(iq: &[C32], up: usize) -> f32 {
    let mut sum = 0.0f32;
    let mut count = 0usize;
    for c in iq.iter().step_by(up.max(1)) {
        sum += c.norm_sqr();
        count += 1;
    }
    if count == 0 { 0.0 } else { sum / count as f32 }
}

/// The C/N geometry for a DVB-T burst.
///
/// [`NoiseDomain::Complex`]: the generator adds independent noise to both
/// components of the baseband sample, so its power is white over the **full**
/// rate, not over a Nyquist half-span.  `fs` here is the *waveform's* rate — see
/// [`DvbTSource::render`].
fn cn_reference(signal_power: f32, occupied_bw_hz: f32, fs: f32) -> CnReference {
    CnReference {
        signal_power,
        occupied_bw_hz,
        fs,
        domain: NoiseDomain::Complex,
    }
}
