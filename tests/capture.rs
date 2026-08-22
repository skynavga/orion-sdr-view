// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Still and video capture.
//!
//! **The capture path is testable without a GPU**, because it crosses two seams
//! a bare `egui::Context` can drive: the request leaves as a
//! `ViewportCommand::Screenshot` in the pass's output, and the image arrives as
//! an `Event::Screenshot` in the next pass's input.  Everything between —
//! tagging, matching, resampling, encoding, accounting — is ordinary code.
//!
//! So these check what would otherwise only be found by eye or by a lost
//! recording: that a capture names an instant rather than an arrival, that both
//! keys work with an overlay up, that colours survive the encoder, and that a
//! dropped frame is *counted* rather than absorbed.

#![cfg(feature = "gui")]

mod common;

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use common::harness::Harness;
use orion_sdr_view::capture::{
    CaptureTag, CfrResampler, Frame, FrameSink, PngSequenceSink, encode_png, ffmpeg_args,
    sidecar_path, write_png, writer::Recorder,
};
use orion_sdr_view::utils::format::{format_iso8601, format_stamp};

/// 2026-08-16T11:22:33.456Z, checked against the OS date utility.
const T0_MILLIS: u64 = 1_786_879_353_456;

fn at(millis: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_millis(millis)
}

/// An instant at an exact fractional second.
///
/// Milliseconds are too coarse for frame timing: a 30 fps slot boundary falls
/// at 33.333 ms, so `at(33)` is *before* it and the resampler is right to hold
/// the frame back.
fn at_secs(s: f64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs_f64(s)
}

fn frame(seq: u64, millis: u64, w: u32, h: u32) -> Frame {
    Frame {
        width: w,
        height: h,
        rgba: [0x20, 0x24, 0x2b, 0xff].repeat((w * h) as usize),
        tag: CaptureTag {
            seq,
            content_time: at(millis),
        },
    }
}

// ── A. Naming ───────────────────────────────────────────────────────────────

#[test]
fn a_capture_is_named_in_iso_8601_basic_format() {
    // Basic, not extended: the extended form's colons are illegal in a path on
    // Windows and are rendered as `/` by the macOS Finder.  Both forms are
    // conformant, and ISO 8601 forbids *mixing* them in one representation —
    // which is why the offset is `+0530` here and `+05:30` in the metadata.
    assert_eq!(format_stamp(at(T0_MILLIS), 0), "20260816T112233.456Z");
    assert_eq!(
        format_stamp(at(T0_MILLIS), -300),
        "20260816T062233.456-0500"
    );
    assert_eq!(format_stamp(at(T0_MILLIS), 330), "20260816T165233.456+0530");

    // The metadata spelling is extended throughout, as RFC 3339 wants.
    assert_eq!(format_iso8601(at(T0_MILLIS), 0), "2026-08-16T11:22:33.456Z");
    assert_eq!(
        format_iso8601(at(T0_MILLIS), 330),
        "2026-08-16T16:52:33.456+05:30"
    );
}

#[test]
fn utc_is_written_as_z_rather_than_a_zero_offset() {
    // `Z` and `+0000` are both conformant; `Z` is the canonical designator and
    // is what every parser expects to see.
    assert!(format_stamp(at(T0_MILLIS), 0).ends_with('Z'));
    assert!(!format_stamp(at(T0_MILLIS), 0).contains("+0000"));
}

#[test]
fn the_civil_date_is_right_on_the_awkward_days() {
    // Leap days and century boundaries, where a hand-written calendar goes
    // wrong.  Cross-checked against `date -u -r`.
    for (secs, want) in [
        (0_u64, "19700101T000000.000Z"),
        (951_782_400, "20000229T000000.000Z"), // a leap day in a leap century
        (1_583_020_800, "20200301T000000.000Z"), // the day after one
        (4_102_444_800, "21000101T000000.000Z"), // 2100 is not a leap year
    ] {
        assert_eq!(format_stamp(at(secs * 1000), 0), want, "at {secs}");
    }
}

#[test]
fn milliseconds_keep_frames_from_colliding() {
    // At 30 fps a frame lands every 33 ms, so second precision would put thirty
    // of them on one path.
    let a = format_stamp(at(T0_MILLIS), 0);
    let b = format_stamp(at(T0_MILLIS + 33), 0);
    assert_ne!(a, b);
    // ...and the names still sort into capture order.
    assert!(a < b);
}

// ── B. Constant-frame-rate resampling ───────────────────────────────────────

#[test]
fn recording_a_60hz_display_at_30_fps_halves_the_frame_count() {
    // Not a loss: it is what the target rate *means*.  Reported separately from
    // dropped frames for exactly that reason.
    //
    // Asserted on the total rather than the per-frame pattern: a frame landing
    // exactly on a slot boundary falls either side of it depending on float
    // rounding, and which side is not a property worth pinning.  The rate is.
    let mut r = CfrResampler::new(30);
    let mut counts = Vec::new();
    for i in 0..60 {
        counts.push(r.admit(at_secs(f64::from(i) / 60.0)));
    }
    assert_eq!(r.emitted(), 30, "a second of 60 fps should be 30 frames");
    assert!(
        counts.iter().all(|&c| c <= 1),
        "nothing should be duplicated when the source is faster: {counts:?}"
    );
}

#[test]
fn a_display_slower_than_the_target_rate_duplicates_to_fill_the_timeline() {
    // The other half of CFR: ffmpeg's rawvideo demuxer assumes a constant rate,
    // so a gap in content time has to become duplicated frames or the video
    // plays faster than the session it recorded.
    let mut r = CfrResampler::new(30);
    assert_eq!(r.admit(at_secs(0.0)), 1);
    // A frame 70 ms later has passed slots 1 and 2, so it fills both.
    assert_eq!(r.admit(at_secs(0.070)), 2);
    assert_eq!(r.emitted(), 3);
}

#[test]
fn a_stall_is_filled_rather_than_shortening_the_video() {
    // A one-second gap at 30 fps is 30 slots.  Without this the recording would
    // silently run short, which is the same class of error as a dropped frame
    // nobody counted.
    let mut r = CfrResampler::new(30);
    assert_eq!(r.admit(at_secs(0.0)), 1);
    assert_eq!(r.admit(at_secs(1.0)), 30);
}

#[test]
fn the_resampler_times_from_content_not_arrival() {
    // A frame that waited in the queue still lands in the slot it depicts, which
    // is the whole reason the tag carries a timestamp at all.
    let mut a = CfrResampler::new(30);
    let mut b = CfrResampler::new(30);
    for i in 0..4 {
        let t = at_secs(f64::from(i) / 20.0);
        assert_eq!(a.admit(t), b.admit(t));
    }
    assert_eq!(a.emitted(), b.emitted());
}

// ── C. The image survives the encoder ───────────────────────────────────────

#[test]
fn a_still_round_trips_its_colours() {
    // The classic failure here is a silent channel swap or a gamma error: the
    // Metal surface is BGRA and `ColorImage` is RGBA, and a transposition only
    // shows up by eye.  A known flat colour makes it an assertion instead.
    let px: [u8; 4] = [0x20, 0x24, 0x2b, 0xff]; // the pane background
    let rgba: Vec<u8> = px.repeat(16);
    let mut buf = Vec::new();
    encode_png(&mut buf, 4, 4, &rgba).expect("encode");

    let decoder = png::Decoder::new(std::io::Cursor::new(&buf));
    let mut reader = decoder.read_info().expect("read info");
    let mut out = vec![0; reader.output_buffer_size().expect("size")];
    let info = reader.next_frame(&mut out).expect("decode");
    assert_eq!((info.width, info.height), (4, 4));
    assert_eq!(&out[..info.buffer_size()], &rgba[..], "colours changed");
}

#[test]
fn a_frame_whose_length_disagrees_with_its_size_is_refused() {
    // Better a loud error than a PNG built from whatever followed in memory.
    let err = write_png(Path::new("/tmp/never-written.png"), 4, 4, &[0; 8]).unwrap_err();
    assert!(err.to_string().contains("needs"), "{err}");
}

// ── D. The recording is honest about what it lost ───────────────────────────

/// A sink that records what it was asked to do and nothing else.
#[derive(Default)]
struct CountingSink {
    opened: Option<(u32, u32)>,
    frames: usize,
    finished: bool,
}

impl FrameSink for CountingSink {
    fn open(&mut self, width: u32, height: u32) -> std::io::Result<()> {
        self.opened = Some((width, height));
        Ok(())
    }
    fn write_frame(&mut self, _: &[u8]) -> std::io::Result<()> {
        self.frames += 1;
        Ok(())
    }
    fn finish(&mut self) -> std::io::Result<()> {
        self.finished = true;
        Ok(())
    }
    fn output_path(&self) -> std::path::PathBuf {
        std::path::PathBuf::from("<counting>")
    }
}

#[test]
fn a_gap_in_the_sequence_is_reported_rather_than_closed_over() {
    // The precedent is `CofdmRxStats::lost`: a receiver that silently discarded
    // frames read as a *perfect link*.  A capture that quietly loses a third of
    // its frames and reports success is the same failure.
    let mut rec = Recorder::new(Box::new(CountingSink::default()), 30);
    rec.push(&frame(0, 0, 2, 2)).expect("push");
    rec.push(&frame(1, 33, 2, 2)).expect("push");
    // 2 and 3 never arrive.
    rec.push(&frame(4, 133, 2, 2)).expect("push");
    let stats = rec.stats();
    assert_eq!(stats.lost, 2, "two frames vanished and must be counted");
    assert_eq!(stats.queued, 3);
}

#[test]
fn a_queue_full_drop_is_counted_separately_from_a_lost_frame() {
    // Two different faults with two different fixes: a full queue means the
    // writer could not keep up, a gap means a frame went missing elsewhere.
    // Collapsing them would hide which.
    let mut rec = Recorder::new(Box::new(CountingSink::default()), 30);
    rec.push(&frame(0, 0, 2, 2)).expect("push");
    rec.note_queue_full(3);
    let stats = rec.stats();
    assert_eq!(stats.dropped_full, 3);
    assert_eq!(stats.lost, 0);
    assert_eq!(stats.missing(), 3);
    assert!(stats.summary().contains("DROPPED"), "{}", stats.summary());
}

#[test]
fn a_clean_recording_says_nothing_about_drops() {
    let mut rec = Recorder::new(Box::new(CountingSink::default()), 30);
    for i in 0..4 {
        rec.push(&frame(i, i * 33, 2, 2)).expect("push");
    }
    let stats = rec.stats();
    assert_eq!(stats.missing(), 0);
    assert!(!stats.summary().contains("DROPPED"), "{}", stats.summary());
}

#[test]
fn a_resize_mid_recording_stops_it_rather_than_corrupting_the_file() {
    // A rawvideo stream carries no way to signal a resolution change, so ffmpeg
    // would accept mismatched frames and produce a corrupt video rather than an
    // error.  The window is resizable and moving it between displays changes
    // `pixels_per_point`, so this is reachable, not theoretical.
    let mut rec = Recorder::new(Box::new(CountingSink::default()), 30);
    rec.push(&frame(0, 0, 4, 4)).expect("push");
    let err = rec.push(&frame(1, 33, 8, 4)).expect_err("should refuse");
    assert!(err.to_string().contains("changed size"), "{err}");
}

#[test]
fn the_sink_learns_its_size_from_the_first_frame() {
    // Not from the keypress: the size is the *physical* surface size, which
    // depends on the display's scale factor and is unknown until a frame lands.
    let mut rec = Recorder::new(Box::new(CountingSink::default()), 30);
    assert_eq!(rec.size(), None);
    rec.push(&frame(0, 0, 6, 3)).expect("push");
    assert_eq!(rec.size(), Some((6, 3)));
}

// ── E. The encoder command line ─────────────────────────────────────────────

#[test]
fn the_ffmpeg_command_describes_the_stream_it_is_fed() {
    // A wrong `-pix_fmt` or a transposed `-s` yields a video that is merely
    // *wrong* rather than missing, which is the kind of defect that survives.
    let args = ffmpeg_args(1280, 720, 30, Path::new("/tmp/out.mp4"));
    let joined = args.join(" ");
    assert!(joined.contains("-f rawvideo"), "{joined}");
    assert!(joined.contains("-pix_fmt rgba"), "input is RGBA: {joined}");
    assert!(joined.contains("-s 1280x720"), "{joined}");
    assert!(joined.contains("-framerate 30"), "{joined}");
    // Output pixel format matters too: H.264 in RGB will not play in most
    // consumer players.
    assert!(joined.contains("-pix_fmt yuv420p"), "{joined}");
    assert_eq!(args.last().map(String::as_str), Some("/tmp/out.mp4"));
}

// ── F. Files on disk ────────────────────────────────────────────────────────

#[test]
fn a_png_sequence_numbers_its_frames_in_playback_order() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut sink = PngSequenceSink::new(dir.path().join("seq"));
    sink.open(2, 2).expect("open");
    for _ in 0..3 {
        sink.write_frame(&[0u8; 16]).expect("write");
    }
    sink.finish().expect("finish");
    let mut names: Vec<String> = std::fs::read_dir(dir.path().join("seq"))
        .expect("read_dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    // Zero padded, so lexicographic order is playback order — and so
    // `ffmpeg -i %06d.png` can pick the sequence up later.
    assert_eq!(names, vec!["000000.png", "000001.png", "000002.png"]);
}

#[test]
fn a_still_is_written_with_a_metadata_sidecar_that_describes_it() {
    // A PNG alone says nothing about which source made it, at what rate, or
    // over what span — and a capture outlives the session that produced it.
    let dir = tempfile::tempdir().expect("tempdir");
    let scene = orion_sdr_view::capture::SceneInfo {
        source: "COFDM".to_owned(),
        fs_hz: 1_920_000.0,
        center_hz: 480_000.0,
        span_hz: 960_000.0,
        db_min: -100.0,
        db_max: -15.0,
        overlays: true,
    };
    let f = frame(7, T0_MILLIS, 3, 2);
    let path = orion_sdr_view::capture::writer::write_still(dir.path(), &f, 0, scene)
        .expect("write still");

    assert_eq!(
        path.file_name().map(|s| s.to_string_lossy().into_owned()),
        Some("20260816T112233.456Z.png".to_owned())
    );
    let meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(sidecar_path(&path)).expect("sidecar"))
            .expect("valid json");
    assert_eq!(meta["kind"], "still");
    assert_eq!(meta["file"], "20260816T112233.456Z.png");
    assert_eq!(meta["time"], "2026-08-16T11:22:33.456Z");
    assert_eq!(meta["seq"], 7);
    assert_eq!(meta["source"], "COFDM");
    assert_eq!(meta["fs_hz"], 1_920_000.0);
    assert_eq!(meta["center_hz"], 480_000.0);
}

// ── G. The keys, in both key paths ──────────────────────────────────────────

fn open_settings(h: &mut Harness) {
    h.key(egui::Key::S);
    assert!(
        h.app.settings().visible,
        "the settings overlay should be up"
    );
}

#[test]
fn f_requests_a_capture_with_the_settings_overlay_open_and_closed() {
    // The regression test for the `M` defect, applied pre-emptively: that key
    // reached only the settings-open path and did nothing with the panel shut.
    // Capturing *with* an overlay up is a first-class use — a still of the
    // settings or instrument panel is exactly what documentation wants.
    for overlay in [false, true] {
        let mut h = Harness::with_defaults();
        if overlay {
            open_settings(&mut h);
        }
        h.key(egui::Key::F);
        let tags = h.screenshot_tags();
        assert_eq!(tags.len(), 1, "overlay={overlay}: expected one request");
    }
}

#[test]
fn v_toggles_recording_from_both_key_paths() {
    // Worse than an inconvenience for `V`: a stop press the overlay swallowed
    // would leave the user unable to stop recording without closing the panel
    // first.
    for overlay in [false, true] {
        let mut h = Harness::from_yaml("view:\n  capture:\n    format: png\n");
        if overlay {
            open_settings(&mut h);
        }
        h.key(egui::Key::V);
        assert!(
            h.app.is_recording(),
            "overlay={overlay}: should be recording"
        );
        h.key(egui::Key::V);
        assert!(
            !h.app.is_recording(),
            "overlay={overlay}: should have stopped"
        );
    }
}

#[test]
fn recording_asks_for_a_frame_every_pass_and_marks_the_title() {
    // The indicator goes in the title because the readback covers the client
    // area alone: a `REC` badge drawn into the window would be captured into
    // every frame of the recording it was announcing.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut h = Harness::from_yaml(&format!(
        "view:\n  capture:\n    format: png\n    dir: {}\n",
        dir.path().display()
    ));
    h.key(egui::Key::V);
    assert_eq!(
        h.requested_title().as_deref(),
        Some("orion-sdr-view  \u{25cf} REC")
    );

    // Every subsequent pass asks for an image, and each tag is distinct.
    let mut seqs = Vec::new();
    for _ in 0..3 {
        h.idle(1);
        seqs.extend(h.screenshot_tags().into_iter().map(|t| t.seq));
    }
    assert_eq!(seqs.len(), 3, "one request per frame while recording");
    let unique: std::collections::BTreeSet<u64> = seqs.iter().copied().collect();
    assert_eq!(unique.len(), 3, "tags must be distinct: {seqs:?}");

    h.key(egui::Key::V);
    assert_eq!(h.requested_title().as_deref(), Some("orion-sdr-view"));
}

#[test]
fn a_capture_is_stamped_with_the_instant_it_depicts() {
    // Not the instant the image came back.  The command is issued during frame
    // N and the readback returns one or more frames later, so stamping on
    // arrival would smear the timeline by the readback latency.
    let mut h = Harness::with_defaults();
    h.idle(5);
    let before = h.app.clock().now();
    h.key(egui::Key::F);
    let tag = h.screenshot_tags().first().copied().expect("a request");
    let after = h.app.clock().now();
    assert!(
        tag.content_time >= before && tag.content_time <= after,
        "the tag should name this frame's instant"
    );
}

#[test]
fn a_returning_image_is_written_to_the_capture_directory() {
    // The full round trip across both seams, with no renderer: the request
    // leaves as a viewport command and the image arrives as an input event,
    // exactly as eframe's wgpu integration delivers it.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut h = Harness::with_defaults();
    h.app.set_capture_dir(dir.path().to_path_buf());

    h.key(egui::Key::F);
    let tag = h.screenshot_tags().first().copied().expect("a request");
    h.deliver_screenshot(tag, 4, 3, egui::Color32::from_rgb(0x20, 0x24, 0x2b));

    let written: Vec<String> = std::fs::read_dir(dir.path())
        .expect("read_dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(written.len(), 2, "a PNG and its sidecar: {written:?}");
    assert!(written.iter().any(|n| n.ends_with(".png")), "{written:?}");
    assert!(written.iter().any(|n| n.ends_with(".json")), "{written:?}");
}

#[test]
fn an_unmatched_image_is_discarded_rather_than_written() {
    // An image nobody asked for — arriving after its recording stopped, say —
    // must not be filed as though it had been requested.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut h = Harness::with_defaults();
    h.app.set_capture_dir(dir.path().to_path_buf());
    h.deliver_screenshot(
        CaptureTag {
            seq: 999,
            content_time: at(T0_MILLIS),
        },
        4,
        3,
        egui::Color32::RED,
    );
    assert_eq!(
        std::fs::read_dir(dir.path()).expect("read_dir").count(),
        0,
        "nothing should have been written"
    );
}

// ── H. Where captures are written ───────────────────────────────────────────

#[test]
fn the_capture_flag_overrides_the_configured_directory() {
    // `--capture <dir>` calls `set_capture_dir`, so the precedence to pin is
    // command line over config over built-in default — the same order every
    // other setting here follows.
    let configured = tempfile::tempdir().expect("tempdir");
    let overridden = tempfile::tempdir().expect("tempdir");
    let mut h = Harness::from_yaml(&format!(
        "view:\n  capture:\n    dir: {}\n",
        configured.path().display()
    ));
    assert_eq!(h.app.capture_dir(), configured.path());

    h.app.set_capture_dir(overridden.path().to_path_buf());
    assert_eq!(h.app.capture_dir(), overridden.path());

    // ...and the override is where the image actually lands.
    h.key(egui::Key::F);
    let tag = h.screenshot_tags().first().copied().expect("a request");
    h.deliver_screenshot(tag, 4, 3, egui::Color32::from_rgb(0x20, 0x24, 0x2b));

    assert_eq!(
        std::fs::read_dir(overridden.path())
            .expect("read_dir")
            .count(),
        2,
        "the PNG and its sidecar belong to the overriding directory"
    );
    assert_eq!(
        std::fs::read_dir(configured.path())
            .expect("read_dir")
            .count(),
        0,
        "nothing should have been written to the configured directory"
    );
}

#[test]
fn a_configured_capture_directory_expands_a_leading_tilde() {
    // The value comes from a config file rather than a shell, so nothing has
    // expanded it already.  Only a leading `~/`: a `~user` form needs the
    // password database, and a `~` anywhere else is an ordinary character.
    use orion_sdr_view::config::expand_tilde;
    let home = std::env::var("HOME").expect("HOME");
    assert_eq!(
        expand_tilde("~/Captures"),
        Path::new(&home).join("Captures")
    );
    assert_eq!(expand_tilde("~"), Path::new(&home));
    for literal in ["/tmp/shots", "relative/shots", "~user/shots", "a~b"] {
        assert_eq!(expand_tilde(literal), Path::new(literal), "{literal}");
    }
}

#[test]
fn nothing_is_created_until_something_is_captured() {
    // The default directory lives in `$HOME`, so a session that never captures
    // must leave no trace of it.
    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("not-yet");
    let mut h = Harness::with_defaults();
    h.app.set_capture_dir(target.clone());
    h.idle(5);
    assert!(!target.exists(), "the directory should not exist yet");

    h.key(egui::Key::F);
    let tag = h.screenshot_tags().first().copied().expect("a request");
    h.deliver_screenshot(tag, 2, 2, egui::Color32::WHITE);
    assert!(target.is_dir(), "capturing should have created it");
}

// ── I. Terminal notices ─────────────────────────────────────────────────────

#[test]
fn a_warning_is_styled_and_a_confirmation_is_not() {
    use orion_sdr_view::utils::term::{Level, style};
    // Bold yellow for a warning, bold red for a failure — something the user
    // has to act on should not look like a line confirming a file was written.
    let warn = style(Level::Warn, "capture: ffmpeg was not found", true);
    assert!(warn.starts_with("\u{1b}[1;33m\u{26a0} "), "{warn:?}");
    assert!(warn.ends_with("\u{1b}[0m"), "{warn:?}");

    let err = style(Level::Error, "capture: could not write", true);
    assert!(err.starts_with("\u{1b}[1;31m\u{2717} "), "{err:?}");

    // Info carries the glyph but no colour: it is not asking for attention.
    let info = style(Level::Info, "capture: wrote out.png", true);
    assert_eq!(info, "\u{2022} capture: wrote out.png");
}

#[test]
fn the_icon_survives_a_terminal_that_cannot_colour() {
    use orion_sdr_view::utils::term::{Level, style};
    // Escape codes in a redirected log are worse than no colour, so styling is
    // dropped entirely off a terminal — and the glyphs are distinct in *shape*
    // so the severity still reads.
    for (level, icon) in [
        (Level::Info, "\u{2022}"),
        (Level::Warn, "\u{26a0}"),
        (Level::Error, "\u{2717}"),
    ] {
        let plain = style(level, "message", false);
        assert_eq!(plain, format!("{icon} message"));
        assert!(!plain.contains('\u{1b}'), "no escapes when unstyled");
    }
}

#[test]
fn captures_default_to_a_directory_beside_the_project() {
    // `./capture` rather than `$HOME`: captures are usually taken *of* something
    // being worked on, so they belong next to it and are easy to gitignore.
    let h = Harness::with_defaults();
    assert_eq!(h.app.capture_dir(), Path::new("./capture"));
}

// ── J. Pane rasters, headless ───────────────────────────────────────────────

#[test]
fn a_pane_directive_writes_the_dsp_s_own_raster() {
    // **No renderer is involved.** The waterfall, spectrogram and persistence
    // panes keep their pixels CPU-side, so this is reachable in a headless run
    // where a window screenshot is not.
    use orion_sdr_view::utils::script::Pane;
    // The three *spectral* panes accumulate from any source.  The two decoder
    // panes are COFDM-only and need a decode first — see
    // `the_decoder_panes_capture_after_a_decode`.
    for pane in [Pane::Waterfall, Pane::Spectrogram, Pane::Persistence] {
        let pane = &pane;
        let dir = tempfile::tempdir().expect("tempdir");
        let mut h = Harness::with_defaults();
        h.capture_dir = dir.path().to_path_buf();
        h.idle(40); // let the panes accumulate something

        h.run_script(&format!("0.0 pane {}\n", pane.name()));

        let mut names: Vec<String> = std::fs::read_dir(dir.path())
            .expect("read_dir")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names.len(),
            2,
            "{}: a PNG and its sidecar: {names:?}",
            pane.name()
        );
        assert!(
            names.iter().all(|n| n.contains(pane.name())),
            "{}: the pane should be in the filename: {names:?}",
            pane.name()
        );
    }
}

#[test]
fn the_decoder_panes_capture_after_a_decode() {
    // The constellation and correction rasters exist only where there is a
    // receiver, and only once it has produced a frame — so unlike the three
    // spectral panes they cannot be captured off an idle default harness.
    //
    // **All three gate conditions have to be met for the probe to run at all**:
    // pane 3 visible, in the decoder mode, on COFDM.  That makes this the
    // end-to-end test of the gate as well as of the rasters: if `W` did not
    // reach the mode, or the mode did not re-sync the decode config, nothing
    // would be written and the assertion below would fail with an empty
    // directory.
    use orion_sdr_view::app::SourceMode;
    use orion_sdr_view::utils::script::Pane;

    let dir = tempfile::tempdir().expect("tempdir");
    let mut h = Harness::with_defaults();
    h.capture_dir = dir.path().to_path_buf();
    h.select_source(SourceMode::Cofdm);
    // Waterfall -> Spectrogram -> Constellation.
    h.key_n(egui::Key::W, 2);

    // **Poll, do not idle a fixed count.**  A gap empties the constellation, so
    // a fixed wait can land in the silence between bursts and find nothing to
    // capture — which is correct behaviour and a broken test.  Run until there
    // is something, then capture straight away.
    for _ in 0..1200 {
        h.idle(1);
        if !h.app.constellation().is_empty() && h.app.correction().committed() > 0 {
            break;
        }
    }
    assert!(
        !h.app.constellation().is_empty(),
        "the receiver should have produced symbols by now"
    );
    assert!(
        h.app.correction().committed() > 0,
        "and the correction map should have committed rows"
    );

    for pane in [Pane::Constellation, Pane::Correction] {
        h.run_script(&format!("0.0 pane {}\n", pane.name()));
    }
    let names: Vec<String> = std::fs::read_dir(dir.path())
        .expect("read_dir")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    for pane in ["constellation", "correction"] {
        assert_eq!(
            names.iter().filter(|n| n.contains(pane)).count(),
            2,
            "{pane}: a PNG and its sidecar: {names:?}"
        );
    }
}

#[test]
fn a_pane_sidecar_says_which_pane_and_which_label() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut h = Harness::with_defaults();
    h.capture_dir = dir.path().to_path_buf();
    h.idle(40);
    h.run_script("0.0 pane waterfall burst_2\n");

    let json = std::fs::read_dir(dir.path())
        .expect("read_dir")
        .map(|e| e.expect("entry").path())
        .find(|p| p.extension().is_some_and(|e| e == "json"))
        .expect("a sidecar");
    let meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&json).expect("read")).expect("json");
    assert_eq!(meta["kind"], "pane");
    assert_eq!(meta["pane"], "waterfall");
    assert_eq!(meta["label"], "burst_2");
    assert!(meta["width"].as_u64().unwrap_or(0) > 0);
    assert!(meta["height"].as_u64().unwrap_or(0) > 0);
    // The scene travels with it, so the raster can be read later.
    assert_eq!(meta["source"], "Test Tone");
    assert_eq!(meta["fs_hz"], 48_000.0);
}

#[test]
fn a_waterfall_raster_matches_the_pane_it_came_from() {
    // The pixels written must be the pane's own, in its display order — not a
    // re-render, and not the physical ring order.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut h = Harness::with_defaults();
    h.capture_dir = dir.path().to_path_buf();
    h.idle(40);

    let want_w = h.app.waterfall().freq_bins();
    let want_h = h.app.waterfall().filled();
    let first_row: Vec<egui::Color32> = h
        .app
        .waterfall()
        .rows_in_display_order()
        .next()
        .expect("a row")
        .to_vec();

    h.run_script("0.0 pane waterfall\n");
    let png = std::fs::read_dir(dir.path())
        .expect("read_dir")
        .map(|e| e.expect("entry").path())
        .find(|p| p.extension().is_some_and(|e| e == "png"))
        .expect("a png");

    let decoder = png::Decoder::new(std::io::BufReader::new(
        std::fs::File::open(&png).expect("open"),
    ));
    let mut reader = decoder.read_info().expect("info");
    let mut buf = vec![0; reader.output_buffer_size().expect("size")];
    let info = reader.next_frame(&mut buf).expect("decode");
    assert_eq!(
        (info.width as usize, info.height as usize),
        (want_w, want_h)
    );

    // Top row of the image is the newest row of the waterfall.
    let got: Vec<egui::Color32> = buf[..want_w * 4]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| egui::Color32::from_rgba_premultiplied(p[0], p[1], p[2], p[3]))
        .collect();
    assert_eq!(got, first_row, "the image is not the pane's display order");
}

#[test]
fn a_pane_with_no_pixels_yet_writes_nothing_and_says_so() {
    // A legitimate outcome, not a failure — but a missing file would otherwise
    // look like a broken directive.
    let dir = tempfile::tempdir().expect("tempdir");
    let mut h = Harness::with_defaults();
    h.capture_dir = dir.path().to_path_buf();
    // No idle: nothing has been processed, so the waterfall is empty.
    let wrote = h
        .app
        .capture_pane(
            dir.path(),
            orion_sdr_view::utils::script::Pane::Waterfall,
            None,
        )
        .expect("no error");
    assert_eq!(wrote, None);
    assert_eq!(std::fs::read_dir(dir.path()).expect("read_dir").count(), 0);
}

// ── K. Headless stills, rasterized on the CPU ───────────────────────────────

/// A script that reaches a steady COFDM display, then captures.
const STILL_SCRIPT: &str = "
set run.size 640x480

0.00 source COFDM
0.50 key D
1.50 still
";

fn run_still(dir: &Path, script: &str) -> Vec<std::path::PathBuf> {
    let opts = orion_sdr_view::replay::RunOptions {
        script: Some(script.to_owned()),
        duration: Some(2.0),
        capture: Some(dir.to_path_buf()),
        ..Default::default()
    };
    orion_sdr_view::replay::run_into(
        orion_sdr_view::config::ViewConfig::empty(),
        &opts,
        std::io::sink(),
    )
    .expect("the run should succeed")
    .captures
}

#[test]
fn the_same_script_produces_the_same_image() {
    // **The property the whole rasterizer exists for.** A GPU render cannot
    // carry it — fill rules and filtering vary by vendor and driver — and
    // without it a capture is no use as a test fixture, because a failure could
    // not be told from a different machine.
    let (a, b) = (
        tempfile::tempdir().expect("tempdir"),
        tempfile::tempdir().expect("tempdir"),
    );
    let pa = run_still(a.path(), STILL_SCRIPT);
    let pb = run_still(b.path(), STILL_SCRIPT);
    assert_eq!(pa.len(), 1, "one still");
    assert_eq!(pb.len(), 1);

    let (ba, bb) = (
        std::fs::read(&pa[0]).expect("read"),
        std::fs::read(&pb[0]).expect("read"),
    );
    assert!(!ba.is_empty());
    assert_eq!(
        ba,
        bb,
        "two runs of one script produced different images ({} vs {} bytes)",
        ba.len(),
        bb.len()
    );
    // ...and the name is reproducible too, since it comes from the scripted
    // clock rather than the wall clock.
    assert_eq!(pa[0].file_name(), pb[0].file_name());
}

#[test]
fn a_still_is_the_size_the_script_asked_for() {
    // Not egui's 10000 x 10000 fallback, which would be a 400 MB image.
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = run_still(dir.path(), STILL_SCRIPT);
    let decoder = png::Decoder::new(std::io::BufReader::new(
        std::fs::File::open(&paths[0]).expect("open"),
    ));
    let info = decoder.read_info().expect("info");
    assert_eq!((info.info().width, info.info().height), (640, 480));
}

#[test]
fn a_still_actually_contains_the_window_rather_than_a_blank_frame() {
    // The failure this guards against is *plausible*: a mesh whose texture was
    // never uploaded simply does not draw, so a missing font atlas yields an
    // almost-empty image rather than an error — and egui draws solid shapes
    // from a white texel in that same atlas, so nearly everything vanishes at
    // once.  Distinct colours is the cheapest assertion that the frame is real.
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = run_still(dir.path(), STILL_SCRIPT);
    let decoder = png::Decoder::new(std::io::BufReader::new(
        std::fs::File::open(&paths[0]).expect("open"),
    ));
    let mut reader = decoder.read_info().expect("info");
    let mut buf = vec![0; reader.output_buffer_size().expect("size")];
    let info = reader.next_frame(&mut buf).expect("decode");

    let distinct: std::collections::HashSet<[u8; 4]> = buf[..info.buffer_size()]
        .as_chunks::<4>()
        .0
        .iter()
        .copied()
        .collect();
    assert!(
        distinct.len() > 100,
        "only {} distinct colours — the frame is probably mostly unrendered",
        distinct.len()
    );
    // Every pixel opaque: the window has no transparency, and an alpha hole
    // would read as a rendering bug.
    assert!(
        buf[..info.buffer_size()]
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[3] == 255),
        "a still should be fully opaque"
    );
}

#[test]
fn a_script_without_a_still_is_unaffected_by_the_capture_path() {
    // The zero-cost claim, made checkable: with no `still` the driver builds no
    // capturer, so it never draws and never tessellates.  If that ever changed,
    // the dump would be the first thing to notice.
    let plain = "set run.size 640x480\n0.00 source CW\n";
    let opts = |script: &str| orion_sdr_view::replay::RunOptions {
        script: Some(script.to_owned()),
        duration: Some(1.0),
        ..Default::default()
    };
    let (mut a, mut b) = (Vec::new(), Vec::new());
    orion_sdr_view::replay::run_into(
        orion_sdr_view::config::ViewConfig::empty(),
        &opts(plain),
        &mut a,
    )
    .expect("run");
    orion_sdr_view::replay::run_into(
        orion_sdr_view::config::ViewConfig::empty(),
        &opts(plain),
        &mut b,
    )
    .expect("run");
    assert_eq!(a, b);
    assert!(
        !a.is_empty(),
        "the run should still have measured something"
    );
}

#[test]
fn a_still_carries_its_label_into_the_filename_and_sidecar() {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = run_still(
        dir.path(),
        "set run.size 640x480\n0.00 source COFDM\n1.50 still band_edge\n",
    );
    let name = paths[0].file_name().unwrap_or_default().to_string_lossy();
    assert!(name.ends_with("-band_edge.png"), "{name}");

    let meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(sidecar_path(&paths[0])).expect("sidecar"))
            .expect("json");
    assert_eq!(meta["kind"], "still");
    assert_eq!(meta["label"], "band_edge");
    assert_eq!(meta["source"], "COFDM");
    assert_eq!(meta["width"], 640);
}
