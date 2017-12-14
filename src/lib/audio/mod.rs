use gui;
use metres::Metres;
use nannou;
use nannou::audio::Buffer;
use nannou::math::{MetricSpace, Point2};
use osc;
use std;
use std::collections::HashMap;
use std::sync::mpsc;
pub use self::detector::Detector;
pub use self::sound::Sound;
pub use self::source::Source;
pub use self::speaker::Speaker;
pub use self::wav::Wav;

pub mod detector;
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

/// The desired sample rate of the output stream.
pub const SAMPLE_RATE: f64 = 44_100.0;

/// The desired number of frames requested at a time.
pub const FRAMES_PER_BUFFER: usize = 64;

/// Simplified type alias for the nannou audio output stream used by the audio server.
pub type OutputStream = nannou::audio::stream::Output<Model>;

/// A sound that is currently active on the audio thread.
pub struct ActiveSound {
    sound: Sound,
    channel_detectors: Box<[Detector]>,
}

pub struct ActiveSpeaker {
    speaker: Speaker,
    detector: Detector,
}

impl ActiveSound {
    /// Create a new `ActiveSound`.
    pub fn new(sound: Sound) -> Self {
        let channel_detectors = (0..sound.channels)
            .map(|_| Detector::new())
            .collect::<Vec<_>>()
            .into_boxed_slice();
        ActiveSound { sound, channel_detectors }
    }
}

impl From<Sound> for ActiveSound {
    fn from(sound: Sound) -> Self {
        ActiveSound::new(sound)
    }
}

struct SpeakerAnalysis {
    rms: f32,
    peak: f32,
    index: usize,
}

/// State that lives on the audio thread.
pub struct Model {
    /// A map from audio sound IDs to the audio sounds themselves.
    sounds: HashMap<sound::Id, ActiveSound>,
    /// A map from speaker IDs to the speakers themselves.
    speakers: HashMap<speaker::Id, ActiveSpeaker>,
    /// A buffer for collecting the speakers within proximity of the sound's position.
    speakers_in_proximity: Vec<(Amplitude, usize)>,
    /// A buffer for collecting the speakers within proximity of the sound's position.
    unmixed_samples: Vec<f32>,
    /// A buffer for collecting sounds that have been removed due to completing.
    exhausted_sounds: Vec<sound::Id>,
    /// Channel for communicating active sound info to the GUI.
    gui_audio_monitor_msg_tx: mpsc::SyncSender<gui::AudioMonitorMessage>,
    /// Channel for sending sound analysis data to the OSC output thread.
    osc_output_msg_tx: mpsc::Sender<osc::output::Message>,
    /// An analysis per installation to re-use for sending to the OSC output thread.
    installation_analyses: HashMap<osc::output::Installation, Vec<SpeakerAnalysis>>,
}

impl Model {
    /// Initialise the `Model`.
    pub fn new(
        gui_audio_monitor_msg_tx: mpsc::SyncSender<gui::AudioMonitorMessage>,
        osc_output_msg_tx: mpsc::Sender<osc::output::Message>,
    ) -> Self {
        // A map from audio sound IDs to the audio sounds themselves.
        let sounds: HashMap<sound::Id, ActiveSound> = HashMap::with_capacity(1024);

        // A map from speaker IDs to the speakers themselves.
        let speakers: HashMap<speaker::Id, ActiveSpeaker> = HashMap::with_capacity(MAX_CHANNELS);

        // A buffer for collecting the speakers within proximity of the sound's position.
        let speakers_in_proximity = Vec::with_capacity(MAX_CHANNELS);

        // A buffer for collecting frames from `Sound`s that have not yet been mixed and written.
        let unmixed_samples = vec![0.0; 1024];

        // A buffer for collecting exhausted `Sound`s.
        let exhausted_sounds = Vec::with_capacity(128);

        // A map from installations to audio analysis frames that can be re-used.
        let installation_analyses = HashMap::with_capacity(64);

        Model {
            sounds,
            speakers,
            speakers_in_proximity,
            unmixed_samples,
            exhausted_sounds,
            installation_analyses,
            gui_audio_monitor_msg_tx,
            osc_output_msg_tx,
        }
    }

    /// Inserts the speaker and sends an `Add` message to the GUI.
    pub fn insert_speaker(&mut self, id: speaker::Id, speaker: Speaker) -> Option<Speaker> {
        // Re-use the old detector if there is one.
        let (detector, old_speaker) = match self.speakers.remove(&id) {
            None => (Detector::new(), None),
            Some(ActiveSpeaker { speaker, detector }) => (detector, Some(speaker)),
        };

        // TODO: Update `installation_analyses` if speaker's installation is new.
        let installation = osc::output::Installation::Cacophony;
        self.installation_analyses.entry(installation).or_insert_with(Vec::new);

        let speaker = ActiveSpeaker { speaker, detector };
        let speaker_msg = gui::SpeakerMessage::Add;
        let msg = gui::AudioMonitorMessage::Speaker(id, speaker_msg);
        self.gui_audio_monitor_msg_tx.try_send(msg).ok();
        self.speakers.insert(id, speaker);
        old_speaker
    }

    /// Removes the speaker and sens a `Removed` message to the GUI.
    pub fn remove_speaker(&mut self, id: speaker::Id) -> Option<Speaker> {
        let removed = self.speakers.remove(&id);
        if removed.is_some() {
            let speaker_msg = gui::SpeakerMessage::Remove;
            let msg = gui::AudioMonitorMessage::Speaker(id, speaker_msg);
            self.gui_audio_monitor_msg_tx.try_send(msg).ok();
        }
        removed.map(|ActiveSpeaker { speaker, .. }| speaker)
    }

    // /// Mutable access to the speaker at the given Id.
    // pub fn speaker_mut(&mut self, id: &speaker::Id) -> Option<&mut Speaker> {
    //     self.speakers.get_mut(id).map(|active| &mut active.speaker)
    // }

    /// Inserts the sound and sends an `Start` active sound message to the GUI.
    pub fn insert_sound(&mut self, id: sound::Id, sound: ActiveSound) -> Option<ActiveSound> {
        let position = sound.sound.point;
        let channels = sound.sound.channels;
        let source_id = sound.sound.source_id;
        let sound_msg = gui::ActiveSoundMessage::Start { source_id, position, channels };
        let msg = gui::AudioMonitorMessage::ActiveSound(id, sound_msg);
        self.gui_audio_monitor_msg_tx.try_send(msg).ok();
        self.sounds.insert(id, sound)
    }

    /// Removes the sound and sends an `End` active sound message to the GUI.
    pub fn remove_sound(&mut self, id: sound::Id) -> Option<ActiveSound> {
        let removed = self.sounds.remove(&id);
        if removed.is_some() {
            let sound_msg = gui::ActiveSoundMessage::End;
            let msg = gui::AudioMonitorMessage::ActiveSound(id, sound_msg);
            self.gui_audio_monitor_msg_tx.try_send(msg).ok();
        }
        removed
    }

    /// Mutable access to the sound at the given Id.
    pub fn sound_mut(&mut self, id: &sound::Id) -> Option<&mut Sound> {
        self.sounds.get_mut(id).map(|active| &mut active.sound)
    }
}

/// The function given to nannou to use for rendering.
pub fn render(mut model: Model, mut buffer: Buffer) -> (Model, Buffer) {
    {
        let Model {
            ref mut sounds,
            ref mut speakers_in_proximity,
            ref mut unmixed_samples,
            ref mut exhausted_sounds,
            ref mut installation_analyses,
            ref mut speakers,
            ref gui_audio_monitor_msg_tx,
            ref osc_output_msg_tx,
        } = model;

        // For each sound, request `buffer.len()` number of frames and sum them onto the
        // relevant output channels.
        for (&sound_id, active_sound) in sounds.iter_mut() {
            let ActiveSound { ref mut sound, ref mut channel_detectors } = *active_sound;

            // The number of samples to request from the sound for this buffer.
            let num_samples = buffer.len_frames() * sound.channels;

            // Clear the unmixed samples, ready to collect the new ones.
            unmixed_samples.clear();
            {
                let mut samples_written = 0;
                for sample in sound.signal.by_ref().take(num_samples) {
                    unmixed_samples.push(sample);
                    channel_detectors[samples_written % sound.channels].next(sample);
                    samples_written += 1;
                }

                // If we didn't write the expected number of samples, the sound has been exhausted.
                if samples_written < num_samples {
                    exhausted_sounds.push(sound_id);
                    for _ in samples_written..num_samples {
                        unmixed_samples.push(0.0);
                    }
                }

                // Send the latest RMS and peak for each channel to the GUI for monitoring.
                for (index, detector) in channel_detectors.iter().enumerate() {
                    let (rms, peak) = detector.current();
                    let sound_msg = gui::ActiveSoundMessage::UpdateChannel { index, rms, peak };
                    let msg = gui::AudioMonitorMessage::ActiveSound(sound_id, sound_msg);
                    gui_audio_monitor_msg_tx.try_send(msg).ok();
                }
            }

            // Mix the audio from the signal onto each of the output channels.
            for i in 0..sound.channels {

                // Find the absolute position of the channel.
                let channel_point =
                    channel_point(sound.point, i, sound.channels, sound.spread, sound.radians);

                // Find the speakers that are closest to the channel.
                find_closest_speakers(&channel_point, &speakers, speakers_in_proximity);
                let mut sample_index = i;
                for frame in buffer.frames_mut() {
                    let channel_sample = unmixed_samples[sample_index];
                    for &(amp, channel) in speakers_in_proximity.iter() {
                        // Only write to the channels that will be read by the audio device.
                        if let Some(sample) = frame.get_mut(channel) {
                            *sample += channel_sample * amp;
                        }
                    }
                    sample_index += sound.channels;
                }
            }
        }

        // For each speaker, feed its amplitude into its detector.
        let n_channels = buffer.channels();
        let mut sum_peak = 0.0;
        let mut sum_rms = 0.0;
        for (&id, active) in speakers.iter_mut() {
            let mut channel_i = active.speaker.channel;
            if channel_i >= n_channels {
                continue;
            }
            let detector = &mut active.detector;
            for frame in buffer.frames() {
                detector.next(frame[channel_i]);
            }

            // The current detector state.
            let (rms, peak) = detector.current();

            // Send the detector state for the speaker to the GUI.
            let speaker_msg = gui::SpeakerMessage::Update { rms, peak };
            let msg = gui::AudioMonitorMessage::Speaker(id, speaker_msg);
            gui_audio_monitor_msg_tx.try_send(msg).ok();

            // Sum the rms and peak.
            // TODO: Get installation associated with speaker.
            let installation = osc::output::Installation::Cacophony;
            //let installation = active.speaker.installation;
            if let Some(speakers) = installation_analyses.get_mut(&installation) {
                sum_peak += peak;
                sum_rms += rms;
                let analysis = SpeakerAnalysis { peak, rms, index: channel_i };
                speakers.push(analysis);
            }
        }

        // Send the collected analysis to the OSC output thread.
        for (&installation, speakers) in installation_analyses.iter_mut() {
            speakers.sort_by(|a, b| a.index.cmp(&b.index));
            {
                let avg_peak = sum_peak / speakers.len() as f32;
                let avg_rms = sum_rms / speakers.len() as f32;
                let avg_fft = osc::output::FftData { lmh: [0.0; 3], bins: [0.0; 8] };
                let speakers = speakers.iter()
                    .map(|s| osc::output::Speaker { rms: s.rms, peak: s.peak })
                    .collect();
                let data = osc::output::AudioFrameData { avg_peak, avg_rms, avg_fft, speakers };
                let msg = osc::output::Message::Audio(installation, data);
                osc_output_msg_tx.send(msg).ok();
            }
            // Clear the speakers for the next loop.
            speakers.clear();
        }

        // Remove all sounds that have been exhausted.
        for sound_id in exhausted_sounds.drain(..) {
            // TODO: Possibly send this with the `End` message to avoid de-allocating on audio
            // thread.
            let _sound = sounds.remove(&sound_id).unwrap();
            // Send signal of completion back to GUI/Composer threads.
            let sound_msg = gui::ActiveSoundMessage::End;
            let msg = gui::AudioMonitorMessage::ActiveSound(sound_id, sound_msg);
            gui_audio_monitor_msg_tx.try_send(msg).ok();
        }
    }

    (model, buffer)
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

/// Tests whether or not the given speaker position is within the `PROXIMITY_LIMIT` distance of the
/// given `point` (normally a `Sound`'s channel position). If so, returns the amplitude multiplier
/// for the sound being delivered from the `point` to the `speaker`.
pub fn speaker_is_in_range(point: &Point2<Metres>, speaker: &Point2<Metres>) -> Option<f32> {
    // let speaker = &active.speaker;
    // let speaker_point_f = Point2 { x: speaker.point.x.0, y: speaker.point.y.0 };
    let point_f = Point2 { x: point.x.0, y: point.y.0 };
    let speaker_f = Point2 { x: speaker.x.0, y: speaker.y.0 };
    let distance_2 = Metres(point_f.distance2(speaker_f));
    if distance_2 < PROXIMITY_LIMIT_2 {
        // Use a function to map distance to amp.
        let amp = distance_2_to_amplitude(distance_2);
        return Some(amp);
    }
    None
}

fn find_closest_speakers(
    point: &Point2<Metres>,
    speakers: &HashMap<speaker::Id, ActiveSpeaker>,
    closest: &mut Vec<(Amplitude, usize)>, // Amplitude along with the speaker's channel index.
) {
    closest.clear();
    for (_, active) in speakers.iter() {
        if let Some(amp) = speaker_is_in_range(point, &active.speaker.point) {
            closest.push((amp, active.speaker.channel));
        }
    }
}
