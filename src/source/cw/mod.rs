// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

mod config;
mod decode;
mod source;

#[allow(unused_imports)]
pub use config::CwConfig;
pub use decode::{CwState, cw_char_timing, morse_char_units};
pub use source::{
    CW_DEFAULT_CANNED_TEXT, CW_DEFAULT_CARRIER_HZ, CW_DEFAULT_CHAR_SPACE, CW_DEFAULT_CUSTOM_TEXT,
    CW_DEFAULT_DASH_WEIGHT, CW_DEFAULT_FALL_MS, CW_DEFAULT_GAP_SECS, CW_DEFAULT_JITTER_PCT,
    CW_DEFAULT_NOISE_AMP, CW_DEFAULT_REPEAT, CW_DEFAULT_RISE_MS, CW_DEFAULT_WORD_SPACE,
    CW_DEFAULT_WPM, CwSource,
};
