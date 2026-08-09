// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Integration tests for the COFDM instrumentation model: value formatting,
//! provenance rendering, the fixed nine-column panel grid, the prioritised
//! Di-bar line, and the simulated metrics' response to measured C/N.
//!
//! `src/app/**` is bin-only, so nothing here can reach the painting side.  That
//! is exactly why the layout arithmetic lives in `src/decode/instrument.rs`:
//! everything below asserts the grid without a GUI.

use orion_sdr_view::decode::instrument::*;
use orion_sdr_view::decode::{DecodeResult, SPECTRUM_WINDOW_SAMPLES};
use orion_sdr_view::source::cofdm::CofdmState;
use orion_sdr_view::source::{
    COFDM_CP_LEN, COFDM_FS, COFDM_GAIN, COFDM_N_FFT, COFDM_NOMINAL_CENTER, CofdmBwFraction,
    CofdmShaping, CofdmSource, SignalSource, cofdm_data_carriers, cofdm_edge_guard_for,
    cofdm_mcs_facts, cofdm_occupied_bw,
};

// ── Fixtures ──────────────────────────────────────────────────────────────────

/// Facts for the synthetic source at a given bandwidth fraction and C/N,
/// built the same way the provider builds them.
fn facts_for(fraction: CofdmBwFraction, cn_db: f32) -> CofdmFacts {
    let guard = CofdmShaping::default_for(fraction)
        .effective(fraction)
        .edge_guard;
    let (constellation, bits_per_symbol, inner_code_rate) = cofdm_mcs_facts();
    CofdmFacts {
        center_hz: COFDM_NOMINAL_CENTER,
        bandwidth_hz: cofdm_occupied_bw(COFDM_FS, guard),
        // Raw amplitudes against the source's own full-scale reference.
        level_amp: 1.8,
        peak_amp: 14.7,
        full_scale: COFDM_GAIN,
        cn_db,
        fs: COFDM_FS,
        n_fft: COFDM_N_FFT,
        cp_len: COFDM_CP_LEN,
        data_carriers: cofdm_data_carriers(guard, false),
        constellation,
        bits_per_symbol,
        inner_code_rate,
        error_count: 0,
        error_count_wrapped: false,
        error_unit: ErrorUnit::Frame,
    }
}

fn facts() -> CofdmFacts {
    facts_for(CofdmBwFraction::OneQuarter, 28.5)
}

fn instrument() -> CofdmInstrument {
    CofdmInstrument::from_facts(&facts())
}

/// Character-count measurer, standing in for the binary's glyph measurer.
fn chars(s: &str) -> f32 {
    s.chars().count() as f32
}

/// Every label the panel renders, in row order.
fn all_labels(inst: &CofdmInstrument) -> Vec<String> {
    let mut out = Vec::new();
    for row in inst.panel_rows() {
        out.push(row.section.to_owned());
        for (i, cell) in row.cells.iter().enumerate() {
            if i % 2 == 0 {
                out.push(cell.text.clone());
            }
        }
        for l in &row.locks {
            out.push(l.label.to_owned());
        }
    }
    out
}

// ── Value formatting ──────────────────────────────────────────────────────────

#[test]
fn ber_renders_in_exponent_form() {
    assert_eq!(fmt_ber(2.1e-4), "2.1E-4");
    assert_eq!(fmt_ber(7.9e-2), "7.9E-2");
    assert_eq!(fmt_ber(1.0), "1.0E0");
}

#[test]
fn ber_floors_and_zeroes() {
    // Exactly zero is a real reading: no errors in the window.
    assert_eq!(fmt_ber(0.0), "0.0E0");
    // Below the floor, a value would be false precision.
    assert_eq!(fmt_ber(1.0e-12), "<1E-9");
    assert_eq!(fmt_ber(BER_FLOOR / 2.0), "<1E-9");
    // At the floor itself a value is still reported.
    assert_eq!(fmt_ber(BER_FLOOR), "1.0E-9");
    // Non-finite input must not panic or print "NaN" into the grid.
    assert_eq!(fmt_ber(f32::NAN), "0.0E0");
}

#[test]
fn ber_mantissa_never_carries_to_ten() {
    // 9.99e-5 rounds to 10.0 at one decimal place; renormalising keeps the
    // rendered width bounded so the column cannot reflow.
    let s = fmt_ber(9.99e-5);
    assert_eq!(s, "1.0E-4", "mantissa carried without renormalising: {s}");
    assert!(s.chars().count() <= 6);
}

#[test]
fn signed_formats_carry_an_explicit_sign() {
    assert_eq!(fmt_signed_hz(123.0), "+123 Hz");
    assert_eq!(fmt_signed_hz(-40.0), "-40 Hz");
    // Exactly zero reads as "+0", not a bare "0" — the sign column stays put.
    assert_eq!(fmt_signed_hz(0.0), "+0 Hz");
    assert_eq!(fmt_ppm(1.4), "+1.4 ppm");
    assert_eq!(fmt_ppm(0.0), "+0.0 ppm");
    assert_eq!(fmt_signed_db(-6.4), "-6.4 dB");
    assert_eq!(fmt_signed_db(0.0), "+0.0 dB");
}

#[test]
fn fft_renders_as_a_mode_label() {
    // A bin count for the synthetic source, a mode label for the DVB sizes —
    // so the field survives a 256-point plan becoming a 2048-point one.
    assert_eq!(fmt_fft_mode(256), "256");
    assert_eq!(fmt_fft_mode(2048), "2K");
    assert_eq!(fmt_fft_mode(8192), "8K");
}

#[test]
fn fractions_reduce() {
    assert_eq!(fmt_fraction(32, 256), "1/8");
    assert_eq!(fmt_fraction(256, 512), "1/2");
    assert_eq!(fmt_fraction(7, 8), "7/8");
}

#[test]
fn overload_is_words_not_a_dot() {
    // The lock dots read "● = good"; an overload dot would have to read
    // "● = bad" in the same panel.
    assert_eq!(fmt_yes_no(false), "no");
    assert_eq!(fmt_yes_no(true), "YES");
}

// ── Provenance ────────────────────────────────────────────────────────────────

#[test]
fn provenance_drives_cell_style() {
    let mut inst = instrument();
    inst.center_hz = Metric::measured(480_000.0);
    inst.constellation = Metric::known("QPSK".to_owned());
    inst.mer_db = Metric::simulated(31.7);
    inst.evm_pct = Metric::unavailable();

    let rows = inst.panel_rows();
    let cell = |section: &str, label: &str| -> Cell {
        let row = rows.iter().find(|r| r.section == section).unwrap();
        let i = row.cells.iter().position(|c| c.text == label).unwrap();
        row.cells[i + 1].clone()
    };

    // Measured and Known both render as authoritative.
    assert_eq!(cell("Tuning", "ctr").style, CellStyle::Normal);
    assert_eq!(cell("Config", "mod").style, CellStyle::Normal);
    // Simulated renders distinctly from measured — this is what the SIM badge
    // and the dim colour hang off.
    assert_eq!(cell("Quality", "MER").style, CellStyle::Simulated);
    assert_ne!(
        cell("Quality", "MER").style,
        cell("Tuning", "ctr").style,
        "a placeholder must not be indistinguishable from a measurement"
    );
    // Unavailable renders as the em-dash, with no number at all.
    let evm = cell("Quality", "EVM");
    assert_eq!(evm.style, CellStyle::Absent);
    assert_eq!(evm.text, ABSENT);
}

#[test]
fn the_measured_block_is_not_marked_simulated() {
    // C/N, level, peak, overload, and the tuning/config facts are real today.
    // If any of them regressed to Simulated the panel would understate what it
    // actually knows.
    let inst = instrument();
    for m in [
        inst.cn_db.prov,
        inst.level_dbfs.prov,
        inst.peak_dbfs.prov,
        inst.overload.prov,
        inst.center_hz.prov,
    ] {
        assert_eq!(m, Provenance::Measured);
    }
    for m in [
        inst.bandwidth_hz.prov,
        inst.constellation.prov,
        inst.n_fft.prov,
        inst.guard_interval.prov,
        inst.code_rate.prov,
        inst.bitrate_bps.prov,
    ] {
        assert_eq!(m, Provenance::Known);
    }
    // And the panel must own up to the rest.
    assert!(inst.any_simulated(), "SIM badge would never appear");
}

// ── Grid integrity ────────────────────────────────────────────────────────────

#[test]
fn rows_alternate_label_and_value_with_no_unlabelled_cell() {
    for row in instrument().panel_rows() {
        assert!(
            row.cells.len() < COLUMNS,
            "{}: {} cells exceeds the grid",
            row.section,
            row.cells.len()
        );
        assert_eq!(
            row.cells.len() % 2,
            0,
            "{}: odd cell count means a value has no label",
            row.section
        );
        // Labels are always authoritative; only values carry a style.
        for (i, cell) in row.cells.iter().enumerate() {
            if i % 2 == 0 {
                assert_eq!(cell.style, CellStyle::Normal, "{}: label", row.section);
                assert!(!cell.text.is_empty(), "{}: empty label", row.section);
            }
        }
    }
}

#[test]
fn every_column_is_its_widest_content_plus_two() {
    let rows = CofdmInstrument::layout_reference().panel_rows();
    let widths = column_widths(&rows, chars);
    let mut widest = [0.0_f32; COLUMNS];
    for row in &rows {
        widest[0] = widest[0].max(chars(row.section));
        for (i, cell) in row.cells.iter().enumerate() {
            widest[1 + i] = widest[1 + i].max(chars(&cell.text));
        }
    }
    for c in 0..COLUMNS {
        assert_eq!(
            widths[c],
            widest[c] + COLUMN_PAD,
            "column {c} is not its widest content plus two"
        );
    }
}

#[test]
fn no_cell_overflows_its_column() {
    // The property that keeps the columns aligned with no rules to hide behind.
    let widths = reference_column_widths(chars);
    for row in instrument().panel_rows() {
        assert!(chars(row.section) < widths[0]);
        for (i, cell) in row.cells.iter().enumerate() {
            assert!(
                chars(&cell.text) < widths[1 + i],
                "{}: {:?} overflows column {}",
                row.section,
                cell.text,
                1 + i
            );
        }
    }
}

#[test]
fn the_maximum_error_count_still_fits() {
    let mut f = facts();
    f.error_count = 999;
    let inst = CofdmInstrument::from_facts(&f);
    let widths = reference_column_widths(chars);
    let rows = inst.panel_rows();
    let errors = rows.iter().find(|r| r.section == "Errors").unwrap();
    let i = errors.cells.iter().position(|c| c.text == "err").unwrap();
    assert_eq!(errors.cells[i + 1].text, "000999 ");
    assert!(chars(&errors.cells[i + 1].text) < widths[1 + i + 1]);
}

#[test]
fn the_grid_does_not_reflow_as_values_change() {
    // Widths come from the worst-case reference specimen, never from live
    // content — otherwise a Δf gaining a digit shifts every column right of it
    // and the panel jitters as the signal moves.
    let baseline = reference_column_widths(chars);
    for cn in [5.0_f32, 17.0, 28.5, 38.0] {
        let inst = CofdmInstrument::from_facts(&facts_for(CofdmBwFraction::OneQuarter, cn));
        for row in inst.panel_rows() {
            for (i, cell) in row.cells.iter().enumerate() {
                assert!(
                    chars(&cell.text) < baseline[1 + i],
                    "C/N {cn}: {:?} outgrew its column",
                    cell.text
                );
            }
        }
    }
    assert_eq!(baseline, reference_column_widths(chars));
}

#[test]
fn the_merged_span_starts_at_the_c3_origin() {
    // So the lock run aligns with the bw / pk / MER / IBER column above it
    // rather than floating.
    let widths = reference_column_widths(chars);
    let origins = column_origins(&widths);
    let lines = render_text(&instrument().panel_rows());
    let demod = lines.last().unwrap();
    let at = demod.find("CAR").expect("lock run present");
    assert_eq!(
        at as f32, origins[MERGED_FROM],
        "merged span must begin exactly at the column-3 origin"
    );
    // And it must be the only row with locks.
    let with_locks = instrument()
        .panel_rows()
        .into_iter()
        .filter(|r| !r.locks.is_empty())
        .count();
    assert_eq!(with_locks, 1);
}

#[test]
fn the_panel_renders_seven_rows_in_order() {
    let rows = instrument().panel_rows();
    let sections: Vec<&str> = rows.iter().map(|r| r.section).collect();
    assert_eq!(
        sections,
        vec![
            "Tuning", "RF", "Quality", "Errors", "Channel", "Config", "Demod"
        ]
    );
}

// ── Labelling ─────────────────────────────────────────────────────────────────

#[test]
fn no_label_appears_twice() {
    // The assertion that pins `FEC` to the inner-decoder lock alone.  It has
    // previously meant the error count, the code rate and the lock at once.
    for unit in [ErrorUnit::Frame, ErrorUnit::Packet] {
        let mut inst = instrument();
        inst.error_unit = unit;
        let labels = all_labels(&inst);
        let mut seen = std::collections::HashSet::new();
        for l in &labels {
            assert!(
                seen.insert(l.clone()),
                "duplicate label {l:?} under {unit:?}"
            );
        }
        assert!(labels.contains(&"FEC".to_owned()));
        assert!(labels.contains(&"CR".to_owned()));
        assert!(labels.contains(&"err".to_owned()));
    }
}

#[test]
fn no_label_names_a_decoding_algorithm_or_code_family() {
    // Keeps the label set stable across every `InnerFec` variant — `None`,
    // LDPC, and convolutional.  Without this the set erodes back to VBER / VIT
    // the first time a DVB-shaped profile lands.
    let banned = ["VBER", "VIT", "LDPC", "RS", "BCH", "CONV"];
    for label in all_labels(&instrument()) {
        let upper = label.to_uppercase();
        for b in banned {
            assert_ne!(upper, b, "label {label:?} names a decoder or code family");
        }
    }
}

#[test]
fn the_error_ladder_reads_channel_then_inner_then_whole_chain() {
    let inst = instrument();
    let rows = inst.panel_rows();
    let errors = rows.iter().find(|r| r.section == "Errors").unwrap();
    let labels: Vec<&str> = errors
        .cells
        .iter()
        .step_by(2)
        .map(|c| c.text.as_str())
        .collect();
    assert_eq!(labels, vec!["CBER", "IBER", "FER", "err"]);
}

#[test]
fn cber_and_iber_come_from_distinct_fields() {
    // Guards against the upstream inner_ok/outer_ok fold being papered over by
    // feeding one value to both rungs.
    let mut inst = instrument();
    inst.cber = Metric::simulated(1.0e-2);
    inst.iber = Metric::simulated(1.0e-6);
    let rows = inst.panel_rows();
    let errors = rows.iter().find(|r| r.section == "Errors").unwrap();
    assert_eq!(errors.cells[1].text, "1.0E-2");
    assert_eq!(errors.cells[3].text, "1.0E-6");
    assert_ne!(errors.cells[1].text, errors.cells[3].text);
}

#[test]
fn the_error_unit_switches_the_rate_label_but_not_the_count_or_the_widths() {
    let widths = reference_column_widths(chars);
    let mut seen = Vec::new();
    for (unit, expect) in [(ErrorUnit::Frame, "FER"), (ErrorUnit::Packet, "PER")] {
        let mut inst = instrument();
        inst.error_unit = unit;
        let rows = inst.panel_rows();
        let errors = rows.iter().find(|r| r.section == "Errors").unwrap();
        let labels: Vec<&str> = errors
            .cells
            .iter()
            .step_by(2)
            .map(|c| c.text.as_str())
            .collect();
        assert_eq!(labels[2], expect);
        // The count label is unit-neutral: no PEC spelling to keep in step.
        assert_eq!(labels[3], "err");
        seen.push(reference_column_widths(chars));
    }
    // A profile switch cannot reflow the grid.
    assert_eq!(seen[0], seen[1]);
    assert_eq!(seen[0], widths);
}

// ── Bit rate ──────────────────────────────────────────────────────────────────

#[test]
fn the_bit_rate_matches_a_hand_computed_value_at_every_fraction() {
    let (_, bits_per_symbol, (k, n)) = cofdm_mcs_facts();
    let symbol_rate = COFDM_FS as f64 / (COFDM_N_FFT + COFDM_CP_LEN) as f64;
    for &fraction in CofdmBwFraction::ALL {
        let f = facts_for(fraction, 28.5);
        let expect =
            f.data_carriers as f64 * bits_per_symbol as f64 * (k as f64 / n as f64) * symbol_rate;
        let got = CofdmInstrument::from_facts(&f).bitrate_bps.value.unwrap();
        assert!(
            (got - expect).abs() < 1.0,
            "{}: expected {expect} bps, got {got}",
            fraction.label()
        );
        // And the carrier count must come off the plan, not from n_fft.
        let guard = cofdm_edge_guard_for(fraction);
        assert_eq!(f.data_carriers, cofdm_data_carriers(guard, false));
    }
}

#[test]
fn the_bit_rate_scales_with_occupied_bandwidth() {
    let narrow = CofdmInstrument::from_facts(&facts_for(CofdmBwFraction::OneEighth, 28.5));
    let wide = CofdmInstrument::from_facts(&facts_for(CofdmBwFraction::SevenEighths, 28.5));
    assert!(wide.bitrate_bps.value.unwrap() > narrow.bitrate_bps.value.unwrap() * 5.0);
}

// ── Di bar ────────────────────────────────────────────────────────────────────

#[test]
fn the_di_line_never_exceeds_its_budget() {
    let inst = instrument();
    for budget in 20..=120 {
        let line = inst.di_bar_str(budget);
        assert!(
            line.chars().count() <= budget.max(line_head_len(&inst)),
            "budget {budget}: {line:?} is {} chars",
            line.chars().count()
        );
    }
}

/// The head (`COFDM ctr bw`) is never dropped — it is what identifies the line.
fn line_head_len(inst: &CofdmInstrument) -> usize {
    inst.di_bar_str(0).chars().count()
}

#[test]
fn the_di_line_drops_fields_in_priority_order() {
    let inst = instrument();
    let wide = inst.di_bar_str(120);
    // Highest priority first: C/N, MER, CBER, locks, Δf, level.
    for f in ["C/N", "MER", "CBER", "\u{394}f", "lvl", "lck"] {
        assert!(wide.contains(f), "wide line missing {f}: {wide}");
    }
    // Narrowing drops from the tail, never from the head.
    let mut last_len = usize::MAX;
    for budget in (30..=120).rev().step_by(5) {
        let line = inst.di_bar_str(budget);
        assert!(line.starts_with("COFDM"), "head dropped at {budget}");
        assert!(
            line.chars().count() <= last_len,
            "line grew as the budget shrank at {budget}"
        );
        last_len = line.chars().count();
        // C/N outranks everything, so it survives as long as anything does.
        if line.contains("MER") {
            assert!(line.contains("C/N"), "{budget}: MER kept but C/N dropped");
        }
        if line.contains("lvl") {
            assert!(line.contains("CBER"), "{budget}: lvl kept but CBER dropped");
        }
    }
}

#[test]
fn the_di_line_badges_simulated_fields() {
    let inst = instrument();
    // MER is simulated, so any line carrying it must own up.
    let with_mer = inst.di_bar_str(120);
    assert!(with_mer.contains("MER"));
    assert!(with_mer.ends_with(SIM_BADGE), "no SIM badge: {with_mer}");
    // A line short enough to carry only measured fields must not.
    let head_only = inst.di_bar_str(34);
    assert!(
        !head_only.contains(SIM_BADGE),
        "spurious badge: {head_only}"
    );
}

// ── Live behaviour ────────────────────────────────────────────────────────────

#[test]
fn the_simulated_metrics_track_measured_cn() {
    // The testable half of "the panel is live": every simulated field is
    // derived from the real C/N, so changing Noise amp moves the whole panel.
    // The paint side is bin-only and falls to the manual pass.
    let clean = CofdmInstrument::from_facts(&facts_for(CofdmBwFraction::OneQuarter, 34.0));
    let noisy = CofdmInstrument::from_facts(&facts_for(CofdmBwFraction::OneQuarter, 18.0));

    assert!(clean.mer_db.value.unwrap() > noisy.mer_db.value.unwrap());
    assert!(clean.evm_pct.value.unwrap() < noisy.evm_pct.value.unwrap());
    assert!(clean.mer_margin_db.value.unwrap() > noisy.mer_margin_db.value.unwrap());
    // Errors fall as C/N rises, at every rung of the ladder.
    assert!(clean.cber.value.unwrap() < noisy.cber.value.unwrap());
    assert!(clean.iber.value.unwrap() < noisy.iber.value.unwrap());
    assert!(clean.error_rate.value.unwrap() < noisy.error_rate.value.unwrap());
    // Sync error shrinks as C/N rises.
    assert!(clean.freq_error_hz.value.unwrap() < noisy.freq_error_hz.value.unwrap());
    assert!(clean.delay_spread_us.value.unwrap() < noisy.delay_spread_us.value.unwrap());
}

#[test]
fn the_inner_code_improves_on_the_channel_ber() {
    // IBER sits below CBER wherever the inner code is doing anything at all —
    // the rungs must not be reported the same way round.
    for cn in [20.0_f32, 25.0, 30.0] {
        let inst = CofdmInstrument::from_facts(&facts_for(CofdmBwFraction::OneQuarter, cn));
        let (cber, iber) = (inst.cber.value.unwrap(), inst.iber.value.unwrap());
        assert!(iber <= cber, "C/N {cn}: IBER {iber} above CBER {cber}");
    }
}

#[test]
fn locks_drop_when_the_link_fails() {
    let dead = CofdmInstrument::from_facts(&facts_for(CofdmBwFraction::OneQuarter, 0.0));
    let good = CofdmInstrument::from_facts(&facts_for(CofdmBwFraction::OneQuarter, 34.0));
    assert_eq!(dead.carrier_lock.value, Some(false));
    assert_eq!(dead.fec_lock.value, Some(false));
    assert_eq!(good.carrier_lock.value, Some(true));
    assert_eq!(good.fec_lock.value, Some(true));
}

// ── Regressions from the first manual pass ────────────────────────────────────

#[test]
fn the_di_line_fields_hold_their_positions_across_a_cn_sweep() {
    // The Di line jittered a few times a second because CBER crossing the
    // `<1E-9` floor changes its rendering by one character, shifting every
    // field after it — and pushing the last one in and out of the budget, so
    // fields blinked as well as slid.  Every field is now padded to a fixed
    // width and the fit is decided on that width.
    let mut layouts = std::collections::HashSet::new();
    for cn in [18.0_f32, 20.0, 24.0, 28.0, 31.0, 34.0, 36.0] {
        // Wide enough for every field plus the badge — the `lck` label pushed
        // the full line past 100.
        let line = CofdmInstrument::from_facts(&facts_for(CofdmBwFraction::OneQuarter, cn))
            .di_bar_str(110);
        // Record where each label starts.  Identical across the sweep means
        // nothing moved.
        let offsets: Vec<Option<usize>> =
            ["C/N", "MER", "CBER", "\u{394}f", "lvl", "lck", SIM_BADGE]
                .iter()
                .map(|f| line.find(f))
                .collect();
        assert!(
            offsets.iter().all(Option::is_some),
            "C/N {cn}: a field dropped out of a 110-char line: {line}"
        );
        layouts.insert(offsets);
    }
    assert_eq!(
        layouts.len(),
        1,
        "field positions moved across the C/N sweep: {layouts:?}"
    );
}

#[test]
fn a_simulated_error_rate_never_claims_exactly_zero() {
    // `0.0E0` means "no errors observed", which a simulation cannot claim —
    // the deep tail of the error function underflows to zero in f32.
    for cn in [28.0_f32, 34.0, 40.0, 60.0] {
        let inst = CofdmInstrument::from_facts(&facts_for(CofdmBwFraction::OneQuarter, cn));
        for (name, m) in [
            ("CBER", &inst.cber),
            ("IBER", &inst.iber),
            ("FER", &inst.error_rate),
        ] {
            let v = m.value.unwrap();
            assert!(v > 0.0, "C/N {cn}: {name} underflowed to exactly zero");
            assert_ne!(fmt_ber(v), "0.0E0", "C/N {cn}: {name} claims zero errors");
        }
    }
}

#[test]
fn the_link_reads_healthy_at_every_bandwidth_on_default_settings() {
    // The simulated waterfall used an offset that placed its knee inside the
    // *bandwidth*-driven C/N spread, so wide fractions showed a failing link —
    // frame errors and dropped FEC/TS locks — on otherwise-default settings.
    // The measured envelope at default noise runs ~17.9 dB (7/8) to ~35.7 dB
    // (1/8); nothing in it may read as broken.
    for cn in [17.5_f32, 20.0, 24.0, 28.0, 32.0, 36.0] {
        for &fraction in CofdmBwFraction::ALL {
            let inst = CofdmInstrument::from_facts(&facts_for(fraction, cn));
            let at = format!("{} at C/N {cn}", fraction.label());
            for (name, lock) in [
                ("carrier", &inst.carrier_lock),
                ("timing", &inst.timing_lock),
                ("FEC", &inst.fec_lock),
                ("TS", &inst.ts_lock),
            ] {
                assert_eq!(lock.value, Some(true), "{at}: {name} lock dropped");
            }
            // And no whole-chain errors worth counting.
            assert!(
                inst.error_rate.value.unwrap() < 1.0e-2,
                "{at}: frame error rate {} would tick the counter",
                inst.error_rate.value.unwrap()
            );
        }
    }
}

#[test]
fn the_simulation_still_degrades_when_pushed_below_the_envelope() {
    // The fix above must not have flattened the model into "always healthy" —
    // the knee is below the reachable range, not absent.
    let bad = CofdmInstrument::from_facts(&facts_for(CofdmBwFraction::OneQuarter, 5.0));
    assert_eq!(bad.fec_lock.value, Some(false));
    assert!(bad.cber.value.unwrap() > 1.0e-2);
}

// ── Provider: error counting across bursts ────────────────────────────────────

/// Drive `CofdmState` with signal blocks and collect the instruments it emits.
fn run_provider(
    state: &mut CofdmState,
    blocks: usize,
    noise: f32,
    gap_edge_at_end: bool,
) -> Vec<Option<Box<CofdmInstrument>>> {
    let fraction = CofdmBwFraction::OneQuarter;
    let shaping = CofdmShaping::default_for(fraction);
    let guard = shaping.effective(fraction).edge_guard;
    let bw = cofdm_occupied_bw(COFDM_FS, guard);
    let mut src = CofdmSource::new(60.0, 1.0, noise, fraction, shaping, COFDM_FS);
    let (tx, rx) = std::sync::mpsc::sync_channel(4096);

    for _ in 0..blocks {
        let s = src.next_samples(SPECTRUM_WINDOW_SAMPLES);
        state.process(
            &s,
            true,
            false,
            COFDM_NOMINAL_CENTER,
            bw,
            guard,
            false,
            COFDM_FS,
            &tx,
        );
    }
    if gap_edge_at_end {
        state.process(
            &[],
            false,
            true,
            COFDM_NOMINAL_CENTER,
            bw,
            guard,
            false,
            COFDM_FS,
            &tx,
        );
    }
    drop(tx);
    rx.into_iter()
        .filter_map(|r| match r {
            DecodeResult::Instrument(i) => Some(i),
            _ => None,
        })
        .collect()
}

#[test]
fn a_gap_edge_clears_the_panel_and_the_error_count() {
    // The count is per-burst: carrying it across a gap would attribute one
    // transmission's errors to the next, and the panel it annotates has already
    // been cleared.
    let mut state = CofdmState::new();
    let emitted = run_provider(&mut state, 40, 0.5, true);
    assert!(
        emitted.len() >= 2,
        "provider emitted nothing to test: {}",
        emitted.len()
    );
    // The gap edge is the clear.
    assert!(
        emitted.last().unwrap().is_none(),
        "gap edge did not clear the panel"
    );
    // The next burst starts from zero.  Needs enough blocks to clear the
    // provider's emit interval again after the gap reset the accumulator.
    let next = run_provider(&mut state, 40, 0.5, false);
    let first = next.iter().flatten().next().expect("a fresh instrument");
    assert_eq!(
        first.error_count.value,
        Some(0),
        "error count carried across the gap"
    );
}

#[test]
fn the_error_count_stays_inside_its_fixed_width_field() {
    // It wraps rather than clamping — a pinned 999 reads as "999 errors"
    // forever instead of "still counting" — and either way it must never
    // outgrow the fixed-width column.
    let widths = reference_column_widths(chars);
    let mut state = CofdmState::new();
    for _ in 0..6 {
        for inst in run_provider(&mut state, 30, 0.5, false).iter().flatten() {
            let n = inst.error_count.value.unwrap();
            assert!(n < ERROR_COUNT_WRAP, "error count {n} escaped its field");
            for wrapped in [false, true] {
                assert!(chars(&fmt_count(n, wrapped)) < widths[8]);
            }
        }
    }
}

#[test]
fn dbfs_is_measured_against_the_sources_own_full_scale() {
    // The COFDM modulator applies a large fixed gain (bare OFDM at unit gain
    // sits below the decoder's signal threshold) and the viewer's f32 spectrum
    // pipeline has no [-1, 1] clamp, so raw samples peak above 30.  Measuring
    // against 1.0 reported *positive* dBFS and a permanent overload.
    let mut f = facts();
    f.level_amp = 1.79; // measured block RMS at the 1/4 fraction
    f.peak_amp = 14.7; // measured block peak
    let inst = CofdmInstrument::from_facts(&f);
    let lvl = inst.level_dbfs.value.unwrap();
    let pk = inst.peak_dbfs.value.unwrap();
    assert!(lvl < 0.0, "RMS read as {lvl:.1} dBFS — above full scale");
    assert!(pk < 0.0, "peak read as {pk:.1} dBFS — above full scale");
    assert!(pk > lvl, "peak must sit above RMS");
    assert_eq!(inst.overload.value, Some(false));
}

#[test]
fn overload_trips_only_at_full_scale() {
    let mut f = facts();
    f.peak_amp = f.full_scale * 0.99;
    assert_eq!(
        CofdmInstrument::from_facts(&f).overload.value,
        Some(false),
        "overload tripped below full scale"
    );
    f.peak_amp = f.full_scale;
    assert_eq!(CofdmInstrument::from_facts(&f).overload.value, Some(true));
}

#[test]
fn silence_reads_as_a_floor_not_negative_infinity() {
    let mut f = facts();
    f.level_amp = 0.0;
    f.peak_amp = 0.0;
    let inst = CofdmInstrument::from_facts(&f);
    assert!(inst.level_dbfs.value.unwrap().is_finite());
    assert!(chars(&fmt_dbfs(inst.level_dbfs.value.unwrap())) < reference_column_widths(chars)[2]);
}

#[test]
fn the_error_count_marks_a_rollover() {
    // `000042` and `000042+` are very different readings; without the marker a
    // wrapped counter silently under-reports by a million.  The marker slot is
    // always present so the field width never changes.
    assert_eq!(fmt_count(0, false), "000000 ");
    assert_eq!(fmt_count(42, false), "000042 ");
    assert_eq!(fmt_count(42, true), "000042+");
    assert_eq!(fmt_count(ERROR_COUNT_WRAP - 1, true), "999999+");
    assert_eq!(
        fmt_count(0, false).chars().count(),
        fmt_count(ERROR_COUNT_WRAP - 1, true).chars().count(),
        "the rollover marker must not change the field width"
    );
}

#[test]
fn the_error_count_tracks_the_frame_error_rate() {
    // `err` counts frames that failed; `FER` is the probability a given frame
    // fails.  They correlate through the frame *rate* — hundreds per second
    // here.  Accumulating the rate once per emit instead treated one emit as
    // one frame, under-counting by that factor: a displayed `FER 6.7E-5` took
    // about an hour to tick `err` once, so the two readings looked unrelated.
    let mut state = CofdmState::new();
    let emitted = run_provider(&mut state, 120, 0.5, false);
    let instruments: Vec<_> = emitted.iter().flatten().collect();
    assert!(
        instruments.len() >= 2,
        "provider emitted too little to judge"
    );

    let last = instruments.last().unwrap();
    let fer = last.error_rate.value.unwrap();
    let count = last.error_count.value.unwrap();
    // A non-negligible FER over this many frames must produce a visible count.
    if fer > 1.0e-4 {
        assert!(
            count > 0,
            "FER {fer:.2e} over the run but err stayed at {count}"
        );
    }
    // And the count must be monotone across the burst.
    let counts: Vec<u32> = instruments
        .iter()
        .map(|i| i.error_count.value.unwrap())
        .collect();
    assert!(
        counts.windows(2).all(|w| w[1] >= w[0]),
        "error count went backwards within a burst: {counts:?}"
    );
}

#[test]
fn the_lock_run_sits_at_the_end_of_the_di_line() {
    // Pinned just before the SIM badge whatever else survives the fit, and
    // labelled so it is not a bare row of glyphs.
    let inst = instrument();
    for budget in [40_usize, 55, 70, 85, 100, 120] {
        let line = inst.di_bar_str(budget);
        let Some(lck) = line.find("lck ") else {
            continue;
        };
        // Nothing but the badge may follow it.
        let tail = &line[lck..];
        assert!(
            tail.strip_prefix("lck ")
                .unwrap()
                .trim_start_matches(['\u{25cf}', '\u{25cb}'])
                .trim()
                == SIM_BADGE
                || tail
                    .strip_prefix("lck ")
                    .unwrap()
                    .trim_start_matches(['\u{25cf}', '\u{25cb}'])
                    .trim()
                    .is_empty(),
            "budget {budget}: something follows the lock run: {line:?}"
        );
        // And every other field precedes it.
        for f in ["C/N", "MER", "CBER", "\u{394}f", "lvl"] {
            if let Some(at) = line.find(f) {
                assert!(at < lck, "budget {budget}: {f} follows the lock run");
            }
        }
    }
}

#[test]
fn the_lock_run_outlives_the_lower_priority_readouts() {
    // Moving the run to the end is a *rendering* change; it keeps its rank in
    // the drop priority, above level and frequency error.  It is the most
    // compressed health indicator on the bar, so it should not be the first
    // thing a narrowing window discards.
    let inst = instrument();
    for budget in 30..=120 {
        let line = inst.di_bar_str(budget);
        if line.contains("lvl") || line.contains("\u{394}f") {
            assert!(
                line.contains("lck"),
                "budget {budget}: kept a lower-priority readout but dropped the locks: {line:?}"
            );
        }
    }
}
