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

    /// Push this source's committed message text into the live source.
    ///
    /// The path Enter in the settings popover already takes, reached here by
    /// dispatch so a `set` on a text row commits the same way rather than
    /// through a second copy of the mapping.  Sources with no message text rely
    /// on the no-op default.
    fn apply_message(&self, _source: &mut dyn SignalSource, _settings: &SettingsState) {}

    /// The rows a script's `set` directive may name for this source, in the
    /// config file's spelling.  Defined beside the row indices they point at,
    /// in `app::settings::<S>`, and surfaced here because the script parser
    /// resolves a key before any `SettingsState` exists.
    ///
    /// Required rather than defaulted to empty: a source whose settings no
    /// script can reach is a decision, and it should be made rather than
    /// inherited.
    fn set_keys(&self) -> &'static [crate::app::settings::SetKey];

    /// Requested carrier-to-noise ratio (dB) for this source, read from
    /// settings.  Shown in the top HUD.
    ///
    /// Uniform across sources only because the impairment is a *ratio*: while
    /// it was an absolute amplitude the same number meant a different link on
    /// every source, so this was a six-arm `match` in `view.rs` with a comment
    /// apologising for it.
    fn cn_db(&self, settings: &SettingsState) -> f32;

    // ── Wideband viewport preferences ───────────────────────────────────────
    //
    // Narrowband sources return `None` for the two *viewport* preferences, so
    // the window is never auto-reframed on switch (historical behavior).  A
    // wideband source returns `Some(..)` for both so `switch_source` frames its
    // band, sizes the spectrum span, and widens the horizontal-spectrogram
    // window automatically.  The source's sample rate itself is not a
    // preference here — it flows from `SignalSource::sample_rate()` on the
    // constructed source.
    //
    // The two *scale* preferences are different: each defaults to the shared
    // `Defaults` value rather than to `None`, so every source states both.
    // That matters because COFDM's reference is 21 dB away from the shared one —
    // a source that declared no preference would simply inherit whatever COFDM
    // last set and draw its spectrum against a scale meant for a different
    // waveform.  The same argument applies to the floor now that one source
    // moves it, which is why it is stated the same way rather than left as an
    // override that never gets undone.

    /// Nominal center frequency to place at the display center on switch.
    fn nominal_center_hz(&self, _settings: &SettingsState) -> Option<f32> {
        None
    }

    /// Preferred spectrum viewport span (Hz) on switch.
    fn preferred_span_hz(&self, _settings: &SettingsState) -> Option<f32> {
        None
    }

    /// Preferred spectrum reference level (dBFS, scale top) on switch.
    ///
    /// Defaults to the shared level; override only for a source whose signal
    /// does not sit near it.
    fn preferred_ref_db(&self, _settings: &SettingsState) -> Option<f32> {
        Some(crate::config::Defaults::DB_MAX)
    }

    /// Preferred spectrum floor (dBFS, scale bottom) on switch.
    ///
    /// **The reference level alone does not size the window**, and that is what
    /// this exists for: `preferred_ref_db` moves the *top* while the floor stays
    /// where the config left it, so a source that prefers a low reference gets a
    /// *shorter* scale rather than a lower one.  DVB-T is the first source for
    /// which that matters — its power spreads over 83% of the display span, so
    /// its per-bin level sits ~10 dB under COFDM's, and at the shared -80 dB
    /// floor the injected noise fell off the bottom of the scale entirely.  The
    /// spectrum still looked plausible: a band with nothing below it reads as a
    /// clean channel rather than as a clipped one.
    ///
    /// Stated by every source rather than overridden by one, for the reason
    /// `preferred_ref_db` is: a `None` here would leave DVB-T's floor in place
    /// after switching away from it.
    fn preferred_db_min(&self, _settings: &SettingsState) -> Option<f32> {
        Some(crate::config::Defaults::DB_MIN)
    }
}

/// Static dispatch table of per-source factories, indexed by `SourceMode as
/// usize`.  Order MUST match the `SourceMode` enum.
pub(in crate::app) static FACTORIES: &[&'static (dyn SourceFactory + Sync)] = &[
    &super::tone::Factory,
    &super::cw::Factory,
    &super::amdsb::Factory,
    &super::psk31::Factory,
    &super::ft8::Factory,
    &super::cofdm::Factory,
    &super::dvbt::Factory,
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
    debug_assert_eq!(
        FACTORIES[SourceMode::Cofdm as usize].decode_mode(settings, &view),
        DecodeMode::Cofdm,
        "FACTORIES order mismatch at Cofdm"
    );
    debug_assert_eq!(
        FACTORIES[SourceMode::DvbT as usize].decode_mode(settings, &view),
        DecodeMode::DvbT,
        "FACTORIES order mismatch at DvbT"
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
