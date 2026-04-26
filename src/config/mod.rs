// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

mod common;
mod defaults;
mod display;

pub use common::ViewConfig;

#[allow(unused_imports)]
pub use common::SourcesConfig;
#[allow(unused_imports)]
pub use defaults::Defaults;
#[allow(unused_imports)]
pub use display::{DisplayConfig, TzMode, format_offset_min};

// Per-source configs are defined under src/source/<S>/config.rs and re-exported
// here so existing `crate::config::<S>Config` paths keep working.
#[allow(unused_imports)]
pub use crate::source::amdsb::AmDsbConfig;
#[allow(unused_imports)]
pub use crate::source::cw::CwConfig;
#[allow(unused_imports)]
pub use crate::source::ft8::Ft8Config;
#[allow(unused_imports)]
pub use crate::source::psk31::Psk31Config;
#[allow(unused_imports)]
pub use crate::source::tone::TestToneConfig;
