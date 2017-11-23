use audio;
use hound::{self, SampleFormat};
use nannou::audio::sample::{self, Sample, signal, Signal};
use std::io::BufReader;
use std::fs::File;
use std::path::{Path, PathBuf};
use time_calc::{Ms, Samples, SampleHz};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Wav {
    pub path: PathBuf,
    pub channels: usize,
    pub duration: Samples,
    pub sample_hz: SampleHz,
}

impl Wav {
    /// Attempts to load the WAV header and read the number of channels.
    pub fn from_path(path: PathBuf) -> Result<Self, hound::Error> {
        let reader = hound::WavReader::open(&path)?;
        let spec = reader.spec();
        let channels = spec.channels as usize;
        let sample_hz = spec.sample_rate as _;
        let duration = Samples(reader.duration() as _);
        Ok(Wav { path, channels, duration, sample_hz })
    }

    /// The duration of the `Wav` in milliseconds.
    pub fn duration_ms(&self) -> Ms {
        self.duration.to_ms(self.sample_hz)
    }
}

/// Load the WAV file at the given path and return a boxed signal.
pub fn stream_signal(path: &Path) -> Result<Box<Iterator<Item=f32> + Send>, hound::Error> {
    let reader = hound::WavReader::open(path)?;
    let spec = reader.spec();

    // A macro to abstract the boilerplate required for different sample_rate/bit_depth combos.
    macro_rules! box_signal {
        ($T:ty) => {{
            //let num_frames = reader.duration();
            let frames = reader
                .into_samples::<$T>()
                .map(|s| [s.unwrap().to_sample::<f32>()]);
            let source_hz = spec.sample_rate as _;
            let target_hz = audio::SAMPLE_RATE as _;
            if source_hz != target_hz {
                let mut signal = signal::from_iter(frames);
                let interp = sample::interpolate::Linear::from_source(&mut signal);
                let signal = signal.from_hz_to_hz(interp, source_hz, target_hz);
                //let new_num_frames = ((target_hz / source_hz) * num_frames as f64) as usize + 1;
                let iter = signal.until_exhausted().map(|s| s[0]);
                //let iter = signal.take(new_num_frames).map(|s| s[0]);
                Box::new(iter) as Box<Iterator<Item=f32> + Send>
            } else {
                let iter = frames.map(|s| s[0]);
                Box::new(iter) as Box<Iterator<Item=f32> + Send>
            }
        }};
    }

    let boxed_signal = match (spec.sample_format, spec.bits_per_sample) {
        (SampleFormat::Float, 32) => box_signal!(f32),
        (SampleFormat::Int, 8) => box_signal!(i8),
        (SampleFormat::Int, 16) => box_signal!(i16),
        (SampleFormat::Int, 32) => box_signal!(i32),
        _ => panic!("Unsupported bit depth {} - currently only 8, 16 and 32 are supported",
                    spec.bits_per_sample),
    };

    Ok(boxed_signal)
}

/// An iterator yielding samples from a `WavReader`. When the `WavReader` is exhausted, `WavCycle`
/// seeks back to the beginning of the file in order to play from the top again.
///
/// Returns `None` only if some error occurs when seeking to the beginning of the file.
pub struct WavCycle {
    // `Option` is used so that when the `WavReader` is exhausted it can be consumed to produce the
    // inner `BufReader` so that we may seek to the beginning of the file. The existing `BufReader`
    // is then used to construct a new `WavReader` which is put back into the `Option`.
    reader: hound::WavReader<BufReader<File>>,
}

impl Iterator for WavCycle {
    type Item = f32;
    fn next(&mut self) -> Option<Self::Item> {
        macro_rules! next_sample {
            ($T:ty) => {{
                if let Some(sample) = self.reader.samples::<$T>().next() {
                    return Some(sample.unwrap().to_sample::<f32>());
                }
            }};
        }

        let spec = self.reader.spec();
        loop {
            match (spec.sample_format, spec.bits_per_sample) {
                (SampleFormat::Float, 32) => next_sample!(f32),
                (SampleFormat::Int, 8) => next_sample!(i8),
                (SampleFormat::Int, 16) => next_sample!(i16),
                (SampleFormat::Int, 32) => next_sample!(i32),
                _ => panic!("Unsupported bit depth {} - currently only 8, 16 and 32 are supported",
                            spec.bits_per_sample),
            }

            if self.reader.seek_sample(0).is_err() {
                return None;
            }
        }
    }
}

/// Load the WAV file at the given path and return a cycled boxed signal.
pub fn stream_signal_cycled(path: &Path) -> Result<Box<Iterator<Item=f32> + Send>, hound::Error> {
    let reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    let cycled = WavCycle { reader };
    let source_hz = spec.sample_rate as _;
    let target_hz = audio::SAMPLE_RATE as _;
    if source_hz != target_hz {
        let frames = cycled.map(|s| [s]);
        let mut signal = signal::from_iter(frames);
        let interp = sample::interpolate::Linear::from_source(&mut signal);
        let signal = signal.from_hz_to_hz(interp, source_hz, target_hz);
        let iter = signal.until_exhausted().map(|s| s[0]);
        Ok(Box::new(iter) as Box<Iterator<Item=f32> + Send>)
    } else {
        Ok(Box::new(cycled) as Box<Iterator<Item=f32> + Send>)
    }
}
