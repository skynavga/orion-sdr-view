use serde::Deserialize;
use super::Defaults;

#[derive(Debug, Deserialize)]
pub struct TestToneConfig {
    pub freq_hz:    Option<f32>,
    pub noise_amp:  Option<f32>,
    pub amp_max:    Option<f32>,
    pub ramp_secs:  Option<f32>,
    pub pause_secs: Option<f32>,
}

impl super::ViewConfig {
    pub fn freq_hz(&self) -> f32 {
        self.sources.as_ref()
            .and_then(|s| s.test_tone.as_ref())
            .and_then(|t| t.freq_hz)
            .unwrap_or(Defaults::FREQ_HZ)
    }
    pub fn noise_amp(&self) -> f32 {
        self.sources.as_ref()
            .and_then(|s| s.test_tone.as_ref())
            .and_then(|t| t.noise_amp)
            .unwrap_or(Defaults::NOISE_AMP)
    }
    pub fn amp_max(&self) -> f32 {
        self.sources.as_ref()
            .and_then(|s| s.test_tone.as_ref())
            .and_then(|t| t.amp_max)
            .unwrap_or(Defaults::AMP_MAX)
    }
    pub fn ramp_secs(&self) -> f32 {
        self.sources.as_ref()
            .and_then(|s| s.test_tone.as_ref())
            .and_then(|t| t.ramp_secs)
            .unwrap_or(Defaults::RAMP_SECS)
    }
    pub fn pause_secs(&self) -> f32 {
        self.sources.as_ref()
            .and_then(|s| s.test_tone.as_ref())
            .and_then(|t| t.pause_secs)
            .unwrap_or(Defaults::PAUSE_SECS)
    }
}
