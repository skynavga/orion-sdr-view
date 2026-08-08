// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

mod config;
mod decode;
mod source;

#[allow(unused_imports)]
pub use config::CofdmConfig;
pub use decode::CofdmState;
pub use source::{
    COFDM_DEFAULT_BW_FRACTION, COFDM_DEFAULT_GAP_SECS, COFDM_DEFAULT_MASK, COFDM_DEFAULT_NOISE_AMP,
    COFDM_DEFAULT_SHAPING_ENABLED, COFDM_DEFAULT_SIG_SECS, COFDM_DEFAULT_TAPER, COFDM_FS,
    COFDM_MAX_EDGE_GUARD, COFDM_MIN_EDGE_GUARD, COFDM_NOMINAL_CENTER, COFDM_PREFERRED_REF_DB,
    COFDM_SHAPING_SLACK, CofdmBwFraction, CofdmMask, CofdmShaping, CofdmSource, CofdmTaper,
    cofdm_edge_guard_for, cofdm_occupied_bw, cofdm_occupied_half, hud_submode_str,
};
