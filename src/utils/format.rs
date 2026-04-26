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
