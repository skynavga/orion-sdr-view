// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Styling for the messages the app writes to the terminal.
//!
//! A GUI app's stderr is easy to miss.  Something the user needs to act on —
//! ffmpeg missing when they pressed `V`, or a recording that dropped frames —
//! should not look the same as a line confirming a file was written.

use std::io::IsTerminal;

/// How much attention a message deserves.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    /// Something happened that was asked for.
    Info,
    /// Something did not happen, or happened differently than intended.
    Warn,
    /// Something failed.
    Error,
}

impl Level {
    /// The prefix glyph.  Distinct in shape as well as colour, so the
    /// distinction survives a monochrome terminal, a log file, or a reader who
    /// cannot separate red from green.
    pub fn icon(self) -> &'static str {
        match self {
            Self::Info => "\u{2022}",  // •
            Self::Warn => "\u{26a0}",  // ⚠
            Self::Error => "\u{2717}", // ✗
        }
    }

    /// SGR parameters: bold, plus a colour for anything above `Info`.
    fn sgr(self) -> &'static str {
        match self {
            Self::Info => "",
            Self::Warn => "1;33",  // bold yellow
            Self::Error => "1;31", // bold red
        }
    }
}

/// Whether stderr should carry ANSI styling.
///
/// Two conditions, both necessary.  Escape codes in a redirected log are worse
/// than no colour at all, so the stream has to be a terminal; and `NO_COLOR` is
/// honoured by its *presence*, whatever its value, which is what the convention
/// specifies.
pub fn stderr_is_styled() -> bool {
    std::env::var_os("NO_COLOR").is_none() && std::io::stderr().is_terminal()
}

/// Format a message for stderr, styling it if the stream can carry it.
pub fn notice(level: Level, msg: &str) -> String {
    style(level, msg, stderr_is_styled())
}

/// As [`notice`], with the styling decision supplied.
///
/// Split out so a test can assert both forms without needing a terminal on one
/// run and a pipe on the other.
pub fn style(level: Level, msg: &str, styled: bool) -> String {
    let icon = level.icon();
    match (styled, level.sgr()) {
        (true, sgr) if !sgr.is_empty() => format!("\u{1b}[{sgr}m{icon} {msg}\u{1b}[0m"),
        _ => format!("{icon} {msg}"),
    }
}
