// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Shared bin-side helpers for sources.  Anything that's source-related but
//! not specific to a single per-source `<S>.rs` module lives here.

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
pub(in crate::app) trait SourceFactory: Sync {
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
pub(in crate::app) static FACTORIES: &[&'static (dyn SourceFactory + Sync)] = &[
    &super::tone::Factory,
    &super::cw::Factory,
    &super::amdsb::Factory,
    &super::psk31::Factory,
    &super::ft8::Factory,
];

/// Belt-and-suspenders: panic loudly at startup if `FACTORIES` ever drifts
/// from the `SourceMode` enum.  If this fires, every `source_mode_factory()`
/// call would silently dispatch to the wrong source's `make`/`decode_mode`/
/// `set_carrier_hz`, producing the wrong source type or carrier setter.
/// Failing here is much easier to diagnose than failing later inside an
/// M-key handler or the source-locked carrier tracker.
///
/// Called from `ViewApp::new` once at startup; runs in debug builds only.
pub(in crate::app) fn debug_assert_factory_order(settings: &SettingsState) {
    use crate::app::SourceMode;
    let view = Ft8ViewState::new();
    debug_assert_eq!(
        FACTORIES[SourceMode::TestTone as usize].decode_mode(settings, &view),
        DecodeMode::TestTone,
        "FACTORIES order mismatch at TestTone"
    );
    debug_assert_eq!(
        FACTORIES[SourceMode::Cw as usize].decode_mode(settings, &view),
        DecodeMode::Cw,
        "FACTORIES order mismatch at Cw"
    );
    debug_assert_eq!(
        FACTORIES[SourceMode::AmDsb as usize].decode_mode(settings, &view),
        DecodeMode::AmDsb,
        "FACTORIES order mismatch at AmDsb"
    );
    // PSK31's decode_mode depends on the BPSK31/QPSK31 toggle; default is BPSK31.
    debug_assert_eq!(
        FACTORIES[SourceMode::Psk31 as usize].decode_mode(settings, &view),
        DecodeMode::Bpsk31,
        "FACTORIES order mismatch at Psk31"
    );
    // FT8's decode_mode reads from Ft8ViewState; default `view.mode` is Ft8.
    debug_assert_eq!(
        FACTORIES[SourceMode::Ft8 as usize].decode_mode(settings, &view),
        DecodeMode::Ft8,
        "FACTORIES order mismatch at Ft8"
    );
}

// ── Burst delimiters (shared by sources that decode incrementally) ──────────
//
// Modes that emit text character-by-character during a burst (CW, PSK31)
// frame each burst in the Dt ticker as `"|| HH:MM:SS.mmm | <text> ||"` —
// matching the FT8 frame format produced by `Ft8ViewState::format_decoded_text`.
// The opening delimiter is pushed on the loop-timer signal-onset edge; the
// closing delimiter is pushed on the gap-onset edge.

/// Closing delimiter pushed on the loop-timer gap-onset edge.
pub(in crate::app) const BURST_CLOSE_DELIMITER: &str = " ||";

/// Opening delimiter pushed on the loop-timer signal-onset edge:
/// `"|| HH:MM:SS.mmm | "`.  `onset` is the captured rising-edge time.
pub(in crate::app) fn format_burst_open_delimiter(
    onset: std::time::SystemTime,
    time_zone_offset_min: i32,
) -> String {
    let ts = crate::utils::format::format_time(onset, time_zone_offset_min);
    let ts_str = if ts.is_empty() {
        "--:--:--.---".to_owned()
    } else {
        ts
    };
    format!("|| {ts_str} | ")
}
