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

// ── Pane 3's decoder mode across a burst boundary ───────────────────────────

#[test]
fn a_gap_empties_the_constellation_and_bands_the_correction_map() {
    // **The two halves answer a silence differently, on purpose.** The cloud is
    // a picture of what is arriving *now*, so it resets — holding the last
    // burst's across a gap shows a link that is not there, and the off-scale
    // tally under it would go on quoting a denominator from a transmission that
    // ended. The map is scrollback, so it keeps scrolling and bands instead:
    // how the link failed on the way down is what should still be on screen.
    //
    // This is an *integration* test on purpose. The unit tests in `panes.rs`
    // prove the mechanism; what they cannot see is whether the app's notion of
    // "in a gap" reaches it. It did not, at first: the pane read
    // `decode_ticker.in_gap`, which is only ever set while the `Di`/`Dt` bar is
    // visible — so with the bar off, which is the default, a silence looked
    // exactly like a frozen link.
    use orion_sdr_view::app::correction::CorrectionMap;

    // The colour a silence paints, read back through the public surface rather
    // than duplicated here.
    let no_signal = {
        let mut m = CorrectionMap::new(8);
        m.tick(1.0, true);
        m.rows_in_display_order().next().expect("a row")[0]
    };

    let mut h = Harness::with_defaults();
    h.select_source(SourceMode::Cofdm);
    h.key_n(egui::Key::W, 2); // waterfall -> spectrogram -> constellation

    // Run until the source has decoded something, so there is a cloud and a
    // pace to lose.
    for _ in 0..900 {
        h.idle(1);
        if !h.app.constellation().is_empty() && h.app.correction().committed() > 0 {
            break;
        }
    }
    assert!(
        !h.app.constellation().is_empty(),
        "the receiver should have produced symbols before the first gap"
    );

    // Then run until the burst ends, and catch the pane in the silence.
    let mut banded = false;
    let mut emptied = false;
    for _ in 0..1800 {
        h.idle(1);
        if h.app.constellation().is_empty() {
            emptied = true;
        }
        if h.app
            .correction()
            .rows_in_display_order()
            .next()
            .is_some_and(|r| r[0] == no_signal)
        {
            banded = true;
        }
        if emptied && banded {
            break;
        }
    }
    assert!(emptied, "a gap must empty the constellation");
    assert!(
        banded,
        "a gap must keep the correction map scrolling in its own colour"
    );
    assert_eq!(
        h.app.constellation().off_scale(),
        (0, 0),
        "and reset the off-scale readout with it"
    );
}

#[test]
fn full_stop_holds_pane_threes_decoder_view() {
    // `.` — "full stop" — holds the decoder view so a burst can be read at
    // leisure. At the wide bandwidth fractions the map scrolls a full pane in
    // seconds, so being able to stop it is the difference between seeing an
    // event and knowing one went past.
    //
    // **A hold, not a pause.** The receiver keeps running and the probe keeps
    // arriving; it is simply not folded in. Resuming therefore shows live data
    // rather than fast-forwarding through a backlog, which is what "freeze the
    // picture" means to anyone who presses it.
    let mut h = Harness::with_defaults();
    h.select_source(SourceMode::Cofdm);
    h.key_n(egui::Key::W, 2);

    for _ in 0..900 {
        h.idle(1);
        if h.app.correction().committed() > 0 && !h.app.constellation().is_empty() {
            break;
        }
    }
    assert!(
        h.app.correction().committed() > 0,
        "the map should be scrolling before we try to stop it"
    );

    h.text(".");
    let (rows, symbols) = (
        h.app.correction().committed(),
        h.app.constellation().off_scale().1,
    );
    h.idle(120);
    assert_eq!(
        h.app.correction().committed(),
        rows,
        "held: the map must not scroll"
    );
    assert_eq!(
        h.app.constellation().off_scale().1,
        symbols,
        "held: the cloud must not accumulate either — both halves stop together"
    );

    // **Poll rather than idle a fixed count.**  This harness runs the real
    // decode worker, and `try_send`/`try_recv` deliver when the scheduler gets
    // to them — so under parallel test load a fixed wait can pass with no probe
    // frames delivered at all.  Held-ness is safe to assert after a fixed idle
    // because it is an absence; resumption is not.
    h.text(".");
    let mut resumed_map = false;
    let mut resumed_cloud = false;
    for _ in 0..900 {
        h.idle(1);
        resumed_map |= h.app.correction().committed() > rows;
        // Not `>`: a gap inside the window resets the cloud's counters, and
        // "different" is the claim — that it is live, not that it only grows.
        resumed_cloud |= h.app.constellation().off_scale().1 != symbols;
        if resumed_map && resumed_cloud {
            break;
        }
    }
    assert!(resumed_map, "released: the map resumes");
    assert!(resumed_cloud, "released: so does the cloud");
}

// ── F. DVB-T selectability ──────────────────────────────────────────────────

/// DVB-T reports a display rate twice its waveform's, and the app has to adopt
/// *that* one.
///
/// The band is 83% of the waveform's own rate, so at 1× it does not fit the
/// one-sided span the viewer draws and folds over itself.  If
/// `apply_source_sample_rate` ever read the waveform rate instead, the spectrum
/// would still render — aliased — which is exactly the failure that does not
/// announce itself.
#[test]
fn selecting_dvbt_adopts_the_oversampled_display_rate() {
    use orion_sdr_view::app::settings::DvbTSettings;
    let mut h = Harness::with_defaults();
    h.select_source(SourceMode::DvbT);

    let bw = h.app.settings().dvbt_bandwidth();
    assert_eq!(h.app.source_sample_rate(), bw.display_fs());
    assert_eq!(
        h.app.source_sample_rate(),
        bw.fs() * bw.display_oversample() as f32
    );

    // The axis knows the whole stream, the frame shows the group's width, and
    // the band fits inside the frame with room to spare.
    let view = h.app.freq_view();
    assert!((view.nyquist - bw.display_nyquist_hz()).abs() < 1.0);
    assert!((view.span_hz - bw.display_span_hz()).abs() < 1.0);
    assert!(
        bw.occupied_hz() < bw.display_span_hz(),
        "the band must fit the framed window"
    );
}

/// The `L` key retunes the DVB-T band, as it does every other source.
///
/// Worth its own test rather than trusting the COFDM one: DVB-T's centre range
/// is far tighter — the band width is fixed, so there is no narrower fallback to
/// fall back on — and a clamp that collapsed the range to a point would make
/// `L` silently inert, which is the state COFDM's was in until 0.0.24.
#[test]
fn the_lock_key_retunes_the_dvbt_band() {
    use orion_sdr_view::app::settings::DvbTSettings;
    let mut h = Harness::with_defaults();
    h.select_source(SourceMode::DvbT);

    let (lo, hi) = orion_sdr_view::source::dvbt_center_bounds(h.app.settings().dvbt_bandwidth());
    assert!(hi > lo, "the centre range must not be a point");

    let before = h.app.settings().dvbt_center_hz();
    h.key_n(egui::Key::ArrowUp, 4);
    h.key(egui::Key::L);
    assert!(h.app.source_locked());

    h.key_n(egui::Key::ArrowRight, 6);
    let viewport = h.app.freq_view().center_hz;
    let band = h.app.settings().dvbt_center_hz();
    assert!(
        viewport > before,
        "the viewport should have panned up-band: {viewport} vs {before}"
    );
    assert!(
        (band - FreqView::snap_hz(viewport, 10.0).clamp(lo, hi)).abs() < 1.0,
        "locked band centre {band} should track the viewport centre {viewport}"
    );
}

/// A bandwidth change moves the rate by up to 24×, and everything derived from
/// the rate has to move with it.
///
/// **The first settings row in the app that can change a source's sample
/// rate.**  Until DVB-T the only ways `SignalSource::sample_rate` could move
/// were a source switch and a rebuild, both of which re-derive the display on
/// their own — COFDM's `fs` is config-only precisely to avoid this.  Without the
/// re-derivation in `sync_settings`, the waveform re-renders at the new rate
/// while the frequency axis keeps the old Nyquist: measured before the fix, the
/// source went to 4.80 MS/s while `FreqView` stayed at 1.20 MHz, so the spectrum
/// was drawn against a scale off by 2× with nothing on screen to say so.
#[test]
fn a_dvbt_bandwidth_change_moves_the_rate_and_the_display_together() {
    use orion_sdr_view::app::settings::DvbTSettings;
    let mut h = Harness::with_defaults();
    h.select_source(SourceMode::DvbT);

    // Open settings and walk to the Bandwidth row.  The first row of the
    // overlay is the *source selector*, so the source's own rows start one
    // below it: Source, Center, Bandwidth.
    h.key(egui::Key::S);
    h.key_n(egui::Key::ArrowDown, 3);

    // Every mode, not one step.  The rate does not move on every press — 1M and
    // 2M oversample by 4 and 2 onto the same 4.80 MS/s — so a test built on one
    // transition would be asserting a coincidence.  What holds at all six is
    // that the rate is the mode's own and the axis agrees with it.
    let mut rates = Vec::new();
    for _ in 0..orion_sdr_view::source::DvbTBandwidth::ALL.len() {
        h.key(egui::Key::ArrowRight);
        let bw = h.app.settings().dvbt_bandwidth();
        assert_eq!(h.app.source_mode(), SourceMode::DvbT, "still on DVB-T");
        assert_eq!(h.app.source_sample_rate(), bw.display_fs());
        assert!(
            (h.app.freq_view().nyquist - bw.display_nyquist_hz()).abs() < 1.0,
            "{}: the display Nyquist must follow the rate, not lag it",
            bw.label()
        );

        // And the band lands back mid-frame rather than pinned to whichever edge
        // the clamp left it against — stepping the toggle should look like a
        // mode change, not like the band walking off to one side.
        let center = h.app.settings().dvbt_center_hz();
        let (lo, hi) = orion_sdr_view::source::dvbt_center_bounds(bw);
        assert!(
            (lo..=hi).contains(&center),
            "{}: centre {center} outside {lo}..{hi}",
            bw.label()
        );
        assert!(
            (center - bw.display_span_hz() / 2.0).abs() < 1.0,
            "{}: a mode change should land the band mid-frame",
            bw.label()
        );
        rates.push(h.app.source_sample_rate());
    }
    h.key(egui::Key::S);
    // The rate really does move across the six, so the re-derivation above is
    // being exercised rather than passing on a stationary axis.
    assert!(
        rates.iter().any(|r| *r != rates[0]),
        "expected the rate to move somewhere across the six modes: {rates:?}"
    );
}

/// A bandwidth change has to move the *viewport*, not just the Nyquist behind
/// it.
///
/// The distinction the previous test does not make, and the one that shipped
/// broken: `apply_source_sample_rate` re-derives the axis, but `FreqView` keeps
/// whatever centre and span it was on.  A step from 1M to 2M therefore left the
/// window centred on the *outgoing* band centre while the band itself had been
/// re-centred an octave up — measured on screen, the trace ran off the right
/// edge with the left sixth of the window sitting below 0 Hz.  Every number in
/// the HUD agreed with every other; only the picture was wrong.
#[test]
fn a_dvbt_bandwidth_change_reframes_the_viewport() {
    use orion_sdr_view::app::settings::DvbTSettings;
    let mut h = Harness::with_defaults();
    h.select_source(SourceMode::DvbT);

    // Two steps: 1M -> 2M -> 6M, which crosses into the broadcast group.  One
    // step would not do — 1M and 2M share a display width *and* a band centre,
    // so a stale frame would still look right.
    h.key(egui::Key::S);
    h.key_n(egui::Key::ArrowDown, 3);
    h.key(egui::Key::ArrowRight);
    h.key(egui::Key::ArrowRight);
    h.key(egui::Key::S);

    let bw = h.app.settings().dvbt_bandwidth();
    assert_eq!(bw.label(), "6M", "expected to have crossed groups");
    let view = h.app.freq_view();
    assert!(
        (view.center_hz - h.app.settings().dvbt_center_hz()).abs() < 1.0,
        "viewport centre {} should be the band centre {}",
        view.center_hz,
        h.app.settings().dvbt_center_hz()
    );
    assert!(
        (view.span_hz - bw.display_span_hz()).abs() < 1.0,
        "viewport span {} should be the new group width {}",
        view.span_hz,
        bw.display_span_hz()
    );
    // The window that follows holds the whole band with room at both ends, and
    // starts at or above DC — the two ways the stale frame failed.
    let (lo, hi) = (view.lo(), view.hi());
    assert!(lo >= -1.0, "the window should not run below 0 Hz: {lo}");
    let (band_lo, band_hi) = (
        h.app.settings().dvbt_center_hz() - bw.occupied_hz() / 2.0,
        h.app.settings().dvbt_center_hz() + bw.occupied_hz() / 2.0,
    );
    assert!(
        band_lo > lo && band_hi < hi,
        "band {band_lo}..{band_hi} should sit inside the window {lo}..{hi}"
    );
}

/// The frequency axis holds still across a bandwidth change within a group.
///
/// **The point of the whole per-mode oversampling table.**  Framing at the
/// display Nyquist drew every mode's band at the same 83% of a *different*
/// window, so stepping 333k -> 1M -> 2M rescaled the axis under the trace
/// instead of widening the trace on the axis — the one thing a bandwidth toggle
/// exists to show was the one thing it could not.  With the span fixed the
/// band's share of the window *is* its width: 14.5%, 43.5%, 87.0%.
#[test]
fn the_dvbt_axis_holds_still_across_a_bandwidth_change() {
    use orion_sdr_view::app::settings::DvbTSettings;
    let mut h = Harness::with_defaults();
    h.select_source(SourceMode::DvbT);

    // Walk to the Bandwidth row and step down to the narrowest mode, then back
    // up through the group, recording the span and the band's share at each.
    h.key(egui::Key::S);
    h.key_n(egui::Key::ArrowDown, 3);
    h.key(egui::Key::ArrowLeft);

    let mut seen = Vec::new();
    for _ in 0..3 {
        let bw = h.app.settings().dvbt_bandwidth();
        let view = h.app.freq_view();
        seen.push((bw, view.span_hz, view.zoom_ratio()));
        h.key(egui::Key::ArrowRight);
    }
    h.key(egui::Key::S);

    let (_, span, _) = seen[0];
    for &(bw, got, zoom) in &seen {
        assert!(
            (got - span).abs() < 1.0,
            "{}: span {got} should equal the group's {span}",
            bw.label()
        );
        assert!(
            (zoom - 1.0).abs() < 0.01,
            "{}: a framed source should read zoom 1x, got {zoom}",
            bw.label()
        );
        // The band's share of that fixed window is its width and nothing else.
        assert!(
            (bw.occupied_hz() / got - bw.occupied_hz() / span).abs() < 1e-6,
            "{}: fill should follow the width alone",
            bw.label()
        );
    }
    // Three genuinely different widths on one axis, narrowest first.
    let widths: Vec<f32> = seen.iter().map(|(bw, _, _)| bw.occupied_hz()).collect();
    assert!(
        widths[0] < widths[1] && widths[1] < widths[2],
        "expected three increasing widths, got {widths:?}"
    );
}

/// The Nyquist above the framed span is real spectrum, reachable but not framed.
///
/// The half of `set_display_span` that is easy to get wrong in the other
/// direction: narrowing zoom 1x must not narrow the *data*.  At 333k the stream
/// runs to 2.40 MHz while the frame stops at 2.30, and the bin mapping, the
/// texture UVs and the pan bound all still work in the full range.
#[test]
fn the_dvbt_display_span_does_not_shrink_the_spectrum() {
    use orion_sdr_view::app::settings::DvbTSettings;
    let mut h = Harness::with_defaults();
    h.select_source(SourceMode::DvbT);
    h.key(egui::Key::S);
    h.key_n(egui::Key::ArrowDown, 3);
    h.key(egui::Key::ArrowLeft); // 1M -> 333k, the deepest oversampling
    h.key(egui::Key::S);

    let bw = h.app.settings().dvbt_bandwidth();
    assert_eq!(bw.display_oversample(), 12, "expected the 333k mode");
    let view = h.app.freq_view();
    assert!(
        view.nyquist > view.display_span() + 1.0,
        "333k should carry headroom: nyquist {} vs span {}",
        view.nyquist,
        view.display_span()
    );
    assert!(
        (view.nyquist - bw.display_nyquist_hz()).abs() < 1.0,
        "the axis must still know the real Nyquist"
    );
    assert!(
        (view.span_hz - bw.display_span_hz()).abs() < 1.0,
        "but frame the group's width"
    );

    // Panning can still reach the headroom, so the extra spectrum is not walled
    // off — it is merely not where the source opens.
    h.key_n(egui::Key::ArrowUp, 4);
    h.key_n(egui::Key::ArrowRight, 40);
    assert!(
        h.app.freq_view().hi() > bw.display_span_hz(),
        "a pan should be able to leave the framed window"
    );
}

/// The reference level follows the `Bandwidth` row even when the rate does not.
///
/// **The trap in keying display state off the sample rate.**  DVB-T's reference
/// tracks its oversampling factor, and 1M -> 2M steps that factor from 4 to 2
/// while *both* modes render at the same 4.80 MS/s — 1 201 173 x 4 and
/// 2 402 346 x 2.  Hung off the rate guard, the scale would not have moved and
/// the 2M trace would have sat 3 dB up the pane with nothing to say why.
#[test]
fn the_dvbt_reference_follows_the_bandwidth_row() {
    use orion_sdr_view::app::settings::DvbTSettings;
    use orion_sdr_view::source::dvbt_preferred_ref_db;
    let mut h = Harness::with_defaults();
    h.select_source(SourceMode::DvbT);

    let bw_before = h.app.settings().dvbt_bandwidth();
    let rate_before = h.app.source_sample_rate();
    let ref_before = h.app.settings().db_max();
    assert!(
        (ref_before - dvbt_preferred_ref_db(bw_before)).abs() < 0.01,
        "the 1M reference should be its own: {ref_before}"
    );

    h.key(egui::Key::S);
    h.key_n(egui::Key::ArrowDown, 3);
    h.key(egui::Key::ArrowRight); // 1M -> 2M
    h.key(egui::Key::S);

    let bw = h.app.settings().dvbt_bandwidth();
    assert_eq!(bw.label(), "2M");
    assert_eq!(
        h.app.source_sample_rate(),
        rate_before,
        "1M and 2M render at the same rate, which is the whole point here"
    );
    let now = h.app.settings().db_max();
    assert!(
        (now - dvbt_preferred_ref_db(bw)).abs() < 0.01,
        "the reference should have followed the row: {now} vs {}",
        dvbt_preferred_ref_db(bw)
    );
    assert!(
        (now - ref_before).abs() > 1.0,
        "and it should actually have moved: {ref_before} -> {now}"
    );
    // The floor is not part of it — the noise per bin does not move with the
    // factor, so only the top of the scale tracks it.
    assert!(
        (h.app.settings().db_min() - orion_sdr_view::source::DVBT_PREFERRED_FLOOR_DB).abs() < 0.01
    );
}

/// `R` after a bandwidth excursion must restore the band the source started
/// with — the bandwidth *and* the centre that belongs to it.
///
/// **The row default is not a value to clamp.**  `reseed_center_bounds` used to
/// clamp `default` into the new mode's range alongside `value`, which reads as
/// tidy and is not: the default is what `R` restores *together with* the
/// bandwidth row, so one visit to 2M rewrote it from 600.6 kHz to that mode's
/// lower bound of 1.000 MHz and left it there.  `R` then paired a 1M bandwidth
/// with a centre 300 kHz outside its legal range; the source clamped to the band
/// edge, the band drew hard against one side, and the C/N estimator measured a
/// window that no longer held it — 26 dB against a requested 35.  The settings
/// panel read exactly what it had been reset to throughout.
#[test]
fn resetting_after_a_dvbt_bandwidth_change_restores_the_default_band() {
    use orion_sdr_view::app::settings::DvbTSettings;
    let mut h = Harness::with_defaults();
    h.select_source(SourceMode::DvbT);
    let bw_before = h.app.settings().dvbt_bandwidth();
    let center_before = h.app.settings().dvbt_center_hz();

    // Two steps, to 6M: it is in the other bandwidth group, so its centre
    // default differs from 1M's and a failure to restore is visible.  One step
    // lands on 2M, which shares 1M's display width and band centre exactly.
    h.key(egui::Key::S);
    h.key_n(egui::Key::ArrowDown, 3);
    h.key(egui::Key::ArrowRight);
    h.key(egui::Key::ArrowRight);
    h.key(egui::Key::S);
    assert_ne!(h.app.settings().dvbt_bandwidth(), bw_before, "moved off 1M");
    assert!(
        (h.app.settings().dvbt_center_hz() - center_before).abs() > 1.0,
        "the excursion should have moved the centre, or this proves nothing"
    );

    h.key(egui::Key::R);

    let bw = h.app.settings().dvbt_bandwidth();
    let center = h.app.settings().dvbt_center_hz();
    assert_eq!(bw, bw_before, "R restores the bandwidth");
    assert!(
        (center - center_before).abs() < 1.0,
        "R restores the centre that belongs to it: {center} vs {center_before}"
    );
    // And the restored pair is *coherent*: the centre is inside the range its
    // own bandwidth allows, so the source renders it unclamped.
    let (lo, hi) = orion_sdr_view::source::dvbt_center_bounds(bw);
    assert!(
        (lo..=hi).contains(&center),
        "restored centre {center} outside {lo}..{hi}"
    );
    assert!(
        (center - bw.display_span_hz() / 2.0).abs() < 1.0,
        "the restored band should be mid-frame"
    );
}

/// DVB-T lowers the spectrum floor, and every other source gets the shared one
/// back.
///
/// A reference level alone sizes nothing: it moves the top of the scale while
/// the floor stays where the config left it, so DVB-T's -41 dB preference made
/// the window *shorter* rather than lower.  Its power spreads over 83% of the
/// display span, which puts the per-bin trace near -52 dBFS and the injected
/// noise near -88 — below the shared -80 floor, so the out-of-band region drew
/// pinned at the bottom of the scale.  That is what a *noiseless* channel looks
/// like, which is why nothing about it read as broken.
///
/// The half that matters as much: the floor is *given back*.  A preference no
/// other source states would have left DVB-T's -90 in place for whatever was
/// selected next.
#[test]
fn dvbt_lowers_the_display_floor_and_gives_it_back() {
    use orion_sdr_view::source::dvbt::DVBT_PREFERRED_FLOOR_DB;
    let mut h = Harness::with_defaults();

    h.select_source(SourceMode::DvbT);
    let floor = h.app.settings().db_min();
    let top = h.app.settings().db_max();
    assert!(
        (floor - DVBT_PREFERRED_FLOOR_DB).abs() < 0.01,
        "DVB-T should state its own floor: {floor}"
    );
    // Deep enough to hold the noise the default C/N puts under the band.
    assert!(
        top - floor > 35.0 + 10.0,
        "the scale must span the C/N plus headroom: {floor}..{top}"
    );

    h.select_source(SourceMode::Cofdm);
    let floor = h.app.settings().db_min();
    assert!(
        (floor - orion_sdr_view::config::Defaults::DB_MIN).abs() < 0.01,
        "switching away must restore the shared floor, not inherit DVB-T's: {floor}"
    );
}

/// An `X` panel is granted by naming yourself, not by being COFDM.
///
/// The regression this pins: DVB-T built and shipped a full `OfdmInstrument`
/// every block while three render paths each asked `source_mode !=
/// SourceMode::Cofdm` and threw it away, so the panel told the operator "DVB-T
/// has no instrumentation" about a source that had just filled one.
#[test]
fn an_instrument_panel_follows_the_source_not_the_mode() {
    let mut h = Harness::with_defaults();

    h.select_source(SourceMode::Cofdm);
    assert_eq!(h.app.instrument_label(), Some("COFDM"));

    h.select_source(SourceMode::DvbT);
    assert_eq!(
        h.app.instrument_label(),
        Some("DVB-T"),
        "DVB-T fills an instrument, so it must be shown one"
    );

    // And the sources that genuinely have no receiver still say so.
    for mode in [SourceMode::TestTone, SourceMode::Cw, SourceMode::Ft8] {
        h.select_source(mode);
        assert_eq!(
            h.app.instrument_label(),
            None,
            "{} has no receiver filling an instrument",
            mode.label()
        );
    }
}
