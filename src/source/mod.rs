// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

mod common;

pub mod amdsb;
pub mod cofdm;
pub mod cw;
pub mod dvbt;
pub mod ft8;
pub mod psk31;
pub mod tone;

pub use common::{
    CONTINUOUS_SIG_SECS, CnNoise, CnReference, MAX_CN_DB, MAX_SIG_SECS, MIN_CN_DB, NoiseDomain,
    SignalSource, is_continuous_sig, keyed_carrier_power, mean_power, mean_power_c,
};

pub use amdsb::{AmDsbSource, BuiltinAudio, load_builtin};
pub use cofdm::{
    COFDM_CP_LEN, COFDM_DEFAULT_CN_DB, COFDM_DEFAULT_FS, COFDM_DISPLAY_RMS_DBFS,
    COFDM_MAX_EDGE_GUARD, COFDM_MAX_FS, COFDM_MIN_FS, COFDM_N_FFT, COFDM_PAYLOAD_BYTES,
    COFDM_SHAPING_SLACK, CofdmBwFraction, CofdmMask, CofdmRx, CofdmRxFacts, CofdmRxStats,
    CofdmShaping, CofdmSource, CofdmTaper, cofdm_center_bounds, cofdm_clamp_fs,
    cofdm_data_carriers, cofdm_default_center_hz, cofdm_edge_guard_for, cofdm_link_config,
    cofdm_mcs_facts, cofdm_min_edge_guard, cofdm_occupied_bw, cofdm_occupied_half,
    cofdm_spacing_hz,
};
pub use cw::CwSource;
pub use dvbt::{
    DVBT_DEFAULT_BANDWIDTH, DVBT_DEFAULT_CN_DB, DVBT_DEFAULT_CODE_RATE,
    DVBT_DEFAULT_CONSTELLATION, DVBT_DEFAULT_GUARD, DVBT_DISPLAY_OVERSAMPLE,
    DVBT_DISPLAY_RMS_DBFS, DVBT_RX_WINDOW_BACKOFF, DVBT_SHAPING_SLACK, DVBT_SYMBOLS_PER_FRAME,
    DvbTBandwidth, DvbTMask, DvbTRx, DvbTRxFacts, DvbTRxStats, DvbTShaping, DvbTSource, DvbTTaper,
    dvbt_center_bounds, dvbt_clamp_center, dvbt_default_center_hz, dvbt_frame_payload_bytes,
    dvbt_super_frame_samples,
};
pub use ft8::{Ft8Mode, Ft8MsgType, Ft8Source};
pub use psk31::{Psk31Mode, Psk31Source};
