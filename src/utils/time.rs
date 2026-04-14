// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! System time-zone helpers.
//!
//! These live in `utils` so both the config loader (which may need to resolve
//! `time_zone: local` at startup) and the view layer (which formats wall-clock
//! timestamps) can share a single implementation.

/// Return the local UTC offset in seconds using POSIX `localtime_r` / `gmtime_r`.
/// Returns 0 on non-Unix platforms.
#[cfg(unix)]
pub fn local_utc_offset_secs() -> i64 {
    // Raw C bindings — avoids a libc crate dependency.
    // tm struct layout is identical on macOS and Linux (9 × i32).
    #[repr(C)]
    struct Tm {
        tm_sec: i32,
        tm_min: i32,
        tm_hour: i32,
        tm_mday: i32,
        tm_mon: i32,
        tm_year: i32,
        tm_wday: i32,
        tm_yday: i32,
        tm_isdst: i32,
        // macOS has two extra fields; pad generously.
        _pad: [i32; 8],
    }
    unsafe extern "C" {
        fn localtime_r(timep: *const i64, result: *mut Tm) -> *mut Tm;
        fn gmtime_r(timep: *const i64, result: *mut Tm) -> *mut Tm;
    }

    let unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let mut local_tm: Tm = unsafe { std::mem::zeroed() };
    let mut gm_tm: Tm = unsafe { std::mem::zeroed() };
    unsafe {
        localtime_r(&unix, &mut local_tm);
        gmtime_r(&unix, &mut gm_tm);
    }

    let local_secs =
        local_tm.tm_hour as i64 * 3600 + local_tm.tm_min as i64 * 60 + local_tm.tm_sec as i64;
    let gm_secs = gm_tm.tm_hour as i64 * 3600 + gm_tm.tm_min as i64 * 60 + gm_tm.tm_sec as i64;

    let mut diff = local_secs - gm_secs;
    if diff > 14 * 3600 {
        diff -= 24 * 3600;
    }
    if diff < -12 * 3600 {
        diff += 24 * 3600;
    }
    diff
}

#[cfg(not(unix))]
pub fn local_utc_offset_secs() -> i64 {
    0
}

/// Local UTC offset in minutes, clamped to the display range [-12*60, 14*60].
pub fn local_utc_offset_min() -> i32 {
    let m = (local_utc_offset_secs() / 60) as i32;
    m.clamp(-12 * 60, 14 * 60)
}
