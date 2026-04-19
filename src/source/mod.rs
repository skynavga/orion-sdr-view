// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

pub mod amdsb;
pub mod cw;
pub mod ft8;
pub mod psk31;
pub mod tone;

/// Maximum continuous signal duration in a single loop cycle.
/// Sources clamp the signal burst to this value so the decode-bar timer
/// ("sig NN.NN") never overflows its fixed-width display.
pub const MAX_SIG_SECS: f32 = 99.99;

#[allow(unused_imports)]
pub use amdsb::{AmDsbSource, BuiltinAudio, load_builtin};
#[allow(unused_imports)]
pub use cw::CwSource;
#[allow(unused_imports)]
pub use ft8::{Ft8Mode, Ft8MsgType, Ft8Source};
#[allow(unused_imports)]
pub use psk31::{Psk31Mode, Psk31Source};

/// Common interface for all signal sources.
///
/// Implementations produce real-valued (f32) samples ready to push into the
/// existing `RingBuffer` and spectrum display pipeline.
///
/// `as_any_mut` enables downcasting a `Box<dyn SignalSource>` to a concrete type:
/// ```ignore
/// if let Some(am) = source.as_any_mut().downcast_mut::<am_dsb::AmDsbSource>() { ... }
/// ```
pub trait SignalSource {
    fn next_samples(&mut self, n: usize) -> Vec<f32>;
    #[allow(dead_code)] // used by integration tests, not the binary
    fn sample_rate(&self) -> f32;
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
    /// Reset playback to the beginning of the first loop cycle.
    #[allow(dead_code)] // used by integration tests, not the binary
    fn restart(&mut self) {}
}
