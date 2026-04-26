// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Per-source application glue: settings → live source construction and sync.
//!
//! Each `<S>.rs` module here owns the bin-side mapping between
//! `SettingsState` (UI values) and `crate::source::<S>` (signal generation).
//! The lib's `<S>Source::apply_params(...)` does the actual field updates and
//! change-detection; this layer reads the settings, dispatches to that
//! method, and threads any returned flags back to `ViewApp`.

pub(super) mod amdsb;
pub(super) mod cw;
pub(super) mod ft8;
pub(super) mod psk31;
pub(super) mod tone;

use crate::app::settings::SettingsState;
use crate::decode::DecodeMode;
use crate::source::SignalSource;
use crate::source::ft8::Ft8ViewState;

/// Per-source orchestration trait.  Implemented by a unit type (ZST) per
/// source; lives at `app::source::<S>::Factory`.  `ViewApp` holds a static
/// table indexed by source-mode index, so dispatch is a single trait call
/// with no `match`.
///
/// Adding a new source: implement this trait for a new ZST, push it into
/// `FACTORIES`.  `app/sources.rs` doesn't change.
pub(super) trait SourceFactory: Sync {
    /// Construct a fresh signal source from current settings.
    fn make(&self, settings: &SettingsState) -> Box<dyn SignalSource>;

    /// Decode mode for this source.  `ft8_view` is consulted only by the
    /// FT8 factory (FT8 ↔ FT4 split); other sources ignore it.
    fn decode_mode(&self, settings: &SettingsState, ft8_view: &Ft8ViewState) -> DecodeMode;

    /// Carrier frequency for this source, read from settings.
    fn decode_carrier_hz(&self, settings: &SettingsState) -> f32;

    /// Write a new carrier frequency into this source's settings rows
    /// (called by the source-locked center-frequency tracker).
    fn set_carrier_hz(&self, settings: &mut SettingsState, hz: f32);
}

/// Static dispatch table of per-source factories, indexed by `SourceMode as
/// usize`.  Order MUST match the `SourceMode` enum.
pub(super) static FACTORIES: &[&'static (dyn SourceFactory + Sync)] = &[
    &tone::Factory,
    &cw::Factory,
    &amdsb::Factory,
    &psk31::Factory,
    &ft8::Factory,
];
