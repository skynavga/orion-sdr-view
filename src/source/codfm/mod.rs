// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

mod config;
mod decode;
mod source;

#[allow(unused_imports)]
pub use config::CodfmConfig;
pub use decode::CodfmState;
pub use source::{
    CODFM_DEFAULT_BW_FRACTION, CODFM_DEFAULT_GAP_SECS, CODFM_DEFAULT_MASK, CODFM_DEFAULT_NOISE_AMP,
    CODFM_DEFAULT_SHAPING_ENABLED, CODFM_DEFAULT_SIG_SECS, CODFM_DEFAULT_TAPER, CODFM_FS,
    CODFM_MAX_EDGE_GUARD, CODFM_MIN_EDGE_GUARD, CODFM_NOMINAL_CENTER, CODFM_PREFERRED_REF_DB,
    CODFM_SHAPING_SLACK, CodfmBwFraction, CodfmMask, CodfmShaping, CodfmSource, CodfmTaper,
    codfm_edge_guard_for, codfm_occupied_bw, codfm_occupied_half, hud_submode_str,
};
