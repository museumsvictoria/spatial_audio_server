//! The capture function implementation for the audio server's input streams.
//!
//! Each device has its own associated input stream, and each input stream has a number of
//! `Source`s that read from one or more of the stream's channels.

use audio::MAX_CHANNELS;
use audio::source;
use nannou;
use nannou::audio::Buffer;
use std::collections::HashMap;
use std::sync::mpsc;

/// Simplified type alias for the nannou audio input stream used by the audio server.
pub type Stream = nannou::audio::Stream<Model>;

/// The state stored on each device's input audio stream.
pub struct Model {
    // All sources that currently exist.
    sources: HashMap<source::Id, source::Realtime>,
    // A map from channels to the sources that request audio from them them.
    channel_targets: HashMap<usize, Vec<source::Id>>,
    // The currently active sounds using the realtime source with the given source ID.
    active_sounds: HashMap<source::Id, Vec<ActiveSound>>,
}

/// A sound with a realtime input source that is currently being played.
///
/// Every `input::ActiveSound` has an associated `output::ActiveSound`. Once the
/// `input::ActiveSound` has no more frames, the `output::ActiveSound`'s signal iterator will yield
/// `None`.
pub struct ActiveSound {
    /// The number of frames left to play of this source before it should end.
    remaining_frames: usize,
    /// Feeds samples from the input buffer to the associated `Sound`'s `Box<Iterator<Item=f32>>`.
    sample_sender: mpsc::SyncSender<f32>,
}

impl Model {
    /// Initialise the input audio device stream model.
    ///
    /// This pre-allocates all possibly required memory for all of the model's buffers in order to
    /// avoid unexpected dynamic allocation within on the audio thread.
    pub fn new() -> Self {
        let sources = HashMap::with_capacity(1024);
        let channel_targets = (0..MAX_CHANNELS)
            .map(|i| (i, Vec::with_capacity(1024)))
            .collect();
        let active_sounds = HashMap::with_capacity(1024);
        Model {
            sources,
            channel_targets,
            active_sounds,
        }
    }
}

/// The function given to nannou to use for capturing audio for a device.
pub fn capture(mut model: Model, buffer: &Buffer) -> Model {
    {
        let Model {
            ref sources,
            ref mut channel_targets,
            ref mut active_sounds,
        } = model;

        assert!(channel_targets.len() <= buffer.len());

        // First, update the channel targets based on the current sources.
        for sources in channel_targets.values_mut() {
            sources.clear();
        }
        for (&id, source) in sources {
            for ch in source.channels.clone() {
                if let Some(sources) = channel_targets.get_mut(&ch) {
                    sources.push(id);
                }
            }
        }

        // Convert the channel_targets and active_sounds maps to a single `Vec<Vec<[ActiveSound]>>`
        // where the outer `Vec` is indexed by the channel number for all active sounds on that
        // channel.
        {
            let n_channels = buffer.channels();
            // TODO: Re-use a buffer for this somehow... The model can't own this buffer due to
            // ownership issues. Maybe use `rental` crate to bypass issue?
            let mut active_sounds_per_channel: Vec<_> = (0..n_channels).map(|_| vec![]).collect();
            for (&ch, sources) in channel_targets.iter() {
                for source in sources {
                    if let Some(sounds) = active_sounds.get(source) {
                        active_sounds_per_channel[ch].push(sounds);
                    }
                }
            }

            // Send every sample in chronological order to the active sounds.
            for (i, frame) in buffer.frames().enumerate() {
                for (ch, &sample) in frame.iter().enumerate() {
                    for sounds in &active_sounds_per_channel[ch] {
                        for sound in &sounds[..] {
                            if i < sound.remaining_frames {
                                sound.sample_sender.send(sample).ok();
                            }
                        }
                    }
                }
            }
        }

        // Subtract from the remaining frames from each active sound.
        //
        // Remove sounds that have no more remaining samples to capture.
        let n_frames = buffer.len_frames();
        for sounds in active_sounds.values_mut() {
            sounds.retain(|s| s.remaining_frames > n_frames);
            for sound in sounds.iter_mut() {
                sound.remaining_frames -= n_frames;
            }
        }
    }
    model
}
