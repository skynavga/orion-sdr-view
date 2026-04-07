pub struct Defaults;
impl Defaults {
    pub const DB_MIN:        f32 = -80.0;
    pub const DB_MAX:        f32 = -20.0;
    pub const FREQ_HZ:       f32 = 12_000.0;
    pub const NOISE_AMP:     f32 = 0.05;
    pub const AMP_MAX:       f32 = 0.65;
    pub const RAMP_SECS:     f32 = 3.0;
    pub const PAUSE_SECS:    f32 = 7.0;
    pub const CARRIER_HZ:    f32 = 12_000.0;
    pub const MOD_INDEX:     f32 = 1.0;
    pub const LOOP_GAP_SECS: f32 = 7.0;
    pub const AM_NOISE_AMP:  f32 = 0.05;
}
