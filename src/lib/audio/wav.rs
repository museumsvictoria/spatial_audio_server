use hound;
use std::path::PathBuf;
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
