// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use num_complex::Complex32 as C32;
use orion_sdr::dsp::Rotator;
use orion_sdr::fec::{CrcKind, FrameMetadata, FramePacket, InnerFec, PunctureRate};
use orion_sdr::modulate::{ConstellationOrder, McsTable, OfdmConfig, OfdmFrameMod};
use orion_sdr::multicarrier::{CarrierPlan, TxLowpass};
use orion_sdr::sync::OfdmPreamble;

use crate::source::{CnNoise, CnReference, MAX_SIG_SECS, NoiseDomain, SignalSource, mean_power_c};

// ── COFDM constants ───────────────────────────────────────────────────────────
//
// COFDM is a synthetic *wideband* coded-OFDM source.  Unlike the narrowband
// sources it does not sit near a single tunable *carrier* — an OFDM band has no
// carrier, the DC subcarrier being null by convention — but it does sit
// somewhere: it occupies a sub-band centred at `center_hz` and runs at its own
// sample rate, which the viewer adopts per-source (see
// `ViewApp::apply_source_sample_rate`).  Both are configurable; the defaults are
// `fs/4` (mid-display) and `COFDM_DEFAULT_FS`.  The signal is rendered natively
// at `fs` — there is NO resampling, so the source mirrors the PSK31
// single-play→gap→repeat shape rather than FT8's resample+shift path.
//
// **Modulation is at baseband, upconversion is ours.**  `OfdmConfig`'s `rf_hz`
// applies its rotation *inside* `OfdmMod::process`, i.e. per symbol, before
// `OfdmFrameMod::modulate_frame` runs its spectral-shaping post-passes.  The
// symbol-window taper is a real-valued magnitude ramp and commutes with that
// rotation, but `TxLowpass` is a low-pass centered on DC: applied to a stream
// already sitting at the band centre it would delete the signal outright.  So
// the config is built with `rf_hz = 0.0` and `render` upconverts afterwards with
// a single continuous `Rotator`.  Two artifacts of the old arrangement go away
// with it: `generate_ofdm_preamble` ignores its config, so the preamble and
// training symbol used to be emitted at baseband while header/payload sat at the
// band centre; and `map_bits_to_iq` builds a fresh rotator per block, so there
// was a phase step at every header→payload and frame→frame seam.
//
// A centre knob is a change to *that* rotator and nothing else: `rf_hz` stays
// 0.0, which orion-sdr 0.0.58's frame layer asserts, and the receiver consumes
// complex baseband so it needs no retuning at all.

/// OFDM FFT size (number of subcarriers).
pub const COFDM_N_FFT: usize = 256;
/// Cyclic-prefix length in samples.
pub const COFDM_CP_LEN: usize = 32;

/// Largest usable signed carrier index: the Nyquist bin at `-(n_fft/2)` is
/// conventionally null, so the plan spans `±(n_fft/2 - 1)`.
const COFDM_MAX_CARRIER: usize = COFDM_N_FFT / 2 - 1;

/// Widest edge guard the settings row allows: the narrowest bandwidth
/// fraction's own guard (1/8 ⇒ `n_fft/32` carriers per side).
///
/// This is a practical bound, not a numerical one.  Fewer carriers spread the
/// same fixed-size payload over proportionally more OFDM symbols, so the frame
/// — and the 40-frame render buffer with it — grows without bound as the guard
/// approaches `n_fft/2`, for a band too narrow to be worth looking at.
pub const COFDM_MAX_EDGE_GUARD: usize = COFDM_MAX_CARRIER - COFDM_N_FFT / 32;

/// Occupied half-width (carriers per side) at the widest edge guard — the
/// narrowest band the source will render, and so the one that fits at the most
/// extreme band centre.  [`cofdm_center_bounds`] is derived from it.
const COFDM_MIN_OCCUPIED_HALF: usize = COFDM_MAX_CARRIER - COFDM_MAX_EDGE_GUARD;

/// Receiver FFT-window back-off, in samples.  RX-only — it does not change what
/// is transmitted — but it is what makes the TX shaping below transparent, so it
/// is set on the config now: the taper and the mask's group delay live in
/// exactly the guard samples a backed-off window discards.  `cp_len/2` maximizes
/// the resulting slack, `min(cp_len - b, b)`.
///
/// COFDM is the favorable case for this.  The `TrainingSymbolHold` equalizer
/// estimates every bin at full resolution and absorbs any back-off the guard
/// allows; it is DVB-T's scattered-pilot *interpolation* that caps the back-off.
const COFDM_RX_WINDOW_BACKOFF: usize = COFDM_CP_LEN / 2;

/// Guard samples available to TX spectral shaping: `min(cp_len - b, b)`.  The
/// symbol taper and the mask's group delay share this one budget —
/// `roll_off + group_delay ≤ COFDM_SHAPING_SLACK`.
pub const COFDM_SHAPING_SLACK: usize = {
    let a = COFDM_CP_LEN - COFDM_RX_WINDOW_BACKOFF;
    if a < COFDM_RX_WINDOW_BACKOFF {
        a
    } else {
        COFDM_RX_WINDOW_BACKOFF
    }
};

/// Schmidl & Cox preamble geometry: `COFDM_PREAMBLE_REPEATS` copies of a
/// `COFDM_PREAMBLE_REPEAT_LEN`-sample segment.
///
/// The repeat length is set by the spectral mask, not by acquisition: a TX
/// low-pass filters the whole burst, preamble included, and the repetition the
/// receiver correlates on only survives where the taps see repeated samples —
/// so `group_delay ≪ repeat_len`.  The mask's group delay is bounded by
/// `COFDM_SHAPING_SLACK` (16), which a 16-sample repeat would not clear at all;
/// 64 keeps it under ~15%.  Costs 192 extra samples on a frame of several
/// thousand.
const COFDM_PREAMBLE_REPEATS: usize = 4;
const COFDM_PREAMBLE_REPEAT_LEN: usize = 64;

/// Default native sample rate of the COFDM waveform (Hz).  Nyquist = 960 kHz;
/// subcarrier spacing = `fs / COFDM_N_FFT` = 7 500 Hz.
///
/// **A default, not a property.**  The rate is configurable per source
/// (`sources.cofdm.fs_hz`), because a narrowband DVB-T profile is three
/// bandwidth modes over one 2K structure — three sample rates over one
/// numerology.  Everything downstream already takes `fs` as a parameter:
/// [`cofdm_link_config`], [`cofdm_occupied_bw`], the [`Rotator`], and the
/// viewer's `apply_source_sample_rate`.
///
/// What makes a configured rate *safe* is that the impairment is a ratio.
/// While it was an absolute amplitude, changing `fs` would have silently
/// changed the link: the same amplitude spread over twice the bandwidth is
/// 3 dB less noise in the occupied band, with nothing on screen to say so.  A
/// C/N in dB is invariant to the rate by construction — see [`CnReference`].
pub const COFDM_DEFAULT_FS: f32 = 1_920_000.0;

/// Bounds on a configured sample rate.  Wide, because nothing in the render
/// path is rate-dependent beyond the arithmetic — the bounds exist to reject a
/// typo (`fs_hz: 1920` for 1.92 MHz) rather than to express a real limit.
pub const COFDM_MIN_FS: f32 = 48_000.0;
pub const COFDM_MAX_FS: f32 = 20_000_000.0;

/// A configured sample rate, clamped to the supported range.  Non-finite and
/// non-positive values fall back to the default rather than propagating a NaN
/// into every derived frequency.
pub fn cofdm_clamp_fs(fs: f32) -> f32 {
    if fs.is_finite() && fs > 0.0 {
        fs.clamp(COFDM_MIN_FS, COFDM_MAX_FS)
    } else {
        COFDM_DEFAULT_FS
    }
}

/// Subcarrier spacing (Hz) at `fs`.
pub fn cofdm_spacing_hz(fs: f32) -> f32 {
    fs / COFDM_N_FFT as f32
}

/// Default band centre (Hz) at `fs`: Nyquist/2, i.e. mid-display.
///
/// The DC-centered carriers make the occupied band symmetric about the
/// upconversion frequency, so `.re` lands the band centered on the marker.
/// This being mid-display is a *choice of default*, not a property of the
/// waveform — see [`cofdm_center_bounds`] for the range it can move over.
pub fn cofdm_default_center_hz(fs: f32) -> f32 {
    fs / 4.0
}

/// Legal range for the band centre (Hz) at `fs`.
///
/// A centre too close to either end of `0..Nyquist` leaves no room for even the
/// narrowest renderable band, so the outermost carrier would land outside the
/// display window and fold back on itself.  The bound is the narrowest band
/// ([`COFDM_MIN_OCCUPIED_HALF`]) plus the one bin of margin
/// [`cofdm_min_edge_guard`] keeps.
pub fn cofdm_center_bounds(fs: f32) -> (f32, f32) {
    let margin = (COFDM_MIN_OCCUPIED_HALF + 1) as f32 * cofdm_spacing_hz(fs);
    (margin, fs / 2.0 - margin)
}

/// Narrowest edge guard that keeps the whole occupied band inside
/// `0..Nyquist` when the band is centred at `center_hz`.
///
/// **This used to be a constant, and it was a constant only because the centre
/// was.**  With the upconversion pinned at `fs/4` — `n_fft/4` bins from DC —
/// the widest band that fits is `n_fft/4 - 1` carriers per side, giving
/// `COFDM_MAX_CARRIER - (COFDM_N_FFT / 4 - 1)` = 64.  Move the centre and that
/// bound moves with it:
///
/// ```text
/// headroom = min(center, nyquist - center)     // distance to the nearer end
/// max_half = floor(headroom / spacing) - 1     // one bin of margin
/// min_edge_guard = COFDM_MAX_CARRIER - max_half
/// ```
///
/// At `center = fs/4` this reproduces 64 exactly, which is the check that says
/// the generalisation is a generalisation rather than a rewrite — see
/// `the_generalised_guard_bound_reproduces_the_old_constant`.
///
/// The result never exceeds [`COFDM_MAX_EDGE_GUARD`], so the clamp range it
/// bounds is always non-empty; a centre outside [`cofdm_center_bounds`] simply
/// pins to the narrowest band rather than producing an inverted range.
pub fn cofdm_min_edge_guard(center_hz: f32, fs: f32) -> usize {
    COFDM_MAX_CARRIER - cofdm_max_occupied_half(center_hz, fs)
}

/// Widest occupied half-width (carriers per side) that fits at `center_hz`.
fn cofdm_max_occupied_half(center_hz: f32, fs: f32) -> usize {
    let spacing = cofdm_spacing_hz(fs);
    let headroom = center_hz.min(fs / 2.0 - center_hz);
    if !(spacing.is_finite() && spacing > 0.0) || !headroom.is_finite() {
        return COFDM_MIN_OCCUPIED_HALF;
    }
    let half = (headroom / spacing).floor() - 1.0;
    if half < COFDM_MIN_OCCUPIED_HALF as f32 {
        return COFDM_MIN_OCCUPIED_HALF;
    }
    (half as usize).min(COFDM_MAX_CARRIER)
}

/// QPSK payload from the default MCS ladder (index 1: BPSK/QPSK/QAM16/QAM64).
const COFDM_MCS_INDEX: u8 = 1;
/// Payload bytes per COFDM frame (RS(204,188)-style block minus a 4-byte CRC).
pub const COFDM_PAYLOAD_BYTES: usize = 184;

/// Number of back-to-back COFDM frames in the looping signal buffer.  This sets
/// the buffer *content* (enough frames that the loop point isn't obvious), not
/// the signal-phase duration — that is timed by real `dt` in `next_samples`.
/// ~40 frames ≈ 0.3 s of native signal at `COFDM_FS`.
pub(crate) const COFDM_BUFFER_FRAMES: usize = 40;

/// Target signal-phase RMS for the rendered burst, in dBFS.
///
/// **The display level is derived to hit this, not tuned to produce it.**  The
/// old arrangement was the other way round: a hand-fitted `COFDM_GAIN` of 121.0
/// scaled every configuration alike, and the reference level was then chosen to
/// suit whatever that produced.  One constant cannot fit — bare OFDM spreads its
/// energy across the active subcarriers, so the rendered power is proportional
/// to the occupied bandwidth, and a single gain left the measured signal-phase
/// RMS spanning 1.344 to 3.646 (a 2.7x spread) across the bandwidth fractions.
///
/// Normalising instead makes the source **unit-scale like every other one**,
/// which is what lets the shared [`orion_sdr::util::SIGNAL_THRESHOLD`] apply to
/// it, lets `CofdmFacts::full_scale` be 1.0, and lets a new multicarrier source
/// get correct scaling with no tuning session.  DFT-s-OFDM is the case that
/// makes this structural rather than tidy: its whole point is lower PAPR, so a
/// COFDM-shaped constant would be wrong for it rather than merely untuned.
///
/// -15 dBFS RMS leaves roughly 10-12 dB of OFDM crest factor inside full scale,
/// so the peaks do not read as an overload.
pub const COFDM_DISPLAY_RMS_DBFS: f32 = -15.0;

/// Display reference level (dBFS, spectrum-scale top) preferred by COFDM.
///
/// Tracks [`COFDM_DISPLAY_RMS_DBFS`]: the burst now sits ~20.6 dB lower than
/// the old fixed gain of 121.0 put it, so the scale top drops by the same
/// amount and the on-screen picture is unchanged.
pub const COFDM_PREFERRED_REF_DB: f32 = -36.0;

/// Default signal-burst duration, in **wall-clock seconds**.
pub const COFDM_DEFAULT_SIG_SECS: f32 = 10.0;
/// Default silence gap between bursts, in **wall-clock seconds**.
pub const COFDM_DEFAULT_GAP_SECS: f32 = 2.0;
/// Default C/N (dB).
///
/// **Chosen so the out-of-band noise floor is visible**, which is the one thing
/// this source exists to show.  The guard, taper and mask rows all shape the
/// skirt outside the occupied band; at the 45 dB this used to sit at, the floor
/// they shape against fell below what the display resolves, so the controls
/// moved a skirt into blackness and looked inert.  10 dB lower puts the floor
/// on screen and gives the shaping something to sit against.
///
/// **It is not near the cliff.**  Every bandwidth fraction decodes with zero
/// frame errors here — see `the_default_cn_decodes_cleanly_at_every_bandwidth`
/// in `tests/cofdm_rx.rs`, which is what keeps this a display choice rather than
/// a link one.  The FEC cliff is around 11-14 dB depending on the fraction; the
/// measured tables are in the 0.0.23 `CHANGELOG` entry.
///
/// Unlike the other five defaults this is *not* the level that reproduces the
/// pre-`C/N` amplitude default — it is deliberately 10 dB noisier.  COFDM's
/// 240 kHz occupancy against noise spread over 1.92 MHz is only a 9 dB
/// spreading factor, where the narrowband sources sit at 20-27 dB, so the same
/// requested ratio buys a much less prominent floor here than there.
pub const COFDM_DEFAULT_CN_DB: f32 = 35.0;

// ── Bandwidth fraction ──────────────────────────────────────────────────────

/// Occupied bandwidth as a fraction of the full display span (Nyquist).  The
/// viewport span is pinned to full Nyquist for COFDM, so this directly controls
/// how much of the display width the band fills.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CofdmBwFraction {
    OneEighth,
    OneQuarter,
    OneThird,
    OneHalf,
    TwoThirds,
    ThreeQuarters,
    SevenEighths,
}

impl CofdmBwFraction {
    /// All variants in display order (matches the settings toggle options).
    pub const ALL: &'static [CofdmBwFraction] = &[
        CofdmBwFraction::OneEighth,
        CofdmBwFraction::OneQuarter,
        CofdmBwFraction::OneThird,
        CofdmBwFraction::OneHalf,
        CofdmBwFraction::TwoThirds,
        CofdmBwFraction::ThreeQuarters,
        CofdmBwFraction::SevenEighths,
    ];

    /// The fraction value in `(0, 1)`.
    pub fn value(self) -> f32 {
        match self {
            CofdmBwFraction::OneEighth => 1.0 / 8.0,
            CofdmBwFraction::OneQuarter => 1.0 / 4.0,
            CofdmBwFraction::OneThird => 1.0 / 3.0,
            CofdmBwFraction::OneHalf => 1.0 / 2.0,
            CofdmBwFraction::TwoThirds => 2.0 / 3.0,
            CofdmBwFraction::ThreeQuarters => 3.0 / 4.0,
            CofdmBwFraction::SevenEighths => 7.0 / 8.0,
        }
    }

    /// Short label for the HUD / settings toggle (e.g. "1/4").
    pub fn label(self) -> &'static str {
        match self {
            CofdmBwFraction::OneEighth => "1/8",
            CofdmBwFraction::OneQuarter => "1/4",
            CofdmBwFraction::OneThird => "1/3",
            CofdmBwFraction::OneHalf => "1/2",
            CofdmBwFraction::TwoThirds => "2/3",
            CofdmBwFraction::ThreeQuarters => "3/4",
            CofdmBwFraction::SevenEighths => "7/8",
        }
    }

    /// Half-width (in subcarriers) of the DC-centered active carrier set for
    /// this fraction: the band spans `±half` about DC, i.e. `2*half` carriers.
    /// Clamped to the plan's usable range `±(n_fft/2 - 1)`.
    ///
    /// **Independent of `fs`, and that is what lets the sample rate be
    /// configured without the bandwidth toggle changing meaning.**  The
    /// fraction is of Nyquist (`fs/2`) and the spacing is `fs/n_fft`, so the
    /// rate cancels: `half = round(fraction * n_fft / 4)`.  "1/4" is a quarter
    /// of the display at every rate.
    fn carrier_half(self) -> i32 {
        let half = (self.value() * COFDM_N_FFT as f32 / 4.0).round() as i32;
        half.clamp(1, (COFDM_N_FFT / 2) as i32 - 1)
    }
}

/// Default bandwidth fraction on startup / reset.
pub const COFDM_DEFAULT_BW_FRACTION: CofdmBwFraction = CofdmBwFraction::OneQuarter;

/// The edge guard (null carriers per band edge) that reproduces `fraction`'s
/// occupied band.  The bandwidth toggle *is* the edge-guard lever: the carrier
/// set it selects, `±1..=±half`, is exactly what
/// `CarrierPlan::with_contiguous_data(COFDM_MAX_CARRIER - half, false)` fills.
pub fn cofdm_edge_guard_for(fraction: CofdmBwFraction) -> usize {
    COFDM_MAX_CARRIER - fraction.carrier_half() as usize
}

/// Outermost occupied carrier, in bins from DC, for an edge guard.
pub fn cofdm_occupied_half(edge_guard: usize) -> usize {
    COFDM_MAX_CARRIER.saturating_sub(edge_guard)
}

/// Occupied bandwidth (Hz) at `fs` for an edge guard:
/// `2 * occupied_half * fs / n_fft`.  Keyed off the guard rather than the
/// bandwidth fraction, since the guard is separately overridable in settings.
pub fn cofdm_occupied_bw(fs: f32, edge_guard: usize) -> f32 {
    let active = (2 * cofdm_occupied_half(edge_guard)) as f32;
    active * fs / COFDM_N_FFT as f32
}

/// Number of *data* carriers in the plan an edge guard produces.
///
/// Read off a real [`CarrierPlan`] rather than re-derived from the guard, so
/// the instrumentation's bit rate cannot drift from the waveform actually
/// transmitted.  This matters more than it looks: carrier counts across the
/// profiles this must eventually serve span two orders of magnitude — the
/// synthetic source is 32 carriers at the default 1/4 fraction, while DVB-T 2K
/// carries 1512 data carriers out of 2048 bins at a reduced sample rate.
/// Anything derived from `n_fft` would be silently wrong for the latter.
pub fn cofdm_data_carriers(edge_guard: usize, include_dc: bool) -> usize {
    CarrierPlan::new(COFDM_N_FFT, COFDM_CP_LEN)
        .with_contiguous_data(edge_guard, include_dc)
        .data_carriers()
        .len()
}

/// The MCS the source transmits, as instrumentation facts: the constellation
/// name, its bits per symbol, and the **inner** code rate as `(k, n)`.
///
/// The outer code (`BCH t=8`) is deliberately not folded into the rate — `CR`
/// and the derived bit rate both advertise the inner code alone.  Note also
/// that the inner code is whatever the MCS selects, not "LDPC": `InnerFec` is
/// `None | Ldpc | Convolutional`, and the default ladder's LDPC entry is a
/// current default, not a property of the format.
pub fn cofdm_mcs_facts() -> (&'static str, usize, (usize, usize)) {
    let mcs = McsTable::default_ladder()
        .get(COFDM_MCS_INDEX)
        .expect("default MCS ladder covers COFDM_MCS_INDEX");
    let name = match mcs.constellation {
        ConstellationOrder::Bpsk => "BPSK",
        ConstellationOrder::Qpsk => "QPSK",
        ConstellationOrder::Qam16 => "QAM16",
        ConstellationOrder::Qam64 => "QAM64",
        ConstellationOrder::Qam256 => "QAM256",
    };
    let rate = match mcs.inner_fec {
        InnerFec::None => (1, 1),
        InnerFec::Ldpc(code) => (code.k(), code.n()),
        InnerFec::Convolutional { rate, .. } => match rate {
            PunctureRate::R1_2 => (1, 2),
            PunctureRate::R2_3 => (2, 3),
            PunctureRate::R3_4 => (3, 4),
            PunctureRate::R5_6 => (5, 6),
            PunctureRate::R7_8 => (7, 8),
        },
    };
    (name, mcs.constellation.bits_per_symbol(), rate)
}

// ── Spectral shaping ────────────────────────────────────────────────────────

/// Symbol-window roll-off, as a fraction of the guard (cyclic prefix).
///
/// There is deliberately **no `1/2` option** even though `cp_len/2` is the
/// maximum RX-transparent taper: a roll-off of 16 consumes the whole of
/// [`COFDM_SHAPING_SLACK`], leaving zero group delay for the mask, which would
/// silently drop the mask while the settings row still named a stop-band depth.
/// Capping at `3/8` keeps at least 4 samples of delay — a 9-tap filter — for it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CofdmTaper {
    Off,
    Eighth,
    Quarter,
    ThreeEighths,
}

impl CofdmTaper {
    /// All variants in display order (matches the settings toggle options).
    pub const ALL: &'static [CofdmTaper] = &[
        CofdmTaper::Off,
        CofdmTaper::Eighth,
        CofdmTaper::Quarter,
        CofdmTaper::ThreeEighths,
    ];

    /// Raised-cosine taper length per symbol edge, in samples.
    pub fn roll_off(self) -> usize {
        match self {
            CofdmTaper::Off => 0,
            CofdmTaper::Eighth => COFDM_CP_LEN / 8,
            CofdmTaper::Quarter => COFDM_CP_LEN / 4,
            CofdmTaper::ThreeEighths => 3 * COFDM_CP_LEN / 8,
        }
    }

    /// Short label for the HUD / settings toggle.
    pub fn label(self) -> &'static str {
        match self {
            CofdmTaper::Off => "off",
            CofdmTaper::Eighth => "1/8",
            CofdmTaper::Quarter => "1/4",
            CofdmTaper::ThreeEighths => "3/8",
        }
    }
}

/// Baseband spectral-mask stop-band depth.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CofdmMask {
    Off,
    Db40,
    Db60,
    Db80,
}

impl CofdmMask {
    /// All variants in display order (matches the settings toggle options).
    pub const ALL: &'static [CofdmMask] = &[
        CofdmMask::Off,
        CofdmMask::Db40,
        CofdmMask::Db60,
        CofdmMask::Db80,
    ];

    /// Kaiser stop-band attenuation target, or `None` when the mask is off.
    pub fn stopband_db(self) -> Option<f32> {
        match self {
            CofdmMask::Off => None,
            CofdmMask::Db40 => Some(40.0),
            CofdmMask::Db60 => Some(60.0),
            CofdmMask::Db80 => Some(80.0),
        }
    }

    /// Short label for the HUD / settings toggle.
    pub fn label(self) -> &'static str {
        match self {
            CofdmMask::Off => "off",
            CofdmMask::Db40 => "40 dB",
            CofdmMask::Db60 => "60 dB",
            CofdmMask::Db80 => "80 dB",
        }
    }
}

/// Default taper and mask when shaping is enabled.
pub const COFDM_DEFAULT_TAPER: CofdmTaper = CofdmTaper::Quarter;
pub const COFDM_DEFAULT_MASK: CofdmMask = CofdmMask::Db60;
/// Shaping is on by default.
pub const COFDM_DEFAULT_SHAPING_ENABLED: bool = true;

/// The out-of-band spectral-shaping parameter set.
///
/// Three levers, all off by default in `orion-sdr` and composed here: the
/// edge-carrier guard (fewer carriers, so the strongest `sinc` generators move
/// inward), the symbol-window taper (softens the symbol seam; acts on the near
/// skirt), and the baseband mask (a FIR low-pass over the composite stream;
/// acts far out, and is the only lever not bounded by the taper's ceiling).
///
/// Grouped into a struct so `CofdmSource::new` / `apply_params` stay under the
/// clippy argument threshold and so `!=` decides the re-render.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CofdmShaping {
    pub enabled: bool,
    /// Null carriers per band edge.  Seeded from the bandwidth fraction, then
    /// overridable — see [`cofdm_edge_guard_for`].
    pub edge_guard: usize,
    /// Occupy the DC subcarrier (null by default, as OFDM convention has it).
    pub include_dc: bool,
    pub taper: CofdmTaper,
    pub mask: CofdmMask,
}

impl CofdmShaping {
    /// The shaping-disabled configuration for a fraction: the guard the
    /// bandwidth toggle implies, no DC, no taper, no mask.  This is the carrier
    /// layout and shaping state the source had before shaping existed — though
    /// not sample-for-sample the old buffer, since the move to baseband
    /// modulation also upconverted the preamble and removed the per-block
    /// rotator phase steps (see the module header).
    pub fn derived(fraction: CofdmBwFraction) -> Self {
        Self {
            enabled: false,
            edge_guard: cofdm_edge_guard_for(fraction),
            include_dc: false,
            taper: CofdmTaper::Off,
            mask: CofdmMask::Off,
        }
    }

    /// The enabled defaults for a fraction.
    pub fn default_for(fraction: CofdmBwFraction) -> Self {
        Self {
            enabled: COFDM_DEFAULT_SHAPING_ENABLED,
            edge_guard: cofdm_edge_guard_for(fraction),
            include_dc: false,
            taper: COFDM_DEFAULT_TAPER,
            mask: COFDM_DEFAULT_MASK,
        }
    }

    /// What is actually rendered: this set with its edge guard clamped to what
    /// fits at `center_hz`, or [`derived`](Self::derived) with the same clamp
    /// when shaping is off.  One resolver rather than a per-field pile, so
    /// every consumer — the renderer and the Di bar's bandwidth readout —
    /// agrees.
    ///
    /// **The centre and the guard are one constraint, not two.**  Nudging
    /// either can invalidate the other, so both are resolved here rather than
    /// clamped separately at the settings rows.  A consequence worth stating:
    /// an off-centre band cannot be as wide as a centred one, so the wider
    /// bandwidth *fractions* stop being reachable as the centre moves out.  The
    /// fraction remains a label — the Di bar's `BW` readout, which is keyed off
    /// the guard this returns, is authoritative for what is transmitted.
    ///
    /// The disabled branch is clamped too, which it did not need to be while
    /// the centre was fixed: every fraction's own guard clears the old constant
    /// bound of 64, so `derived` could never fold.  Off centre it can.
    pub fn effective(&self, fraction: CofdmBwFraction, center_hz: f32, fs: f32) -> Self {
        let base = if self.enabled {
            *self
        } else {
            Self::derived(fraction)
        };
        Self {
            edge_guard: base
                .edge_guard
                .clamp(cofdm_min_edge_guard(center_hz, fs), COFDM_MAX_EDGE_GUARD),
            ..base
        }
    }

    /// The mask spec for a plan whose outermost occupied carrier sits
    /// `occupied_half` bins from DC, or `None` when the mask is off.
    ///
    /// `taps_for_null_band` returns the shortest filter whose transition reaches
    /// the stop band inside the null band; that answers only one of the two
    /// constraints, so the result is clamped to what the guard budget leaves
    /// after the taper (`roll_off + group_delay ≤ COFDM_SHAPING_SLACK`).  A
    /// clamped filter is shorter than ideal, and `for_null_band` then centers
    /// its transition rather than pushing it against the band edge — a shallower
    /// mask, not a broken one.
    pub fn mask_filter(&self, occupied_half: usize) -> Option<TxLowpass> {
        if !self.enabled {
            return None;
        }
        let stopband_db = self.mask.stopband_db()?;
        // Cannot underflow: `CofdmTaper` caps `roll_off` below the slack.
        let max_delay = COFDM_SHAPING_SLACK.checked_sub(self.taper.roll_off())?;
        let taps = TxLowpass::taps_for_null_band(COFDM_N_FFT, occupied_half, stopband_db)
            .min(2 * max_delay + 1);
        (taps >= 3).then(|| TxLowpass::for_null_band(COFDM_N_FFT, occupied_half, taps, stopband_db))
    }
}

/// The `OfdmConfig` and preamble for one COFDM link, from the *effective*
/// shaping.
///
/// **Both ends build from this.**  The modulator in [`CofdmSource::render`] and
/// the receiver in [`crate::source::cofdm::rx`] must agree on every field of the
/// numerology, and a demodulator that differs by one — a window back-off, a
/// symbol taper, a single edge carrier — does not fail loudly.  It simply never
/// acquires, which looks identical to a dead signal.  One builder makes that
/// class of drift unrepresentable.
///
/// **`rf_hz` is 0.0 and must stay so.**  orion-sdr 0.0.58's frame layer asserts
/// it and panics otherwise: the frame assembler restarted its rotator at every
/// seam and the receiver never applied `rf_hz` at all.  `render` upconverts the
/// whole burst itself with one continuous [`Rotator`] — see the module header.
pub fn cofdm_link_config(shaping: &CofdmShaping, fs: f32) -> (OfdmConfig, OfdmPreamble) {
    let roll_off = shaping.taper.roll_off();

    // DC-centered data carriers ±1..=±(COFDM_MAX_CARRIER - edge_guard), so
    // the occupied band is symmetric about DC and centers on the RF
    // frequency after upconversion.  This is Track A's edge-carrier guard:
    // the same contiguous span the bandwidth fraction always selected, now
    // built by the library so `occupied_half_carriers()` can size the mask.
    let plan = CarrierPlan::new(COFDM_N_FFT, COFDM_CP_LEN)
        .with_contiguous_data(shaping.edge_guard, shaping.include_dc);

    let mut cfg = OfdmConfig::new(
        plan,
        fs,
        0.0, // baseband — `render` upconverts, see the module header
        1.0, // unit scale — `render` normalises, see COFDM_DISPLAY_RMS_DBFS
        ConstellationOrder::Qpsk,
    )
    .with_payload_crc(CrcKind::Crc32)
    .with_header_crc(CrcKind::Crc16)
    .with_rx_window_backoff(COFDM_RX_WINDOW_BACKOFF);
    if roll_off > 0 {
        cfg = cfg.with_symbol_window(roll_off);
    }
    debug_assert!(
        shaping
            .mask_filter(cfg.carrier_plan.occupied_half_carriers())
            .is_none_or(|m| m.fits_guard(COFDM_CP_LEN, roll_off, COFDM_RX_WINDOW_BACKOFF)),
        "shaping overran the guard budget"
    );

    let preamble = OfdmPreamble::new(COFDM_PREAMBLE_REPEATS, COFDM_PREAMBLE_REPEAT_LEN)
        .with_training_symbol(cfg.carrier_plan.n_fft(), cfg.carrier_plan.cp_len());
    (cfg, preamble)
}

// ── COFDM HUD helper ──────────────────────────────────────────────────────────

/// Submode line for the top HUD: the bandwidth fraction, plus a compact shaping
/// tag when shaping is on, e.g. "  bw 1/4  shp 1/4·60 dB".  The bandwidth label
/// names the *fraction*, which no longer implies the occupied band once the edge
/// guard is overridden — the Di bar's BW readout is authoritative there.
pub fn hud_submode_str(fraction: CofdmBwFraction, shaping: &CofdmShaping) -> String {
    let mut s = format!("  bw {}", fraction.label());
    if shaping.enabled {
        s.push_str(&format!(
            "  shp {}·{}",
            shaping.taper.label(),
            shaping.mask.label()
        ));
    }
    s
}

// ── CofdmSource ───────────────────────────────────────────────────────────────

/// Wideband coded-OFDM (COFDM) signal source.
///
/// Pre-renders a fixed-length looping buffer of back-to-back COFDM frames —
/// each `[preamble+training][header][payload]` via [`OfdmFrameMod`] — taking the
/// real part of the upconverted IQ.  Playback alternates a `sig_secs` signal
/// phase (the buffer, looped) with a `gap_secs` silence phase, repeating
/// indefinitely.
///
/// **Timing is driven by real wall-clock `dt`, not sample counts.**  COFDM plays
/// back NON-realtime (the viewer consumes a fixed rate regardless of the native
/// `fs`, and the render frame rate is uncapped), so counting emitted samples
/// would make the phase durations scale with the frame rate.  Instead the app
/// (or a test) advances the source's timeline via [`SignalSource::advance_time`]
/// with the real per-frame `dt`, flipping the signal/gap phase when the elapsed
/// phase time reaches `sig_secs` / `gap_secs`.  A "Gap 2 s" setting thus yields a
/// ~2 s on-screen pause regardless of frame rate — consistent with the
/// narrowband sources — and the timing is deterministically testable.
pub struct CofdmSource {
    pub sig_secs: f32,
    pub gap_secs: f32,
    noise: CnNoise,
    pub fraction: CofdmBwFraction,
    pub shaping: CofdmShaping,
    /// Band centre (Hz) the baseband buffer is upconverted to.  Bounded by
    /// [`cofdm_center_bounds`] and coupled to the edge guard through
    /// [`CofdmShaping::effective`].
    center_hz: f32,
    fs: f32,
    /// Looping COFDM signal buffer, **complex baseband and noise-free** (fixed
    /// length; content, not duration).
    ///
    /// The buffer is stored pre-upconversion because the receiver needs an
    /// analytic signal and the real projection cannot supply one: mixing a real
    /// stream back down leaves the conjugate image, which forces the Schmidl &
    /// Cox correlation to be real and its frequency-offset estimate to be a
    /// constant.  See [`crate::source::cofdm::rx`].
    iq: Vec<C32>,
    /// Wrapping read cursor into `iq` during the signal phase.
    pos: usize,
    /// Upconversion oscillator, advanced once per emitted sample and never
    /// reset mid-run, so there is no phase step at a block, loop or phase
    /// boundary.
    rot: Rotator,
    /// Impaired complex baseband for the block most recently returned by
    /// `next_samples`, exposed through [`SignalSource::last_samples_iq`].
    last_iq: Vec<C32>,
    /// Scalar `render` applied to reach [`COFDM_DISPLAY_RMS_DBFS`].  Derived,
    /// not tuned; kept so callers can report what full scale means for this
    /// burst without re-deriving it.
    display_gain: f32,
    /// True during the signal phase, false during the silence gap.
    in_signal: bool,
    /// Wall-clock seconds elapsed in the current phase.
    phase_secs: f32,
    rng: u64,
}

impl CofdmSource {
    pub fn new(
        sig_secs: f32,
        gap_secs: f32,
        cn_db: f32,
        fraction: CofdmBwFraction,
        shaping: CofdmShaping,
        center_hz: f32,
        fs: f32,
    ) -> Self {
        let fs = cofdm_clamp_fs(fs);
        let center_hz = clamp_center(center_hz, fs);
        let mut src = Self {
            sig_secs,
            gap_secs,
            noise: CnNoise::new(cn_db, cn_reference(0.0, 0.0, fs)),
            fraction,
            shaping,
            center_hz,
            fs,
            iq: Vec::new(),
            pos: 0,
            rot: Rotator::new(center_hz, fs),
            last_iq: Vec::new(),
            display_gain: 1.0,
            in_signal: true,
            phase_secs: 0.0,
            rng: 0x853c_49e6_748f_ea9b,
        };
        src.render();
        src
    }

    /// Band centre (Hz), after clamping to [`cofdm_center_bounds`].
    pub fn center_hz(&self) -> f32 {
        self.center_hz
    }

    /// The shaping actually rendered — this source's set resolved through
    /// [`CofdmShaping::effective`] at its centre and rate.
    pub fn effective_shaping(&self) -> CofdmShaping {
        self.shaping
            .effective(self.fraction, self.center_hz, self.fs)
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

    /// True while in the signal phase (exposed for tests / decode gating).
    pub fn in_signal(&self) -> bool {
        self.in_signal
    }

    /// Build the carrier plan (sized by the edge guard), config, preamble, and
    /// MCS table, then stream enough COFDM frames to fill the looping buffer
    /// and mask the result.
    ///
    /// **Stops at complex baseband.**  Noise and upconversion both happen at
    /// read time in `next_samples`, so the two outputs — the real projection the
    /// display consumes and the complex baseband the decoder consumes — are
    /// derived from one impaired sample each rather than being built twice.
    fn render(&mut self) {
        let shaping = self.effective_shaping();
        let (cfg, preamble) = cofdm_link_config(&shaping, self.fs);
        let mask = shaping.mask_filter(cfg.carrier_plan.occupied_half_carriers());
        let table = McsTable::default_ladder();
        let modu = OfdmFrameMod::new(cfg, table, preamble);

        // Render a fixed-length looping buffer of back-to-back COFDM frames.
        // This is *content*, not duration — the signal-phase length is timed by
        // real `dt` in `next_samples`, and this buffer just loops.  Each frame
        // carries a fresh deterministic pseudo-random payload with an
        // incrementing sequence number, so the spectrum stays fully populated
        // across all subcarriers and the loop point isn't obvious.
        //
        // The symbol taper is `modulate_frame`'s own per-frame post-pass, but
        // the mask is applied ONCE over the concatenation rather than per frame:
        // a filter run per frame leaves a group-delay-long transient at each of
        // the 39 interior seams, which is exactly the spectral step the mask is
        // there to remove.  `DvbTSuperFrameMod` filters across its frame seams
        // for the same reason.
        let mut iq: Vec<C32> = Vec::new();
        for seq in 0..COFDM_BUFFER_FRAMES as u32 {
            let payload = self.build_payload();
            let frame = FramePacket::new(FrameMetadata::new(seq, COFDM_MCS_INDEX), payload);
            iq.extend_from_slice(&modu.modulate_frame(&frame, 0));
        }
        if let Some(mask) = mask {
            mask.apply(&mut iq);
        }

        // Display scaling, **derived** from what was actually rendered rather
        // than fitted once and applied to everything.  The target is a real
        // projection RMS of `COFDM_DISPLAY_RMS_DBFS`, so `RMS_real =
        // sqrt(P_complex / 2)` is the quantity to normalise.
        //
        // Applied **once across the whole concatenation** so preamble, training
        // symbol and payload are scaled alike.  That uniformity is the
        // invariant: a non-uniform gain is what made this source unacquirable
        // before orion-sdr 0.0.57, when the preamble was emitted at unit
        // amplitude while the payload was not.  See
        // `the_display_gain_scales_every_segment_alike`.
        //
        // Referenced to the **whole buffer**, unlike the C/N reference below —
        // buffer RMS is what the eye and the `lvl` readout see, while the noise
        // must be referenced to the payload alone.  Two measurements, because
        // they answer two different questions.
        let unit_rms = (mean_power_c(&iq) / 2.0).sqrt();
        self.display_gain = if unit_rms > 0.0 {
            display_target_rms() / unit_rms
        } else {
            1.0
        };
        for c in &mut iq {
            *c *= self.display_gain;
        }

        // The C/N reference: **payload power, at the scale the noise is injected
        // at.**  The preamble is deliberately hotter than the payload, and the
        // prefix is a bandwidth-dependent fraction of the frame — 6.3% of a 7/8
        // frame against 1% of a 1/8 one — so a buffer-mean reference would
        // inject a different C/N at every fraction, which is exactly the
        // cross-fraction tilt a ratio exists to remove.
        let signal_power = payload_power(&iq);
        let occupied_bw = cofdm_occupied_bw(self.fs, shaping.edge_guard);
        self.noise
            .set_reference(cn_reference(signal_power, occupied_bw, self.fs));

        self.iq = iq;
        self.pos = 0;
    }

    /// Build a deterministic pseudo-random payload of `COFDM_PAYLOAD_BYTES`.
    fn build_payload(&mut self) -> Vec<u8> {
        (0..COFDM_PAYLOAD_BYTES)
            .map(|_| (self.next_u64() & 0xff) as u8)
            .collect()
    }

    /// Apply fresh parameters.  Only the bandwidth fraction, the shaping set
    /// and the centre change the rendered buffer; `sig_secs` / `gap_secs` are
    /// wall-clock phase durations applied live, and `cn_db` is arithmetic on
    /// the cached reference — **neither re-renders**.
    ///
    /// That the C/N knob stays off the re-render path is deliberate: `render`
    /// runs FEC encoding, 40 frames of FFTs and a mask filter, so a 1 dB step
    /// held down on the arrow keys would otherwise rebuild the buffer per
    /// keypress.
    ///
    /// **The centre is on it, and has to be.**  The buffer itself is baseband
    /// and would not care, but the centre clamps the edge guard
    /// ([`CofdmShaping::effective`]), so a nudge can change the carrier plan —
    /// and the C/N reference bandwidth with it.  Re-rendering only when the
    /// *effective* shaping actually moved keeps the common case (a retune that
    /// leaves the band width alone) off the expensive path.
    ///
    /// The rotator is rebuilt on a centre change, which steps its phase.  That
    /// is a retune; a continuous phase across one is neither achievable nor
    /// meaningful.
    pub fn apply_params(
        &mut self,
        sig_secs: f32,
        gap_secs: f32,
        cn_db: f32,
        fraction: CofdmBwFraction,
        shaping: CofdmShaping,
        center_hz: f32,
    ) {
        let center_hz = clamp_center(center_hz, self.fs);
        let retuned = center_hz != self.center_hz;
        // The *effective* set is the whole of what `render` consumes, so
        // comparing it is exactly the re-render condition — no need to also
        // test the fraction or the raw set, both of which reach the buffer only
        // through this.  (The raw set is still stored, since a later clamp
        // resolves from it.)
        let rerender = self.effective_shaping() != shaping.effective(fraction, center_hz, self.fs);
        self.sig_secs = sig_secs;
        self.gap_secs = gap_secs;
        self.fraction = fraction;
        self.shaping = shaping;
        self.center_hz = center_hz;
        if retuned {
            self.rot = Rotator::new(center_hz, self.fs);
        }
        if rerender {
            self.render();
        }
        // After any re-render, so the C/N is derived against the geometry the
        // new buffer implies rather than the outgoing one's.
        self.noise.set_cn_db(cn_db);
    }

    fn next_u64(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }
}

impl SignalSource for CofdmSource {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn restart(&mut self) {
        self.pos = 0;
        self.in_signal = true;
        self.phase_secs = 0.0;
    }

    /// Advance the signal/gap phase timer by `dt` seconds and flip the phase
    /// when it reaches the current phase's duration.  Frame-rate independent.
    fn advance_time(&mut self, dt_secs: f32) {
        self.phase_secs += dt_secs;
        // Clamp the signal phase so a runaway can't overflow the decode-bar
        // timer's fixed-width display.
        let limit = if self.in_signal {
            self.sig_secs.min(MAX_SIG_SECS)
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

    /// Emits `n` real samples, and records their complex-baseband counterparts
    /// for [`last_samples_iq`](SignalSource::last_samples_iq).
    ///
    /// Each sample is impaired **once**, at baseband, and the real output is the
    /// projection of that same impaired sample:
    ///
    /// ```text
    /// iq[k]   = buffer[pos] + noise[k]
    /// real[k] = re(iq[k] * exp(j*2*pi*f0*k/fs))
    /// ```
    ///
    /// That ordering is what keeps the decoder and the display honest about each
    /// other.  Noising the real stream and handing the decoder the clean render
    /// buffer — the obvious shortcut — would report `CBER`/`IBER` of exactly
    /// zero at every `Noise amp` setting while the spectrum on screen was
    /// visibly noisy.
    ///
    /// Noise is complex Gaussian with per-component standard deviation derived
    /// from the requested C/N, so its total power is `2 * sigma^2` spread white
    /// over the full `fs` — see [`NoiseDomain::Complex`].  Signal and noise are
    /// projected alike, so the decoder's C/N and the display's agree.
    ///
    /// **Injection stays here, per sample.**  The buffer loops with a ~0.3 s
    /// period, so noise baked into `render` would be one realisation replayed
    /// forever: a static speckle in the persistence and waterfall panes, and
    /// frame-error trials that are correlated rather than independent — which
    /// silently defeats any FER measured over more frames than the buffer
    /// holds.  Only the *reference* the amplitude is derived from is a
    /// render-time quantity.
    fn next_samples(&mut self, n: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(n);
        self.last_iq.clear();
        self.last_iq.reserve(n);
        let len = self.iq.len();
        let live = self.in_signal && len > 0;
        for _ in 0..n {
            // Silence gap (or empty buffer): noise only.
            let mut c = if live {
                let c = self.iq[self.pos];
                self.pos = (self.pos + 1) % len; // loop the content buffer
                c
            } else {
                C32::default()
            };
            c.re += self.noise.next();
            c.im += self.noise.next();
            let r = self.rot.next();
            out.push(c.re * r.re - c.im * r.im);
            self.last_iq.push(c);
        }
        out
    }

    fn last_samples_iq(&self) -> Option<&[C32]> {
        Some(&self.last_iq)
    }

    fn signal_phase(&self) -> Option<bool> {
        Some(self.in_signal)
    }

    fn sample_rate(&self) -> f32 {
        self.fs
    }
}

// ── Derived display level and C/N geometry ──────────────────────────────────

/// Target real-projection RMS implied by [`COFDM_DISPLAY_RMS_DBFS`].
fn display_target_rms() -> f32 {
    10f32.powf(COFDM_DISPLAY_RMS_DBFS / 20.0)
}

/// A requested band centre, clamped to [`cofdm_center_bounds`].  A non-finite
/// request falls back to the default centre rather than poisoning the rotator.
fn clamp_center(center_hz: f32, fs: f32) -> f32 {
    let (lo, hi) = cofdm_center_bounds(fs);
    if center_hz.is_finite() {
        center_hz.clamp(lo, hi)
    } else {
        cofdm_default_center_hz(fs)
    }
}

/// Samples at the head of every frame that are **not** payload: the Schmidl &
/// Cox preamble plus the training symbol.
///
/// The preamble is deliberately hotter than the payload, so it must be excluded
/// from the power the C/N is referenced to.  Frames are equal-length — same
/// payload size, same MCS, same carrier plan — so the offsets are arithmetic
/// rather than something the modulator has to report back.
const fn frame_prefix_len() -> usize {
    COFDM_PREAMBLE_REPEATS * COFDM_PREAMBLE_REPEAT_LEN + COFDM_N_FFT + COFDM_CP_LEN
}

/// Mean power of the payload portion of a rendered buffer of
/// [`COFDM_BUFFER_FRAMES`] equal-length frames.
///
/// Falls back to the whole-buffer mean if the geometry does not divide as
/// expected — a wrong-by-6% reference is better than a panic, and the
/// `the_cn_reference_excludes_the_preamble` test is what catches the fallback
/// being taken.
fn payload_power(iq: &[C32]) -> f32 {
    let frame_len = iq.len() / COFDM_BUFFER_FRAMES;
    let prefix = frame_prefix_len();
    if frame_len <= prefix {
        return mean_power_c(iq);
    }
    let mut sum = 0.0f32;
    let mut count = 0usize;
    for f in 0..COFDM_BUFFER_FRAMES {
        let lo = f * frame_len + prefix;
        let hi = (f + 1) * frame_len;
        for c in &iq[lo..hi] {
            sum += c.norm_sqr();
        }
        count += hi - lo;
    }
    if count == 0 { 0.0 } else { sum / count as f32 }
}

/// The C/N geometry for a COFDM burst.
///
/// [`NoiseDomain::Complex`]: the generator adds independent noise to both
/// components of the baseband sample, so its power is white over the **full
/// `fs`**, not over the display's `fs / 2` Nyquist span.  That factor of two is
/// the easiest thing to get wrong here, and it is wrong by 3 dB rather than
/// visibly.
fn cn_reference(signal_power: f32, occupied_bw_hz: f32, fs: f32) -> CnReference {
    CnReference {
        signal_power,
        occupied_bw_hz,
        fs,
        domain: NoiseDomain::Complex,
    }
}
