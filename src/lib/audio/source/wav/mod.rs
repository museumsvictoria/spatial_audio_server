use audio;
use hound;
use std::path::PathBuf;
use time_calc::{Ms, SampleHz, Samples};

pub mod reader;
pub mod samples;

/// The WAV file audio source type.
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
