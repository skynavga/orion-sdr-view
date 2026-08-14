// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

mod common;
pub mod instrument;
pub mod spectral;

pub use common::{
    DecodeChunk, DecodeConfig, DecodeMode, DecodeResult, DecodeTicker, DecodeWorker,
    SPECTRUM_WINDOW_SAMPLES,
};

// Re-export used by the binary.
pub use orion_sdr::util::SIGNAL_THRESHOLD;

// Re-exports for integration tests (not used by the binary itself).
pub use crate::source::cw::{cw_char_timing, morse_char_units};
pub use crate::source::ft8::{FT4_BW_HZ, FT8_BW_HZ};
pub use crate::source::psk31::{
    INFO_INTERVAL, PSK31_MAX_ACCUM_SYMS, SYNC_MIN_SYMS, SYNC_SEARCH_HZ,
};
pub use orion_sdr::codec::psk31::Psk31Stream;
pub use orion_sdr::util::{
    PSK31_BW_HZ, best_sync, nb_spectrum_snr_db, power_spectrum, spectrum_bw_hz, wb_spectrum_snr_db,
};
pub use spectral::wb_cn_db;
