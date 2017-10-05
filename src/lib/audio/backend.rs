//! A module for setting up the audio backend using CPAL.

extern crate cpal;
extern crate futures;

use audio;
use sample::{Frame, Sample, FromSampleSliceMut, ToSample};
use std;

use self::futures::Stream;
use self::futures::task::{self, Executor, Run};


struct MyExecutor;

impl Executor for MyExecutor {
    fn execute(&self, r: Run) {
        r.run();
    }
}

/// Runs the given `SoundRequester` using a CPAL `EventLoop`.
///
/// Returns the `Voice` with which the `SoundEngine` is played back.
pub fn spawn<F, M>(mut audio_requester: audio::Requester<F, M>, sample_hz: f64)
    -> Result<cpal::Voice, cpal::CreationError>
    where F: 'static + Frame + Send + std::fmt::Debug,
          F::Float: Send,
          F::Sample: cpal::Sample + ToSample<u16> + ToSample<i16> + ToSample<f32>,
          M: 'static + audio::requester::Message<Frame=F>,
          for<'a> &'a mut [F]: FromSampleSliceMut<'a, F::Sample>,
{
    // Use the system's default endpoint (aka output audio device).
    let endpoint = cpal::get_default_endpoint().expect("Failed to get default endpoint");

    // Use the default endpoint format but with the specified sample_hz and channel layout.
    let mut format = endpoint
        .get_supported_formats_list()
        .unwrap()
        .next()
        .expect("Failed to get endpoint format");
    format.samples_rate = cpal::SamplesRate(sample_hz as u32);
    format.channels = match F::n_channels() {
        1 => vec![cpal::ChannelPosition::FrontLeft],
        2 => {
            vec![
                cpal::ChannelPosition::FrontLeft,
                cpal::ChannelPosition::FrontRight,
            ]
        }
        n => vec![cpal::ChannelPosition::FrontLeft; n],
    };

    let event_loop = cpal::EventLoop::new();
    let executor = std::sync::Arc::new(MyExecutor);

    let (mut voice, stream) =
        cpal::Voice::new(&endpoint, &format, &event_loop).expect("Failed to create a voice");

    // A buffer to which the `SoundEngine`'s output can be written.
    let mut buffer = Vec::with_capacity(1_024);

    voice.play();
    task::spawn(stream.for_each(move |mut output| -> Result<_, ()> {

        // Ensure that `buffer` is large enough to request enough audio for `output`.
        let frames_to_fill = output.len() / F::n_channels();
        buffer.resize(frames_to_fill, F::equilibrium());

        // Fill the buffer using the `audio_requester`.
        match audio_requester.fill_buffer(&mut buffer, sample_hz) {
            audio::requester::ControlFlow::Continue => (),
            audio::requester::ControlFlow::Complete => return Err(()),
        }

        // A function to simplify filling the unknown buffer type.
        fn fill_output<S, F>(output: &mut [S], buffer: &[F])
        where
            F: Frame,
            F::Sample: ToSample<S>,
        {
            for (out_frame, frame) in output.chunks_mut(F::n_channels()).zip(buffer) {
                for (out_sample, sample) in out_frame.iter_mut().zip(frame.channels()) {
                    *out_sample = sample.to_sample::<S>();
                }
            }
        }

        // Fill the CPAL output.
        match output {
            cpal::UnknownTypeBuffer::U16(ref mut output) => fill_output(output, &buffer),
            cpal::UnknownTypeBuffer::I16(ref mut output) => fill_output(output, &buffer),
            cpal::UnknownTypeBuffer::F32(ref mut output) => fill_output(output, &buffer),
        }

        Ok(())
    })).execute(executor);

    std::thread::spawn(move || { event_loop.run(); });

    Ok(voice)
}
