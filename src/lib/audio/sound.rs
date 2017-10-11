use cgmath::Point2;
use metres::Metres;
use sample::Signal;

/// `Sound`s can be thought of as a stack of three primary components:
///
/// 1. **Source**: for generating audio data (via oscillator, wave, audio input, etc).
/// 2. **Pre-spatial effects processing**: E.g. fades.
/// 3. **Spatial Output**: maps the sound from a position in space to the output channels.
pub struct Sound {
    // The number of channels yielded by the `Sound`.
    pub channels: usize,
    // Includes the source and pre-spatial effects.
    //
    // The signal is unique in that channels are interleaved rather than presented side-by-side in
    // the `Frame` type itself. This allows having a dynamic number of channels.
    pub signal: Box<Signal<Frame=[f32; 1]> + Send>,
    // The location of the sound within the space.
    pub point: Point2<Metres>,
    pub spread: Metres,
    pub radians: f32,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct Id(u64);
