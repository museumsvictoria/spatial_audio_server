use metres::Metres;
use nannou_audio::{Device, Host};
use time_calc::Ms;

pub use self::detector::{EnvDetector, Fft, FftDetector, FFT_BIN_STEP_HZ, FFT_WINDOW_LEN};
pub use self::sound::Sound;
pub use self::source::Source;
pub use self::speaker::Speaker;

pub mod dbap;
pub mod detection;
pub mod detector;
pub mod fft;
pub mod input;
pub mod output;
pub mod sound;
pub mod source;
pub mod speaker;

/// The maximum number of audio channels.
#[cfg(not(feature = "test_with_stereo"))]
pub const MAX_CHANNELS: usize = 128;
#[cfg(feature = "test_with_stereo")]
pub const MAX_CHANNELS: usize = 2;

/// The absolute maximum number of simultaneous sounds allowed per exhibition.
///
/// NOTE: This value is simply used for pre-allocation (to avoid allocating on the audio thread).
/// The number is arbitrary - feel free to increase/decrease this as necessary.
pub const MAX_SOUNDS: usize = 1024;

/// The desired sample rate of the output stream.
pub const SAMPLE_RATE: f64 = 48_000.0;

/// The desired number of frames requested at a time.
pub const FRAMES_PER_BUFFER: usize = 1024;

/// The initial, default master volume.
pub const DEFAULT_MASTER_VOLUME: f32 = 0.5;

/// The initial, default latency applied to real-time input sources for synchronisation with the
/// audio output thread.
pub const DEFAULT_REALTIME_SOURCE_LATENCY: Ms = Ms(512.0);

/// The default rolloff decibel amount, used to attenuate speaker gains over distances.
pub const DEFAULT_DBAP_ROLLOFF_DB: f64 = 4.0;

/// The "blurring" amount applied to the distance function used for calculating DBAP.
pub const DISTANCE_BLUR: f64 = 0.01;

/// The initial, default proximity limit.
pub const DEFAULT_PROXIMITY_LIMIT: Metres = Metres(7.0);
/// Proximity limit squared for efficientcy efficiency.
pub const DEFAULT_PROXIMITY_LIMIT_2: Metres = Metres(DEFAULT_PROXIMITY_LIMIT.0 * DEFAULT_PROXIMITY_LIMIT.0);

/// Retrieve the desired audio host for the system.
///
/// In general, this uses the default host, but uses the ASIO host if the "asio" feature is enabled
/// when building for a windows target.
pub fn host() -> Host {
    #[cfg(all(windows, feature = "asio"))]
    {
        return Host::from_id(nannou_audio::HostId::Asio)
            .expect("failed to initialise ASIO audio host");
    }
    #[cfg(not(features = "asio"))]
    {
        return Host::default();
    }
}

/// Given a target device name, find the device within the host and return it.
///
/// If no device with the given name can be found, or if the given `target_name` is empty, the
/// default will be returned.
///
/// Returns `None` if no input devices could be found.
pub fn find_input_device(host: &Host, target_name: &str) -> Option<Device> {
    if target_name.is_empty() {
        host.default_input_device()
    } else {
        host.input_devices()
            .ok()
            .into_iter()
            .flat_map(std::convert::identity)
            .find(|d| d.name().map(|n| n.contains(&target_name)).unwrap_or(false))
            .or_else(|| host.default_input_device())
    }
}

/// Given a target device name, find the device within the host and return it.
///
/// If no device with the given name can be found, or if the given `target_name` is empty, the
/// default will be returned.
///
/// Returns `None` if no output devices could be found.
pub fn find_output_device(host: &Host, target_name: &str) -> Option<Device> {
    if target_name.is_empty() {
        host.default_output_device()
    } else {
        host.output_devices()
            .ok()
            .into_iter()
            .flat_map(std::convert::identity)
            .find(|d| d.name().map(|n| n.contains(&target_name)).unwrap_or(false))
            .or_else(|| host.default_output_device())
    }
}
