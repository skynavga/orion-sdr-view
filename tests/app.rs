// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The app layer, driven headless.
//!
//! Every UI-layer defect this project has produced was found by *reading* —
//! `L` inert on COFDM, `M` alive only with the settings popover open,
//! `switch_source` reading rows that `reset_playback` then restored, the `Zoom`
//! row diverging from the keyboard clamp.  None of them needs a rendered pixel;
//! they are all state.  These are those four, plus the reproducibility the
//! injected `dt` buys.
//!
//! Runs against a bare `egui::Context` — no window, no renderer, no GPU.  See
//! `tests/common/harness.rs`.

#![cfg(feature = "gui")]

mod common;

use common::harness::Harness;
use orion_sdr_view::app::SourceMode;
use orion_sdr_view::app::settings::{CofdmSettings, Psk31Settings};
use orion_sdr_view::viewport::{FreqView, PAN_AUTO_ZOOM};

// ── A. `L` on COFDM ─────────────────────────────────────────────────────────

#[test]
fn the_lock_key_retunes_the_cofdm_band() {
    // `L` was a documented no-op on COFDM until 0.0.24: the band centre was a
    // constant, so `set_carrier_hz` had nothing to write.  Now it writes the
    // `Center` row, and the whole point is that the band follows the viewport.
    let mut h = Harness::with_defaults();
    h.select_source(SourceMode::Cofdm);
    let before = h.app.settings().cofdm_center_hz();

    // Zoom in first: at full span `pan` is a no-op by construction (the centre
    // clamp range collapses to a point), so a lock test that skipped this would
    // pass against a broken `L` as readily as a working one.
    h.key_n(egui::Key::ArrowUp, 4);
    h.key(egui::Key::L);
    assert!(h.app.source_locked(), "L should engage the lock");

    h.key_n(egui::Key::ArrowRight, 6);
    let viewport = h.app.freq_view().center_hz;
    let band = h.app.settings().cofdm_center_hz();
    assert!(
        viewport > before,
        "the viewport should have panned up-band: {viewport} vs {before}"
    );
    assert!(
        (band - FreqView::snap_hz(viewport, 10.0)).abs() < 1.0,
        "locked band centre {band} should track the viewport centre {viewport}"
    );
}

#[test]
fn an_unlocked_pan_moves_the_viewport_and_leaves_the_band() {
    // The negative control, and the distinction that matters when reading the
    // display: the top HUD's `ctr` is the *viewport* centre and always moves on
    // a pan; the `X` panel's Tuning `ctr` is the *band* centre and only moves
    // while the lock is engaged.  Conflating the two makes this look broken.
    let mut h = Harness::with_defaults();
    h.select_source(SourceMode::Cofdm);
    h.key_n(egui::Key::ArrowUp, 4);
    h.key_n(egui::Key::ArrowRight, 3);

    assert!(!h.app.source_locked());
    let viewport_before = h.app.freq_view().center_hz;
    let band_before = h.app.settings().cofdm_center_hz();

    h.key_n(egui::Key::ArrowRight, 4);
    assert!(
        h.app.freq_view().center_hz > viewport_before,
        "an unlocked pan must still move the viewport"
    );
    assert_eq!(
        h.app.settings().cofdm_center_hz(),
        band_before,
        "an unlocked pan must not retune the band"
    );
}

// ── B. `M` in both key paths ────────────────────────────────────────────────

#[test]
fn the_mode_key_behaves_the_same_with_the_settings_overlay_open() {
    // `M` cycled PSK31's mode from the main key path and from the settings
    // overlay's repeat of the global keys — two duplicated matches that drifted.
    // They are one shared method now; this is what says they stay one.
    let mut h = Harness::with_defaults();
    h.select_source(SourceMode::Psk31);
    let start = h.app.settings().psk31_mode_str().to_owned();

    h.key(egui::Key::M);
    let closed = h.app.settings().psk31_mode_str().to_owned();
    assert_ne!(
        closed, start,
        "M should cycle the mode with settings closed"
    );

    h.key(egui::Key::S);
    assert!(
        h.app.settings().visible,
        "S should open the settings overlay"
    );
    h.key(egui::Key::M);
    let open = h.app.settings().psk31_mode_str().to_owned();
    assert_ne!(
        open, closed,
        "M should cycle the mode with settings open too"
    );
    assert_eq!(
        open, start,
        "two cycles of a two-way toggle return to the start"
    );
}

#[test]
fn the_mode_key_is_inert_on_cofdm_in_both_paths() {
    // `M` was unbound from COFDM in 0.0.24 rather than completed.  Occupied
    // bandwidth is a 7-way parameter with its own row, not a variant of the
    // waveform — and DVB-T, the next queued source, already means "2K/8K FFT
    // size" by *mode*.  Inertness here is the deliberate answer, so it is worth
    // a test: the failure this guards against is a future source quietly
    // inheriting a binding.
    let mut h = Harness::with_defaults();
    h.select_source(SourceMode::Cofdm);
    let before = h.app.settings().cofdm_bw_fraction();

    h.key(egui::Key::M);
    assert_eq!(h.app.settings().cofdm_bw_fraction(), before);

    h.key(egui::Key::S);
    h.key(egui::Key::M);
    assert_eq!(h.app.settings().cofdm_bw_fraction(), before);
    assert_eq!(
        h.app.source_mode(),
        SourceMode::Cofdm,
        "M must not switch source"
    );
}

// ── C. `switch_source` ordering ─────────────────────────────────────────────

#[test]
fn returning_to_a_source_restores_its_configured_rows() {
    // `switch_source` reads the incoming source's factory preferences from its
    // settings rows, and `reset_playback` — called in the same function —
    // restores those rows to their configured defaults.  Reading them first
    // framed the band wherever the row was left on the way *in*.
    //
    // Note what this does and does not catch.  COFDM's `preferred_span_hz` is
    // full Nyquist, so `reframe` clamps the viewport centre to mid-band whatever
    // it is handed; the ordering is therefore not observable through
    // `freq_view` today.  What is observable — and what the fix protects — is
    // that the row itself is back to the configured value on re-entry.
    let mut h = Harness::from_yaml(
        r#"
view:
  sources:
    cofdm:
      center_hz: 300000.0
"#,
    );
    h.select_source(SourceMode::Cofdm);
    let configured = h.app.settings().cofdm_center_hz();
    assert!(
        (configured - 300_000.0).abs() < 1.0,
        "the configured centre should reach the row: {configured}"
    );

    // Move it well away, the way a locked pan would.
    h.key_n(egui::Key::ArrowUp, 4);
    h.key(egui::Key::L);
    h.key_n(egui::Key::ArrowRight, 6);
    assert!(h.app.settings().cofdm_center_hz() > configured + 1000.0);

    // Drop the lock first.  `I` pairs `switch_source` with
    // `lock_source_to_center`, so a still-engaged lock would legitimately
    // retune the incoming source to the viewport centre on arrival — which is
    // the lock working, not the rows failing to reset.
    h.key(egui::Key::L);
    assert!(!h.app.source_locked());

    // All the way round the source list and back.
    for _ in 0..SourceMode::ALL.len() {
        h.key(egui::Key::I);
    }
    assert_eq!(h.app.source_mode(), SourceMode::Cofdm);
    assert!(
        (h.app.settings().cofdm_center_hz() - configured).abs() < 1.0,
        "re-entry should restore the configured centre, not the panned one: {}",
        h.app.settings().cofdm_center_hz()
    );
}

// ── D. The `Zoom` row and the keyboard ──────────────────────────────────────

#[test]
fn the_zoom_row_follows_the_keyboard() {
    // Two writers of one value.  The keyboard owns the viewport until the next
    // source switch (precedence step 3) and the row has to follow it, or the
    // panel shows a ratio the viewport is not at — and pushes it back in the
    // next time the overlay opens.
    let mut h = Harness::with_defaults();
    for _ in 0..5 {
        h.key(egui::Key::ArrowUp);
        assert_eq!(
            h.app.settings().zoom_ratio(),
            h.app.freq_view().zoom_ratio(),
            "row and viewport diverged while zooming in"
        );
    }
    for _ in 0..8 {
        h.key(egui::Key::ArrowDown);
        assert_eq!(
            h.app.settings().zoom_ratio(),
            h.app.freq_view().zoom_ratio(),
            "row and viewport diverged while zooming out"
        );
    }
    assert_eq!(
        h.app.freq_view().zoom_ratio(),
        1.0,
        "should be back to full span"
    );
}

#[test]
fn a_pan_from_full_span_auto_zooms_and_says_so() {
    // The ←/→ handler has to zoom off full span or the key is inert, and the
    // `Zoom` row has to learn about it — an auto-zoom the row did not see would
    // be pushed straight back to 1.0 the next time the settings overlay opened,
    // silently undoing the pan.
    let mut h = Harness::with_defaults();
    h.select_source(SourceMode::Cofdm);
    assert_eq!(
        h.app.freq_view().zoom_ratio(),
        1.0,
        "COFDM frames full span"
    );
    let before = h.app.freq_view().center_hz;

    h.key(egui::Key::ArrowRight);
    assert_eq!(
        h.app.freq_view().zoom_ratio(),
        PAN_AUTO_ZOOM,
        "the auto-zoom should land on the chosen ratio, not a step_zoom accident"
    );
    assert_ne!(
        h.app.freq_view().center_hz,
        before,
        "the same press should also pan"
    );
    assert_eq!(
        h.app.settings().zoom_ratio(),
        h.app.freq_view().zoom_ratio(),
        "the Zoom row must follow the auto-zoom"
    );
}

#[test]
fn a_source_switch_rebounds_the_zoom_row_to_the_new_nyquist() {
    // The row's upper bound is per-source (`nyquist / MIN_SPAN_HZ`): 24x at
    // 48 kHz, 960x for COFDM at 1.92 MHz.  If the row kept a wideband bound
    // after a switch back to a narrowband source it would display a ratio the
    // viewport had silently refused.
    let mut h = Harness::with_defaults();
    h.select_source(SourceMode::Cofdm);
    assert_eq!(h.app.source_sample_rate(), 1_920_000.0);
    h.key_n(egui::Key::ArrowUp, 60); // far past a narrowband source's 24x
    let wide = h.app.freq_view().zoom_ratio();
    assert!(wide > 24.0, "expected a wideband zoom, got {wide}");

    h.select_source(SourceMode::Cw);
    assert_eq!(h.app.source_sample_rate(), 48_000.0);
    assert_eq!(
        h.app.settings().zoom_ratio(),
        h.app.freq_view().zoom_ratio()
    );
    assert!(
        h.app.freq_view().zoom_ratio() <= h.app.freq_view().max_zoom_ratio(),
        "the row kept a ratio past the new bound"
    );
}

// ── E. Determinism ──────────────────────────────────────────────────────────

#[test]
fn a_fixed_dt_gives_the_same_run_twice() {
    // The property everything downstream rests on: both PRNGs are seeded from
    // fixed constants, so once the clock read is out of the per-frame path the
    // same script must produce the same samples.  Comparing the waterfall
    // pixels rather than a scalar checks the whole chain — source, impairment,
    // FFT, dB mapping and the scroll pacing — in one assertion.
    const SCRIPT: &str = "
0.00   key I x5      # to COFDM
0.10   key ArrowUp x3
0.20   key L
0.30   key ArrowRight x4
0.40   assert source 5
0.40   assert locked 1
";
    let run = || {
        let mut h = Harness::with_defaults();
        h.run_script(SCRIPT);
        h.idle(30);
        let rows: Vec<Vec<egui::Color32>> = h
            .app
            .waterfall()
            .rows_in_display_order()
            .map(<[_]>::to_vec)
            .collect();
        (rows, h.app.freq_view().center_hz, h.app.freq_view().span_hz)
    };
    let (rows_a, c_a, s_a) = run();
    let (rows_b, c_b, s_b) = run();

    assert!(!rows_a.is_empty(), "the run should have committed rows");
    assert_eq!(c_a, c_b, "viewport centre differed between runs");
    assert_eq!(s_a, s_b, "viewport span differed between runs");
    assert_eq!(
        rows_a.len(),
        rows_b.len(),
        "row count differed between runs"
    );
    assert!(
        rows_a == rows_b,
        "the same script produced different waterfall pixels"
    );
}

#[test]
fn every_source_survives_being_driven() {
    // A smoke test over the dispatch tables: each source constructs, produces
    // samples, feeds the decode pipeline and renders CPU-side pixels without
    // panicking.  Cheap, and it is what would catch a new source registered in
    // `FACTORIES` but missing from `SettingsState::new`'s row list.
    for &mode in SourceMode::ALL {
        let mut h = Harness::with_defaults();
        h.select_source(mode);
        h.idle(20);
        assert_eq!(h.app.source_mode(), mode, "{} did not stick", mode.label());
        assert!(
            h.app.source_sample_rate() > 0.0,
            "{} reported no sample rate",
            mode.label()
        );
        assert!(
            h.app.waterfall().filled() > 0,
            "{} produced no waterfall rows",
            mode.label()
        );
    }
}
