// Copyright (c) 2026 G & R Associates LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

/// Maximum continuous signal duration in a single loop cycle.
/// Sources clamp the signal burst to this value so the decode-bar timer
/// ("sig NN.NN") never overflows its fixed-width display.
pub const MAX_SIG_SECS: f32 = 99.99;

/// Common interface for all signal sources.
///
/// Implementations produce real-valued (f32) samples ready to push into the
/// existing `RingBuffer` and spectrum display pipeline.
///
/// `as_any_mut` enables downcasting a `Box<dyn SignalSource>` to a concrete type:
/// ```no_run
/// use orion_sdr_view::source::{SignalSource, amdsb::AmDsbSource};
/// fn poke_am(source: &mut dyn SignalSource) {
///     if let Some(_am) = source.as_any_mut().downcast_mut::<AmDsbSource>() {
///         // ... mutate the AM source ...
///     }
/// }
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
