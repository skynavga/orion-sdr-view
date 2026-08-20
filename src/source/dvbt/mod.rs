// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

mod config;
mod decode;
mod rx;
mod source;

pub use config::DvbTConfig;
pub use decode::DvbTState;
pub use rx::{DvbTRx, DvbTRxFacts, DvbTRxStats};
pub use source::{
    DVBT_BUFFER_TARGET_SECS, DVBT_CELL_ID, DVBT_CODE_RATES, DVBT_CONSTELLATIONS,
    DVBT_DEFAULT_BANDWIDTH, DVBT_DEFAULT_CN_DB, DVBT_DEFAULT_CODE_RATE, DVBT_DEFAULT_CONSTELLATION,
    DVBT_DEFAULT_GAP_SECS, DVBT_DEFAULT_GUARD, DVBT_DEFAULT_MASK, DVBT_DEFAULT_SHAPING_ENABLED,
    DVBT_DEFAULT_SIG_SECS, DVBT_DEFAULT_TAPER, DVBT_DISPLAY_OVERSAMPLE, DVBT_DISPLAY_RMS_DBFS,
    DVBT_GUARDS, DVBT_MAX_BUFFER_SUPER_FRAMES, DVBT_PREFERRED_REF_DB, DVBT_RX_WINDOW_BACKOFF,
    DVBT_SHAPING_SLACK, DVBT_SYMBOLS_PER_FRAME, DvbTBandwidth, DvbTMask, DvbTShaping, DvbTSource,
    DvbTTaper, code_rate_fraction, code_rate_label, constellation_label, dvbt_buffer_super_frames,
    dvbt_center_bounds, dvbt_clamp_center, dvbt_default_center_hz, dvbt_frame_capacity_bits,
    dvbt_frame_payload_bytes, dvbt_inner_fec, dvbt_super_frame_samples, guard_label,
    hud_submode_str,
};
