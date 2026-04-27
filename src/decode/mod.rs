// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

mod common;
pub mod spectral;

pub use common::{
    DecodeConfig, DecodeMode, DecodeResult, DecodeTicker, DecodeWorker, SPECTRUM_WINDOW_SAMPLES,
};

// Re-export used by the binary.
pub use orion_sdr::util::SIGNAL_THRESHOLD;

// Re-exports for integration tests (not used by the binary itself).
#[allow(unused_imports)]
pub use crate::source::cw::{cw_char_timing, morse_char_units};
#[allow(unused_imports)]
pub use crate::source::ft8::{FT4_BW_HZ, FT8_BW_HZ};
#[allow(unused_imports)]
pub use crate::source::psk31::{
    INFO_INTERVAL, PSK31_MAX_ACCUM_SYMS, SYNC_MIN_SYMS, SYNC_SEARCH_HZ,
};
#[allow(unused_imports)]
pub use orion_sdr::codec::psk31::Psk31Stream;
#[allow(unused_imports)]
pub use orion_sdr::util::{
    PSK31_BW_HZ, best_sync, power_spectrum, spectrum_bw_hz, spectrum_snr_db,
};
