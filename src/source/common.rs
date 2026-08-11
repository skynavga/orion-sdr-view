// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

/// Maximum continuous signal duration in a single loop cycle.
/// Sources clamp the signal burst to this value so the decode-bar timer
/// ("sig NN.NN") never overflows its fixed-width display.
pub const MAX_SIG_SECS: f32 = 99.99;

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
    #[allow(dead_code)] // used by the lib receiver and integration tests, not yet by the binary
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
    /// boundary. COFDM's `Noise amp` was capped at 0.50 for exactly that
    /// reason — gap noise is `noise_amp / sqrt(3)`, so a louder setting climbed
    /// past the discriminator and gap detection silently stopped, well before
    /// the noise was high enough to show the FEC cliff.
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
    #[allow(dead_code)] // used by integration tests, not the binary
    fn restart(&mut self) {}

    /// Advance the source's wall-clock timeline by `dt` seconds.  Called once
    /// per frame by the app before `next_samples`, and by tests with synthetic
    /// `dt`.  Sources whose timing is driven by real elapsed time (rather than
    /// emitted-sample counts) use this to advance signal/gap phases in a
    /// frame-rate-independent, deterministically-testable way.  Default: no-op.
    fn advance_time(&mut self, _dt_secs: f32) {}
}
