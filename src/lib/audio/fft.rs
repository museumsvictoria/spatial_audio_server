//! An audio-specific FFT implementation using RustFFT.

use nannou::prelude::Zero;
use rustfft::num_complex::Complex;
use rustfft::{FftDirection, FftPlanner};

/// An FFT generic over its window type.
pub struct Fft<S> {
    input_window: S,
    output_window: S,
}

/// The FFT planner type used within the audio server.
pub type Planner = FftPlanner<f32>;

/// Slice types that may be used as buffers within the `Fft`.
pub trait Slice {
    type Element;
    fn slice(&self) -> &[Self::Element];
    fn slice_mut(&mut self) -> &mut [Self::Element];
}

impl Slice for [Complex<f32>; super::detector::FFT_WINDOW_LEN] {
    type Element = Complex<f32>;
    fn slice(&self) -> &[Self::Element] {
        &self[..]
    }
    fn slice_mut(&mut self) -> &mut [Self::Element] {
        &mut self[..]
    }
}

impl<S> Fft<S>
where
    S: Slice<Element = Complex<f32>>,
{
    /// Create a new `Fft` from the necessary buffers.
    ///
    /// **Panics** if any of the given window lengths are not equal.
    pub fn new(input_window: S, output_window: S) -> Self {
        assert_eq!(input_window.slice().len() % 2, 0);
        assert_eq!(input_window.slice().len(), output_window.slice().len());
        Fft {
            input_window,
            output_window,
        }
    }

    /// Perform an FFT on the given channel samples.
    ///
    /// **Panics** if the length of the given buffer differs from the inner buffers.
    pub fn process<I>(
        &mut self,
        planner: &mut Planner,
        channel_samples: I,
        freq_amplitudes: &mut [f32],
        direction: FftDirection,
    ) where
        I: IntoIterator<Item = f32>,
    {
        process(
            planner,
            channel_samples,
            self.input_window.slice_mut(),
            self.output_window.slice_mut(),
            freq_amplitudes,
            direction,
        );
    }
}

/// Perform an FFT on the given channel samples.
///
/// - `planner` is re-used to shared data between FFT calculations.
/// - `channel_samples` is the PCM audio data.
/// - `input_window` is the buffer for preparing the PCM data as complex numbers.
/// - `output_window` is the buffer to which the FFT result is written.
/// - `frequency_amplitudes_2` is the result amplitude^2 of each frequency bin. The step between
/// each frequency bin is equal to `samplerate / 2 * windowlength`.
pub fn process<I>(
    planner: &mut Planner,
    channel_samples: I,
    input_window: &mut [Complex<f32>],
    output_window: &mut [Complex<f32>],
    frequency_amplitudes_2: &mut [f32],
    direction: rustfft::FftDirection,
) where
    I: IntoIterator<Item = f32>,
{
    assert_eq!(input_window.len(), output_window.len());
    assert_eq!(output_window.len() / 2, frequency_amplitudes_2.len());

    // Feed the amplitude data into the window of complex values.
    //
    // The real part is set to the amplitude, the imaginary set to 0.
    let mut count = 0;
    for (complex, sample) in input_window.iter_mut().zip(channel_samples) {
        *complex = Complex {
            re: sample,
            im: 0.0,
        };
        count += 1;
    }
    // Ensure there were as many samples in channel_samples as the length of the windows.
    assert_eq!(count, input_window.len());

    // Perform the fourier transform.
    let fft = planner.plan_fft(input_window.len(), direction);

    let mut scratch = vec![Complex::zero(); fft.get_outofplace_scratch_len()];
    fft.process_outofplace_with_scratch(input_window, output_window, &mut scratch);

    // Retrieve the magnitude of the complex numbers as the amplitude of each frequency.
    //
    // NOTE: We ignore the last half of the output window as these represent negative frequencies.
    for (freq_amp, &complex) in frequency_amplitudes_2.iter_mut().zip(output_window.iter()) {
        *freq_amp = complex.re.powi(2) + complex.im.powi(2);
    }
}

/// The low, mid and high peaks given some frequency amplitudes squared (produced via fft).
pub fn lmh(freq_amps_2: &[f32]) -> (f32, f32, f32) {
    const LOW_MAX_HZ: f32 = 200.0;
    const MID_MAX_HZ: f32 = 2_000.0;
    assert_eq!(freq_amps_2.len(), super::detector::FFT_WINDOW_LEN / 2);
    freq_amps_2
        .iter()
        .enumerate()
        .map(|(i, &amp_2)| (linear_bin_max_hz(i), amp_2))
        .fold((0.0, 0.0, 0.0), |(l, m, h), (f_max, amp)| {
            if f_max < LOW_MAX_HZ {
                (l.max(amp), m, h)
            } else if f_max < MID_MAX_HZ {
                (l, m.max(amp), h)
            } else {
                (l, m, h.max(amp))
            }
        })
}

/// Find the maximum frequency bound of a linear fourier transform bin in hz.
pub fn linear_bin_max_hz(bin_i: usize) -> f32 {
    (bin_i + 1) as f32 * super::detector::FFT_BIN_STEP_HZ as f32
}

/// Find the maximum frequency bound of a logarithmic fourier transform bin in hz.
pub fn mel_bin_max_hz(bin_i: usize, total_bins: usize, sample_rate: f32) -> f32 {
    use pitch_calc::{Hz, Mel};
    let max_hz = sample_rate / 2.0;
    let max_mel = Hz(max_hz).mel();
    let mel = Mel(((bin_i + 1) as f32 / total_bins as f32).powi(2) * max_mel);
    mel.hz()
}

/// Maps the peak values from the given input frequency amplitudes (with linear spacing) to the
/// given output frequency amplitudes (with logarithmic spacing).
pub fn mel_bins(in_freq_amps_2: &[f32], out_freq_amps_2: &mut [f32]) {
    // Ensure the output bins are first zeroed, ready for finding the peak.
    for out_bin in out_freq_amps_2.iter_mut() {
        *out_bin = 0.0;
    }
    let n_out_bins = out_freq_amps_2.len();

    // The the input bin frequency ranges and their amplitudes.
    let mut in_bins = in_freq_amps_2
        .iter()
        .enumerate()
        .map(|(i, &amp_2)| {
            let freq_max = (i + 1) as f32 * super::detector::FFT_BIN_STEP_HZ as f32;
            (freq_max, amp_2)
        })
        .peekable();

    // Fill the output bins with the peek of each input bin within range.
    'out_bins: for (out_i, out_bin) in out_freq_amps_2.iter_mut().enumerate() {
        let out_freq_max = mel_bin_max_hz(out_i, n_out_bins, super::SAMPLE_RATE as f32);
        while let Some(&(in_freq_max, amp_2)) = in_bins.peek() {
            if in_freq_max < out_freq_max {
                *out_bin = out_bin.max(amp_2);
                in_bins.next();
            } else {
                continue 'out_bins;
            }
        }
    }
}
