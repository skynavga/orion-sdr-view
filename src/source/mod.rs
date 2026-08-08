// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

mod common;

pub mod amdsb;
pub mod codfm;
pub mod cw;
pub mod ft8;
pub mod psk31;
pub mod tone;

pub use common::{MAX_SIG_SECS, SignalSource};

#[allow(unused_imports)]
pub use amdsb::{AmDsbSource, BuiltinAudio, load_builtin};
#[allow(unused_imports)]
pub use codfm::{
    CODFM_FS, CODFM_MAX_EDGE_GUARD, CODFM_MIN_EDGE_GUARD, CODFM_NOMINAL_CENTER,
    CODFM_SHAPING_SLACK, CodfmBwFraction, CodfmMask, CodfmShaping, CodfmSource, CodfmTaper,
    codfm_edge_guard_for, codfm_occupied_bw, codfm_occupied_half,
};
#[allow(unused_imports)]
pub use cw::CwSource;
#[allow(unused_imports)]
pub use ft8::{Ft8Mode, Ft8MsgType, Ft8Source};
#[allow(unused_imports)]
pub use psk31::{Psk31Mode, Psk31Source};
