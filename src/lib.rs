// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

/// The viewer application: settings rows, key handling, HUD and panes.
///
/// Behind the `gui` feature because it is built on egui, so
/// `--no-default-features` still gives a pure-DSP library.  It lives here
/// rather than in the binary because the binary cannot be reached from
/// `tests/` — see `ViewApp::advance` for the injected `dt` that makes a
/// scripted run reproducible.
#[cfg(feature = "gui")]
pub mod app;
pub mod config;
pub mod decode;
pub mod source;
pub mod utils;
pub mod viewport;
