use sample::{self, Frame};
use std;
use std::sync::mpsc;

/// A `sound::Requester` for converting backend audio requests into requests for buffers of a fixed
/// size called from a separate thread.
///
/// The `Requester` works by calling `fill_buffer` with the requested buffer and sample rate from
/// the audio backend each time the callback is invoked.
pub struct Requester<F, M> {
    frames: Vec<F>,
    sound_engine_tx: mpsc::Sender<M>,
    num_frames: usize,
    buffer_tx: mpsc::Sender<Vec<F>>,
    buffer_rx: mpsc::Receiver<Vec<F>>,
    // `Some` if part of `frames` has not yet been written to output.
    pending_range: Option<std::ops::Range<usize>>,
}

/// The message type received by the audio::engine thread.
pub trait Message: Send {
    type Frame: Send;
    fn audio_request(buffer: Buffer<Self::Frame>, sample_hz: f64) -> Self;
}

/// A `Buffer` for safely sending frames to the `sound::Engine` to be filled.
///
/// The `Buffer` will automatically the `frames` back to the `Requester` when dropped.
pub struct Buffer<F> {
    /// The frames stored within the buffer.
    frames: Vec<F>,
    /// The channel for sending the frames back to the `Requester`.
    tx: mpsc::Sender<Vec<F>>,
}

impl<F> Buffer<F> {
    /// Submit the `Buffer` back to the `sound::Requester` for writing to the backend.
    pub fn submit(mut self) -> Result<(), mpsc::SendError<Vec<F>>> {
        self.submit_inner()
    }

    fn submit_inner(&mut self) -> Result<(), mpsc::SendError<Vec<F>>> {
        let frames = std::mem::replace(&mut self.frames, Vec::new());
        self.tx.send(frames)
    }
}

impl<F> std::ops::Deref for Buffer<F> {
    type Target = [F];
    fn deref(&self) -> &Self::Target {
        &self.frames[..]
    }
}

impl<F> std::ops::DerefMut for Buffer<F> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.frames[..]
    }
}

impl<F> Drop for Buffer<F> {
    fn drop(&mut self) {
        // If the frames haven't been sent back yet, do so.
        if self.frames.len() > 0 {
            self.submit_inner().ok();
        }
    }
}


/// Indicates whether the audio thread should stop or continue.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum ControlFlow {
    Continue,
    Complete,
}


impl<F, M> Requester<F, M>
where
    F: Frame + Send,
    M: Message<Frame=F>,
{
    /// Construct a new `sound::Requester`.
    ///
    /// `num_frames` must be greater than `0`.
    pub fn new(sound_engine_tx: mpsc::Sender<M>, num_frames: usize) -> Self {
        // We can't make any progress filling buffers of `0` frames.
        assert!(num_frames > 0);
        let (buffer_tx, buffer_rx) = mpsc::channel();
        Requester {
            frames: vec![F::equilibrium(); num_frames],
            sound_engine_tx: sound_engine_tx,
            num_frames: num_frames,
            buffer_tx: buffer_tx,
            buffer_rx: buffer_rx,
            pending_range: None,
        }
    }

    /// Fill the given `buffer` with frames requested from the audio `Engine`.
    ///
    /// Returns `Continue` until communication with the `SoundEngine` is lost, at which `Complete`
    /// is returned.
    ///
    /// `Panic!`s if `sample_hz` is not greater than `0`.
    pub fn fill_buffer(&mut self, output: &mut [F], sample_hz: f64) -> ControlFlow {
        let Requester {
            ref mut frames,
            ref sound_engine_tx,
            num_frames,
            ref buffer_tx,
            ref buffer_rx,
            ref mut pending_range,
        } = *self;

        // if `output` is empty, there's nothing to fill.
        if output.is_empty() {
            return ControlFlow::Continue;
        }

        // Zero the buffer before doing anything else.
        sample::slice::equilibrium(output);

        // Have to have a positive sample_hz or nothing will happen!
        assert!(sample_hz > 0.0);

        // The starting index of the output slice we'll write to.
        let mut start = 0;

        // If there is some un-read range of `frames`, read those first.
        if let Some(range) = pending_range.take() {

            // If the pending range would not fill the output, write what we can before going on to
            // request more frames.
            if range.len() < output.len() {
                start = range.len();
                sample::slice::write(&mut output[..range.len()], &frames[range]);

            // If we have the exact number of frames as output, write them and return.
            } else if range.len() == output.len() {
                sample::slice::write(output, &frames[range]);
                return ControlFlow::Continue;
            } else {
                let end = range.start + output.len();
                sample::slice::write(output, &frames[range.start..end]);
                *pending_range = Some(end..range.end);
                return ControlFlow::Continue;
            }
        }

        // Ensure that our buffer has `num_frames` `frames`.
        frames.resize(num_frames, F::equilibrium());

        // Loop until the given `output` is filled.
        loop {
            // See how many frames are left to fill.
            let num_frames_remaining = output.len() - start;

            // The number of frames to write to output on this iteration.
            let num_frames_to_fill = std::cmp::min(frames.len(), num_frames_remaining);

            // Zero the `frames` buffer read for summing.
            sample::slice::equilibrium(frames);

            // Request audio from the sound::Engine.
            let buffer = Buffer {
                frames: std::mem::replace(frames, Vec::new()),
                tx: buffer_tx.clone(),
            };
            let message = M::audio_request(buffer, sample_hz);
            if sound_engine_tx.send(message).is_err() {
                return ControlFlow::Complete;
            }

            // Receive the written audio and put it back in `frames`.
            match buffer_rx.recv() {
                Ok(mut written_frames) => std::mem::swap(frames, &mut written_frames),
                Err(_) => return ControlFlow::Complete,
            };

            // Write the `frames` to output.
            let end = start + num_frames_to_fill;
            let range = start..end;
            sample::slice::write(&mut output[range.clone()], &frames[..range.len()]);

            // If this was the last frame, break from the loop.
            if end == output.len() {

                // If this is the last iteration and not all of `frames` were read, store the
                // `pending_range` to be read next time this method is called.
                if range.len() < frames.len() {
                    *pending_range = Some(range.len()..frames.len());
                }

                break;
            }

            // Continue looping through the next frames.
            start = end;
        }

        ControlFlow::Continue
    }
}
