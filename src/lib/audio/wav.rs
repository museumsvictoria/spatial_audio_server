use hound;
use std::path::PathBuf;

#[derive(Deserialize, Serialize)]
pub struct Wav {
    pub path: PathBuf,
    pub channels: usize,
}

impl Wav {
    /// Attempts to load the WAV header and read the number of channels.
    pub fn from_path(path: PathBuf) -> Result<Self, hound::Error> {
        let reader = hound::WavReader::open(&path)?;
        let spec = reader.spec();
        let channels = spec.channels as usize;
        Ok(Wav { path, channels })
    }
}
