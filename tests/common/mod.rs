// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Shared test support modules.
//!
//! Each top-level file under `tests/` is compiled as its own crate, so code
//! reuse between test files must go through a `common` subdirectory.  Files
//! here are *not* auto-discovered as test binaries; test files explicitly opt
//! in with `mod common;` and reference `common::ticker`, etc.

#![allow(dead_code)] // each test binary uses a subset of this module

/// The app layer needs egui, so it is only reachable when `gui` is on.
#[cfg(feature = "gui")]
pub mod harness;
pub mod ticker;
