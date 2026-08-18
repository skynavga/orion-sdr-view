// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

mod amdsb;
mod cofdm;
mod common;
mod cw;
mod display;
mod field;
mod ft8;
mod psk31;
mod tone;

pub use common::{HandleKeysResult, SetKey, SetOutcome, SetScope, SetTarget, SettingsState};

// Per-source `set` key tables.  Each names its own rows in the config file's
// spelling; `app::source::<S>::Factory::set_keys` hands them to the parser, so
// resolving `set cofdm.cn_db` needs no list anywhere central.
pub(in crate::app) use amdsb::SET_KEYS as AMDSB_SET_KEYS;
pub(in crate::app) use cofdm::SET_KEYS as COFDM_SET_KEYS;
pub(in crate::app) use cw::SET_KEYS as CW_SET_KEYS;
pub(in crate::app) use ft8::SET_KEYS as FT8_SET_KEYS;
pub(in crate::app) use psk31::SET_KEYS as PSK31_SET_KEYS;
pub(in crate::app) use tone::SET_KEYS as TONE_SET_KEYS;

// Per-source typed-accessor traits.  Call sites bring them in scope via
// `use crate::app::settings::{CwSettings, ...}` (or the `*` umbrella).
pub use amdsb::AmDsbSettings;
pub use cofdm::CofdmSettings;
pub use cw::CwSettings;
pub use ft8::Ft8Settings;
pub use psk31::Psk31Settings;
pub use tone::ToneSettings;
