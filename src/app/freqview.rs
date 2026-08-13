// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Display-side frequency markers.
//!
//! The viewport itself ([`FreqView`]) is UI-independent arithmetic and lives in
//! the library at [`crate::viewport`]; it is re-exported here so the rest of the
//! app keeps one import path for the pair.

pub use crate::viewport::FreqView;

// ── FreqMarker ────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub enum MarkerKind {
    Primary,  // center marker — tracks FreqView.center_hz; shown in cyan
    BracketA, // user-placed bracket A; shown in yellow
    BracketB, // user-placed bracket B; shown in orange
}

#[derive(Clone)]
pub struct FreqMarker {
    pub kind: MarkerKind,
    pub hz: f32,
    pub enabled: bool,
}

impl FreqMarker {
    pub fn primary(hz: f32) -> Self {
        Self {
            kind: MarkerKind::Primary,
            hz,
            enabled: true,
        }
    }

    pub fn bracket_a(hz: f32) -> Self {
        Self {
            kind: MarkerKind::BracketA,
            hz,
            enabled: false,
        }
    }

    pub fn bracket_b(hz: f32) -> Self {
        Self {
            kind: MarkerKind::BracketB,
            hz,
            enabled: false,
        }
    }

    pub fn color(&self) -> eframe::egui::Color32 {
        match self.kind {
            MarkerKind::Primary => eframe::egui::Color32::from_rgb(0, 220, 255),
            MarkerKind::BracketA => eframe::egui::Color32::from_rgb(255, 220, 0),
            MarkerKind::BracketB => eframe::egui::Color32::from_rgb(255, 140, 0),
        }
    }

    pub fn label(&self) -> &'static str {
        match self.kind {
            MarkerKind::Primary => "▼",
            MarkerKind::BracketA => "A",
            MarkerKind::BracketB => "B",
        }
    }
}
