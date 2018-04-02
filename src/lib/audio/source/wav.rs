use audio;
use hound::{self, SampleFormat};
use nannou::audio::sample::Sample;
use std::io::{self, BufReader};
use std::fs::File;
use std::path::{Path, PathBuf};
use time_calc::{Ms, SampleHz, Samples};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Wav {
    pub path: PathBuf,
    pub channels: usize,
    pub duration: Samples,
    pub sample_hz: SampleHz,
    #[serde(default = "default_should_loop")]
    pub should_loop: bool,
    #[serde(default = "default_playback")]
    pub playback: Playback,
}

/// The playback mode of the WAV file.
#[derive(Copy, Clone, Debug, Deserialize, Serialize)]
pub enum Playback {
    /// When the WAV is introduced, play it out from the beginning of the file.
    Retrigger,
    /// When the WAV is introduced start playing from its position within the global continuous
    /// timeline.
    ///
    /// This acts as though the WAV is constantly being played but is just "muted" or "unmuted" as
    /// we remove it and re-introduce it respectively.
    Continuous,
}

/// The number of variants within the `Playback` enum.
pub const NUM_PLAYBACK_OPTIONS: usize = 2;

/// Default to `Retrigger` mode.
fn default_playback() -> Playback {
    Playback::Retrigger
}

/// The default WAV `should_loop` state.
fn default_should_loop() -> bool {
    false
}

impl Wav {
    /// Attempts to load the WAV header and read the number of channels.
    pub fn from_path(path: PathBuf) -> Result<Self, hound::Error> {
        let reader = hound::WavReader::open(&path)?;
        let spec = reader.spec();
        let channels = spec.channels as usize;
        let sample_hz = spec.sample_rate as _;
        assert_eq!(sample_hz, audio::SAMPLE_RATE,
                   "WAV files must have a sample rate of {}", audio::SAMPLE_RATE);
        let duration = Samples(reader.duration() as _);
        let playback = default_playback();
        let should_loop = default_should_loop();
        Ok(Wav {
            path,
            channels,
            duration,
            sample_hz,
            playback,
            should_loop,
        })
    }

    /// The duration of the `Wav` in milliseconds.
    pub fn duration_ms(&self) -> Ms {
        self.duration.to_ms(self.sample_hz)
    }
}

/// An iterator streaming f32 samples from a WAV file.
pub struct SampleStream {
    reader: hound::WavReader<BufReader<File>>,
    spec: hound::WavSpec,
    sample_index: usize,
}

/// An iterator yielding samples from a `SampleStream`. When the `SampleStream` is exhausted,
/// `CycledSampleStream` seeks back to the beginning of the file in order to play from the top
/// again.
///
/// Returns `None` only if some error occurs when seeking to the beginning of the file.
pub struct CycledSampleStream {
    stream: SampleStream,
}

/// A signal wrapper around the `SampleStream` and `CycledSampleStream` types.
pub enum Signal {
    Once(SampleStream),
    Cycled(CycledSampleStream),
}

impl SampleStream {
    /// Load the WAV file at the given path and return an iterator streaming samples.
    pub fn from_path<P>(path: P) -> Result<Self, hound::Error>
    where
        P: AsRef<Path>,
    {
        let reader = hound::WavReader::open(path)?;
        let spec = reader.spec();
        let sample_index = 0;
        Ok(SampleStream { sample_index, reader, spec })
    }

    /// Seek to the given `frame` within the file.
    ///
    /// The given `frame` is the time measured as the number of samples (independent of the number
    /// of channels) since the beginning of the audio data.
    ///
    /// If `frame` is larger than the number of samples in the file the remaining duration will be
    /// wrapped around to the beginning.
    pub fn seek(&mut self, frames: u64) -> io::Result<()> {
        let duration_frames = self.reader.duration() as u64;
        let frames = frames % duration_frames;
        self.sample_index = self.channels() * frames as usize;
        self.reader.seek(frames as u32)
    }

    /// Consume self and produce an iterator that endlessly cycles the stream.
    pub fn cycle(self) -> CycledSampleStream {
        let stream = self;
        CycledSampleStream { stream }
    }

    /// The number of channels in the samples.
    pub fn channels(&self) -> usize {
        self.spec.channels as _
    }

    /// The number of remaining samples.
    pub fn remaining_samples(&self) -> usize {
        assert!(self.reader.len() >= self.sample_index as u32);
        self.reader.len() as usize - self.sample_index
    }

    /// The number of remaining frames.
    pub fn remaining_frames(&self) -> usize {
        let remaining_samples = self.remaining_samples();
        let channels = self.channels();
        remaining_samples / channels
    }
}

impl Signal {
    /// Borrow the inner iterator yielding samples.
    pub fn samples(&mut self) -> &mut Iterator<Item = f32> {
        match *self {
            Signal::Once(ref mut s) => s as _,
            Signal::Cycled(ref mut s) => s as _,
        }
    }

    /// Seek to the given `frame` within the file.
    ///
    /// The given `frame` is the time measured as the number of samples (independent of the number
    /// of channels) since the beginning of the audio data.
    ///
    /// If `frame` is larger than the number of samples in the file the remaining duration will be
    /// wrapped around to the beginning.
    pub fn seek(&mut self, frame: u64) -> io::Result<()> {
        match *self {
            Signal::Once(ref mut s) => s.seek(frame),
            Signal::Cycled(ref mut s) => s.stream.seek(frame),
        }
    }

    /// The number of channels in the signal.
    pub fn channels(&self) -> usize {
        match *self {
            Signal::Once(ref s) => s.channels(),
            Signal::Cycled(ref s) => s.stream.channels(),
        }
    }

    /// The remaining number of frames in the `Signal`.
    pub fn remaining_frames(&self) -> Option<Samples> {
        match *self {
            Signal::Once(ref s) => Some(Samples(s.remaining_frames() as _)),
            Signal::Cycled(_) => None,
        }
    }
}

impl From<SampleStream> for CycledSampleStream {
    fn from(s: SampleStream) -> Self {
        s.cycle()
    }
}

impl From<SampleStream> for Signal {
    fn from(s: SampleStream) -> Self {
        Signal::Once(s)
    }
}

impl From<CycledSampleStream> for Signal {
    fn from(s: CycledSampleStream) -> Self {
        Signal::Cycled(s)
    }
}

impl Iterator for SampleStream {
    type Item = f32;
    fn next(&mut self) -> Option<Self::Item> {
        const FAILED_READ: &str = "failed to read sample in WAV file";

        // A macro to simplify requesting and returning the next sample.
        macro_rules! next_sample {
            ($T:ty) => {{
                if let Some(sample) = self.reader.samples::<$T>().next() {
                    self.sample_index += 1;
                    return Some(sample.expect(FAILED_READ).to_sample::<f32>());
                }
            }};
        }

        loop {
            match (self.spec.sample_format, self.spec.bits_per_sample) {
                (SampleFormat::Float, 32) => next_sample!(f32),
                (SampleFormat::Int, 8) => next_sample!(i8),
                (SampleFormat::Int, 16) => next_sample!(i16),
                (SampleFormat::Int, 32) => next_sample!(i32),
                _ => panic!(
                    "Unsupported bit depth {} - currently only 8, 16 and 32 are supported",
                    self.spec.bits_per_sample
                ),
            }
            return None;
        }
    }
}

impl Iterator for CycledSampleStream {
    type Item = f32;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(sample) = self.stream.next() {
                return Some(sample);
            }

            if self.stream.reader.seek(0).is_err() {
                return None;
            }
        }
    }
}
