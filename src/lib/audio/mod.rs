use metres::Metres;

pub use self::detector::{FFT_WINDOW_LEN, FFT_BIN_STEP_HZ, EnvDetector, FftDetector, Fft};
pub use self::sound::Sound;
pub use self::source::Source;
pub use self::speaker::Speaker;
pub use self::wav::Wav;

pub mod dbap;
pub mod detector;
pub mod fft;
pub mod input;
pub mod output;
pub mod sound;
pub mod source;
pub mod speaker;
pub mod wav;

/// Sounds should only be output to speakers that are nearest to avoid the need to render each
/// sound to every speaker on the map.
pub const PROXIMITY_LIMIT: Metres = Metres(5.0);
/// The proximity squared (for more efficient distance comparisons).
pub const PROXIMITY_LIMIT_2: Metres = Metres(PROXIMITY_LIMIT.0 * PROXIMITY_LIMIT.0);

/// The maximum number of audio channels.
pub const MAX_CHANNELS: usize = 32;

/// The desired sample rate of the output stream.
pub const SAMPLE_RATE: f64 = 44_100.0;

/// The desired number of frames requested at a time.
pub const FRAMES_PER_BUFFER: usize = 64;

/// The rolloff decibel amount, used to attenuate speaker gains over distances.
pub const ROLLOFF_DB: f64 = 6.0;
