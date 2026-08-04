// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

mod config;
mod decode;
mod source;

#[allow(unused_imports)]
pub use config::CodfmConfig;
pub use decode::CodfmState;
pub use source::{
    CODFM_DEFAULT_GAP_SECS, CODFM_DEFAULT_NOISE_AMP, CODFM_FS, CODFM_NOMINAL_CENTER, CodfmSource,
    codfm_occupied_bw, hud_submode_str,
};
