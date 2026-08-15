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
duration 30
dump     run.jsonl

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
    let s = Script::parse("0.0 key Q\nduration 5\n").expect("parses");
    assert_eq!(s.settings.duration, Some(5.0));
    assert_eq!(s.steps.len(), 1);
    assert_eq!(s.duration_secs(), 0.0, "`duration` is not a step time");
}

#[test]
fn a_repeated_setting_is_an_error_rather_than_last_wins() {
    // Two `duration` lines mean the author believed one of them.  Silently
    // taking the other is the kind of thing only noticed after a run has
    // produced the wrong answer.
    let e = Script::parse("duration 5\nduration 10\n").expect_err("should refuse");
    assert_eq!(e.line, 2);
    assert!(e.message.contains("more than once"), "{}", e.message);
}

#[test]
fn a_bad_setting_names_itself() {
    // A zero or negative duration is refused rather than clamped: it can only be
    // a mistake, and a run that silently did nothing would be worse than one
    // that would not start.
    for (src, line, needle) in [
        ("duration nope\n", 1, "not a duration"),
        ("duration -1\n", 1, "greater than 0"),
        ("duration 0\n", 1, "greater than 0"),
        ("0.0 key Q\nduration\n", 2, "exactly one argument"),
        ("dump a b\n", 1, "exactly one argument"),
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
        let s = Script::parse(&format!("dump {spec}\n0.0 key Q\n")).expect("parses");
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
    let s = Script::parse("size 1600x900\nscale 2\n0.0 key Q\n").expect("parses");
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
        let s = Script::parse(&format!("size {spec}\n0.0 key Q\n")).expect("parses");
        assert_eq!(s.settings.size, Some(want), "{spec}");
    }
}

#[test]
fn a_bad_size_or_scale_names_itself() {
    // Bounded at both ends.  The upper bound is the point of the setting: the
    // 10000 x 10000 fallback is what it exists to replace, so accepting it back
    // through the front door would be absurd.
    for (src, needle) in [
        ("size 1200\n", "not a size"),
        ("size axb\n", "not a size"),
        ("size 10000x10000\n", "not a size"),
        ("size 4x4\n", "not a size"),
        ("scale nope\n", "not a scale factor"),
        ("scale 0\n", "between 0.1 and 8.0"),
        ("scale 99\n", "between 0.1 and 8.0"),
        ("size 800x600\nsize 640x480\n", "more than once"),
        ("scale 1\nscale 2\n", "more than once"),
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
