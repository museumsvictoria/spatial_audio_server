//! An envelope detector for a single channel.
//!
//! Detects RMS and Peak envelopes.

use audio;
use nannou::audio::sample::{self, ring_buffer};
use rustfft::num_complex::Complex;

// The frame type used within the detectors.
type FrameType = [f32; 1];

// The Rms type used within the RMS envelope detector.
type Rms = sample::rms::Rms<FrameType, Box<[FrameType]>>;

// The Peak type used within the peak envelope detector.
type Peak = sample::envelope::detect::Peak<sample::peak::FullWave>;

// The RMS envelope detector used for monitoring the signal.
type RmsDetector = sample::envelope::Detector<FrameType, Rms>;

// The Peak envelope detector used for monitoring the signal.
type PeakDetector = sample::envelope::Detector<FrameType, Peak>;

// The type of FFT used for analysis.
pub type Fft = super::fft::Fft<[Complex<f32>; FFT_WINDOW_LEN]>;

// RMS is monitored for visualisation, so we want a window size roughly the duration of one frame.
//
// A new visual frame is displayed roughly 60 times per second compared to 44_100 audio frames.
const WINDOW_SIZE: usize = audio::SAMPLE_RATE as usize / 60;

// The number of frames used to smooth the attack/release of the RMS detection.
const RMS_ATTACK_FRAMES: f32 = 0.0;
const RMS_RELEASE_FRAMES: f32 = 0.0;
const PEAK_ATTACK_FRAMES: f32 = WINDOW_SIZE as f32 / 8.0;
const PEAK_RELEASE_FRAMES: f32 = WINDOW_SIZE as f32 / 8.0;

/// The length of the window used for performing the FFT.
pub const FFT_WINDOW_LEN: usize = 1024;

/// The step between each frequency bin is equal to `samplerate / 2 * windowlength`.
pub const FFT_BIN_STEP_HZ: f64 = audio::SAMPLE_RATE / (2.0 * FFT_WINDOW_LEN as f64);

/// An envelope detector for a single channel.
///
/// Detects RMS and Peak envelopes.
pub struct EnvDetector {
    rms: RmsDetector,
    peak: PeakDetector,
    current_rms: f32,
    current_peak: f32,
}

/// An FFT frequency amplitude detector for a single channel.
pub struct FftDetector {
    fft_samples: ring_buffer::Fixed<[f32; FFT_WINDOW_LEN]>,
}

impl EnvDetector {
    /// Construct a new `EnvDetector` with a zeroed RMS window.
    pub fn new() -> Self {
        let slice = vec![[0.0]; WINDOW_SIZE].into_boxed_slice();
        let ring_buffer = ring_buffer::Fixed::from(slice);
        let rms = RmsDetector::rms(ring_buffer, RMS_ATTACK_FRAMES, RMS_RELEASE_FRAMES);
        let peak = PeakDetector::peak(PEAK_ATTACK_FRAMES, PEAK_RELEASE_FRAMES);
        let current_rms = 0.0;
        let current_peak = 0.0;
        EnvDetector {
            rms,
            peak,
            current_rms,
            current_peak,
        }
    }

    /// Step forward the detector with the given sample.
    ///
    /// Returns the current RMS and peak.
    pub fn next(&mut self, sample: f32) -> (f32, f32) {
        let rms = self.rms.next([sample]);
        let peak = self.peak.next([sample]);
        self.current_rms = rms[0];
        self.current_peak = peak[0];
        self.current()
    }

    /// Returns the current RMS and peak.
    pub fn current(&self) -> (f32, f32) {
        (self.current_rms, self.current_peak)
    }
}

impl FftDetector {
    /// Construct a new `EnvDetector` with a zeroed RMS window.
    pub fn new() -> Self {
        let fft_samples = ring_buffer::Fixed::from([0.0; FFT_WINDOW_LEN]);
        FftDetector { fft_samples }
    }

    /// Insert a new sample into the ring buffer.
    ///
    /// Returns the current RMS and peak.
    pub fn push(&mut self, sample: f32) {
        self.fft_samples.push(sample);
    }

    /// Calculate the FFT for all samples currently in the current ring buffer.
    pub fn calc_fft(&self, fft: &mut Fft, freq_amps: &mut [f32]) {
        fft.process(self.fft_samples.iter().cloned(), freq_amps);
    }
}
