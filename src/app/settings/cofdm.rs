// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

use super::field::{CoarseStep, NumField, Row, ToggleField};
use crate::config::ViewConfig;
use crate::source::cofdm::{
    COFDM_DEFAULT_BW_FRACTION, COFDM_DEFAULT_FS, COFDM_DEFAULT_MASK, COFDM_DEFAULT_SHAPING_ENABLED,
    COFDM_DEFAULT_TAPER, COFDM_MAX_EDGE_GUARD, CofdmBwFraction, CofdmMask, CofdmShaping,
    CofdmTaper, cofdm_center_bounds, cofdm_default_center_hz, cofdm_edge_guard_for,
    cofdm_min_edge_guard, cofdm_spacing_hz,
};

// ── Row indices (local) ───────────────────────────────────────────────────
const CENTER: usize = 0;
const BANDWIDTH: usize = 1;
const SHAPING: usize = 2;
const EDGE_GUARD: usize = 3;
const INCLUDE_DC: usize = 4;
const TAPER: usize = 5;
const MASK: usize = 6;
const SIGNAL: usize = 7;
const GAP: usize = 8;
const CN: usize = 9;

/// Toggle option labels, in `CofdmBwFraction::ALL` order.
const BW_OPTIONS: &[&str] = &["1/8", "1/4", "1/3", "1/2", "2/3", "3/4", "7/8"];
/// Toggle option labels, in `CofdmTaper::ALL` order.
const TAPER_OPTIONS: &[&str] = &["off", "1/8", "1/4", "3/8"];
/// Toggle option labels, in `CofdmMask::ALL` order.
const MASK_OPTIONS: &[&str] = &["off", "40 dB", "60 dB", "80 dB"];
/// Boolean toggle labels (index 0 = false, 1 = true).
const OFF_ON_OPTIONS: &[&str] = &["Off", "On"];
const NO_YES_OPTIONS: &[&str] = &["No", "Yes"];

/// Index into `CofdmBwFraction::ALL` for the default fraction.
fn default_bw_index() -> usize {
    index_of(CofdmBwFraction::ALL, COFDM_DEFAULT_BW_FRACTION)
}

/// Position of `value` in `all`, or 0 if absent.
fn index_of<T: PartialEq + Copy>(all: &[T], value: T) -> usize {
    all.iter().position(|&v| v == value).unwrap_or(0)
}

/// Read a toggle row's index, or `fallback` if the row is not a toggle.
fn toggle_index(rows: &[Row], idx: usize, fallback: usize) -> usize {
    match &rows[idx] {
        Row::Toggle(f) => f.index,
        _ => fallback,
    }
}

/// Read a boolean toggle row (index 1 == true).
fn toggle_bool(rows: &[Row], idx: usize, fallback: bool) -> bool {
    toggle_index(rows, idx, usize::from(fallback)) == 1
}

/// Set a toggle row's index and default together.
fn set_toggle(rows: &mut [Row], idx: usize, value: usize) {
    if let Row::Toggle(f) = &mut rows[idx] {
        f.index = value;
        f.default = value;
    }
}

pub(super) struct CofdmRows {
    pub rows: Vec<Row>,
    /// Native sample rate (Hz).  **Not a row** — see `CofdmConfig::fs_hz` for
    /// why a live knob would be wrong.  It lives here anyway because
    /// `SettingsState` is the only thing threaded to `make()` / `sync()` /
    /// `occupied_bw_hz()`, so a per-source value that is not a row still has to
    /// reach them through it.
    ///
    /// Being off the row list also means an R-reset leaves it alone, which is
    /// right: resetting the display would otherwise silently re-derive Nyquist.
    fs_hz: f32,
}

impl CofdmRows {
    pub fn new() -> Self {
        let bw_idx = default_bw_index();
        // One source of truth for the shaping defaults: the same struct the
        // source renders from.
        let d = CofdmShaping::default_for(COFDM_DEFAULT_BW_FRACTION);
        let (shaping_idx, dc_idx) = (usize::from(d.enabled), usize::from(d.include_dc));
        let (taper_idx, mask_idx) = (
            index_of(CofdmTaper::ALL, d.taper),
            index_of(CofdmMask::ALL, d.mask),
        );
        let fs_hz = COFDM_DEFAULT_FS;
        let center = cofdm_default_center_hz(fs_hz);
        let (center_lo, center_hi) = cofdm_center_bounds(fs_hz);
        Self {
            fs_hz,
            rows: vec![
                Row::Num(NumField {
                    label: "Center",
                    value: center,
                    default: center,
                    // One subcarrier per press.  Coarse for a nudge row, but
                    // the arrow keys are not the tuning control here — `L`
                    // (lock source to viewport centre) is, and it writes the
                    // centre in directly at 10 Hz resolution.  A finer step
                    // would need thousands of presses to cross the range and
                    // would leave the band off the subcarrier grid for no gain.
                    step: cofdm_spacing_hz(fs_hz),
                    min: center_lo,
                    max: center_hi,
                    unit: " Hz",
                    coarse: None,
                }),
                Row::Toggle(ToggleField {
                    label: "Bandwidth",
                    options: BW_OPTIONS,
                    index: bw_idx,
                    default: bw_idx,
                }),
                Row::Toggle(ToggleField {
                    label: "Shaping",
                    options: OFF_ON_OPTIONS,
                    index: shaping_idx,
                    default: shaping_idx,
                }),
                Row::Num(NumField {
                    label: "Edge guard",
                    value: d.edge_guard as f32,
                    default: d.edge_guard as f32,
                    step: 1.0,
                    // Re-derived whenever `Center` moves — see
                    // `reseed_edge_guard_bounds`.  The band must fit inside
                    // `0..Nyquist`, and how much room there is depends on where
                    // it sits.
                    min: cofdm_min_edge_guard(center, fs_hz) as f32,
                    max: COFDM_MAX_EDGE_GUARD as f32,
                    unit: "",
                    coarse: None,
                }),
                Row::Toggle(ToggleField {
                    label: "Include DC",
                    options: NO_YES_OPTIONS,
                    index: dc_idx,
                    default: dc_idx,
                }),
                Row::Toggle(ToggleField {
                    label: "Taper",
                    options: TAPER_OPTIONS,
                    index: taper_idx,
                    default: taper_idx,
                }),
                Row::Toggle(ToggleField {
                    label: "Mask",
                    options: MASK_OPTIONS,
                    index: mask_idx,
                    default: mask_idx,
                }),
                Row::Num(NumField {
                    label: "Signal",
                    value: crate::source::cofdm::COFDM_DEFAULT_SIG_SECS,
                    default: crate::source::cofdm::COFDM_DEFAULT_SIG_SECS,
                    // 0.5 s steps below 10 s, 1 s steps at/above.
                    step: 0.5,
                    min: 1.0,
                    max: 99.99,
                    unit: " s",
                    coarse: Some(CoarseStep {
                        threshold: 10.0,
                        step: 1.0,
                    }),
                }),
                Row::Num(NumField {
                    label: "Gap",
                    value: crate::source::cofdm::COFDM_DEFAULT_GAP_SECS,
                    default: crate::source::cofdm::COFDM_DEFAULT_GAP_SECS,
                    step: 0.5,
                    min: 0.5,
                    max: 99.99,
                    unit: " s",
                    coarse: None,
                }),
                Row::Num(NumField {
                    label: "C/N",
                    value: crate::source::cofdm::COFDM_DEFAULT_CN_DB,
                    default: crate::source::cofdm::COFDM_DEFAULT_CN_DB,
                    step: 1.0,
                    min: MIN_CN_DB,
                    max: MAX_CN_DB,
                    unit: " dB",
                    coarse: None,
                }),
            ],
        }
    }

    /// Visible rows in the order they appear in the settings overlay.  The four
    /// shaping parameters are shown only while `Shaping` is on; with it off the
    /// source renders the fraction's own edge guard and no taper or mask.
    pub fn visible_indices(&self) -> Vec<usize> {
        let mut v = vec![CENTER, BANDWIDTH, SHAPING];
        if self.shaping_enabled() {
            v.extend([EDGE_GUARD, INCLUDE_DC, TAPER, MASK]);
        }
        v.extend([SIGNAL, GAP, CN]);
        v
    }

    fn shaping_enabled(&self) -> bool {
        toggle_bool(&self.rows, SHAPING, COFDM_DEFAULT_SHAPING_ENABLED)
    }

    fn bw_fraction(&self) -> CofdmBwFraction {
        CofdmBwFraction::ALL
            .get(toggle_index(&self.rows, BANDWIDTH, default_bw_index()))
            .copied()
            .unwrap_or(COFDM_DEFAULT_BW_FRACTION)
    }

    fn center_hz(&self) -> f32 {
        match &self.rows[CENTER] {
            Row::Num(f) => f.value,
            _ => cofdm_default_center_hz(self.fs_hz),
        }
    }

    /// Re-seed the `Edge guard` row from the current bandwidth fraction.  The
    /// fraction is what implies a guard; nudging the guard directly then
    /// overrides it until the fraction moves again.
    ///
    /// Sets the row's **value only**.  Its `default` is the startup pairing —
    /// the configured fraction's guard, or a YAML `edge_guard` that pinned it —
    /// and an R-reset restores bandwidth and guard to that pair together, so
    /// overwriting the default here would lose a pinned value.
    fn reseed_edge_guard(&mut self) {
        let guard = cofdm_edge_guard_for(self.bw_fraction()) as f32;
        if let Row::Num(f) = &mut self.rows[EDGE_GUARD] {
            f.value = guard.clamp(f.min, f.max);
        }
    }

    /// Re-derive the `Edge guard` row's lower bound from where the band now
    /// sits, and pull the current value up into the new range.
    ///
    /// This is the settings-side half of the centre/guard coupling.  It does
    /// not *decide* the guard — [`CofdmShaping::effective`] is still the single
    /// resolver, and it clamps again — but without it the row would display a
    /// guard the source is not using, which is the disagreement the resolver
    /// exists to prevent everywhere else.
    fn reseed_edge_guard_bounds(&mut self) {
        let min = cofdm_min_edge_guard(self.center_hz(), self.fs_hz) as f32;
        if let Row::Num(f) = &mut self.rows[EDGE_GUARD] {
            f.min = min;
            f.value = f.value.clamp(f.min, f.max);
        }
    }

    /// Re-derive the `Center` row's range and step from `fs_hz`, keeping the
    /// current value inside it.  Called after the configured rate lands.
    fn reseed_center_bounds(&mut self) {
        let (lo, hi) = cofdm_center_bounds(self.fs_hz);
        let step = cofdm_spacing_hz(self.fs_hz);
        if let Row::Num(f) = &mut self.rows[CENTER] {
            f.min = lo;
            f.max = hi;
            f.step = step;
            f.value = f.value.clamp(lo, hi);
            f.default = f.default.clamp(lo, hi);
        }
    }
}

// ── SourceRows ─────────────────────────────────────────────────────────────

impl super::common::SourceRows for CofdmRows {
    fn rows(&self) -> &[Row] {
        &self.rows
    }
    fn rows_mut(&mut self) -> &mut [Row] {
        &mut self.rows
    }
    fn visible_indices(&self) -> Vec<usize> {
        self.visible_indices()
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn after_nudge(&mut self, local_idx: usize) {
        // Two triggers, one coupling: the fraction *implies* a guard, the
        // centre *bounds* it.  A bandwidth nudge re-seeds the value against
        // whatever bound the current centre left, so at an off-centre position
        // the wider fractions land clamped — which is the same answer
        // `CofdmShaping::effective` gives, and the point of routing both here.
        if local_idx == CENTER {
            self.reseed_edge_guard_bounds();
        }
        if local_idx == BANDWIDTH {
            self.reseed_edge_guard();
        }
    }
    fn patch_from_config(&mut self, cfg: &ViewConfig) {
        // Rate first: the centre's range, the centre's default and the edge
        // guard's lower bound are all derived from it.
        self.fs_hz = cfg.cofdm_fs_hz();
        self.reseed_center_bounds();
        self.rows[CENTER].patch_num(cfg.cofdm_center_hz());
        self.reseed_edge_guard_bounds();

        self.rows[SIGNAL].patch_num(cfg.cofdm_sig_secs());
        self.rows[GAP].patch_num(cfg.cofdm_gap_secs());
        self.rows[CN].patch_num(cfg.cofdm_cn_db());

        let fraction = cfg.cofdm_bw_fraction();
        set_toggle(
            &mut self.rows,
            BANDWIDTH,
            index_of(CofdmBwFraction::ALL, fraction),
        );
        set_toggle(
            &mut self.rows,
            SHAPING,
            usize::from(cfg.cofdm_shaping_enabled()),
        );
        set_toggle(
            &mut self.rows,
            INCLUDE_DC,
            usize::from(cfg.cofdm_include_dc()),
        );
        set_toggle(
            &mut self.rows,
            TAPER,
            index_of(CofdmTaper::ALL, cfg.cofdm_taper()),
        );
        set_toggle(
            &mut self.rows,
            MASK,
            index_of(CofdmMask::ALL, cfg.cofdm_mask()),
        );
        // An absent `edge_guard` key means "whatever the fraction implies".
        // `patch_num` sets value *and* default, so this is also the pair an
        // R-reset restores alongside the bandwidth toggle above.
        self.rows[EDGE_GUARD].patch_num(
            cfg.cofdm_edge_guard()
                .unwrap_or_else(|| cofdm_edge_guard_for(fraction)) as f32,
        );
    }
}

// ── SettingsState accessors ───────────────────────────────────────────────

use crate::app::SourceMode;
use crate::source::{MAX_CN_DB, MIN_CN_DB};

/// Borrow this source's rows from `SettingsState`.
fn rows(state: &super::SettingsState) -> &CofdmRows {
    state.source_as::<CofdmRows>(SourceMode::Cofdm as usize)
}
fn rows_mut(state: &mut super::SettingsState) -> &mut CofdmRows {
    state.source_as_mut::<CofdmRows>(SourceMode::Cofdm as usize)
}

/// Typed accessors for COFDM settings.  Implemented for `SettingsState`;
/// callers `use crate::app::settings::CofdmSettings` to bring these in scope.
///
/// `set_cofdm_center_hz` is what makes the `L` key uniform across all six
/// sources.  It used to be absent, and `cofdm::Factory::set_carrier_hz` was a
/// documented no-op — so `L` was a key that did nothing on one source and said
/// nothing about it.
pub(in crate::app) trait CofdmSettings {
    fn cofdm_sig_secs(&self) -> f32;
    fn cofdm_gap_secs(&self) -> f32;
    fn cofdm_cn_db(&self) -> f32;
    fn cofdm_bw_fraction(&self) -> CofdmBwFraction;
    fn cofdm_shaping(&self) -> CofdmShaping;
    fn cofdm_center_hz(&self) -> f32;
    fn cofdm_fs_hz(&self) -> f32;
    fn set_cofdm_center_hz(&mut self, hz: f32);
}

impl CofdmSettings for super::SettingsState {
    fn cofdm_sig_secs(&self) -> f32 {
        if let Row::Num(f) = &rows(self).rows[SIGNAL] {
            f.value
        } else {
            crate::source::cofdm::COFDM_DEFAULT_SIG_SECS
        }
    }
    fn cofdm_gap_secs(&self) -> f32 {
        if let Row::Num(f) = &rows(self).rows[GAP] {
            f.value
        } else {
            crate::source::cofdm::COFDM_DEFAULT_GAP_SECS
        }
    }
    fn cofdm_cn_db(&self) -> f32 {
        if let Row::Num(f) = &rows(self).rows[CN] {
            f.value
        } else {
            crate::source::cofdm::COFDM_DEFAULT_CN_DB
        }
    }
    fn cofdm_bw_fraction(&self) -> CofdmBwFraction {
        rows(self).bw_fraction()
    }
    fn cofdm_center_hz(&self) -> f32 {
        rows(self).center_hz()
    }
    fn cofdm_fs_hz(&self) -> f32 {
        rows(self).fs_hz
    }
    /// Write a new band centre (the `L` key's source-lock).  Clamped by the row
    /// to what fits, and followed by the same edge-guard re-bound a manual nudge
    /// gets — the two paths must not diverge, or locking to a viewport centre
    /// would leave the guard row describing a band the source cannot render.
    fn set_cofdm_center_hz(&mut self, hz: f32) {
        let r = rows_mut(self);
        if let Row::Num(f) = &mut r.rows[CENTER] {
            f.value = hz.clamp(f.min, f.max);
        }
        r.reseed_edge_guard_bounds();
    }
    fn cofdm_shaping(&self) -> CofdmShaping {
        let r = rows(self);
        let fraction = r.bw_fraction();
        let edge_guard = match &r.rows[EDGE_GUARD] {
            Row::Num(f) => f.value.round().max(0.0) as usize,
            _ => cofdm_edge_guard_for(fraction),
        };
        CofdmShaping {
            enabled: r.shaping_enabled(),
            edge_guard,
            include_dc: toggle_bool(&r.rows, INCLUDE_DC, false),
            taper: CofdmTaper::ALL
                .get(toggle_index(&r.rows, TAPER, 0))
                .copied()
                .unwrap_or(COFDM_DEFAULT_TAPER),
            mask: CofdmMask::ALL
                .get(toggle_index(&r.rows, MASK, 0))
                .copied()
                .unwrap_or(COFDM_DEFAULT_MASK),
        }
    }
}
