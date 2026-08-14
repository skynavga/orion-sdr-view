// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::source::{CnNoise, CnReference, NoiseDomain, SignalSource};

// ── Test-tone constants ───────────────────────────────────────────────────────

/// Reference bandwidth (Hz) the test tone's `C/N` is measured in.
///
/// **A tone has no occupied bandwidth**, so a carrier-to-noise ratio is
/// meaningless for it until a measurement bandwidth is stated — which is the
/// one caveat that makes C/N work for analog and digital waveforms alike.  500
/// Hz is the conventional narrow CW filter, and matching [`super::super::cw`]
/// keeps the two single-carrier sources on the same footing.
pub const TONE_CN_REF_BW_HZ: f32 = 500.0;

/// Default C/N (dB) for the test tone.
///
/// Chosen to reproduce the noise floor the pre-`C/N` amplitude default put on
/// screen, so the schema change was not also a visual change.  See `CHANGELOG`
/// (0.0.23) for the equivalence.
pub const TONE_DEFAULT_CN_DB: f32 = 36.0;

/// Default peak tone amplitude, and the power reference for [`TONE_DEFAULT_CN_DB`].
pub const TONE_DEFAULT_AMP_MAX: f32 = 0.65;

// ── CycleState ────────────────────────────────────────────────────────────────

/// Amplitude cycling state machine.
///
/// Sequence: RampUp → PauseHigh → RampDown → PauseLow → RampUp → …
/// Each state counts down `samples_remaining` to zero then transitions.
#[derive(Clone, Copy)]
enum CycleState {
    RampUp,
    PauseHigh,
    RampDown,
    PauseLow,
}

// ── TestSignalGen ─────────────────────────────────────────────────────────────

/// Simple test signal generator: sine tone + AWGN at a specified C/N.
///
/// When `cycling` is true the tone amplitude follows a 4-phase sequence:
/// ramp 0.0 → 0.65, pause, ramp 0.65 → 0.0, pause. Each ramp takes
/// `ramp_secs` seconds; each pause (at both extremes) lasts `pause_secs`
/// seconds.
///
/// **The C/N is referenced to `amp_max`, not to the live `tone_amp`.**  A
/// reference that tracked the ramp would make the noise floor pump with the
/// signal — visibly wrong, since real noise does not follow the carrier — and
/// would divide by zero at the bottom of the cycle.
pub struct TestSignalGen {
    phase: f32,
    pub freq_hz: f32,
    pub sample_rate: f32,
    pub tone_amp: f32,
    noise: CnNoise,

    // Amplitude cycling
    pub cycling: bool,
    pub amp_min: f32,
    pub amp_max: f32,
    /// Duration of each ramp (up or down) in seconds.
    pub ramp_secs: f32,
    /// Duration of each pause (at top or bottom) in seconds.
    pub pause_secs: f32,
    cycle_state: CycleState,
    samples_remaining: u32,
}

impl TestSignalGen {
    pub fn new(freq_hz: f32, sample_rate: f32) -> Self {
        let ramp_secs = 3.0f32;
        let pause_secs = 7.0f32; // ~2.3× ramp duration
        let pause_samples = (pause_secs * sample_rate) as u32;
        Self {
            phase: 0.0,
            freq_hz,
            sample_rate,
            tone_amp: TONE_DEFAULT_AMP_MAX, // start at maximum, visible immediately
            noise: CnNoise::new(
                TONE_DEFAULT_CN_DB,
                tone_cn_reference(TONE_DEFAULT_AMP_MAX, sample_rate),
            ),
            cycling: false,
            amp_min: 0.0,
            amp_max: TONE_DEFAULT_AMP_MAX,
            ramp_secs,
            pause_secs,
            cycle_state: CycleState::PauseHigh, // FSM starts mid-sequence at peak
            samples_remaining: pause_samples,
        }
    }

    /// Requested carrier-to-noise ratio, in dB.
    pub fn cn_db(&self) -> f32 {
        self.noise.cn_db()
    }

    /// Set the requested C/N, in dB.
    pub fn set_cn_db(&mut self, cn_db: f32) {
        self.noise.set_cn_db(cn_db);
    }

    /// Per-component standard deviation of the injected noise — the absolute
    /// amplitude the requested C/N works out to against the current reference.
    pub fn noise_sigma(&self) -> f32 {
        self.noise.sigma()
    }

    pub fn next_sample(&mut self) -> f32 {
        if self.cycling {
            self.advance_cycle();
        }

        let tone = self.tone_amp * self.phase.sin();
        let noise = self.noise.next();
        self.phase += 2.0 * std::f32::consts::PI * self.freq_hz / self.sample_rate;
        if self.phase > std::f32::consts::PI {
            self.phase -= 2.0 * std::f32::consts::PI;
        }
        tone + noise
    }

    /// Begin cycling: start a ramp-down from the current amplitude peak.
    pub fn start_cycling(&mut self) {
        if self.cycling {
            return;
        }
        self.tone_amp = self.amp_max;
        self.cycle_state = CycleState::RampDown;
        self.samples_remaining = (self.ramp_secs * self.sample_rate) as u32;
        self.cycling = true;
    }

    /// Reset to initial state: zero phase, full amplitude, FSM at PauseHigh.
    pub fn restart(&mut self) {
        self.phase = 0.0;
        self.tone_amp = self.amp_max;
        self.cycle_state = CycleState::PauseHigh;
        self.samples_remaining = (self.pause_secs * self.sample_rate) as u32;
    }

    /// Apply a fresh set of tone parameters.  Pure field copies — no
    /// re-initialisation of cycle state or phase, so the live tone keeps
    /// playing through the change.
    pub fn apply_params(
        &mut self,
        freq_hz: f32,
        cn_db: f32,
        amp_max: f32,
        ramp_secs: f32,
        pause_secs: f32,
    ) {
        self.freq_hz = freq_hz;
        self.amp_max = amp_max;
        self.ramp_secs = ramp_secs;
        self.pause_secs = pause_secs;
        // `amp_max` is the power reference, so it must re-seat the geometry
        // before the C/N is re-derived against it.
        self.noise
            .set_reference(tone_cn_reference(amp_max, self.sample_rate));
        self.noise.set_cn_db(cn_db);
    }

    /// Stop cycling: snap immediately to full amplitude.
    pub fn stop_cycling(&mut self) {
        if !self.cycling {
            return;
        }
        self.cycling = false;
        self.tone_amp = self.amp_max;
        // Reset FSM so next start_cycling begins with a ramp-down again.
        self.cycle_state = CycleState::PauseHigh;
        self.samples_remaining = (self.pause_secs * self.sample_rate) as u32;
    }

    fn advance_cycle(&mut self) {
        let ramp_samples = (self.ramp_secs * self.sample_rate) as u32;
        let pause_samples = (self.pause_secs * self.sample_rate) as u32;

        match self.cycle_state {
            CycleState::RampUp => {
                // Interpolate amp_min → amp_max over ramp_samples.
                let t = 1.0 - (self.samples_remaining as f32 / ramp_samples as f32);
                self.tone_amp = self.amp_min + t * (self.amp_max - self.amp_min);
                if self.samples_remaining == 0 {
                    self.tone_amp = self.amp_max;
                    self.cycle_state = CycleState::PauseHigh;
                    self.samples_remaining = pause_samples;
                } else {
                    self.samples_remaining -= 1;
                }
            }
            CycleState::PauseHigh => {
                self.tone_amp = self.amp_max;
                if self.samples_remaining == 0 {
                    self.cycle_state = CycleState::RampDown;
                    self.samples_remaining = ramp_samples;
                } else {
                    self.samples_remaining -= 1;
                }
            }
            CycleState::RampDown => {
                // Interpolate amp_max → amp_min over ramp_samples.
                let t = 1.0 - (self.samples_remaining as f32 / ramp_samples as f32);
                self.tone_amp = self.amp_max - t * (self.amp_max - self.amp_min);
                if self.samples_remaining == 0 {
                    self.tone_amp = self.amp_min;
                    self.cycle_state = CycleState::PauseLow;
                    self.samples_remaining = pause_samples;
                } else {
                    self.samples_remaining -= 1;
                }
            }
            CycleState::PauseLow => {
                self.tone_amp = self.amp_min;
                if self.samples_remaining == 0 {
                    self.cycle_state = CycleState::RampUp;
                    self.samples_remaining = ramp_samples;
                } else {
                    self.samples_remaining -= 1;
                }
            }
        }
    }
}

/// The C/N geometry for a tone of peak amplitude `amp_max` at `fs`.
///
/// Signal power is `amp_max^2 / 2` — the average power of the sinusoid at its
/// *nominal* peak, which is a property of the settings rather than of whatever
/// point the amplitude ramp happens to be at.
fn tone_cn_reference(amp_max: f32, fs: f32) -> CnReference {
    CnReference {
        signal_power: amp_max * amp_max / 2.0,
        occupied_bw_hz: TONE_CN_REF_BW_HZ,
        fs,
        domain: NoiseDomain::Real,
    }
}

// ── TestToneSource ────────────────────────────────────────────────────────────

/// Adapts the existing `TestSignalGen` to the `SignalSource` trait.
/// All cycling/settings on the inner generator remain accessible via `.gen`.
pub struct TestToneSource {
    pub signal_gen: TestSignalGen,
}

impl TestToneSource {
    pub fn new(signal_gen: TestSignalGen) -> Self {
        Self { signal_gen }
    }
}

impl SignalSource for TestToneSource {
    fn next_samples(&mut self, n: usize) -> Vec<f32> {
        (0..n).map(|_| self.signal_gen.next_sample()).collect()
    }
    fn sample_rate(&self) -> f32 {
        self.signal_gen.sample_rate
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn restart(&mut self) {
        self.signal_gen.restart();
    }
}
