//! The capture function implementation for the audio server's input stream.
//!
//! The input stream has a number of `Source`s that read from one or more of the stream's channels.

use crate::audio::source;
use fxhash::FxHashMap;
use nannou_audio::Buffer;
use std::cmp;
use std::sync::atomic::{self, AtomicBool};
use std::sync::Arc;

/// Simplified type alias for the nannou audio input stream used by the audio server.
pub type Stream = nannou_audio::Stream<Model>;

/// The state stored on each device's input audio stream.
pub struct Model {
    // All sources that currently exist.
    pub sources: FxHashMap<source::Id, source::Realtime>,
    // The currently active sounds using the realtime source with the given source ID.
    pub active_sounds: FxHashMap<source::Id, Vec<ActiveSound>>,
}

/// The duration of an active sound's playback.
#[derive(Copy, Clone, Debug)]
pub enum Duration {
    Infinite,
    Frames(usize),
}

/// A sound with a realtime input source that is currently being played.
///
/// Every `input::ActiveSound` has an associated `output::ActiveSound`. Once the
/// `input::ActiveSound` has no more frames, the `output::ActiveSound`'s signal iterator will yield
/// `None`.
pub struct ActiveSound {
    /// The number of frames left to play of this source before it should end.
    pub duration: Duration,
    /// An indicator from the `Sound` on whether the sound is currently playing or not.
    pub is_capturing: Arc<AtomicBool>,
    /// Whether or not the channel has been closed due to the Signal end dropping.
    pub is_closed: Arc<AtomicBool>,
    /// Feeds buffers of samples from the input buffer to the associated `Sound`'s
    /// `Box<Iterator<Item=f32>>`.
    pub buffer_tx: source::realtime::BufferTx,
    /// Receives used buffers read for re-use.
    pub buffer_rx: source::realtime::BufferRx,
}

impl Model {
    /// Initialise the input audio device stream model.
    ///
    /// This pre-allocates all possibly required memory for all of the model's buffers in order to
    /// avoid unexpected dynamic allocation within on the audio thread.
    pub fn new() -> Self {
        let sources = Default::default();
        let active_sounds = Default::default();
        Model {
            sources,
            active_sounds,
        }
    }

    /// Clear all data related to a specific audio server project.
    ///
    /// This is called when we switch between projects within the GUI.
    pub fn clear_project_specific_data(&mut self) {
        self.sources.clear();
        self.active_sounds.clear();
    }
}

/// The function given to nannou to use for capturing audio for a device.
pub fn capture(model: &mut Model, buffer: &Buffer) {
    let Model {
        ref sources,
        ref mut active_sounds,
    } = *model;

    // Remove any sounds that have been closed.
    for sounds in active_sounds.values_mut() {
        sounds.retain(|s| !s.is_closed.load(atomic::Ordering::Relaxed));
    }

    // Send every sample buffered in chronological order to the active sounds.
    for (source_id, sounds) in active_sounds.iter() {
        // Retrieve the realtime data for this source.
        let realtime = match sources.get(source_id) {
            None => continue,
            Some(rt) => rt,
        };

        for sound in sounds {
            if !sound.is_capturing.load(atomic::Ordering::Relaxed) {
                continue;
            }

            // Determine the number of frames to take.
            let frames_to_take = match sound.duration {
                Duration::Frames(frames) => cmp::min(frames, buffer.len_frames()),
                Duration::Infinite => buffer.len_frames(),
            };

            // If there are no frames to take for this source, skip it.
            if frames_to_take == 0 {
                continue;
            }

            // Retrieve the empty buffer to use for sending samples.
            let mut samples = match sound.buffer_rx.pop() {
                // This branch should never be hit but is here just in case.
                None => {
                    let samples_len = frames_to_take * realtime.channels.len();
                    Vec::with_capacity(samples_len)
                }
                // There should always be a buffer waiting in this channel.
                Some(mut samples) => {
                    samples.clear();
                    samples
                }
            };

            // Get the channel range and ensure it is no greater than the buffer len.
            let start = cmp::min(realtime.channels.start, buffer.channels());
            let end = cmp::min(realtime.channels.end, buffer.channels());

            // Read the necessary samples from the buffer.
            for frame in buffer.frames().take(frames_to_take) {
                samples.extend(frame[start..end].iter().cloned());
            }

            // Send the buffer to the realtime signal.
            sound.buffer_tx.push(samples);
        }
    }

    // Subtract from the remaining frames from each active sound.
    //
    // Remove sounds that have no more remaining samples to capture.
    let n_frames = buffer.len_frames();
    for sounds in active_sounds.values_mut() {
        sounds.retain(|s| match s.duration {
            Duration::Frames(frames) => frames > n_frames,
            Duration::Infinite => true,
        });
        for sound in sounds.iter_mut() {
            if let Duration::Frames(ref mut frames) = sound.duration {
                *frames -= n_frames;
            }
        }
    }
}
