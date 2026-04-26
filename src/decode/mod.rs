// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

pub mod amdsb;
mod common;
pub mod cw;
pub mod ft8;
pub mod psk31;
pub mod spectral;
pub mod tone;

pub use common::{
    DecodeConfig, DecodeMode, DecodeResult, DecodeTicker, DecodeWorker, SPECTRUM_WINDOW_SAMPLES,
};

// Re-export used by the binary.
pub use orion_sdr::util::SIGNAL_THRESHOLD;

// Re-exports for integration tests (not used by the binary itself).
#[allow(unused_imports)]
pub use cw::{cw_char_timing, morse_char_units};
#[allow(unused_imports)]
pub use ft8::{FT4_BW_HZ, FT8_BW_HZ};
#[allow(unused_imports)]
pub use orion_sdr::codec::psk31::Psk31Stream;
#[allow(unused_imports)]
pub use orion_sdr::util::{
    PSK31_BW_HZ, best_sync, power_spectrum, spectrum_bw_hz, spectrum_snr_db,
};
#[allow(unused_imports)]
pub use psk31::{INFO_INTERVAL, PSK31_MAX_ACCUM_SYMS, SYNC_MIN_SYMS, SYNC_SEARCH_HZ};
