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

use orion_sdr_view::utils::script::{Action, Script};

const EXAMPLE: &str = "
# t(s)   directive
0.00     key I x5              # cycle to COFDM
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
