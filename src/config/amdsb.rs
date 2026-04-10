use serde::Deserialize;
use super::Defaults;

#[derive(Debug, Deserialize)]
pub struct AmDsbConfig {
    pub carrier_hz: Option<f32>,
    pub mod_index:  Option<f32>,
    pub gap_secs:   Option<f32>,
    pub noise_amp:  Option<f32>,
    pub msg_repeat: Option<u32>,
}

impl super::ViewConfig {
    pub fn carrier_hz(&self) -> f32 {
        self.sources.as_ref()
            .and_then(|s| s.am_dsb.as_ref())
            .and_then(|a| a.carrier_hz)
            .unwrap_or(Defaults::CARRIER_HZ)
    }
    pub fn mod_index(&self) -> f32 {
        self.sources.as_ref()
            .and_then(|s| s.am_dsb.as_ref())
            .and_then(|a| a.mod_index)
            .unwrap_or(Defaults::MOD_INDEX)
    }
    pub fn am_gap_secs(&self) -> f32 {
        self.sources.as_ref()
            .and_then(|s| s.am_dsb.as_ref())
            .and_then(|a| a.gap_secs)
            .unwrap_or(Defaults::AM_GAP_SECS)
    }
    pub fn am_noise_amp(&self) -> f32 {
        self.sources.as_ref()
            .and_then(|s| s.am_dsb.as_ref())
            .and_then(|a| a.noise_amp)
            .unwrap_or(Defaults::AM_NOISE_AMP)
    }
    pub fn am_msg_repeat(&self) -> usize {
        self.sources.as_ref()
            .and_then(|s| s.am_dsb.as_ref())
            .and_then(|a| a.msg_repeat)
            .map(|v| (v as usize).max(1))
            .unwrap_or(1)
    }
}
