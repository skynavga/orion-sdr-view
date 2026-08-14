// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

pub mod driver;
pub mod dump;

pub use driver::{DEFAULT_TAIL_SECS, RunError, RunOptions, RunSummary, run_file, run_into};
pub use dump::{Dump, Record, sha256_hex};
