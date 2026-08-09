// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

mod common;
mod draw;
mod instrument;
mod source;
mod sources;
mod view;

pub(super) mod freqview;
pub(super) mod persistence;
pub(super) mod settings;
pub(super) mod spectrogram;
pub(super) mod spectrum;
pub(super) mod utils;
pub(super) mod waterfall;

pub(crate) use common::DECODE_BAR_H;
pub(super) use common::{
    DecodeBarMode, FFT_SIZE, MAX_SAMPLES_PER_FRAME, MIN_SAMPLES_PER_FRAME, PANE_BG, SAMPLE_RATE,
    SourceMode, WaterfallMode,
};
pub(crate) use view::ViewApp;
