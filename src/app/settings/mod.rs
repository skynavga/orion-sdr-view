// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

mod amdsb;
mod common;
mod cw;
mod display;
mod field;
mod ft8;
mod psk31;
mod tone;

pub use common::SettingsState;

#[allow(unused_imports)]
pub use common::HandleKeysResult;

// Per-source typed-accessor traits.  Bin call sites bring them in scope via
// `use crate::app::settings::{CwSettings, ...}` (or the `*` umbrella).
pub(in crate::app) use amdsb::AmDsbSettings;
pub(in crate::app) use cw::CwSettings;
pub(in crate::app) use ft8::Ft8Settings;
pub(in crate::app) use psk31::Psk31Settings;
pub(in crate::app) use tone::ToneSettings;
