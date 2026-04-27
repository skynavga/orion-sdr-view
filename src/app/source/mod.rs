// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Per-source application glue: settings → live source construction and sync.
//!
//! Each `<S>.rs` module here owns the bin-side mapping between
//! `SettingsState` (UI values) and `crate::source::<S>` (signal generation).
//! The lib's `<S>Source::apply_params(...)` does the actual field updates and
//! change-detection; this layer reads the settings, dispatches to that
//! method, and threads any returned flags back to `ViewApp`.
//!
//! Shared bin-side helpers live in `common.rs`.

pub(super) mod amdsb;
mod common;
pub(super) mod cw;
pub(super) mod ft8;
pub(super) mod psk31;
pub(super) mod tone;

pub(super) use common::{
    BURST_CLOSE_DELIMITER, FACTORIES, SourceFactory, debug_assert_factory_order,
    format_burst_open_delimiter,
};
