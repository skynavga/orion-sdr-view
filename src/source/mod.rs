// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

mod common;

pub mod amdsb;
pub mod cofdm;
pub mod cw;
pub mod ft8;
pub mod psk31;
pub mod tone;

pub use common::{MAX_SIG_SECS, SignalSource};

#[allow(unused_imports)]
pub use amdsb::{AmDsbSource, BuiltinAudio, load_builtin};
#[allow(unused_imports)]
pub use cofdm::{
    COFDM_CP_LEN, COFDM_FS, COFDM_GAIN, COFDM_MAX_EDGE_GUARD, COFDM_MAX_NOISE_AMP,
    COFDM_MIN_EDGE_GUARD, COFDM_N_FFT, COFDM_NOMINAL_CENTER, COFDM_PAYLOAD_BYTES,
    COFDM_SHAPING_SLACK, COFDM_SIGNAL_THRESHOLD, CofdmBwFraction, CofdmMask, CofdmShaping,
    CofdmSource, CofdmTaper, cofdm_data_carriers, cofdm_edge_guard_for, cofdm_mcs_facts,
    cofdm_occupied_bw, cofdm_occupied_half,
};
#[allow(unused_imports)]
pub use cw::CwSource;
#[allow(unused_imports)]
pub use ft8::{Ft8Mode, Ft8MsgType, Ft8Source};
#[allow(unused_imports)]
pub use psk31::{Psk31Mode, Psk31Source};
