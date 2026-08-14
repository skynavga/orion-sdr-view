// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! System time-zone helpers and the wall clock the view layer stamps
//! timestamps from.
//!
//! These live in `utils` so both the config loader (which may need to resolve
//! `time_zone: local` at startup) and the view layer (which formats wall-clock
//! timestamps) can share a single implementation.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The instant a [`Clock::Scripted`] run starts from: 2026-01-01T00:00:00Z.
///
/// Arbitrary, fixed and obviously synthetic.  A dump's timestamps are then a
/// function of scripted time alone, so they are legible ("14 s into the run")
/// without ever being mistaken for when the run actually happened.
pub const SCRIPT_EPOCH_SECS: u64 = 1_767_225_600;

/// Where user-visible timestamps come from.
///
/// The interactive app reads the system clock.  **A scripted run must not.**
/// CW and PSK31 frame each burst with an opening timestamp and FT8 stamps its
/// decoded frames with the signal onset, so a system clock would put the time of
/// day into decoded text — and two runs of the same script would differ in
/// exactly the output a replay driver exists to compare.
///
/// This is the same move as the injected `dt`, for the same reason and against
/// the same class of problem: take the one impure read out of the path and hand
/// it in.  `Scripted` advances by that same `dt`, so scripted time and stamped
/// time cannot drift apart.
#[derive(Debug, Clone, Copy)]
pub enum Clock {
    /// Real time, read per call.
    System,
    /// [`SCRIPT_EPOCH_SECS`] plus the scripted time elapsed so far.
    Scripted { elapsed_secs: f64 },
}

impl Clock {
    /// A scripted clock at time zero.
    pub fn scripted() -> Self {
        Self::Scripted { elapsed_secs: 0.0 }
    }

    /// Advance a scripted clock by one frame.  A no-op on [`Clock::System`],
    /// which has no state to advance.
    pub fn advance(&mut self, dt: f32) {
        if let Self::Scripted { elapsed_secs } = self {
            *elapsed_secs += f64::from(dt);
        }
    }

    /// The current instant.
    pub fn now(&self) -> SystemTime {
        match self {
            Self::System => SystemTime::now(),
            Self::Scripted { elapsed_secs } => {
                UNIX_EPOCH
                    + Duration::from_secs(SCRIPT_EPOCH_SECS)
                    + secs_to_duration(*elapsed_secs)
            }
        }
    }

    /// Scripted seconds elapsed, or `None` for a system clock.
    pub fn elapsed_secs(&self) -> Option<f64> {
        match self {
            Self::System => None,
            Self::Scripted { elapsed_secs } => Some(*elapsed_secs),
        }
    }
}

/// Seconds to a `Duration`, saturating at zero.  `Duration::from_secs_f64`
/// panics on a negative or non-finite input, and a clock is not worth a panic.
fn secs_to_duration(secs: f64) -> Duration {
    if secs.is_finite() && secs > 0.0 {
        Duration::from_secs_f64(secs)
    } else {
        Duration::ZERO
    }
}

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

    let unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
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
