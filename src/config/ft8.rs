use serde::Deserialize;
use super::Defaults;

#[derive(Debug, Deserialize)]
pub struct Ft8Config {
    pub mode:          Option<String>,
    pub carrier_hz:    Option<f32>,
    pub loop_gap_secs: Option<f32>,
    pub noise_amp:     Option<f32>,
    pub call_to:       Option<String>,
    pub call_de:       Option<String>,
    pub grid:          Option<String>,
    pub free_text:     Option<String>,
    pub msg_repeat:    Option<u32>,
}

impl super::ViewConfig {
    pub fn ft8_mode(&self) -> &str {
        self.sources.as_ref()
            .and_then(|s| s.ft8.as_ref())
            .and_then(|f| f.mode.as_deref())
            .unwrap_or("FT8")
    }
    pub fn ft8_carrier_hz(&self) -> f32 {
        self.sources.as_ref()
            .and_then(|s| s.ft8.as_ref())
            .and_then(|f| f.carrier_hz)
            .unwrap_or(crate::source::ft8::FT8_DEFAULT_CARRIER_HZ)
    }
    pub fn ft8_loop_gap_secs(&self) -> f32 {
        self.sources.as_ref()
            .and_then(|s| s.ft8.as_ref())
            .and_then(|f| f.loop_gap_secs)
            .unwrap_or(crate::source::ft8::FT8_DEFAULT_LOOP_GAP_SECS)
    }
    pub fn ft8_noise_amp(&self) -> f32 {
        self.sources.as_ref()
            .and_then(|s| s.ft8.as_ref())
            .and_then(|f| f.noise_amp)
            .unwrap_or(Defaults::AM_NOISE_AMP)
    }
    pub fn ft8_call_to(&self) -> &str {
        self.sources.as_ref()
            .and_then(|s| s.ft8.as_ref())
            .and_then(|f| f.call_to.as_deref())
            .unwrap_or(crate::source::ft8::FT8_DEFAULT_CALL_TO)
    }
    pub fn ft8_call_de(&self) -> &str {
        self.sources.as_ref()
            .and_then(|s| s.ft8.as_ref())
            .and_then(|f| f.call_de.as_deref())
            .unwrap_or(crate::source::ft8::FT8_DEFAULT_CALL_DE)
    }
    pub fn ft8_grid(&self) -> &str {
        self.sources.as_ref()
            .and_then(|s| s.ft8.as_ref())
            .and_then(|f| f.grid.as_deref())
            .unwrap_or(crate::source::ft8::FT8_DEFAULT_GRID)
    }
    pub fn ft8_free_text(&self) -> &str {
        self.sources.as_ref()
            .and_then(|s| s.ft8.as_ref())
            .and_then(|f| f.free_text.as_deref())
            .unwrap_or(crate::source::ft8::FT8_DEFAULT_FREE_TEXT)
    }
    pub fn ft8_msg_repeat(&self) -> usize {
        self.sources.as_ref()
            .and_then(|s| s.ft8.as_ref())
            .and_then(|f| f.msg_repeat)
            .map(|v| (v as usize).max(1))
            .unwrap_or(crate::source::ft8::FT8_DEFAULT_REPEAT)
    }
}
