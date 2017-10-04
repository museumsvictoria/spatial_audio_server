use atomic::Atomic;
use cgmath::Point2;
use metres::Metres;
use sample::Signal;
use std::sync::Arc;

/// `Sound`s can be thought of as a stack of three primary components:
///
/// 1. **Source**: for generating audio data (via oscillator, wave, audio input, etc).
/// 2. **Pre-spatial effects processing**: E.g. fades.
/// 3. **Spatial Output**: maps the sound from a position in space to the output channels.
pub struct Sound {
    // The number of channels yielded by the `Sound`.
    channels: usize,
    // Includes the source and pre-spatial effects.
    //
    // The signal is unique in that channels are interleaved rather than presented side-by-side in
    // the `Frame` type itself. This allows having a dynamic number of channels.
    signal: Box<Signal<Frame=[f32; 1]> + Send>,
    // The location of the sound within the space.
    point: Atomic<Point2<Metres>>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct Id(u64);

/// Items related to audio sources.
///
/// Audio sources come in two kinds:
///
/// 1. WAV - pre-rendered n-channel .wav files and
/// 2. Realtime - input from some other currently running program (e.g. MSP, Live, etc).
pub struct Source {
    kind: Kind,
}

pub enum Kind {
    Wav(Wav),
    Realtime(Realtime),
}

struct Wav {
    playhead: usize,
    data: Data,
}

struct Data {
    channels: usize,
    // Interleaved audio data loaded entirely into memory.
    interleaved_samples: Arc<[f32; 1]>,
}

struct Realtime {
    channels: usize,
    // Need some input type
}
