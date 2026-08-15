// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

mod capture;
mod common;
mod draw;
mod instrument;
mod source;
mod sources;
mod view;

pub(super) mod freqview;
pub(super) mod persistence;
pub mod settings;
pub mod spectrogram;
pub(super) mod spectrum;
pub(super) mod utils;
pub mod waterfall;

pub(super) use common::{
    BAND_EDGE_COL, DecodeBarMode, FFT_SIZE, MAX_SAMPLES_PER_FRAME, MIN_SAMPLES_PER_FRAME,
    OFF_BAND_DIM, OFF_BAND_SOLID, PANE_BG, SAMPLE_RATE, WaterfallMode,
};
pub use common::{DECODE_BAR_H, SourceMode};
pub use view::ViewApp;
