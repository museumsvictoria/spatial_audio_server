use atomic::{self, Atomic};
use cgmath::{Point2, MetricSpace};
use metres::Metres;
use sample::Signal;
use std;
use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
pub use self::requester::Requester;

pub mod backend;
mod requester;


/// Sounds should only be output to speakers that are nearest to avoid the need to render each
/// sound to every speaker on the map.
pub const PROXIMITY_LIMIT: Metres = Metres(5.0);
/// The proximity squared (for more efficient distance comparisons).
pub const PROXIMITY_LIMIT_2: Metres = Metres(PROXIMITY_LIMIT.0 * PROXIMITY_LIMIT.0);

/// The maximum number of audio channels.
pub const MAX_CHANNELS: usize = 32;

/// A single frame of audio.
pub type Frame = [f32; MAX_CHANNELS];

/// Messages that drive the audio engine forward.
pub enum Message {
    /// A request for some frames from the audio backend thread.
    ///
    /// All frames in `buffer` should be written to and then sent back to the audio IO thread as
    /// soon as possible via the given `buffer_tx`.
    RequestAudio(requester::Buffer<Frame>, f64),
    /// Add a new sound to the map.
    AddSound(SoundId, Sound),
    /// Remove a sound from the map.
    RemoveSound(SoundId),
    /// Add a new speaker to the map.
    AddSpeaker(SpeakerId, Arc<Speaker>),
    /// Remove a speaker from the map.
    RemoveSpeaker(SpeakerId),
}

impl requester::Message for Message {
    type Frame = Frame;
    fn audio_request(buffer: requester::Buffer<Frame>, sample_hz: f64) -> Self {
        Message::RequestAudio(buffer, sample_hz)
    }
}

/// Run the audio engine thread.
///
/// This should be run prior to the backend audio thread so that the audio engine is ready to start
/// processing audio.
pub fn spawn() -> mpsc::Sender<Message> {
    let (msg_tx, msg_rx) = mpsc::channel();

    std::thread::Builder::new()
        .name("audio_engine".into())
        .spawn(move || { run(msg_rx); })
        .unwrap();

    msg_tx
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
struct SoundId(u64);

/// `Sound`s can be thought of as a stack of three primary components:
///
/// 1. **Source**: for generating audio data (via oscillator, wave, audio input, etc).
/// 2. **Pre-spatial effects processing**: E.g. fades.
/// 3. **Spatial Output**: maps the sound from a position in space to the output channels.
pub struct Sound {
    // Includes the source and pre-spatial effects.
    signal: Box<Signal<Frame=[f32; 2]> + Send>,
    // The location of the sound within the space.
    point: Atomic<Point2<Metres>>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct SpeakerId(pub u64);

/// Represents a virtual output at some location within the space.
///
/// These parameters are atomics in order to safely share them with the GUI thread.
pub struct Speaker {
    // The location of the speaker within the space.
    pub point: Atomic<Point2<Metres>>,
    // The channel on which the output is rendered.
    pub channel: AtomicUsize,
}


// The function to be run onthe audio engine thread.
fn run(msg_rx: mpsc::Receiver<Message>) {

    // A map from audio sound IDs to the audio sounds themselves.
    let mut sounds = HashMap::with_capacity(1024);

    // A map from speaker IDs to the speakers themselves.
    let mut speakers = HashMap::with_capacity(MAX_CHANNELS);

    // A buffer for collecting the speakers within proximity of the sound's position.
    let mut speakers_in_proximity: Vec<(Amplitude, SpeakerId)> = Vec::with_capacity(MAX_CHANNELS);

    // Wait for messages.
    for msg in msg_rx {
        match msg {
            Message::RequestAudio(buffer, sample_hz) => {
                // For each sound, request `buffer.len()` number of frames and sum them onto the
                // relevant output channels.
                for (&sound_id, sound) in &sounds {
                }
            },

            Message::AddSound(id, sound) => {
                sounds.insert(id, sound);
            },

            Message::RemoveSound(id) => {
                sounds.remove(&id);
            },

            Message::AddSpeaker(id, speaker) => {
                speakers.insert(id, speaker);
            },

            Message::RemoveSpeaker(id) => {
                speakers.remove(&id);
            },
        }
    }
}

type Amplitude = f32;

// Converts the given squared distance to an amplitude multiplier.
//
// The squared distance is used to avoid the need to perform square root.
fn distance_2_to_amplitude(Metres(distance_2): Metres) -> Amplitude {
    // TODO: This is a linear tail off - experiment with exponential tail off.
    1.0 - (distance_2 / PROXIMITY_LIMIT_2.0) as f32
}

fn find_closest_speakers(
    point: &Point2<Metres>,
    closest: &mut Vec<(Amplitude, SpeakerId)>,
    speakers: &HashMap<SpeakerId, Arc<Speaker>>,
) {
    closest.clear();
    let point_f = Point2 { x: point.x.0, y: point.y.0 };
    for (&id, speaker) in speakers.iter() {
        let speaker_point = speaker.point.load(atomic::Ordering::Relaxed);
        let speaker_point_f = Point2 { x: speaker_point.x.0, y: speaker_point.y.0 };
        let distance_2 = Metres(point_f.distance2(speaker_point_f));
        if distance_2 < PROXIMITY_LIMIT_2 {
            // Use a function to map distance to amp.
            let amp = distance_2_to_amplitude(distance_2);
            closest.push((amp, id));
        }
    }
}
