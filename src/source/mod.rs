// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

mod common;

pub mod amdsb;
pub mod cofdm;
pub mod cw;
pub mod ft8;
pub mod psk31;
pub mod tone;

pub use common::{
    CnNoise, CnReference, MAX_CN_DB, MAX_SIG_SECS, MIN_CN_DB, NoiseDomain, SignalSource,
    keyed_carrier_power, mean_power, mean_power_c,
};

#[allow(unused_imports)]
pub use amdsb::{AmDsbSource, BuiltinAudio, load_builtin};
#[allow(unused_imports)]
pub use cofdm::{
    COFDM_CP_LEN, COFDM_DEFAULT_CN_DB, COFDM_DISPLAY_RMS_DBFS, COFDM_FS, COFDM_MAX_EDGE_GUARD,
    COFDM_MIN_EDGE_GUARD, COFDM_N_FFT, COFDM_NOMINAL_CENTER, COFDM_PAYLOAD_BYTES,
    COFDM_SHAPING_SLACK, CofdmBwFraction, CofdmMask, CofdmRx, CofdmRxFacts, CofdmRxStats,
    CofdmShaping, CofdmSource, CofdmTaper, cofdm_data_carriers, cofdm_edge_guard_for,
    cofdm_link_config, cofdm_mcs_facts, cofdm_occupied_bw, cofdm_occupied_half,
};
#[allow(unused_imports)]
pub use cw::CwSource;
#[allow(unused_imports)]
pub use ft8::{Ft8Mode, Ft8MsgType, Ft8Source};
#[allow(unused_imports)]
pub use psk31::{Psk31Mode, Psk31Source};
