pub mod amdsb;
pub mod ft8;
pub mod psk31;
pub mod tone;

#[allow(unused_imports)]
pub use amdsb::{AmDsbSource, BuiltinAudio, load_builtin};
#[allow(unused_imports)]
pub use ft8::{Ft8Source, Ft8Mode, Ft8MsgType};
#[allow(unused_imports)]
pub use psk31::{Psk31Source, Psk31Mode};

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
    #[allow(dead_code)]
    fn sample_rate(&self) -> f32;
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
    /// Reset playback to the beginning of the first loop cycle.
    fn restart(&mut self) {}
}
