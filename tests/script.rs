// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The timed key script format.
//!
//! One format, two readers: the test harness replays it and executes `assert`,
//! the headless replay driver replays it and ignores `assert`.  That is what
//! makes a reproduction recipe and a regression test the same artifact — so the
//! properties worth pinning are the ones both readers depend on: that times are
//! absolute and ordered, that a repeat means frames rather than events, and that
//! a bad line fails loudly with its line number rather than being skipped.

#![cfg(feature = "gui")]

use std::path::{Path, PathBuf};

use orion_sdr_view::app::SourceMode;
use orion_sdr_view::utils::script::{Action, Script, ScriptSettings, source_mode_by_name};

const EXAMPLE: &str = "
# t(s)   directive
0.00     source COFDM          # select a source by name
0.50     key L                 # lock the source to the viewport centre
0.75     key shift+ArrowRight
0.80     text a
1.00     assert center_hz 520000
1.00     assert zoom 2.0
";

#[test]
fn the_documented_example_parses() {
    let s = Script::parse(EXAMPLE).expect("example parses");
    assert_eq!(s.steps.len(), 6);
    assert_eq!(s.duration_secs(), 1.0);
}

#[test]
fn comments_and_blank_lines_are_not_steps() {
    let s = Script::parse("\n\n# nothing here\n   \n0.0 key Q # trailing\n").expect("parses");
    assert_eq!(s.steps.len(), 1);
    assert_eq!(
        s.steps[0].action,
        Action::Key {
            key: egui::Key::Q,
            modifiers: egui::Modifiers::default(),
        }
    );
}

#[test]
fn a_repeat_count_is_frames_not_events() {
    // The distinction the format exists to get right: `key_pressed` is a
    // per-pass boolean, so five press events inside one pass register as one.
    // `key I x5` has to mean five passes or it switches one source, not five.
    let s = Script::parse("0.0 key I x5").expect("parses");
    assert_eq!(s.steps[0].repeat, 5);
    // ...and the action itself still describes a single frame's worth of input.
    assert_eq!(
        s.steps[0].action.events().len(),
        2,
        "one press, one release"
    );
}

#[test]
fn a_key_is_pressed_and_released_in_the_same_frame() {
    // Otherwise the key stays down for the rest of the run and every later
    // frame sees it held — `key_down` bindings would latch on.
    let s = Script::parse("0.0 key ArrowRight").expect("parses");
    let events = s.steps[0].action.events();
    assert!(matches!(
        events.as_slice(),
        [
            egui::Event::Key { pressed: true, .. },
            egui::Event::Key { pressed: false, .. }
        ]
    ));
}

#[test]
fn modifiers_reach_both_the_event_and_the_pass() {
    // `InputState::modifiers` — what `handle_keys` reads for shift/ctrl/alt —
    // comes from `RawInput`, not from the key event.  Setting only one of the
    // two would give a script a `shift+` the app never sees.
    let s = Script::parse("0.0 key ctrl+shift+ArrowLeft").expect("parses");
    let mods = s.steps[0].action.modifiers();
    assert!(mods.ctrl && mods.shift && !mods.alt);
    let Action::Key { key, modifiers } = &s.steps[0].action else {
        panic!("expected a key action");
    };
    assert_eq!(*key, egui::Key::ArrowLeft);
    assert_eq!(*modifiers, mods, "event and pass modifiers must agree");
}

#[test]
fn text_directives_reach_the_bindings_keys_cannot() {
    // The marker (`a`/`b`), help (`?`) and dB-reference (`[`/`]`) bindings read
    // `Event::Text`, so a key-only format could not drive them at all.
    for literal in ["a", "B", "?", "["] {
        let s = Script::parse(&format!("0.0 text {literal}")).expect("parses");
        assert_eq!(
            s.steps[0].action.events(),
            vec![egui::Event::Text(literal.to_owned())]
        );
    }
}

// ── Naming a source ─────────────────────────────────────────────────────────

#[test]
fn a_source_directive_is_an_i_press_with_the_count_left_to_the_reader() {
    // Not a separate mechanism: it delivers exactly what `key I` delivers, so
    // the same key path runs.  What it drops is the *count*, which is the part
    // a script cannot know — the distance to a source depends on where the app
    // already is — and the part that goes stale when the list changes.
    let s = Script::parse("0.0 source COFDM").expect("parses");
    assert_eq!(
        s.steps[0].action,
        Action::Source {
            mode: SourceMode::Cofdm
        }
    );
    assert_eq!(s.steps[0].action.events(), {
        let i = Script::parse("0.0 key I").expect("parses");
        i.steps[0].action.events()
    });
}

#[test]
fn every_source_is_nameable_by_the_label_it_shows() {
    // The round trip that keeps this honest as sources are added: a new variant
    // in `SourceMode::ALL` is nameable with no edit to the parser, and a label
    // that collided with another would fail here rather than in a dump.
    for mode in SourceMode::ALL {
        let s = Script::parse(&format!("0.0 source {}", mode.label())).expect("label parses");
        assert_eq!(s.steps[0].action, Action::Source { mode: *mode });
    }
}

#[test]
fn source_names_ignore_case_and_punctuation() {
    // A two-word label has no single obvious spelling, so all of them work
    // rather than one being correct and the rest being parse errors.
    for spelling in ["AM DSB", "AM-DSB", "AM_DSB", "amdsb", "Am.Dsb"] {
        assert_eq!(
            source_mode_by_name(spelling),
            Some(SourceMode::AmDsb),
            "`{spelling}` should name AM DSB"
        );
    }
    assert_eq!(source_mode_by_name("cofdm"), Some(SourceMode::Cofdm));
    assert_eq!(source_mode_by_name("TestTone"), Some(SourceMode::TestTone));
    // ...and a multi-word spelling survives the parser's whitespace splitting.
    let s = Script::parse("0.0 source test tone").expect("parses");
    assert_eq!(
        s.steps[0].action,
        Action::Source {
            mode: SourceMode::TestTone
        }
    );
}

#[test]
fn a_name_that_matches_nothing_is_not_a_near_miss() {
    // Folding is not fuzzy matching: dropping punctuation must not make two
    // different sources collide, or a typo would silently select a neighbour.
    for name in ["", "FT9", "PSK", "AM", "COFDM2"] {
        assert_eq!(
            source_mode_by_name(name),
            None,
            "`{name}` should not resolve"
        );
    }
}

#[test]
fn a_bad_source_directive_names_itself() {
    for (src, needle) in [
        ("0.0 source NotASource", "is not a source"),
        ("0.0 source", "needs a source name"),
        ("0.0 source COFDM x5", "takes no repeat count"),
    ] {
        let e = Script::parse(src).expect_err(&format!("`{src}` should not parse"));
        assert_eq!(e.line, 1);
        assert!(
            e.message.contains(needle),
            "`{src}` gave `{}`, expected it to mention `{needle}`",
            e.message
        );
    }
    // The diagnostic lists what *does* exist, so a wrong name is self-service.
    let e = Script::parse("0.0 source NotASource").unwrap_err();
    for mode in SourceMode::ALL {
        assert!(
            e.message.contains(mode.label()),
            "expected `{}` in: {}",
            mode.label(),
            e.message
        );
    }
}

#[test]
fn asserts_carry_their_arguments_and_emit_nothing() {
    // The driver parses them — so a typo is still an error — and then ignores
    // them; only the harness executes.
    let s = Script::parse("1.0 assert center_hz 520000 100").expect("parses");
    let Action::Assert { name, args } = &s.steps[0].action else {
        panic!("expected an assert action");
    };
    assert_eq!(name, "center_hz");
    assert_eq!(args, &["520000".to_owned(), "100".to_owned()]);
    assert!(s.steps[0].action.events().is_empty());
}

#[test]
fn steps_are_sorted_by_time_and_stable_within_an_instant() {
    // Absolute times mean a script may be written out of order, but two
    // directives at the same instant must still happen in the order written —
    // `key L` then `assert locked 1` is not the same as the reverse.
    let s = Script::parse("1.0 key A\n0.5 key B\n1.0 assert zoom 1.0\n").expect("parses");
    let times: Vec<f32> = s.steps.iter().map(|st| st.t_secs).collect();
    assert_eq!(times, vec![0.5, 1.0, 1.0]);
    assert!(matches!(s.steps[1].action, Action::Key { .. }));
    assert!(matches!(s.steps[2].action, Action::Assert { .. }));
}

#[test]
fn a_window_delivers_every_step_exactly_once() {
    // `steps_in` is half-open so that stepping [0,dt), [dt,2dt), … cannot drop a
    // step that lands exactly on a boundary or deliver it twice.
    let s = Script::parse("0.0 key A\n0.25 key B\n0.5 key C\n").expect("parses");
    let dt = 0.25_f32;
    let mut seen = 0;
    for i in 0..4 {
        seen += s.steps_in(i as f32 * dt, (i + 1) as f32 * dt).count();
    }
    assert_eq!(seen, 3);
    assert_eq!(s.steps_in(0.0, 0.25).count(), 1);
    assert_eq!(s.steps_in(0.25, 0.5).count(), 1);
}

// ── Failure is loud ─────────────────────────────────────────────────────────

/// A headless run has nobody watching it, so every one of these has to be an
/// error rather than a skipped line.
#[test]
fn a_bad_line_names_itself() {
    let cases: [(&str, usize, &str); 6] = [
        ("0.0 key Q\nlater key Q\n", 2, "not a time"),
        ("0.0 key NotAKey", 1, "not an egui key name"),
        ("0.0 key hyper+Q", 1, "not a modifier"),
        ("0.0 jump 3", 1, "not a directive"),
        ("0.0 key Q x0", 1, "repeat count of 0"),
        ("0.0 assert", 1, "needs a property name"),
    ];
    for (src, line, needle) in cases {
        let err = Script::parse(src).expect_err(&format!("`{src}` should not parse"));
        assert_eq!(err.line, line, "wrong line for `{src}`");
        assert!(
            err.message.contains(needle),
            "`{src}` gave `{}`, expected it to mention `{needle}`",
            err.message
        );
        // The Display form is what a driver prints; it must carry the line too.
        assert!(err.to_string().starts_with(&format!("line {line}:")));
    }
}

#[test]
fn a_negative_time_is_refused() {
    // Times are absolute offsets from the start of the run, so a negative one
    // is meaningless rather than "as early as possible".
    let err = Script::parse("-1.0 key Q").expect_err("should not parse");
    assert!(err.message.contains("must be finite"), "{}", err.message);
}

// ── Run settings ────────────────────────────────────────────────────────────

#[test]
fn a_script_can_carry_its_own_duration_and_dump() {
    // The point of putting these in the script is that one file is a complete
    // recipe: what to press, how long for, and where the answer goes.  A bug
    // report that needs a remembered command line alongside it is not one.
    let s = Script::parse(
        "
set run.duration 30
set run.dump     run.jsonl

0.00 key I x5
",
    )
    .expect("parses");
    assert_eq!(s.settings.duration, Some(30.0));
    assert_eq!(s.settings.dump.as_deref(), Some(Path::new("run.jsonl")));
    assert_eq!(s.steps.len(), 1, "a setting is not a step");
}

#[test]
fn settings_may_appear_anywhere_and_are_not_timed() {
    // No time column, because they configure the run rather than happen during
    // it.  Convention is to put them at the top; nothing enforces it.
    let s = Script::parse("0.0 key Q\nset run.duration 5\n").expect("parses");
    assert_eq!(s.settings.duration, Some(5.0));
    assert_eq!(s.steps.len(), 1);
    assert_eq!(s.duration_secs(), 0.0, "`duration` is not a step time");
}

#[test]
fn a_repeated_setting_is_an_error_rather_than_last_wins() {
    // Two `duration` lines mean the author believed one of them.  Silently
    // taking the other is the kind of thing only noticed after a run has
    // produced the wrong answer.
    let e = Script::parse("set run.duration 5\nset run.duration 10\n").expect_err("should refuse");
    assert_eq!(e.line, 2);
    assert!(e.message.contains("more than once"), "{}", e.message);
}

#[test]
fn a_bad_setting_names_itself() {
    // A zero or negative duration is refused rather than clamped: it can only be
    // a mistake, and a run that silently did nothing would be worse than one
    // that would not start.
    for (src, line, needle) in [
        ("set run.duration nope\n", 1, "not a duration"),
        ("set run.duration -1\n", 1, "greater than 0"),
        ("set run.duration 0\n", 1, "greater than 0"),
        (
            "0.0 key Q\nset run.duration\n",
            2,
            "one whitespace-free value",
        ),
        ("set run.dump a b\n", 1, "one whitespace-free value"),
        // `duration` is no longer a reserved word, so a bare one is read as a
        // line that should have started with a time.  Worth pinning: it is the
        // whole of what folding the run settings into `set` cost.
        ("duration 30\n", 1, "not a time in seconds"),
        ("set run.nope 1\n", 1, "is not a run setting"),
    ] {
        let e = Script::parse(src).unwrap_err();
        assert_eq!(e.line, line, "wrong line for {src:?}");
        assert!(
            e.message.contains(needle),
            "{src:?}: expected {needle:?}, got {:?}",
            e.message
        );
    }
}

#[test]
fn a_mistyped_time_still_reports_a_bad_time() {
    // The parser dispatches on the *first word* being a known setting verb
    // rather than on "does it parse as a number", precisely so this diagnostic
    // does not degrade into "not a directive".
    let e = Script::parse("0.O5 key Q\n").expect_err("should refuse");
    assert_eq!(e.line, 1);
    assert!(
        e.message.contains("not a time in seconds"),
        "expected a time diagnostic, got: {}",
        e.message
    );
}

#[test]
fn a_dump_path_is_taken_verbatim_whether_relative_or_absolute() {
    // No rewriting either way: the directive is a default for `--dump`, so the
    // same string has to mean the same file through both. A relative path is
    // therefore resolved against the working directory by the OS, exactly as a
    // path from a shell would be.
    for spec in [
        "out.jsonl",
        "runs/out.jsonl",
        "/tmp/run.jsonl",
        "../up.jsonl",
    ] {
        let s = Script::parse(&format!("set run.dump {spec}\n0.0 key Q\n")).expect("parses");
        assert_eq!(s.settings.dump, Some(PathBuf::from(spec)));
    }
}

#[test]
fn a_script_with_no_settings_carries_none() {
    let s = Script::parse("0.0 key Q\n").expect("parses");
    assert_eq!(s.settings, ScriptSettings::default());
    assert_eq!(s.settings.dump, None, "no dump named, no dump written");
}

// ── Viewport size and scale ─────────────────────────────────────────────────

#[test]
fn a_script_can_state_the_size_and_scale_it_lays_out_at() {
    // A headless pass supplies no `screen_rect` unless something sets one, and
    // egui's fallback is 10000 x 10000 — a size no window has, and one that
    // would make a capture 400 MB.
    let s = Script::parse("set run.size 1600x900\nset run.scale 2\n0.0 key Q\n").expect("parses");
    assert_eq!(s.settings.size, Some((1600.0, 900.0)));
    assert_eq!(s.settings.scale, Some(2.0));
    assert_eq!(s.steps.len(), 1, "a setting is not a step");
}

#[test]
fn a_size_is_written_the_way_every_other_tool_writes_one() {
    for (spec, want) in [
        ("1200x828", (1200.0, 828.0)),
        ("1200X828", (1200.0, 828.0)),
        ("640x480", (640.0, 480.0)),
    ] {
        let s = Script::parse(&format!("set run.size {spec}\n0.0 key Q\n")).expect("parses");
        assert_eq!(s.settings.size, Some(want), "{spec}");
    }
}

#[test]
fn a_bad_size_or_scale_names_itself() {
    // Bounded at both ends.  The upper bound is the point of the setting: the
    // 10000 x 10000 fallback is what it exists to replace, so accepting it back
    // through the front door would be absurd.
    for (src, needle) in [
        ("set run.size 1200\n", "not a size"),
        ("set run.size axb\n", "not a size"),
        ("set run.size 10000x10000\n", "not a size"),
        ("set run.size 4x4\n", "not a size"),
        ("set run.scale nope\n", "not a scale factor"),
        ("set run.scale 0\n", "between 0.1 and 8.0"),
        ("set run.scale 99\n", "between 0.1 and 8.0"),
        (
            "set run.size 800x600\nset run.size 640x480\n",
            "more than once",
        ),
        ("set run.scale 1\nset run.scale 2\n", "more than once"),
    ] {
        let e = Script::parse(src).expect_err(&format!("`{src}` should not parse"));
        assert!(
            e.message.contains(needle),
            "`{src}` gave `{}`, expected `{needle}`",
            e.message
        );
    }
}

#[test]
fn a_script_with_no_viewport_settings_carries_none() {
    let s = Script::parse("0.0 key Q\n").expect("parses");
    assert_eq!(s.settings.size, None);
    assert_eq!(s.settings.scale, None);
}

// ── Capture directory and the `pane` directive ──────────────────────────────

#[test]
fn a_script_can_say_where_captures_go() {
    let s = Script::parse("set run.capture ./shots\n0.0 pane waterfall\n").expect("parses");
    assert_eq!(s.settings.capture.as_deref(), Some(Path::new("./shots")));
    assert_eq!(s.steps.len(), 1, "a setting is not a step");
}

#[test]
fn every_pane_that_keeps_pixels_is_nameable() {
    // The round trip that keeps this honest as panes are added.  The spectrum
    // pane is deliberately absent: it is a line plot drawn straight to a
    // painter, with no buffer to hand over.
    use orion_sdr_view::utils::script::Pane;
    for pane in Pane::ALL {
        let s = Script::parse(&format!("0.0 pane {}", pane.name())).expect("parses");
        assert_eq!(
            s.steps[0].action,
            Action::Pane {
                pane: *pane,
                label: None
            }
        );
    }
    assert_eq!(Pane::by_name("WaterFall"), Some(Pane::Waterfall));
    assert_eq!(Pane::by_name("spectrum"), None, "no buffer to capture");
}

#[test]
fn a_pane_capture_can_carry_a_label() {
    // So a script taking several produces readable names rather than a column
    // of timestamps.
    let s = Script::parse("0.0 pane waterfall burst_2\n").expect("parses");
    assert_eq!(
        s.steps[0].action,
        Action::Pane {
            pane: orion_sdr_view::utils::script::Pane::Waterfall,
            label: Some("burst_2".to_owned()),
        }
    );
    // ...and it emits no input events, since it writes a file instead.
    assert!(s.steps[0].action.events().is_empty());
}

#[test]
fn a_label_that_would_not_survive_a_filesystem_is_refused() {
    // A label becomes part of a filename, so anything needing quoting would
    // make the very artifact it names awkward to handle.  Refused rather than
    // silently mangled.
    for (src, needle) in [
        ("0.0 pane waterfall a/b", "only letters, digits"),
        ("0.0 pane waterfall 'x'", "only letters, digits"),
        ("0.0 pane waterfall two words", "one whitespace-free word"),
        ("0.0 pane notapane", "is not a pane"),
        ("0.0 pane", "needs a pane name"),
        ("0.0 pane waterfall x3", "takes no repeat count"),
    ] {
        let e = Script::parse(src).expect_err(&format!("`{src}` should not parse"));
        assert!(
            e.message.contains(needle),
            "`{src}` gave `{}`, expected `{needle}`",
            e.message
        );
    }
}

#[test]
fn a_still_directive_captures_the_whole_window() {
    // Distinct from `pane`, which writes one pane's own raster: a still is
    // everything the viewer draws, so the frame has to be drawn for it.
    let s = Script::parse("0.0 still\n").expect("parses");
    assert_eq!(s.steps[0].action, Action::Still { label: None });
    assert!(s.steps[0].action.events().is_empty(), "it writes a file");

    let s = Script::parse("0.0 still band_edge\n").expect("parses");
    assert_eq!(
        s.steps[0].action,
        Action::Still {
            label: Some("band_edge".to_owned())
        }
    );
}

#[test]
fn a_bad_still_directive_names_itself() {
    for (src, needle) in [
        ("0.0 still two words", "one whitespace-free word"),
        ("0.0 still a/b", "only letters, digits"),
        ("0.0 still x2", "takes no repeat count"),
    ] {
        let e = Script::parse(src).expect_err(&format!("`{src}` should not parse"));
        assert!(
            e.message.contains(needle),
            "`{src}` gave `{}`, expected `{needle}`",
            e.message
        );
    }
}

// ── `set` ───────────────────────────────────────────────────────────────────
//
// One directive over three scopes, and the scope is the whole of what tells a
// run setting from an app setting.  What is worth pinning is the boundary: that
// a key which does not exist stops the parse, that `run.` refuses a time, and
// that a source may be spelled either of the two ways the project spells one.

#[test]
fn an_untimed_set_is_a_setting_and_a_timed_one_is_a_step() {
    let s = Script::parse(
        "
set run.duration 30
set cofdm.cn_db  10

0.00 source COFDM
5.00 set cofdm.cn_db 5
",
    )
    .expect("parses");
    assert_eq!(s.settings.duration, Some(30.0));
    assert_eq!(s.settings.sets.len(), 1, "the app setting is not a step");
    assert_eq!(s.settings.sets[0].value, "10");
    assert_eq!(s.steps.len(), 2, "and the timed one is not a setting");
    assert!(matches!(s.steps[1].action, Action::Set { .. }));
}

#[test]
fn a_source_is_spelled_the_same_way_everywhere() {
    // The config file writes `am_dsb`, the HUD shows `AM DSB`, and folding makes
    // them one word — so `set` and `source` accept the same name.  The point is
    // that this format has one spelling of a source, not one per directive.
    for spec in [
        "am_dsb.cn_db",
        "AM-DSB.cn_db",
        "amdsb.cn_db",
        "AM_dsb.cn_db",
    ] {
        let src = format!("set {spec} 20\n0.0 key Q\n");
        let s = Script::parse(&src).unwrap_or_else(|e| panic!("`{spec}`: {e}"));
        assert_eq!(s.settings.sets.len(), 1, "`{spec}` should resolve");
    }
}

#[test]
fn a_set_of_a_key_that_does_not_exist_lists_the_ones_that_do() {
    // The same courtesy a bad source name gets: the diagnostic carries the
    // answer, so a typo does not send the reader back to the docs.
    let e = Script::parse("set cofdm.cn-db 10\n").expect_err("should refuse");
    assert!(e.message.contains("not a settable key"), "{}", e.message);
    assert!(e.message.contains("cn_db"), "{}", e.message);

    let e = Script::parse("set nosuch.cn_db 10\n").expect_err("should refuse");
    assert!(
        e.message.contains("names nothing settable"),
        "{}",
        e.message
    );
    assert!(e.message.contains("COFDM"), "{}", e.message);
    assert!(e.message.contains("display"), "{}", e.message);
}

#[test]
fn a_config_key_with_no_row_is_refused_rather_than_ignored() {
    // `fs_hz` is a real config key, and deliberately not a row — a live sample
    // rate would re-derive Nyquist underneath the viewport.  Saying so is the
    // point: silently accepting it would be a `set` that does nothing.
    let e = Script::parse("set cofdm.fs_hz 960000\n").expect_err("should refuse");
    assert!(e.message.contains("not a settable key"), "{}", e.message);
}

#[test]
fn a_run_setting_refuses_a_time_and_an_app_setting_accepts_one() {
    let e = Script::parse("0.0 source COFDM\n5.0 set run.duration 10\n").expect_err("refuse");
    assert!(e.message.contains("takes no time"), "{}", e.message);
    assert_eq!(e.line, 2);

    Script::parse("5.0 set display.zoom 4\n").expect("a display row may be set mid-run");
}

#[test]
fn a_bad_set_directive_names_itself() {
    for (src, needle) in [
        ("0.0 set cofdm.cn_db 10 x3", "takes no repeat count"),
        ("0.0 set cofdm.cn_db", "one whitespace-free value"),
        ("set cofdm 10\n", "not a settings key"),
        (
            "set cofdm.cn_db 10\nset cofdm.cn_db 20\n",
            "set more than once",
        ),
    ] {
        let e = Script::parse(src).expect_err(&format!("`{src}` should not parse"));
        assert!(
            e.message.contains(needle),
            "`{src}` gave `{}`, expected `{needle}`",
            e.message
        );
    }
}

#[test]
fn the_parser_resolves_the_key_but_not_the_value() {
    // The split that keeps this layer honest: a key path is static, so it
    // resolves here; a value is checked against the *row*, which only exists
    // once an app does.  The driver pre-flights every value before frame 0, so
    // the format's promise — a bad line stops the run before it starts — is kept
    // either way; see `a_value_no_row_will_take_stops_the_run_before_it_starts`
    // in `tests/replay.rs`.
    let s = Script::parse("set cofdm.bandwidth 9/9\n0.0 key Q\n").expect("the key resolves");
    assert_eq!(s.settings.sets[0].value, "9/9");
}
