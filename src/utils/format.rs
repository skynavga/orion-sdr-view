// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Formatting helpers (wall-clock time, etc.) shared across UI layers.

/// Format a `SystemTime` as `HH:MM:SS.mmm`, offset from UTC by `offset_min`
/// minutes (positive = east of UTC, negative = west, 0 = UTC).
pub fn format_time(t: std::time::SystemTime, offset_min: i32) -> String {
    let dur = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let unix_secs = dur.as_secs() as i64;
    let millis = dur.subsec_millis();

    let secs = unix_secs + offset_min as i64 * 60;

    let s = secs.rem_euclid(60);
    let m = (secs / 60).rem_euclid(60);
    let h = (secs / 3600).rem_euclid(24);
    format!("{h:02}:{m:02}:{s:02}.{millis:03}")
}

/// Format a `SystemTime` as an **ISO 8601 basic-format** stamp with
/// milliseconds, e.g. `20260815T112233.456Z` or `20260815T062233.456-0500`.
///
/// Basic format, not extended, because this names files: extended's colons are
/// illegal in a path on Windows and are rendered as `/` by the macOS Finder.
/// Basic format is fully conformant, sorts lexicographically into time order
/// within a fixed offset, and needs no quoting in a shell.
///
/// **The offset designator is always present.** `20260815T112233` alone does
/// not say which clock it came from; a capture is an artifact that outlives the
/// session that made it, and a bare local time is a reading nobody can check.
///
/// **Milliseconds are part of the name, not decoration.** A recording at 30 fps
/// produces a frame every 33 ms, so second precision would collide thirty times
/// over — and the point of stamping the name is that two captures cannot land
/// on one path.
pub fn format_stamp(t: std::time::SystemTime, offset_min: i32) -> String {
    let (date, time, millis) = civil_parts(t, offset_min);
    let (y, mo, d) = date;
    let (h, mi, s) = time;
    format!(
        "{y:04}{mo:02}{d:02}T{h:02}{mi:02}{s:02}.{millis:03}{}",
        iso_offset(offset_min, false)
    )
}

/// Format a `SystemTime` as an **ISO 8601 extended-format** timestamp, e.g.
/// `2026-08-15T11:22:33.456Z`.
///
/// The readable spelling, for metadata rather than filenames — every JSON
/// consumer parses it, and nothing here has to survive a filesystem.
pub fn format_iso8601(t: std::time::SystemTime, offset_min: i32) -> String {
    let (date, time, millis) = civil_parts(t, offset_min);
    let (y, mo, d) = date;
    let (h, mi, s) = time;
    format!(
        "{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{millis:03}{}",
        iso_offset(offset_min, true)
    )
}

/// Split an instant into `((y, m, d), (h, m, s), millis)` at `offset_min`.
fn civil_parts(
    t: std::time::SystemTime,
    offset_min: i32,
) -> ((i64, u32, u32), (i64, i64, i64), u32) {
    let dur = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let millis = dur.subsec_millis();
    let secs = dur.as_secs() as i64 + offset_min as i64 * 60;
    // `div_euclid`/`rem_euclid`, not `/` and `%`: a pre-1970 instant has a
    // negative second count, where truncating division would land a day out.
    let date = civil_from_days(secs.div_euclid(86_400));
    let tod = secs.rem_euclid(86_400);
    (date, (tod / 3600, (tod / 60) % 60, tod % 60), millis)
}

/// The ISO 8601 offset designator: `Z` at UTC, else `±hhmm` (basic, for
/// filenames) or `±hh:mm` (extended, for metadata).
///
/// Always four digits, never the `±hh` short form.  One width means the stamp
/// is fixed-length whatever the zone, so names align in a listing and sort
/// cleanly — and half-hour zones (India at +05:30, Newfoundland at −03:30) are
/// represented exactly rather than being a special case.
fn iso_offset(offset_min: i32, extended: bool) -> String {
    if offset_min == 0 {
        return "Z".to_owned();
    }
    let sign = if offset_min < 0 { '-' } else { '+' };
    let (h, m) = (offset_min.abs() / 60, offset_min.abs() % 60);
    let sep = if extended { ":" } else { "" };
    format!("{sign}{h:02}{sep}{m:02}")
}

/// Civil date `(year, month, day)` from a count of days since 1970-01-01.
///
/// Howard Hinnant's `civil_from_days`, exact for the proleptic Gregorian
/// calendar over any range reachable here.  Written out rather than taking a
/// dependency on `chrono`, which would be a large addition to a DSP crate for
/// one filename — the same trade as the hand-rolled SHA-256 in `replay::dump`.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    // Shift the epoch to 0000-03-01, which puts the leap day at the end of the
    // year and makes the month arithmetic below a plain linear formula.
    let z = days + 719_468;
    let era = z.div_euclid(146_097); // 146097 days = 400 years, exactly
    let doe = z.rem_euclid(146_097); // day of era, [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of shifted year
    let mp = (5 * doy + 2) / 153; // shifted month, [0, 11] = March..February
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // back to [1, 12]
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}
