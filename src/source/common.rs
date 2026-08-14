// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

/// Longest burst the decode-bar timer can show without its field changing
/// width: `sig NN.NN`.
///
/// **This is a display bound, not a capability.**  Every source used to *clamp*
/// its burst to it — psk31, ft8, amdsb and cw truncated their rendered buffer,
/// COFDM clamped its phase timer — which put a HUD field width in charge of how
/// long a signal could last, and did it silently.  The timer now marks an
/// overflow instead (`sig 99.99+s`), the same convention a wrapped error count
/// already uses, so the field never widens and nothing is cut short.
pub const MAX_SIG_SECS: f32 = 99.99;

/// A `sig_secs` at or above this means **the burst never ends**.
///
/// One step past [`MAX_SIG_SECS`] on the settings row, so "continuous" is one
/// keypress rather than a number nobody can nudge to: at a second per press,
/// the top of any usefully large finite range is hundreds of presses away.
///
/// A sentinel rather than `f32::INFINITY` because it has to survive a YAML
/// round trip, a row clamp and a display format, and infinity is awkward in all
/// three.  Any larger value means the same thing, so a config asking for
/// `1.0e9` gets what it plainly intended.
pub const CONTINUOUS_SIG_SECS: f32 = 100.0;

/// True when `sig_secs` asks for a burst with no gap after it.
pub fn is_continuous_sig(sig_secs: f32) -> bool {
    sig_secs >= CONTINUOUS_SIG_SECS
}

// ── C/N-specified additive noise ─────────────────────────────────────────────
//
// Every source expresses its impairment as a **carrier-to-noise ratio in dB**
// rather than as an absolute noise amplitude.  A ratio is the only figure that
// is comparable between sources, because their signal amplitudes, occupied
// bandwidths and display scalings all differ — and it is the only one that
// survives a display gain that is derived rather than fixed.
//
// The arithmetic, following the standard definition (noise referenced to the
// occupied bandwidth, generated across the whole sampled span):
//
// ```text
// N0      = P_signal / (B_occupied * 10^(cn_db / 10))   // noise PSD, W/Hz
// P_noise = N0 * B_noise                                // total injected power
// ```
//
// `B_noise` is the bandwidth the generator's noise is white over, which is NOT
// the same as the occupied bandwidth and NOT the same for both kinds of source
// — see [`NoiseDomain`].

/// Widest `C/N` any source's settings row allows, in dB.  A shared bound: the
/// per-source *defaults* differ by ~20 dB (a 62.5 Hz PSK31 signal against noise
/// spread over 24 kHz is a 25.8 dB spreading factor, against COFDM's 9 dB), but
/// one range brackets all of them and keeps the row identical everywhere.
pub const MAX_CN_DB: f32 = 70.0;
/// Narrowest `C/N` any source's settings row allows, in dB.
pub const MIN_CN_DB: f32 = 0.0;

/// Which domain a source injects its noise into.
///
/// This is the factor-of-two that is easy to get wrong: a real-valued generator
/// and a complex-baseband one spread the same total noise power over different
/// bandwidths and across a different number of components.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NoiseDomain {
    /// Real-valued output: the noise is white over `fs / 2` and is carried by
    /// a single component.
    Real,
    /// Complex baseband: the noise is white over the full `fs` and is split
    /// across two independent components.
    Complex,
}

impl NoiseDomain {
    /// The bandwidth this domain's noise is white over, at sample rate `fs`.
    pub fn noise_bw_hz(self, fs: f32) -> f32 {
        match self {
            NoiseDomain::Real => fs / 2.0,
            NoiseDomain::Complex => fs,
        }
    }

    /// Number of independent components the noise power is divided between.
    fn components(self) -> f32 {
        match self {
            NoiseDomain::Real => 1.0,
            NoiseDomain::Complex => 2.0,
        }
    }
}

/// The link geometry a `C/N` setting is referenced against.
///
/// **`signal_power` is the power of the *transmitting* portion of the
/// waveform**, and getting that wrong is the trap this type exists to make
/// visible at every call site.  It must not be a mean over the whole buffer
/// when part of that buffer is a gap, a key-up interval, or a preamble that is
/// deliberately hotter than the payload; and it must not track a live
/// amplitude that ramps, or the noise floor would pump with the signal.
///
/// **`occupied_bw_hz` is declared, never measured.**  Several sources estimate
/// their occupied bandwidth from the spectrum for the Di-bar readout; feeding
/// that estimate back into the impairment would make the noise floor wobble
/// with the measurement.  A tone has no bandwidth at all, so its value is a
/// stated *reference* bandwidth — which is exactly what makes "30 dB C/N" mean
/// something for a single carrier.
#[derive(Clone, Copy, Debug)]
pub struct CnReference {
    pub signal_power: f32,
    pub occupied_bw_hz: f32,
    /// Sample rate of the generator, which sets `B_noise` via the domain.
    pub fs: f32,
    pub domain: NoiseDomain,
}

impl CnReference {
    /// Per-component standard deviation that realises `cn_db` against this
    /// geometry.  Zero when the geometry is degenerate (no signal, no
    /// bandwidth), which makes the source silent rather than infinite.
    pub fn sigma_for(&self, cn_db: f32) -> f32 {
        // NaN in any term must land here too, not fall through to a NaN sigma
        // that would silently poison every sample the source emits.
        let finite_positive = |v: f32| v.is_finite() && v > 0.0;
        if !finite_positive(self.signal_power)
            || !finite_positive(self.occupied_bw_hz)
            || !finite_positive(self.fs)
        {
            return 0.0;
        }
        let cn_lin = 10f32.powf(cn_db / 10.0);
        let n0 = self.signal_power / (self.occupied_bw_hz * cn_lin);
        let p_noise = n0 * self.domain.noise_bw_hz(self.fs);
        (p_noise / self.domain.components()).sqrt()
    }
}

/// C/N-specified additive white Gaussian noise.
///
/// **Injection stays per-sample, at read time.**  The multicarrier sources
/// pre-render a *looping* content buffer, so noise baked into that buffer would
/// be one realisation replayed forever: a static speckle in the persistence and
/// waterfall panes, and — worse — frame-error trials that are correlated rather
/// than independent, which silently defeats any FER measurement taken over more
/// frames than the buffer holds.  Only the reference the amplitude is derived
/// from is a render-time quantity.
///
/// Keeping the derivation off the re-render path is the second reason for the
/// split: `set_cn_db` is arithmetic on a cached reference, so nudging the knob
/// on 1 dB steps does not rebuild a buffer that costs FEC encoding, dozens of
/// FFTs and a mask filter.
pub struct CnNoise {
    cn_db: f32,
    reference: CnReference,
    /// Per-component standard deviation, derived from the two above.
    sigma: f32,
    rng: u64,
    /// The second variate from the last polar draw — see [`CnNoise::awgn`].
    spare: Option<f32>,
}

impl CnNoise {
    pub fn new(cn_db: f32, reference: CnReference) -> Self {
        Self {
            cn_db,
            reference,
            sigma: reference.sigma_for(cn_db),
            rng: 0x853c_49e6_748f_ea9b,
            spare: None,
        }
    }

    /// Requested C/N, in dB.
    pub fn cn_db(&self) -> f32 {
        self.cn_db
    }

    /// Per-component standard deviation of the injected noise.
    pub fn sigma(&self) -> f32 {
        self.sigma
    }

    /// Change the requested C/N.  Cheap — no re-render.
    pub fn set_cn_db(&mut self, cn_db: f32) {
        self.cn_db = cn_db;
        self.sigma = self.reference.sigma_for(cn_db);
    }

    /// Re-seat the geometry after a re-render changed the signal power or the
    /// occupied bandwidth.  The requested C/N is unchanged — that is the whole
    /// point of expressing it as a ratio.
    pub fn set_reference(&mut self, reference: CnReference) {
        self.reference = reference;
        self.sigma = reference.sigma_for(self.cn_db);
    }

    /// The geometry currently in force.
    pub fn reference(&self) -> CnReference {
        self.reference
    }

    /// One scaled Gaussian sample.  Returns exactly `0.0` when the noise is
    /// off, so a caller need not test for it.
    #[allow(clippy::should_implement_trait)] // not an iterator: an unbounded sample stream
    pub fn next(&mut self) -> f32 {
        if self.sigma <= 0.0 {
            return 0.0;
        }
        self.sigma * self.awgn()
    }

    /// One standard normal variate, by the Marsaglia polar method.
    ///
    /// **Uniform noise would not do.**  It has the right second-order
    /// statistics and is equally white, but the FEC cliff is a tail phenomenon,
    /// so an FER curve measured against uniform noise cannot be compared to a
    /// published waterfall or to a hardware generator — which is the entire
    /// reason for naming the knob `C/N`.
    ///
    /// **Nor would the 12-uniform CLT sum** used elsewhere in this workspace,
    /// for the same reason one step further in: it is *truncated at ±6 sigma*,
    /// so the rare large excursions that actually break a frame cannot occur at
    /// all.  Polar is exact in the tails, which is the half of the distribution
    /// this knob exists to exercise.
    ///
    /// It is also much cheaper — ~1.3 uniforms per variate against 12, which
    /// matters because COFDM draws two per sample at 1.92 MHz and the test
    /// profile is unoptimised.  The second variate of each pair is cached in
    /// `spare` rather than discarded.
    fn awgn(&mut self) -> f32 {
        if let Some(spare) = self.spare.take() {
            return spare;
        }
        loop {
            let u = 2.0 * self.uniform01() - 1.0;
            let v = 2.0 * self.uniform01() - 1.0;
            let s = u * u + v * v;
            // Reject outside the unit disc, and at the origin where `ln` blows
            // up.  Accepts with probability pi/4, so ~1.27 iterations.
            if s > 0.0 && s < 1.0 {
                let f = (-2.0 * s.ln() / s).sqrt();
                self.spare = Some(v * f);
                return u * f;
            }
        }
    }

    fn uniform01(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 11) as f32 * (1.0 / (1u64 << 53) as f32)
    }
}

// ── Signal-power helpers ────────────────────────────────────────────────────

/// Mean power of a real sample slice.
pub fn mean_power(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32
}

/// Mean power of a complex sample slice.
pub fn mean_power_c(samples: &[num_complex::Complex32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    samples.iter().map(|c| c.norm_sqr()).sum::<f32>() / samples.len() as f32
}

/// Average power of a *keyed* sinusoidal carrier, referenced to its peak
/// envelope rather than to the buffer mean: `peak^2 / 2`.
///
/// CW renders key-up intervals as silence inside the same buffer as its
/// key-down elements, so a buffer mean would report a power that depends on the
/// message text and the WPM — and the C/N would then move when the operator
/// changed either.  The key-down carrier is the reference an RF engineer means
/// by `C`.
pub fn keyed_carrier_power(samples: &[f32]) -> f32 {
    let peak = samples.iter().fold(0.0_f32, |m, s| m.max(s.abs()));
    peak * peak / 2.0
}

/// Common interface for all signal sources.
///
/// Implementations produce real-valued (f32) samples ready to push into the
/// existing `RingBuffer` and spectrum display pipeline.
///
/// `as_any_mut` enables downcasting a `Box<dyn SignalSource>` to a concrete type:
/// ```no_run
/// use orion_sdr_view::source::{SignalSource, amdsb::AmDsbSource};
/// fn poke_am(source: &mut dyn SignalSource) {
///     if let Some(_am) = source.as_any_mut().downcast_mut::<AmDsbSource>() {
///         // ... mutate the AM source ...
///     }
/// }
/// ```
pub trait SignalSource {
    fn next_samples(&mut self, n: usize) -> Vec<f32>;

    /// Complex baseband for the block most recently returned by
    /// [`next_samples`](Self::next_samples), for sources that have one.
    /// Default: `None`.
    ///
    /// **Why this returns the *last* block rather than taking a count.**  A
    /// decoder and the display must consume the *same* samples; two independent
    /// generators would drift, and nothing would catch it.  Returning the
    /// counterpart of the block just emitted makes the correspondence
    /// structural: `real[k] == re(iq[k] * exp(j*2*pi*f0*k/fs))` holds by
    /// construction, not by convention.
    ///
    /// **Why a real-valued stream is not enough for a demodulator.**  The real
    /// projection carries a conjugate image.  Mixing it back down leaves that
    /// image at full power, and for a real input the Schmidl & Cox correlation
    /// then reduces to `s[n]*s[n+L]*exp(-j*w0*L)` — every term shares one phase,
    /// fixed by the mixer and the lag alone, so the frequency-offset estimate is
    /// a *constant* rather than a measurement.  Measured through COFDM's front
    /// end it reported the same -0.0134 Hz for true offsets of 0, 50, 200 and
    /// 1000 Hz.  Filtering the image away restores observability but was
    /// measured to leave a bias large enough to destroy the payload.  A source
    /// that has complex samples must therefore offer them.
    ///
    /// Over the air this is the natural direction anyway: a tuner delivers
    /// complex IQ, and the real projection is something the *viewer* imposes for
    /// its own display.
    fn last_samples_iq(&self) -> Option<&[num_complex::Complex32]> {
        None
    }

    /// Whether the source is transmitting right now, for sources that know.
    /// Default: `None`.
    ///
    /// The viewer otherwise infers this from block RMS against a threshold —
    /// necessary over the air, where nothing declares it, but a workaround for
    /// a synthetic source that has the answer. Inferring it also couples two
    /// unrelated things: the impairment level and the ability to see the burst
    /// boundary. COFDM's old amplitude knob was capped at 0.50 for exactly that
    /// reason — gap noise was a fixed fraction of it, so a louder setting
    /// climbed past the discriminator and gap detection silently stopped, well
    /// before the noise was high enough to show the FEC cliff.
    ///
    /// A source that reports its phase decouples them: the impairment range is
    /// then bounded by the link, which is the only thing it should be bounded
    /// by.
    fn signal_phase(&self) -> Option<bool> {
        None
    }

    /// Native sample rate (Hz).  Used to pace per-frame consumption to
    /// wall-clock and (for wideband sources) to re-derive the display Nyquist.
    fn sample_rate(&self) -> f32;
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
    /// Reset playback to the beginning of the first loop cycle.
    fn restart(&mut self) {}

    /// Advance the source's wall-clock timeline by `dt` seconds.  Called once
    /// per frame by the app before `next_samples`, and by tests with synthetic
    /// `dt`.  Sources whose timing is driven by real elapsed time (rather than
    /// emitted-sample counts) use this to advance signal/gap phases in a
    /// frame-rate-independent, deterministically-testable way.  Default: no-op.
    fn advance_time(&mut self, _dt_secs: f32) {}
}
