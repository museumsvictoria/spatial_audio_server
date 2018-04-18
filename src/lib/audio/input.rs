//! The capture function implementation for the audio server's input stream.
//!
//! The input stream has a number of `Source`s that read from one or more of the stream's channels.

use audio::MAX_CHANNELS;
use audio::source;
use fxhash::FxHashMap;
use nannou;
use nannou::audio::Buffer;
use std::sync::{mpsc, Arc};
use std::sync::atomic::{self, AtomicBool};

/// Simplified type alias for the nannou audio input stream used by the audio server.
pub type Stream = nannou::audio::Stream<Model>;

/// The state stored on each device's input audio stream.
pub struct Model {
    // All sources that currently exist.
    pub sources: FxHashMap<source::Id, source::Realtime>,
    // A map from channels to the sources that request audio from them them.
    channel_targets: FxHashMap<usize, Vec<source::Id>>,
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
    /// Feeds samples from the input buffer to the associated `Sound`'s `Box<Iterator<Item=f32>>`.
    pub sample_tx: mpsc::SyncSender<f32>,
    /// An indicator from the `Sound` on whether the sound is currently playing or not.
    pub is_capturing: Arc<AtomicBool>,
}

impl Model {
    /// Initialise the input audio device stream model.
    ///
    /// This pre-allocates all possibly required memory for all of the model's buffers in order to
    /// avoid unexpected dynamic allocation within on the audio thread.
    pub fn new() -> Self {
        let sources = Default::default();
        let channel_targets = (0..MAX_CHANNELS)
            .map(|i| (i, Vec::with_capacity(1024)))
            .collect();
        let active_sounds = Default::default();
        Model {
            sources,
            channel_targets,
            active_sounds,
        }
    }

    /// Clear all data related to a specific audio server project.
    ///
    /// This is called when we switch between projects within the GUI.
    pub fn clear_project_specific_data(&mut self) {
        self.sources.clear();
        self.channel_targets.clear();
        self.active_sounds.clear();
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
                if let Some(targets) = channel_targets.get_mut(&ch) {
                    targets.push(id);
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
                        for sound in sounds.iter() {
                            if sound.is_capturing.load(atomic::Ordering::Relaxed) {
                                active_sounds_per_channel[ch].push(sound);
                            }
                        }
                    }
                }
            }

            // Send every sample in chronological order to the active sounds.
            for (i, frame) in buffer.frames().enumerate() {
                for (ch, &sample) in frame.iter().enumerate() {
                    for sound in &active_sounds_per_channel[ch] {
                        let send_sample = match sound.duration {
                            Duration::Frames(frames) => i < frames,
                            Duration::Infinite => true,
                        };
                        if send_sample {
                            sound.sample_tx.try_send(sample).ok();
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
    model
}
