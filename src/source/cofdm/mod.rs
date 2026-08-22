// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

mod config;
mod decode;
mod rx;
mod source;

pub use config::CofdmConfig;
pub use decode::CofdmState;
pub use rx::{CofdmRx, CofdmRxStats, OfdmRxFacts};
pub use source::{
    COFDM_CP_LEN, COFDM_DEFAULT_BW_FRACTION, COFDM_DEFAULT_CN_DB, COFDM_DEFAULT_FS,
    COFDM_DEFAULT_GAP_SECS, COFDM_DEFAULT_MASK, COFDM_DEFAULT_SHAPING_ENABLED,
    COFDM_DEFAULT_SIG_SECS, COFDM_DEFAULT_TAPER, COFDM_DISPLAY_RMS_DBFS, COFDM_MAX_EDGE_GUARD,
    COFDM_MAX_FS, COFDM_MIN_FS, COFDM_N_FFT, COFDM_PAYLOAD_BYTES, COFDM_PREFERRED_REF_DB,
    COFDM_SHAPING_SLACK, CofdmBwFraction, CofdmMask, CofdmShaping, CofdmSource, CofdmTaper,
    cofdm_center_bounds, cofdm_clamp_fs, cofdm_data_carriers, cofdm_default_center_hz,
    cofdm_edge_guard_for, cofdm_link_config, cofdm_mcs_facts, cofdm_min_edge_guard,
    cofdm_occupied_bw, cofdm_occupied_half, cofdm_spacing_hz, hud_submode_str,
};
