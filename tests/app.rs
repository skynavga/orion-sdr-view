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
use orion_sdr_view::viewport::FreqView;

// ── A. `L` on COFDM ─────────────────────────────────────────────────────────

#[test]
fn the_lock_key_retunes_the_cofdm_band() {
    // `L` was a documented no-op on COFDM until 0.0.24: the band centre was a
    // constant, so `set_carrier_hz` had nothing to write.  Now it writes the
    // `Center` row, and the whole point is that the band follows the viewport.
    let mut h = Harness::with_defaults();
    h.select_source(SourceMode::Cofdm);
    let before = h.app.settings().cofdm_center_hz();

    // Zoom in first.  A *locked* pan is band-limited, and at full span that is a
    // no-op by construction — the centre clamp range collapses to a point — so a
    // lock test that skipped this would pass against a broken `L` as readily as
    // a working one.  (An unlocked pan does move at full span; it is allowed to
    // leave the band, which is the whole difference.)
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
fn a_pan_from_full_span_moves_without_zooming() {
    // The handler used to zoom in on the first ←/→ press, because the old pan
    // was inert at full span and an inert key reads as broken.  That auto-zoom
    // was an unwinnable trade — it magnified the signal to buy pan range — and
    // overscan removes the need for it: at full span the whole band now slides
    // sideways.  So the press pans, and the `Zoom` row stays where it was.
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
        1.0,
        "panning must no longer zoom to make room for itself"
    );
    assert_ne!(
        h.app.freq_view().center_hz,
        before,
        "the press should pan even at full span"
    );
    assert_eq!(
        h.app.settings().zoom_ratio(),
        h.app.freq_view().zoom_ratio(),
        "the Zoom row must still track the viewport"
    );
}

#[test]
fn engaging_the_lock_pulls_a_panned_out_view_back_into_the_band() {
    // `L` writes the viewport centre into the active source's carrier row, and
    // that row clamps to the source's own range.  Engaging it while panned into
    // empty space would pin the row at its bound while the view stayed out
    // there, so the marker and the band would drift apart with nothing on screen
    // to say why.  The lock re-seats the view instead.
    let mut h = Harness::with_defaults();
    h.select_source(SourceMode::Cofdm);
    let nyquist = h.app.freq_view().nyquist;

    h.key_n(egui::Key::ArrowRight, 40); // out past the top edge
    assert!(
        h.app.freq_view().hi() > nyquist,
        "expected to be panned off the band, hi = {}",
        h.app.freq_view().hi()
    );

    h.key(egui::Key::L);
    assert!(
        h.app.freq_view().hi() <= nyquist && h.app.freq_view().lo() >= 0.0,
        "the lock left the window off the band: [{}, {}]",
        h.app.freq_view().lo(),
        h.app.freq_view().hi()
    );

    // And it stays inside from then on, however hard the arrows are held.
    h.key_n(egui::Key::ArrowRight, 40);
    assert!(
        h.app.freq_view().hi() <= nyquist,
        "a locked pan left the band: hi = {}",
        h.app.freq_view().hi()
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

// ── E. The `Include DC` row ─────────────────────────────────────────────────

#[test]
fn the_include_dc_row_is_reachable_and_reaches_the_source() {
    // The row was withdrawn in 0.0.25 because an occupied DC subcarrier did not
    // demodulate, and restored in 0.0.26 on orion-sdr 0.0.60.  A withdrawn row
    // leaves no trace in the settings state, so nothing but a navigation test
    // can tell "restored" from "still hidden": the toggle is the fifth visible
    // source row only while shaping is on, and `nudge` on the wrong index would
    // silently move the taper instead.
    //
    // `occupying_dc_survives_a_round_trip` in `tests/cofdm_rx.rs` is the other
    // half — that the waveform this produces actually decodes.
    let mut h = Harness::with_defaults();
    h.select_source(SourceMode::Cofdm);
    assert!(
        h.app.settings().cofdm_shaping().enabled,
        "shaping defaults on, which is what makes the row visible"
    );
    assert!(
        !h.app.settings().cofdm_shaping().include_dc,
        "off by default"
    );

    // Open the overlay and walk down to it.  The Source tab's first navigable
    // row is the source *selector*, not the first of the source's own rows, so
    // Include DC is the sixth stop: Source, Center, Bandwidth, Shaping,
    // Edge guard, Include DC.
    h.key(egui::Key::S);
    h.key_n(egui::Key::ArrowDown, 6);
    h.key(egui::Key::ArrowRight);
    assert!(
        h.app.settings().cofdm_shaping().include_dc,
        "the fifth visible row should be Include DC, and nudging it should occupy DC"
    );

    // Everything else must have stayed put, which is what says the walk landed
    // on the right row rather than on a neighbour that happens to be a toggle.
    let s = h.app.settings().cofdm_shaping();
    let d = orion_sdr_view::source::CofdmShaping::default_for(h.app.settings().cofdm_bw_fraction());
    assert_eq!(
        (s.taper, s.mask, s.edge_guard),
        (d.taper, d.mask, d.edge_guard)
    );

    // And the source is rebuilt from it: with DC occupied the plan carries one
    // more data carrier than without.
    h.idle(5);
    let with_dc = orion_sdr_view::source::cofdm_data_carriers(s.edge_guard, true);
    let without = orion_sdr_view::source::cofdm_data_carriers(s.edge_guard, false);
    assert_eq!(
        with_dc,
        without + 1,
        "occupying DC should add exactly one carrier"
    );
}

#[test]
fn the_include_dc_row_hides_with_shaping_off() {
    // `CofdmShaping::effective` returns `derived()` — which never occupies DC —
    // whenever shaping is off, so a visible row there would be a control that
    // does nothing.  This is the same reason edge guard, taper and mask hide.
    let mut h = Harness::with_defaults();
    h.select_source(SourceMode::Cofdm);
    h.key(egui::Key::S);

    // Source, Center, Bandwidth, Shaping — turn shaping off.
    h.key_n(egui::Key::ArrowDown, 4);
    h.key(egui::Key::ArrowLeft);
    assert!(!h.app.settings().cofdm_shaping().enabled);

    // The list has now lost four rows, so the sixth stop is Gap rather than
    // Include DC.  Nudging it must move the gap and leave DC alone — which is
    // what says the shaping group went away wholesale rather than partly.
    let gap = h.app.settings().cofdm_gap_secs();
    h.key_n(egui::Key::ArrowDown, 2);
    h.key(egui::Key::ArrowRight);
    assert_ne!(
        h.app.settings().cofdm_gap_secs(),
        gap,
        "the sixth row should be Gap once the shaping group is hidden"
    );
    assert!(
        !h.app.settings().cofdm_shaping().include_dc,
        "with shaping off there is no reachable Include DC row"
    );
}

// ── F. Determinism ──────────────────────────────────────────────────────────

#[test]
fn a_fixed_dt_gives_the_same_run_twice() {
    // The property everything downstream rests on: both PRNGs are seeded from
    // fixed constants, so once the clock read is out of the per-frame path the
    // same script must produce the same samples.  Comparing the waterfall
    // pixels rather than a scalar checks the whole chain — source, impairment,
    // FFT, dB mapping and the scroll pacing — in one assertion.
    const SCRIPT: &str = "
0.00   source COFDM
0.10   key ArrowUp x3
0.20   key L
0.30   key ArrowRight x4
0.40   assert source COFDM
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

#[test]
fn a_script_names_the_source_it_selects_and_the_source_it_asserts() {
    // Both halves of the same argument: an index is a position in
    // `SourceMode::ALL`, so `key I x5` and `assert source 5` would each keep
    // running — and keep passing — against a different source the moment one is
    // added or reordered.  Names cannot drift that way, and the two sides fold
    // spelling identically, so a script may write either as it likes.
    for &mode in SourceMode::ALL {
        let mut h = Harness::with_defaults();
        h.run_script(&format!(
            "0.00 source {label}\n0.10 assert source {label}\n",
            label = mode.label()
        ));
        assert_eq!(h.app.source_mode(), mode);
    }
    // Spelling is folded on both sides, and independently.
    let mut h = Harness::with_defaults();
    h.run_script("0.00 source am-dsb\n0.10 assert source AM_DSB\n");
    assert_eq!(h.app.source_mode(), SourceMode::AmDsb);
}

#[test]
#[should_panic(expected = "source is COFDM, expected CW")]
fn a_source_assertion_that_is_wrong_says_which_source_it_found() {
    let mut h = Harness::with_defaults();
    h.run_script("0.00 source COFDM\n0.10 assert source CW\n");
}

// ── G. A continuous burst ───────────────────────────────────────────────────

#[test]
fn the_signal_row_reaches_cont_and_hides_the_gap() {
    // `cont` is one press past the top of the finite range, which is the whole
    // reason it is a sentinel rather than a bigger maximum: at a second per
    // press, no usefully long burst is reachable by nudging.
    use orion_sdr_view::source::{CONTINUOUS_SIG_SECS, is_continuous_sig};

    let mut h = Harness::with_defaults();
    h.select_source(SourceMode::Cofdm);
    assert!(!is_continuous_sig(h.app.settings().cofdm_sig_secs()));

    // Source, Center, Bandwidth, Shaping, Edge guard, Include DC, Taper, Mask,
    // Signal — the ninth stop with shaping on.
    h.key(egui::Key::S);
    h.key_n(egui::Key::ArrowDown, 9);
    // Walk to the top: 0.5 s steps to 10 s, then 1 s steps, then the sentinel.
    h.key_n(egui::Key::ArrowRight, 120);
    assert_eq!(h.app.settings().cofdm_sig_secs(), CONTINUOUS_SIG_SECS);
    assert!(is_continuous_sig(h.app.settings().cofdm_sig_secs()));

    // Gap is now hidden, so the next row down is C/N rather than Gap.  Nudging
    // it must move the C/N — which is what says the row list actually shrank.
    let cn = h.app.settings().cofdm_cn_db();
    h.key(egui::Key::ArrowDown);
    h.key(egui::Key::ArrowRight);
    assert_ne!(
        h.app.settings().cofdm_cn_db(),
        cn,
        "with Gap hidden, the row after Signal should be C/N"
    );

    // One press back off the sentinel returns to a finite burst, and Gap with it.
    h.key(egui::Key::ArrowUp);
    h.key(egui::Key::ArrowLeft);
    assert!(!is_continuous_sig(h.app.settings().cofdm_sig_secs()));
    let gap = h.app.settings().cofdm_gap_secs();
    h.key(egui::Key::ArrowDown);
    h.key(egui::Key::ArrowRight);
    assert_ne!(
        h.app.settings().cofdm_gap_secs(),
        gap,
        "Gap should be reachable again once the burst is finite"
    );
}

#[test]
fn a_continuous_burst_never_gaps() {
    // The behaviour the sentinel exists for, and the one the link-budget harness
    // depends on: a gap resets the receiver and restarts its frame accounting,
    // so a measurement that ran past the burst would silently report only its
    // tail.
    let mut h = Harness::from_yaml(
        "
view:
  sources:
    cofdm:
      sig_secs: 1.0e9
",
    );
    h.select_source(SourceMode::Cofdm);
    assert!(
        orion_sdr_view::source::is_continuous_sig(h.app.settings().cofdm_sig_secs()),
        "a large configured sig_secs should mean continuous, not a clamped 99.99"
    );

    // Well past both the old 99.99 s clamp and the default 10 s burst.
    for _ in 0..(120.0 / Harness::DT) as usize {
        h.frame(Vec::new(), egui::Modifiers::default());
    }
    assert!(
        h.app.decode_ticker().last_instrument.is_some(),
        "a continuous burst should still be transmitting after 120 s; a gap \
         would have cleared the instrument"
    );
}
