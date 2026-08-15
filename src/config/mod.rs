// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

mod capture;
mod common;
mod defaults;
mod display;

pub use capture::{CaptureConfig, CaptureFormat, expand_tilde};
pub use common::ViewConfig;

pub use common::SourcesConfig;
pub use defaults::Defaults;
pub use display::{DisplayConfig, TzMode, format_offset_min};

// Per-source configs are defined under src/source/<S>/config.rs and re-exported
// here so existing `crate::config::<S>Config` paths keep working.
pub use crate::source::amdsb::AmDsbConfig;
pub use crate::source::cofdm::CofdmConfig;
pub use crate::source::cw::CwConfig;
pub use crate::source::ft8::Ft8Config;
pub use crate::source::psk31::Psk31Config;
pub use crate::source::tone::TestToneConfig;
