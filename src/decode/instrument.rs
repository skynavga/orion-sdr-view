// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! COFDM instrumentation: the metric model, the panel layout, and the value
//! formatting.
//!
//! All of it lives in the library rather than in `src/app/` because `src/app/`
//! is bin-only (`src/lib.rs` exports `config`/`decode`/`source`/`utils`), so
//! anything placed there is unreachable from the integration tests.  The binary
//! side is reduced to placing and painting the cells this module produces.
//!
//! # Provenance
//!
//! Every metric carries a [`Provenance`] so the renderer never learns where a
//! number came from.  Two providers fill this model: a live COFDM receiver
//! ([`Provenance::Measured`]), and a simulation used when the source offers no
//! complex baseband to demodulate ([`Provenance::Simulated`] — rendered dim,
//! behind a `SIM` badge).  Swapping between them is a change on the provider
//! side alone; the renderer, the layout and the badge all follow without
//! knowing.  That the badge *disappears on its own* under a receiver is
//! asserted in `tests/instrument.rs`, not arranged by hand.
//!
//! # Inner vs outer FEC
//!
//! COFDM here is a concatenated scheme: an inner soft-decision code inside an
//! outer hard-decision block code.  **Every FEC-derived field in this module
//! refers to the inner code**, and the error metrics form a ladder, each rung
//! named for the stage whose *output* it measures:
//!
//! | Field | Measured at the output of |
//! | --- | --- |
//! | [`CofdmInstrument::cber`] | the channel (i.e. before the inner decoder) |
//! | [`CofdmInstrument::iber`] | the inner decoder, before the outer |
//! | [`CofdmInstrument::error_rate`] | the whole chain, at frame/packet granularity |
//!
//! A future outer-decoder BER slots in as an `OBER` rung between `iber` and
//! `error_rate` without renaming anything.  The rungs are deliberately **not**
//! called `pre_fec_ber`/`post_fec_ber`, which read as if there were a single
//! FEC stage, nor `vber` — DVB's spelling names the Viterbi *algorithm*, which
//! applies to at most a convolutional inner code and says nothing about an LDPC
//! one.  For the same reason no label here names a decoding algorithm or a
//! code family.

use std::fmt::Write as _;

// ── Provenance-tagged metrics ─────────────────────────────────────────────────

/// Where a metric's value came from.
///
/// `Measured` and `Known` both render as authoritative — the operationally
/// important split is *"came from the signal"* versus *"was asserted"*.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Provenance {
    /// Derived from the received signal.
    Measured,
    /// Declared by the source, or recovered from signalling (a frame header for
    /// generic COFDM, TPS for DVB-T).  True, but not verified against the air.
    Known,
    /// Placeholder pending a demodulator.
    Simulated,
    /// No provider for this field.
    ///
    /// The simulation fills every field it models, so this is reached only
    /// under a receiver — for the things nothing can actually measure here: a
    /// sample-clock error (no estimator), a delay spread (the band-limited
    /// channel estimate's spread is an occupancy artifact, not a channel
    /// reading) and a transport-stream lock (no such layer for generic COFDM).
    Unavailable,
}

/// A single instrument reading.
///
/// **`value` and `prov` both survive serialization, deliberately.**  A dump that
/// flattened this to a bare number would reintroduce the bug the `Option`
/// exists to prevent: the BER rungs go `None` exactly when the link fails, so
/// writing that as `0.0` inverts its meaning — a dead link would read as a
/// perfect one.  `null` and `0.0` must stay distinguishable in the file, and a
/// simulated placeholder must never be mistaken for a measurement.
#[derive(Clone, Copy, PartialEq, Debug, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub struct Metric<T> {
    #[serde(rename = "v")]
    pub value: Option<T>,
    pub prov: Provenance,
}

impl<T> Metric<T> {
    pub fn measured(v: T) -> Self {
        Self {
            value: Some(v),
            prov: Provenance::Measured,
        }
    }

    pub fn known(v: T) -> Self {
        Self {
            value: Some(v),
            prov: Provenance::Known,
        }
    }

    pub fn simulated(v: T) -> Self {
        Self {
            value: Some(v),
            prov: Provenance::Simulated,
        }
    }

    /// See [`Provenance::Unavailable`].
    pub fn unavailable() -> Self {
        Self {
            value: None,
            prov: Provenance::Unavailable,
        }
    }

    /// True when this reading is a placeholder rather than a real reading.
    /// Drives the panel's `SIM` badge.
    pub fn is_simulated(&self) -> bool {
        self.prov == Provenance::Simulated
    }
}

/// The granularity at which the whole-chain error metrics are counted.
///
/// Generic COFDM has no packet concept — its unit is the frame.  DVB-T is
/// genuinely packet-oriented (188-byte TS packets under `RS(204,188)`).  Only
/// the *rate* label carries the unit; the count is always `err`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ErrorUnit {
    #[default]
    Frame,
    /// Set by a DVB-T provider; the synthetic source is frame-oriented, so
    /// nothing selects this yet.
    Packet,
}

impl ErrorUnit {
    /// The error-*rate* label: `FER` for frames, `PER` for packets.  Both are
    /// three characters, so a profile switch cannot reflow the grid.
    pub fn rate_label(self) -> &'static str {
        match self {
            ErrorUnit::Frame => "FER",
            ErrorUnit::Packet => "PER",
        }
    }
}

/// The full COFDM instrument reading.
#[derive(Clone, Debug, serde::Serialize)]
pub struct CofdmInstrument {
    // Tuning
    pub center_hz: Metric<f32>,
    pub bandwidth_hz: Metric<f32>,
    pub freq_error_hz: Metric<f32>,
    pub clock_error_ppm: Metric<f32>,
    // RF level
    pub level_dbfs: Metric<f32>,
    pub peak_dbfs: Metric<f32>,
    pub overload: Metric<bool>,
    // Quality
    pub cn_db: Metric<f32>,
    pub mer_db: Metric<f32>,
    pub evm_pct: Metric<f32>,
    pub mer_margin_db: Metric<f32>,
    // Errors — the inner-FEC ladder; see the module docs.
    /// BER at the channel's output, before the inner decoder.  Renders `CBER`.
    pub cber: Metric<f32>,
    /// BER at the inner decoder's output, before the outer.  Renders `IBER`.
    pub iber: Metric<f32>,
    /// Whole-chain error rate at frame/packet granularity.
    pub error_rate: Metric<f32>,
    /// Frames received intact.  Labelled `frm`, and the denominator the `err`
    /// count is read against — an error total means little without it.
    pub frame_count: Metric<u32>,
    /// True once `frame_count` has rolled through [`ERROR_COUNT_WRAP`].
    pub frame_count_wrapped: bool,
    /// Whole-chain error count.  Always labelled `err`, whatever the unit.
    pub error_count: Metric<u32>,
    /// True once `error_count` has rolled through [`ERROR_COUNT_WRAP`].
    pub error_count_wrapped: bool,
    pub error_unit: ErrorUnit,
    // Channel
    pub delay_spread_us: Metric<f32>,
    pub echo_within_guard: Metric<bool>,
    // Config — all *inner*-code properties where FEC is involved.
    pub constellation: Metric<String>,
    pub n_fft: Metric<usize>,
    pub guard_interval: Metric<String>,
    /// The **inner** code rate.  The outer code's overhead is not folded in.
    pub code_rate: Metric<String>,
    // Demod / service
    pub carrier_lock: Metric<bool>,
    pub timing_lock: Metric<bool>,
    /// The **inner** decoder converging.  Says nothing about the outer decoder.
    pub fec_lock: Metric<bool>,
    pub ts_lock: Metric<bool>,
    /// Rate after the inner decoder, matching what `code_rate` advertises.
    pub bitrate_bps: Metric<f64>,
}

// ── Value formatting ──────────────────────────────────────────────────────────

/// Rendered for a metric with no value.
pub const ABSENT: &str = "\u{2014}"; // em-dash

pub fn fmt_freq_khz(hz: f32) -> String {
    format!("{:.3} kHz", hz / 1000.0)
}

pub fn fmt_bw_khz(hz: f32) -> String {
    format!("{:.1} kHz", hz / 1000.0)
}

/// Signed Hz, e.g. `+123 Hz` / `-40 Hz` / `+0 Hz`.
pub fn fmt_signed_hz(hz: f32) -> String {
    format!("{:+.0} Hz", hz)
}

pub fn fmt_ppm(ppm: f32) -> String {
    format!("{:+.1} ppm", ppm)
}

pub fn fmt_dbfs(db: f32) -> String {
    format!("{:.1} dBFS", db)
}

pub fn fmt_db(db: f32) -> String {
    format!("{:.1} dB", db)
}

/// Signed dB, for the MER margin.
pub fn fmt_signed_db(db: f32) -> String {
    format!("{:+.1} dB", db)
}

pub fn fmt_pct(pct: f32) -> String {
    format!("{:.1} %", pct)
}

pub fn fmt_us(us: f32) -> String {
    format!("{:.1} \u{b5}s", us)
}

/// Largest error count the display distinguishes; the counter wraps here.
pub const ERROR_COUNT_WRAP: u32 = 1_000_000;

/// Zero-padded fixed-width error count, so an increment cannot reflow the grid.
///
/// `wrapped` marks that the counter has rolled through
/// [`ERROR_COUNT_WRAP`] at least once — `000042` and `000042+` are very
/// different readings, and without the marker a wrapped counter silently
/// under-reports by a million.  The marker slot is always present (a space when
/// it has not wrapped) so the field width never changes.
pub fn fmt_count(n: u32, wrapped: bool) -> String {
    format!(
        "{:06}{}",
        n % ERROR_COUNT_WRAP,
        if wrapped { '+' } else { ' ' }
    )
}

pub fn fmt_bitrate(bps: f64) -> String {
    if bps >= 1.0e6 {
        format!("{:.2} Mb/s", bps / 1.0e6)
    } else {
        format!("{:.1} kb/s", bps / 1.0e3)
    }
}

/// Smallest bit error rate the display distinguishes from zero.  Below this a
/// measurement would need more frames than the window holds, so reporting a
/// value would be false precision.
pub const BER_FLOOR: f32 = 1.0e-9;

/// Bit error rate in exponent form: `2.1E-4`, `0.0E0` for exactly zero, and
/// `<1E-9` for anything below [`BER_FLOOR`].
pub fn fmt_ber(v: f32) -> String {
    if !v.is_finite() || v <= 0.0 {
        return "0.0E0".to_owned();
    }
    if v < BER_FLOOR {
        return "<1E-9".to_owned();
    }
    let mut exp = v.log10().floor() as i32;
    let mut mant = v / 10f32.powi(exp);
    // Rounding the mantissa to one place can carry it to 10.0; renormalise so
    // the rendered width stays bounded.
    if (mant * 10.0).round() / 10.0 >= 10.0 {
        mant /= 10.0;
        exp += 1;
    }
    format!("{mant:.1}E{exp}")
}

/// FFT size as a *mode* label — `2K` / `8K` for the DVB-style sizes, the raw
/// count otherwise.  Keeping this a label rather than a bin count is what lets
/// the field survive a move from the synthetic source's 256-point plan to
/// DVB-T's 2048-point one without a UI change.
pub fn fmt_fft_mode(n_fft: usize) -> String {
    match n_fft {
        2048 => "2K".to_owned(),
        4096 => "4K".to_owned(),
        8192 => "8K".to_owned(),
        n if n >= 1024 && n.is_multiple_of(1024) => format!("{}K", n / 1024),
        n => n.to_string(),
    }
}

/// A reduced fraction label, e.g. `1/2`, `1/8`.  Used for both the guard
/// interval (`cp_len`/`n_fft`) and the inner code rate (`k`/`n`).
pub fn fmt_fraction(num: usize, den: usize) -> String {
    if den == 0 {
        return ABSENT.to_owned();
    }
    let g = gcd(num, den);
    format!("{}/{}", num / g, den / g)
}

fn gcd(a: usize, b: usize) -> usize {
    if b == 0 { a.max(1) } else { gcd(b, a % b) }
}

pub fn fmt_yes_no(v: bool) -> String {
    // Not a lock dot: the lock dots read "● = good", and an overload dot would
    // have to read "● = bad" in the same panel.
    if v { "YES" } else { "no" }.to_owned()
}

/// Echo state relative to the guard interval.
pub fn fmt_echo(within_guard: bool) -> String {
    if within_guard { "OK" } else { "OVER" }.to_owned()
}

// ── Panel layout ──────────────────────────────────────────────────────────────

/// Columns in the instrumentation panel: a section name, then four
/// label/value pairs.
pub const COLUMNS: usize = 9;

/// The column at which the `Demod` row's lock indicators begin.  Everything
/// from here to the right edge is one merged span.
///
/// Column 1/2 hold `BR` and 3/4 hold `frm`, so the locks start at 5.
pub const MERGED_FROM: usize = 5;

/// Padding added to every column's widest content, in character widths.  The
/// padding absorbs the gutter, so there is no separate inter-column spacing.
pub const COLUMN_PAD: f32 = 2.0;

/// How a cell should be presented.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CellStyle {
    /// Authoritative — `Measured` or `Known`.
    Normal,
    /// `Simulated` — render dim.
    Simulated,
    /// `Unavailable` — the em-dash.
    Absent,
}

impl CellStyle {
    fn for_prov(prov: Provenance, has_value: bool) -> Self {
        match prov {
            _ if !has_value => CellStyle::Absent,
            Provenance::Simulated => CellStyle::Simulated,
            Provenance::Unavailable => CellStyle::Absent,
            Provenance::Measured | Provenance::Known => CellStyle::Normal,
        }
    }
}

/// One grid cell: a label or a value.  Never a `label value` pair packed
/// together — packing aligns the labels but leaves the values ragged.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Cell {
    pub text: String,
    pub style: CellStyle,
}

/// A lock indicator in the `Demod` row's merged span.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Lock {
    pub label: &'static str,
    /// `●` locked, `○` unlocked, em-dash when unavailable.
    pub glyph: &'static str,
    pub style: CellStyle,
}

/// One panel row.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Row {
    /// Column 0.
    pub section: &'static str,
    /// Alternating label/value cells starting at column 1.  Rows are
    /// ragged-right: a row with fewer than four pairs simply ends early.
    pub cells: Vec<Cell>,
    /// Lock indicators occupying the merged span from [`MERGED_FROM`].  Empty
    /// for every row but `Demod`.
    pub locks: Vec<Lock>,
}

fn label(text: &str) -> Cell {
    Cell {
        text: text.to_owned(),
        style: CellStyle::Normal,
    }
}

/// Render a metric into a value cell, or the em-dash when it has no value.
fn value<T>(m: &Metric<T>, f: impl FnOnce(&T) -> String) -> Cell {
    let style = CellStyle::for_prov(m.prov, m.value.is_some());
    Cell {
        text: match &m.value {
            Some(v) if style != CellStyle::Absent => f(v),
            _ => ABSENT.to_owned(),
        },
        style,
    }
}

/// A label/value pair.
fn pair<T>(name: &str, m: &Metric<T>, f: impl FnOnce(&T) -> String) -> [Cell; 2] {
    [label(name), value(m, f)]
}

fn lock(name: &'static str, m: &Metric<bool>) -> Lock {
    let style = CellStyle::for_prov(m.prov, m.value.is_some());
    Lock {
        label: name,
        glyph: match m.value {
            _ if style == CellStyle::Absent => ABSENT,
            Some(true) => "\u{25cf}",  // ●
            Some(false) => "\u{25cb}", // ○
            None => ABSENT,
        },
        style,
    }
}

impl CofdmInstrument {
    /// The panel's rows, in display order.
    pub fn panel_rows(&self) -> Vec<Row> {
        let grid = |section, cells: Vec<[Cell; 2]>| Row {
            section,
            cells: cells.into_iter().flatten().collect(),
            locks: Vec::new(),
        };
        vec![
            grid(
                "Tuning",
                vec![
                    pair("ctr", &self.center_hz, |v| fmt_freq_khz(*v)),
                    pair("bw", &self.bandwidth_hz, |v| fmt_bw_khz(*v)),
                    pair("\u{394}f", &self.freq_error_hz, |v| fmt_signed_hz(*v)),
                    pair("clk", &self.clock_error_ppm, |v| fmt_ppm(*v)),
                ],
            ),
            grid(
                "RF",
                vec![
                    pair("lvl", &self.level_dbfs, |v| fmt_dbfs(*v)),
                    pair("pk", &self.peak_dbfs, |v| fmt_dbfs(*v)),
                    pair("OVL", &self.overload, |v| fmt_yes_no(*v)),
                ],
            ),
            grid(
                "Quality",
                vec![
                    pair("C/N", &self.cn_db, |v| fmt_db(*v)),
                    pair("MER", &self.mer_db, |v| fmt_db(*v)),
                    pair("EVM", &self.evm_pct, |v| fmt_pct(*v)),
                    pair("margin", &self.mer_margin_db, |v| fmt_signed_db(*v)),
                ],
            ),
            grid(
                "Errors",
                vec![
                    pair("CBER", &self.cber, |v| fmt_ber(*v)),
                    pair("IBER", &self.iber, |v| fmt_ber(*v)),
                    pair(self.error_unit.rate_label(), &self.error_rate, |v| {
                        fmt_ber(*v)
                    }),
                    pair("err", &self.error_count, |v| {
                        fmt_count(*v, self.error_count_wrapped)
                    }),
                ],
            ),
            grid(
                "Channel",
                vec![
                    pair("\u{394}t", &self.delay_spread_us, |v| fmt_us(*v)),
                    pair("echo", &self.echo_within_guard, |v| fmt_echo(*v)),
                ],
            ),
            grid(
                "Config",
                vec![
                    pair("mod", &self.constellation, |v| v.clone()),
                    pair("FFT", &self.n_fft, |v| fmt_fft_mode(*v)),
                    pair("GI", &self.guard_interval, |v| v.clone()),
                    pair("CR", &self.code_rate, |v| v.clone()),
                ],
            ),
            Row {
                section: "Demod",
                cells: pair("BR", &self.bitrate_bps, |v| fmt_bitrate(*v))
                    .into_iter()
                    .chain(pair("frm", &self.frame_count, |v| {
                        fmt_count(*v, self.frame_count_wrapped)
                    }))
                    .collect(),
                locks: vec![
                    lock("CAR", &self.carrier_lock),
                    lock("TIM", &self.timing_lock),
                    lock("FEC", &self.fec_lock),
                    lock("TS", &self.ts_lock),
                ],
            },
        ]
    }

    /// True when any panel value is a placeholder — drives the `SIM` badge.
    pub fn any_simulated(&self) -> bool {
        self.panel_rows().iter().any(|r| {
            r.cells.iter().any(|c| c.style == CellStyle::Simulated)
                || r.locks.iter().any(|l| l.style == CellStyle::Simulated)
        })
    }
}

/// The rendered text of a `Demod` row's merged span, e.g. `CAR ●  TIM ●  FEC ●  TS ○`.
pub fn merged_span_text(locks: &[Lock]) -> String {
    locks
        .iter()
        .map(|l| format!("{} {}", l.label, l.glyph))
        .collect::<Vec<_>>()
        .join("  ")
}

/// Resolve each column's width to its own widest content plus [`COLUMN_PAD`].
///
/// `measure` returns the rendered width of a string in the caller's units — the
/// binary passes a glyph measurer (`painter.layout_no_wrap(..).size().x`), since
/// the panel uses `Δ`, `µ`, `—` and `●`, which are multi-byte in UTF-8 and not
/// reliably one advance wide.  Tests pass a character counter.
///
/// The `Demod` row's merged span is deliberately excluded: it starts at the
/// [`MERGED_FROM`] origin and runs to the right edge, so it must not widen the
/// columns the rows above it align to.
pub fn column_widths(rows: &[Row], mut measure: impl FnMut(&str) -> f32) -> [f32; COLUMNS] {
    let mut w = [0.0_f32; COLUMNS];
    for row in rows {
        w[0] = w[0].max(measure(row.section));
        for (i, cell) in row.cells.iter().enumerate() {
            let col = 1 + i;
            if col < COLUMNS {
                w[col] = w[col].max(measure(&cell.text));
            }
        }
    }
    for v in &mut w {
        *v += COLUMN_PAD * measure("M");
    }
    w
}

/// Left edge of each column, as a running sum of [`column_widths`].
pub fn column_origins(widths: &[f32; COLUMNS]) -> [f32; COLUMNS] {
    let mut x = [0.0_f32; COLUMNS];
    for i in 1..COLUMNS {
        x[i] = x[i - 1] + widths[i - 1];
    }
    x
}

/// Render the panel as monospace text, one `String` per row.
///
/// The binary paints cells at measured pixel offsets rather than calling this,
/// but the layout is only assertable through it: `src/app/` is bin-only, so a
/// test cannot reach the painting side at all.
pub fn render_text(rows: &[Row]) -> Vec<String> {
    let widths = reference_column_widths(|s| s.chars().count() as f32);
    let pad = |out: &mut String, text: &str, col: usize| {
        let _ = write!(out, "{text}");
        let fill = widths[col] as usize - text.chars().count();
        out.push_str(&" ".repeat(fill));
    };
    rows.iter()
        .map(|row| {
            let mut line = String::new();
            pad(&mut line, row.section, 0);
            for (i, cell) in row.cells.iter().enumerate() {
                pad(&mut line, &cell.text, 1 + i);
            }
            if !row.locks.is_empty() {
                // Pad out to the merged span's origin, then run to the edge.
                let origins = column_origins(&widths);
                let target = origins[MERGED_FROM] as usize;
                if line.chars().count() < target {
                    line.push_str(&" ".repeat(target - line.chars().count()));
                }
                line.push_str(&merged_span_text(&row.locks));
            }
            line.trim_end().to_owned()
        })
        .collect()
}

// ── Di bar ────────────────────────────────────────────────────────────────────

/// The `SIM` badge appended when any rendered field is a placeholder.
pub const SIM_BADGE: &str = "SIM";

/// Fixed body widths for the Di line's fields, chosen so no reachable value
/// changes a field's rendered width.  See [`CofdmInstrument::di_bar_str`].
const DB_W: usize = 5; // "-99.9" / "100.0"
const BER_W: usize = 6; // "1.0E-4"; "<1E-9" and "0.0E0" are shorter
const HZ_W: usize = 8; // "-9999 Hz"
const DBFS_W: usize = 11; // "-100.0 dBFS"

/// One Di-bar field: its padded rendering, whether it is a placeholder, and
/// whether it is pinned to the end of the line.
struct DiField {
    text: String,
    sim: bool,
    /// Rendered last, just before the `SIM` badge, regardless of where it sits
    /// in the drop priority.  Only the lock run uses this.
    at_end: bool,
}

impl CofdmInstrument {
    /// The prioritised one-line Di-bar summary, fitted to `budget_chars`.
    ///
    /// When the budget is short, the **lowest-priority fields are dropped**
    /// rather than the line being clipped or scrolled.  `CBER` outranks `IBER`
    /// (which is not carried at all here): the channel BER moves continuously
    /// with C/N, whereas the post-inner-FEC rate sits pinned at the floor until
    /// the code is close to giving up, so it is the less useful single number.
    /// The `frm`/`err` counters shown left of the Di bar's loop timer, matching
    /// the FT8/FT4 field exactly: three digits each, fixed width, trailing
    /// space, so the layout cannot shift as the counts change.
    ///
    /// Three digits means these wrap at 1000 while the panel's own `frm`/`err`
    /// wrap at [`ERROR_COUNT_WRAP`] — the same counts, rendered at the
    /// precision each surface has room for. Returns `None` when there is no
    /// receiver, since the simulation has no frame tally to show.
    pub fn di_counter_str(&self) -> Option<String> {
        let frames = self.frame_count.value?;
        let errors = self.error_count.value.unwrap_or(0);
        Some(format!(
            "frm {:03} err {:03} ",
            frames % 1000,
            errors % 1000
        ))
    }

    pub fn di_bar_str(&self, budget_chars: usize) -> String {
        let mut line = String::from("COFDM");
        if let Some(c) = self.center_hz.value {
            let _ = write!(line, " {:.1}kHz", c / 1000.0);
        }
        if let Some(b) = self.bandwidth_hz.value {
            let _ = write!(line, " {:.0}kHz", b / 1000.0);
        }

        // Every field is padded to a fixed width, and the fit is decided on
        // that width rather than the current rendering.  Otherwise a value
        // changing width shifts everything after it — CBER crossing the `<1E-9`
        // floor is one character, and that alone makes the whole tail of the
        // line twitch a few times a second.  It can also push the last field in
        // and out of the budget, so fields blink rather than merely shift.
        fn field(label: &str, body: Option<String>, body_w: usize, sim: bool) -> Option<DiField> {
            body.map(|body| DiField {
                text: format!("{label} {body:<body_w$}"),
                sim,
                at_end: false,
            })
        }
        let mut fields: Vec<DiField> = Vec::new();
        fields.extend(field(
            "C/N",
            self.cn_db.value.map(|v| format!("{v:.1}")),
            DB_W,
            self.cn_db.is_simulated(),
        ));
        fields.extend(field(
            "MER",
            self.mer_db.value.map(|v| format!("{v:.1}")),
            DB_W,
            self.mer_db.is_simulated(),
        ));
        fields.extend(field(
            "CBER",
            self.cber.value.map(fmt_ber),
            BER_W,
            self.cber.is_simulated(),
        ));
        let locks = [
            &self.carrier_lock,
            &self.timing_lock,
            &self.fec_lock,
            &self.ts_lock,
        ];
        let dots: String = locks
            .iter()
            .filter_map(|m| m.value.map(|v| if v { '\u{25cf}' } else { '\u{25cb}' }))
            .collect();
        if !dots.is_empty() {
            // Pinned to the end of the line but kept at this rank in the drop
            // priority: the lock run is the most compressed health indicator on
            // the bar, so it should outlive the level and frequency-error
            // readouts when the window narrows, not be the first thing to go.
            fields.push(DiField {
                text: format!("lck {dots}"),
                sim: locks.iter().any(|m| m.is_simulated()),
                at_end: true,
            });
        }
        fields.extend(field(
            "\u{394}f",
            self.freq_error_hz.value.map(fmt_signed_hz),
            HZ_W,
            self.freq_error_hz.is_simulated(),
        ));
        fields.extend(field(
            "lvl",
            self.level_dbfs.value.map(fmt_dbfs),
            DBFS_W,
            self.level_dbfs.is_simulated(),
        ));

        // Greedily take fields in priority order.  The badge is appended once at
        // the end, so its width must be reserved on *every* step from the first
        // simulated field onward — reserving it only when that field is
        // admitted lets the fields after it spend the room it needs.
        let badge_w = 2 + SIM_BADGE.chars().count();
        let mut width = line.chars().count();
        let mut any_sim = false;
        let mut admitted: Vec<DiField> = Vec::new();
        for f in fields {
            let reserve = if any_sim || f.sim { badge_w } else { 0 };
            if width + 2 + f.text.chars().count() + reserve > budget_chars {
                break;
            }
            width += 2 + f.text.chars().count();
            any_sim |= f.sim;
            admitted.push(f);
        }

        // Render the pinned fields last, so the lock run always sits directly
        // before the badge however many fields survived ahead of it.
        let mut out = line;
        for f in admitted.iter().filter(|f| !f.at_end) {
            let _ = write!(out, "  {}", f.text);
        }
        for f in admitted.iter().filter(|f| f.at_end) {
            let _ = write!(out, "  {}", f.text);
        }
        if any_sim {
            let _ = write!(out, "  {SIM_BADGE}");
        }
        out.trim_end().to_owned()
    }
}

// ── Provider input ────────────────────────────────────────────────────────────

/// What a live receiver contributes, when one is running.
///
/// Held apart from the rest of [`CofdmFacts`] so the two providers stay
/// distinguishable: `None` is the simulation, `Some` is measurement, and
/// [`CofdmInstrument::from_facts`] is the one place that chooses. Every field
/// is itself optional because upstream reports "the stage that would produce
/// this did not run" as `None` rather than as a sentinel — and for the BER
/// rungs that is load-bearing, since they are measured by re-encoding a
/// *decoded* frame and so go `None` exactly when the link fails. Rendering
/// that as `0.0` would invert its meaning.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CofdmRxFacts {
    pub sync_score: Option<f32>,
    pub cfo_hz: Option<f32>,
    pub evm_db: Option<f32>,
    pub channel_ber: Option<f32>,
    pub inner_ber: Option<f32>,
    pub inner_fec_ok: Option<bool>,
    pub outer_fec_ok: Option<bool>,
    /// Frames that failed or never arrived, over frames expected.
    pub frame_error_rate: Option<f32>,
    /// Frames counted bad in this burst: decode failures plus unexplained gaps.
    pub error_count: u32,
    pub error_count_wrapped: bool,
    /// Frames received intact in this burst.
    pub frame_count: u32,
    pub frame_count_wrapped: bool,
}

/// Minimum sync score treated as a carrier/timing lock.
///
/// `ofdm_sync`'s own acceptance threshold, so "locked" means exactly "the
/// receiver was willing to decode from this candidate" rather than a second,
/// differently-calibrated opinion about the same thing.
const RX_LOCK_SCORE: f32 = 0.5;

/// Everything the viewer can genuinely measure or declare about the COFDM
/// signal today.  [`CofdmInstrument::from_facts`] fills the rest by simulation.
///
/// Deliberately carries *numerology*, not `orion-sdr` types: the provider
/// resolves the carrier plan and MCS, and this module derives both the labels
/// and the numbers from them.  That keeps the FEC types out of the render path
/// and keeps this module testable without building a modulator.
#[derive(Clone, Debug)]
pub struct CofdmFacts {
    // Tuning / RF — measured.
    pub center_hz: f32,
    pub bandwidth_hz: f32,
    /// Block RMS and peak as **raw amplitudes**, converted against
    /// `full_scale` below.  Passing amplitudes rather than dBFS keeps the
    /// full-scale reference in one place.
    pub level_amp: f32,
    pub peak_amp: f32,
    pub cn_db: f32,
    // Numerology — known from the carrier plan and MCS.
    pub fs: f32,
    pub n_fft: usize,
    pub cp_len: usize,
    pub data_carriers: usize,
    pub constellation: &'static str,
    pub bits_per_symbol: usize,
    /// The **inner** code rate as `(k, n)`.  The outer code is not folded in.
    pub inner_code_rate: (usize, usize),
    /// Accumulated by the provider across emits, so it survives the per-block
    /// rebuild of the instrument.
    pub error_count: u32,
    pub error_count_wrapped: bool,
    pub error_unit: ErrorUnit,
    /// Measurements from a live receiver; `None` keeps the simulated block.
    pub rx: Option<CofdmRxFacts>,
    /// Amplitude that counts as 0 dBFS for this source.
    ///
    /// **Not 1.0.**  The COFDM source applies a large fixed modulator gain
    /// because bare OFDM at unit gain sits below the decoder's signal
    /// threshold, and the viewer's f32 spectrum pipeline has no `[-1, 1]`
    /// clamp — so raw samples routinely peak above 30.  Measuring dBFS against
    /// 1.0 would report *positive* dBFS and a permanent overload.  The gain is
    /// a display-scaling artifact, so it is the full-scale reference.
    pub full_scale: f32,
}

// ── Simulation ────────────────────────────────────────────────────────────────
//
// This is a **display harness, not a measurement**.  Everything below is
// `Provenance::Simulated`, renders dim, and raises the `SIM` badge.  The models
// have the right *shape* — a proper error-function waterfall, a coding gain,
// locks that drop at a threshold — so the layout is exercised against realistic
// ranges, signs and update rates, but none of it is a reading.
//
// The values are driven from the real, measured `cn_db` rather than frozen at
// constants, so changing `Noise amp` or `Shaping` in the settings moves the
// whole panel.  That is what actually exercises the layout.

/// C/N offset applied before the BER waterfall, in dB.
///
/// An honest QPSK error function evaluated at the source's raw C/N is flat zero
/// everywhere — the synthetic signal is far too clean — so the curve is shifted
/// to bring its knee into the reachable range.  Where the knee sits is the
/// whole design of this constant, and it is set by the measured C/N envelope:
///
/// | Fraction | noise 0.01 | 0.05 (default) | 0.2 | 0.5 |
/// | --- | --- | --- | --- | --- |
/// | 1/8 | 41.1 | 35.7 | 26.4 | 19.6 |
/// | 1/4 | 38.1 | 34.6 | 26.2 | 19.6 |
/// | 1/2 | 28.9 | 28.3 | 24.1 | 18.8 |
/// | 7/8 | 18.0 | 17.9 | 17.7 | 15.8 |
///
/// **Bandwidth moves C/N further than the noise knob does** — 17.8 dB across
/// the fractions at default noise, against 2.2 dB across the whole noise range
/// at 7/8.  So an offset placing the knee mid-range splits the *bandwidth*
/// axis: narrow fractions read healthy and wide ones read as a failing link,
/// with locks dropping, on otherwise-default settings.  That is a simulation
/// artifact and it looks exactly like a fault.
///
/// 12 dB puts the knee near C/N 14, below the whole default-noise envelope, so
/// every bandwidth reads as a healthy link at defaults and the noise setting is
/// what degrades it.  Locks still key off real thresholds; the source simply
/// never gets bad enough to trip them, which is the honest outcome for a
/// QPSK-1/2 link at 16 dB C/N.  A real receiver replaces this whole block.
const SIM_WATERFALL_OFFSET_DB: f32 = 12.0;

/// Implementation loss between C/N and MER, in dB.
const SIM_IMPL_LOSS_DB: f32 = 1.5;

/// Minimum MER for a usable QPSK-1/2 link, in dB — the reference for `margin`.
/// A real table would be keyed by constellation and inner code rate; one entry
/// is enough while the source transmits one MCS.
const SIM_MER_THRESHOLD_DB: f32 = 6.8;

/// `erfc(x)` for `x >= 0` — Abramowitz & Stegun 7.1.26, ~1.2e-7 relative.
/// In `f64` so the deep tail does not flush to zero before the BER floor.
fn erfc(x: f64) -> f64 {
    let t = 1.0 / (1.0 + x / 2.0);
    let poly = -1.265_512_23
        + t * (1.000_023_68
            + t * (0.374_091_96
                + t * (0.096_784_18
                    + t * (-0.186_288_06
                        + t * (0.278_868_07
                            + t * (-1.135_203_98
                                + t * (1.488_515_87 + t * (-0.822_152_23 + t * 0.170_872_77))))))));
    t * (-x * x + poly).exp()
}

/// Uncoded QPSK bit error rate at a given Eb/N0, in dB.
fn qpsk_ber(ebn0_db: f32) -> f32 {
    let lin = 10f64.powf(ebn0_db as f64 / 10.0);
    sim_floor((0.5 * erfc(lin.sqrt())) as f32)
}

/// Amplitude to dBFS against `full_scale`, floored so silence does not render
/// as `-inf`.
fn db_fs(amp: f32, full_scale: f32) -> f32 {
    let fs = if full_scale > 0.0 { full_scale } else { 1.0 };
    let ratio = amp / fs;
    if ratio <= 1.0e-6 {
        -120.0
    } else {
        20.0 * ratio.log10()
    }
}

/// Keep a simulated error rate strictly positive so it renders as `<1E-9`
/// rather than `0.0E0`.
///
/// Exactly zero is a legitimate *measurement* — no errors in the window — but a
/// simulation observes nothing, so it cannot claim it.  The deep tail of the
/// error function underflows to zero in `f32`, which would otherwise put the
/// stronger claim on screen for the cleanest links.
fn sim_floor(v: f32) -> f32 {
    v.max(f32::MIN_POSITIVE)
}

impl CofdmInstrument {
    /// Build the instrument: the facts as measured or known, everything else
    /// simulated from the measured C/N.  See the simulation notes above.
    pub fn from_facts(f: &CofdmFacts) -> Self {
        let (k, n) = f.inner_code_rate;
        let rate = if n == 0 { 0.5 } else { k as f32 / n as f32 };

        // Symbol rate = fs / (n_fft + cp_len); the bit rate follows from the
        // *live* data-carrier count, never from anything derived from n_fft —
        // DVB-T's 2K plan carries 1512 data carriers out of 2048 bins.
        let symbol_rate = f.fs as f64 / (f.n_fft + f.cp_len) as f64;
        let bitrate = f.data_carriers as f64 * f.bits_per_symbol as f64 * rate as f64 * symbol_rate;

        let guard_us = f.cp_len as f32 / f.fs * 1.0e6;

        // ── Simulated block ──────────────────────────────────────────────
        let mer_db = f.cn_db - SIM_IMPL_LOSS_DB;
        let evm_pct = 100.0 * 10f32.powf(-mer_db / 20.0);
        let cber = qpsk_ber(f.cn_db - SIM_WATERFALL_OFFSET_DB);
        // The inner code steepens the curve rather than shifting it: a coding
        // gain that grows as the raw rate falls, which is the shape of a real
        // waterfall's knee.
        let iber = sim_floor(cber.powf(2.5).clamp(0.0, 1.0));
        // Whole-chain rate: one frame fails if any of its bits does.
        let frame_bits = (f.data_carriers * f.bits_per_symbol).max(1) as f32;
        let error_rate = sim_floor(1.0 - (1.0 - iber).powf(frame_bits));
        // Residual sync error shrinks as C/N rises.
        let freq_error_hz = 2000.0 * 10f32.powf(-f.cn_db / 20.0);
        let clock_error_ppm = 5.0 * 10f32.powf(-f.cn_db / 40.0);
        let delay_spread_us = guard_us * (0.35 + 0.9 * 10f32.powf(-f.cn_db / 20.0));

        // ── Provider split ───────────────────────────────────────────────
        //
        // The one place that chooses between measurement and simulation. Every
        // field a receiver can supply is taken from `f.rx` when one is running
        // and from the block above when none is; nothing downstream — layout,
        // formatting, the `SIM` badge — knows the difference. That was the
        // whole point of tagging provenance.
        //
        // `Metric::simulated` is reached only on the `None` arm, so
        // `any_simulated()` goes false, and the badge disappears, on its own.
        let rx = f.rx;
        let m = |v: Option<f32>| match v {
            Some(x) => Metric::measured(x),
            None => Metric::unavailable(),
        };

        Self {
            center_hz: Metric::measured(f.center_hz),
            bandwidth_hz: Metric::known(f.bandwidth_hz),
            freq_error_hz: match rx {
                Some(r) => m(r.cfo_hz),
                None => Metric::simulated(freq_error_hz),
            },
            // No sample-clock estimator exists on either provider. The
            // simulation invented one; a receiver is honest about not having it.
            clock_error_ppm: match rx {
                Some(_) => Metric::unavailable(),
                None => Metric::simulated(clock_error_ppm),
            },

            level_dbfs: Metric::measured(db_fs(f.level_amp, f.full_scale)),
            peak_dbfs: Metric::measured(db_fs(f.peak_amp, f.full_scale)),
            overload: Metric::measured(f.peak_amp >= f.full_scale),

            cn_db: Metric::measured(f.cn_db),
            // EVM is measured directly; MER is its reciprocal, so one reading
            // fills both rather than being modelled from C/N.
            mer_db: match rx {
                Some(r) => m(r.evm_db.map(|e| -e)),
                None => Metric::simulated(mer_db),
            },
            evm_pct: match rx {
                Some(r) => m(r.evm_db.map(|e| 100.0 * 10f32.powf(e / 20.0))),
                None => Metric::simulated(evm_pct),
            },
            mer_margin_db: match rx {
                Some(r) => m(r.evm_db.map(|e| -e - SIM_MER_THRESHOLD_DB)),
                None => Metric::simulated(mer_db - SIM_MER_THRESHOLD_DB),
            },

            cber: match rx {
                Some(r) => m(r.channel_ber),
                None => Metric::simulated(cber),
            },
            iber: match rx {
                Some(r) => m(r.inner_ber),
                None => Metric::simulated(iber),
            },
            error_rate: match rx {
                Some(r) => m(r.frame_error_rate),
                None => Metric::simulated(error_rate),
            },
            // The simulation models a *rate*, never a frame tally, so there is
            // nothing honest to put here without a receiver.
            frame_count: match rx {
                Some(r) => Metric::measured(r.frame_count),
                None => Metric::unavailable(),
            },
            frame_count_wrapped: rx.is_some_and(|r| r.frame_count_wrapped),
            error_count: match rx {
                Some(r) => Metric::measured(r.error_count),
                None => Metric::simulated(f.error_count),
            },
            error_count_wrapped: rx.map_or(f.error_count_wrapped, |r| r.error_count_wrapped),
            error_unit: f.error_unit,

            // Delay spread stays unavailable under a receiver: the inverse
            // transform of a band-limited channel estimate is a Dirichlet
            // kernel, so a flat channel measures a large spread that depends
            // only on the occupancy — and calibrating that floor out still left
            // a statistic that moved the *wrong way* for a small echo. A
            // reading worse than none is worse than none.
            delay_spread_us: match rx {
                Some(_) => Metric::unavailable(),
                None => Metric::simulated(delay_spread_us),
            },
            echo_within_guard: match rx {
                Some(_) => Metric::unavailable(),
                None => Metric::simulated(delay_spread_us < guard_us),
            },

            constellation: Metric::known(f.constellation.to_owned()),
            n_fft: Metric::known(f.n_fft),
            guard_interval: Metric::known(fmt_fraction(f.cp_len, f.n_fft)),
            code_rate: Metric::known(fmt_fraction(k, n)),

            // Carrier and timing lock are the same acquisition event: the
            // receiver accepted a sync candidate. Reporting one locked and the
            // other not would be inventing a distinction the receiver does not
            // draw.
            carrier_lock: match rx {
                Some(r) => match r.sync_score {
                    Some(s) => Metric::measured(s >= RX_LOCK_SCORE),
                    None => Metric::unavailable(),
                },
                None => Metric::simulated(f.cn_db > 6.0),
            },
            timing_lock: match rx {
                Some(r) => match r.sync_score {
                    Some(s) => Metric::measured(s >= RX_LOCK_SCORE),
                    None => Metric::unavailable(),
                },
                None => Metric::simulated(f.cn_db > 8.0),
            },
            // The *inner* decoder converging, reported by the decoder itself
            // rather than inferred from a BER threshold.
            fec_lock: match rx {
                Some(r) => match r.inner_fec_ok {
                    Some(ok) => Metric::measured(ok),
                    None => Metric::unavailable(),
                },
                None => Metric::simulated(iber < 1.0e-3),
            },
            // No transport-stream layer exists for generic COFDM.
            ts_lock: match rx {
                Some(_) => Metric::unavailable(),
                None => Metric::simulated(error_rate < 1.0e-2),
            },

            bitrate_bps: Metric::known(bitrate),
        }
    }

    /// The simulated whole-chain error rate, for a provider accumulating an
    /// error count across emits.
    pub fn simulated_error_rate(&self) -> f32 {
        self.error_rate.value.unwrap_or(0.0)
    }
}

// ── Layout stability ──────────────────────────────────────────────────────────

impl CofdmInstrument {
    /// A specimen whose every field renders at its widest plausible value.
    ///
    /// Column widths must be computed from **this**, not from the live
    /// instrument.  Sizing to current content looks tighter but makes the grid
    /// reflow whenever a value gains or loses a digit — `+75 Hz` becoming
    /// `+159 Hz` shifts every column to its right, so the panel jitters as the
    /// signal moves.  Paying a few characters of width once buys a grid that
    /// never moves, which is the whole point of aligning it.
    ///
    /// The values are chosen for *rendered width*, not realism: a centre
    /// frequency near 10 MHz, a full-scale negative level, the widest
    /// constellation name, and the error count at its maximum.
    pub fn layout_reference() -> Self {
        Self {
            center_hz: Metric::measured(9_999_900.0),
            bandwidth_hz: Metric::known(9_999_900.0),
            freq_error_hz: Metric::simulated(-9999.0),
            clock_error_ppm: Metric::simulated(-99.9),

            level_dbfs: Metric::measured(-100.0),
            peak_dbfs: Metric::measured(-100.0),
            overload: Metric::measured(true),

            cn_db: Metric::measured(-99.9),
            mer_db: Metric::simulated(-99.9),
            evm_pct: Metric::simulated(100.0),
            mer_margin_db: Metric::simulated(-99.9),

            cber: Metric::simulated(9.9e-9),
            iber: Metric::simulated(9.9e-9),
            error_rate: Metric::simulated(9.9e-9),
            frame_count: Metric::measured(ERROR_COUNT_WRAP - 1),
            frame_count_wrapped: true,
            error_count: Metric::simulated(ERROR_COUNT_WRAP - 1),
            error_count_wrapped: true,
            // `FER` and `PER` are the same width, so the unit does not matter
            // here — asserted by the tests.
            error_unit: ErrorUnit::Frame,

            delay_spread_us: Metric::simulated(9999.9),
            echo_within_guard: Metric::simulated(false),

            constellation: Metric::known("QAM256".to_owned()),
            n_fft: Metric::known(1024),
            guard_interval: Metric::known("1/32".to_owned()),
            code_rate: Metric::known("7/8".to_owned()),

            carrier_lock: Metric::simulated(false),
            timing_lock: Metric::simulated(false),
            fec_lock: Metric::simulated(false),
            ts_lock: Metric::simulated(false),

            bitrate_bps: Metric::known(999.99e6),
        }
    }
}

/// The stable column widths: [`column_widths`] over
/// [`CofdmInstrument::layout_reference`].  Every caller — the painter and the
/// text renderer alike — must size the grid with this rather than with the
/// live rows.
pub fn reference_column_widths(measure: impl FnMut(&str) -> f32) -> [f32; COLUMNS] {
    column_widths(&CofdmInstrument::layout_reference().panel_rows(), measure)
}
