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

pub use common::{DECODE_BAR_H, SourceMode};
pub(super) use common::{
    DecodeBarMode, FFT_SIZE, MAX_SAMPLES_PER_FRAME, MIN_SAMPLES_PER_FRAME, PANE_BG, SAMPLE_RATE,
    WaterfallMode,
};
pub use view::ViewApp;
