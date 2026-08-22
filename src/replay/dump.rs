// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The measurement dump: one JSON object per line.
//!
//! **JSON Lines rather than CSV, because of the `Option`s.**  Every instrument
//! reading is a [`Metric<T>`](crate::decode::instrument::Metric) carrying both a
//! value that may be absent and the provenance of that value.  A CSV column
//! cannot hold `null` without a sentinel convention, and any sentinel
//! reintroduces exactly the bug the `Option` exists to prevent: `rx.rs`
//! documents that the BER rungs go `None` precisely when the link fails, so
//! serializing that as `0.0` would turn a dead link into a perfect one.
//!
//! It also streams.  A long run can be tailed while it runs, and a run that dies
//! half way still leaves a valid prefix — every complete line is a complete
//! record.
//!
//! The schema is **unversioned and unstable**, exactly as the config schema is.
//! The header record carries the tool version so a consumer can tell what it is
//! reading.

use std::io::Write;

use crate::decode::instrument::CofdmInstrument;

/// One line of the dump.
///
/// `kind` is the discriminant, so a consumer can dispatch on it without
/// positional knowledge.  Times are **scripted** seconds from the start of the
/// run — see the caveat on [`Record::t`].
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Record {
    /// First line of every dump: what produced it, and from what.
    Header {
        /// The tool version, since the schema is not stable.
        version: &'static str,
        /// The source mode at **startup**, before the script has run.  A script
        /// that switches source emits a [`Record::Source`] when it does, so the
        /// active source is always the most recent of the two.
        source: String,
        /// The startup source's sample rate — which varies by source and, for
        /// COFDM, by config, so it cannot be assumed from the mode alone.
        fs_hz: f32,
        /// The script's SHA-256, or `null` when the run was bounded by
        /// `--duration` with no script.  Two dumps are comparable only if this
        /// matches.
        script_sha256: Option<String>,
    },
    /// The active source changed.
    ///
    /// Emitted on the frame the switch takes effect, carrying the new sample
    /// rate: every downstream reading is per-source, and a COFDM `bw_hz` read
    /// against a narrowband Nyquist would be nonsense.
    Source {
        t: f32,
        samples: u64,
        source: String,
        fs_hz: f32,
    },
    /// A non-text signal summary — what the Di bar shows for AM/tone/COFDM.
    Info {
        t: f32,
        samples: u64,
        modulation: String,
        center_hz: f32,
        bw_hz: f32,
        snr_db: f32,
    },
    /// A full COFDM instrument reading, every field with its provenance.
    Instrument {
        t: f32,
        samples: u64,
        #[serde(flatten)]
        inst: Box<CofdmInstrument>,
    },
    /// Decoded text.  Emitted for the text modes; the CW and PSK31 burst
    /// delimiters and the FT8 frame stamps appear here, which is why the replay
    /// run uses a scripted clock.
    Text { t: f32, samples: u64, text: String },
    /// A signal gap.  `decoded` is meaningful for FT8/FT4 alone — true when at
    /// least one CRC-pass frame was found at this edge.
    Gap { t: f32, samples: u64, decoded: bool },
    /// The instrument was **cleared** at a gap edge.
    ///
    /// Distinct from a reading whose fields are all `null`: this says the panel
    /// fell back to em-dashes because the burst ended, not that a receiver
    /// looked and found nothing.
    InstrumentCleared { t: f32, samples: u64 },
    /// No signal detected, or the carrier was not found.
    NoSignal { t: f32, samples: u64 },
    /// Final line: how the run ended and what it cost.
    Summary {
        /// Scripted seconds covered.
        t: f32,
        frames: u64,
        samples: u64,
        /// Chunks the decoder's sequence counter says never arrived.  **Nonzero
        /// invalidates the run** — a hole breaks a streaming demodulator's
        /// framing, so the frame errors downstream of it belong to the harness
        /// rather than to the link.
        dropped_chunks: u64,
        records: u64,
    },
}

impl Record {
    /// The record's scripted time, if it has one.
    pub fn t(&self) -> Option<f32> {
        match self {
            Record::Header { .. } => None,
            Record::Source { t, .. }
            | Record::Info { t, .. }
            | Record::Instrument { t, .. }
            | Record::Text { t, .. }
            | Record::Gap { t, .. }
            | Record::InstrumentCleared { t, .. }
            | Record::NoSignal { t, .. }
            | Record::Summary { t, .. } => Some(*t),
        }
    }
}

/// A JSON Lines sink.
///
/// Write failures are **fatal and reported**, not swallowed: a headless run has
/// nobody watching it, so a dump that silently stopped half way would be
/// indistinguishable from a run that ended early.
pub struct Dump<W: Write> {
    out: W,
    records: u64,
}

impl<W: Write> Dump<W> {
    pub fn new(out: W) -> Self {
        Self { out, records: 0 }
    }

    /// Records written so far, excluding the summary.
    pub fn records(&self) -> u64 {
        self.records
    }

    pub fn write(&mut self, record: &Record) -> std::io::Result<()> {
        serde_json::to_writer(&mut self.out, record)?;
        self.out.write_all(b"\n")?;
        self.records += 1;
        Ok(())
    }

    pub fn flush(&mut self) -> std::io::Result<()> {
        self.out.flush()
    }
}

/// SHA-256 of a script's bytes, lowercase hex.
///
/// Implemented here rather than pulled in as a dependency: this is the only
/// hash the crate needs, and it exists to answer one question — "were these two
/// dumps produced from the same script?" — for which a self-contained 60 lines
/// is a better trade than a supply-chain edge.
pub fn sha256_hex(bytes: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    let mut msg = bytes.to_vec();
    let bit_len = (bytes.len() as u64) * 8;
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for block in msg.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (i, word) in w.iter_mut().take(16).enumerate() {
            *word = u32::from_be_bytes([
                block[i * 4],
                block[i * 4 + 1],
                block[i * 4 + 2],
                block[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (dst, src) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *dst = dst.wrapping_add(src);
        }
    }

    h.iter().map(|w| format!("{w:08x}")).collect()
}
