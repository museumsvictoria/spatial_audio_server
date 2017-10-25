use SAMPLE_HZ;
use hound::{self, SampleFormat};
use sample::{self, Sample, signal, Signal};
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
pub fn stream_signal(path: &Path) -> Result<Box<Signal<Frame=[f32; 1]> + Send>, hound::Error> {
    let reader = hound::WavReader::open(path)?;
    let spec = reader.spec();

    // A macro to abstract the boilerplate required for different sample_rate/bit_depth combos.
    macro_rules! box_signal {
        ($T:ty) => {{
            let frames = reader
                .into_samples::<$T>()
                .map(|s| [s.unwrap().to_sample::<f32>()]);
            let mut signal = signal::from_iter(frames);
            let source_hz = spec.sample_rate as _;
            let target_hz = SAMPLE_HZ as _;
            if source_hz != target_hz {
                let interp = sample::interpolate::Linear::from_source(&mut signal);
                let signal = signal.from_hz_to_hz(interp, source_hz, target_hz);
                Box::new(signal) as Box<Signal<Frame=[f32; 1]> + Send>
            } else {
                Box::new(signal) as Box<Signal<Frame=[f32; 1]> + Send>
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
