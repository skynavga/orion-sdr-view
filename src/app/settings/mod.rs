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

pub use common::{HandleKeysResult, SettingsState};

// Per-source typed-accessor traits.  Call sites bring them in scope via
// `use crate::app::settings::{CwSettings, ...}` (or the `*` umbrella).
pub use amdsb::AmDsbSettings;
pub use cofdm::CofdmSettings;
pub use cw::CwSettings;
pub use ft8::Ft8Settings;
pub use psk31::Psk31Settings;
pub use tone::ToneSettings;
