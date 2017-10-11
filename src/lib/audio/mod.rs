use cgmath::{Point2, MetricSpace};
use metres::Metres;
use std;
use std::collections::HashMap;
use std::sync::mpsc;
pub use self::requester::Requester;
pub use self::sound::Sound;
pub use self::source::Source;
pub use self::speaker::Speaker;
pub use self::wav::Wav;

pub mod backend;
mod requester;
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
    AddSound(sound::Id, Sound),
    /// Remove a sound from the map.
    RemoveSound(sound::Id),

    /// Update the speaker at the given `Id`.
    UpdateSpeaker(speaker::Id, Speaker),
    /// Remove a speaker from the map.
    RemoveSpeaker(speaker::Id),

    /// The window has been closed and it's time to finish up.
    Exit,
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
pub fn spawn() -> (std::thread::JoinHandle<()>, mpsc::Sender<Message>) {
    let (msg_tx, msg_rx) = mpsc::channel();

    let handle = std::thread::Builder::new()
        .name("audio_engine".into())
        .spawn(move || { run(msg_rx); })
        .unwrap();

    (handle, msg_tx)
}

// The function to be run onthe audio engine thread.
fn run(msg_rx: mpsc::Receiver<Message>) {

    // A map from audio sound IDs to the audio sounds themselves.
    let mut sounds: HashMap<sound::Id, Sound> = HashMap::with_capacity(1024);

    // A map from speaker IDs to the speakers themselves.
    let mut speakers: HashMap<speaker::Id, Speaker> = HashMap::with_capacity(MAX_CHANNELS);

    // A buffer for collecting the speakers within proximity of the sound's position.
    let mut speakers_in_proximity = Vec::with_capacity(MAX_CHANNELS);

    // A buffer for collecting frames from `Sound`s that have not yet been mixed and written.
    let mut unmixed_samples = vec![0.0f32; 1024];

    // Wait for messages.
    for msg in msg_rx {
        match msg {
            Message::RequestAudio(mut buffer, _sample_hz) => {

                // For each sound, request `buffer.len()` number of frames and sum them onto the
                // relevant output channels.
                for (&_sound_id, sound) in &mut sounds {
                    let num_samples = buffer.len() * sound.channels;

                    unmixed_samples.clear();
                    {
                        let signal = (0..num_samples).map(|_| sound.signal.next()[0]);
                        unmixed_samples.extend(signal);
                    }

                    // Mix the audio from the signal onto each of the output channels.
                    for i in 0..sound.channels {

                        // Find the absolute position of the channel.
                        let channel_point =
                            channel_point(sound.point, i, sound.channels, sound.spread, sound.radians);

                        // Find the speakers that are closest to the channel.
                        find_closest_speakers(&channel_point, &mut speakers_in_proximity, &speakers);
                        let mut sample_index = i;
                        for frame in buffer.iter_mut() {
                            let channel_sample = unmixed_samples[sample_index];
                            for &(amp, channel) in &speakers_in_proximity {
                                frame[channel] += channel_sample * amp;
                            }
                            sample_index += sound.channels;
                        }
                    }
                }

                buffer.submit().ok();
            },

            Message::AddSound(id, sound) => {
                sounds.insert(id, sound);
            },

            Message::RemoveSound(id) => {
                sounds.remove(&id);
            },

            Message::UpdateSpeaker(id, speaker) => {
                speakers.insert(id, speaker);
            },

            Message::RemoveSpeaker(id) => {
                speakers.remove(&id);
            },

            Message::Exit => break,
        }
    }
}

pub fn channel_point(
    sound_point: Point2<Metres>,
    channel_index: usize,
    total_channels: usize,
    spread: Metres,
    radians: f32,
) -> Point2<Metres>
{
    assert!(channel_index < total_channels);
    if total_channels == 1 {
        sound_point
    } else {
        let phase = channel_index as f32 / total_channels as f32;
        let default_radians = phase * std::f32::consts::PI * 2.0;
        let radians = (radians + default_radians) as f64;
        let rel_x = Metres(-radians.cos() * spread.0);
        let rel_y = Metres(radians.sin() * spread.0);
        let x = sound_point.x + rel_x;
        let y = sound_point.y + rel_y;
        Point2 { x, y }
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
    closest: &mut Vec<(Amplitude, usize)>, // Amplitude along with the speaker's channel index.
    speakers: &HashMap<speaker::Id, Speaker>,
) {
    closest.clear();
    let point_f = Point2 { x: point.x.0, y: point.y.0 };
    for (_, speaker) in speakers.iter() {
        let speaker_point_f = Point2 { x: speaker.point.x.0, y: speaker.point.y.0 };
        let distance_2 = Metres(point_f.distance2(speaker_point_f));
        if distance_2 < PROXIMITY_LIMIT_2 {
            // Use a function to map distance to amp.
            let amp = distance_2_to_amplitude(distance_2);
            closest.push((amp, speaker.channel));
        }
    }
}
