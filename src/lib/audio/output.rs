//! The audio render function implementation.
//!
//! The render function is passed to `nannou::App`'s build output stream method and describes how
//! audio should be rendered to the output.

use audio::{DISTANCE_BLUR, FRAMES_PER_BUFFER, MAX_CHANNELS, MAX_SOUNDS, PROXIMITY_LIMIT_2};
use audio::{Sound, Speaker};
use audio::{dbap, detection, source, sound, speaker};
use fxhash::{FxHashMap, FxHashSet};
use gui;
use installation;
use metres::Metres;
use nannou;
use nannou::audio::Buffer;
use nannou::math::{MetricSpace, Point2};
use osc;
use soundscape;
use std;
use std::ops::{self, Deref, DerefMut};
use std::sync::{atomic, mpsc, Arc};
use std::sync::atomic::AtomicUsize;
use time_calc::Samples;
use utils;

/// Simplified type alias for the nannou audio output stream used by the audio server.
pub type Stream = nannou::audio::Stream<Model>;

type Channel = usize;

// The most recently recorded DBAP speaker gain for each speaker per active sound.
//
// TODO: Should possibly move these into their associated `ActiveSound`s - will be easier to track
// removal etc this way.
type DbapSpeakerGains = FxHashMap<sound::Id, FxHashMap<Channel, FxHashMap<speaker::Id, f32>>>;

/// A sound that is currently active on the audio thread.
pub struct ActiveSound {
    sound: Sound,
    total_duration_frames: Option<Samples>,
}

/// A speaker that is currently active on the audio thread.
pub struct ActiveSpeaker {
    speaker: Speaker,
}

/// Information relevant to a single `Sound` for the duration of a `render` pass.
struct SoundOrdered {
    /// The unique identifier associated with this `Sound`.
    id: sound::Id,
    /// Samples retrieved from the sound for the duration of the current `render` output buffer.
    unmixed_samples: Vec<f32>,
    /// The number of channels in the sound.
    channels: usize,
}

/// Information about a single channel within a single sound.
///
/// The `render` function collects a `Vec` of these to improve efficiency of writing to the output
/// buffer.
struct SoundChannel {
    /// The index into the `sounds_ordered` vec for this channel's sound.
    sound_index: usize,
    /// The index of the channel within the sound.
    sound_channel_index: usize,
    /// The index range into the speaker_infos vec for this channel.
    speaker_infos_range: ops::Range<usize>,
}

/// Information about a single "DBAP" speaker relevant to a single sound channel.
///
/// The `render` function collects a `Vec` of these to improve efficiency of writing to the output
/// buffer.
struct DbapSpeakerInfo {
    /// The last known gain for the speaker for this channel.
    previous_gain: f32,
    /// The current gain for the speaker for this channel.
    current_gain: f32,
    /// The output buffer channel associated with this speaker.
    output_channel: usize,
}

impl ActiveSound {
    /// Create a new `ActiveSound`.
    pub fn new(sound: Sound) -> Self {
        let total_duration_frames = sound.signal.remaining_frames();
        ActiveSound {
            sound,
            total_duration_frames,
        }
    }

    /// The normalised progress through playback.
    pub fn normalised_progress(&self) -> Option<f64> {
        let remaining_duration = self.signal.remaining_frames();
        let total_duration = self.total_duration_frames;
        let normalised_progress = match (remaining_duration, total_duration) {
            (Some(Samples(remaining)), Some(Samples(total))) => {
                let current_frame = total - remaining;
                Some(current_frame as f64 / total as f64)
            },
            _ => None,
        };
        normalised_progress
    }
}

impl From<Sound> for ActiveSound {
    fn from(sound: Sound) -> Self {
        ActiveSound::new(sound)
    }
}

impl Deref for ActiveSound {
    type Target = Sound;
    fn deref(&self) -> &Self::Target {
        &self.sound
    }
}

impl DerefMut for ActiveSound {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.sound
    }
}

impl Deref for ActiveSpeaker {
    type Target = Speaker;
    fn deref(&self) -> &Self::Target {
        &self.speaker
    }
}

impl DerefMut for ActiveSpeaker {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.speaker
    }
}

/// State that lives on the audio thread.
pub struct Model {
    /// the total number of frames written since the model was created or the project was switched.
    ///
    /// this is used for synchronising `continuous` wavs to the audio timeline with sample-perfect
    /// accuracy.
    pub frame_count: Arc<AtomicUsize>,
    /// Indicates whether or not CPU saving mode is currently enabled.
    ///
    /// If so, envelope detection will be skipped as currently its only role is for GUI feedback
    /// and it is showing up as the primary bottleneck on the audio output thread.
    ///
    /// NOTE: If using this server in the future and you actually want to use peak and RMS values
    /// via OSC, remove this flag so that env detection is performed despite whether or not cpu
    /// saving mode is enabled.
    pub cpu_saving_enabled: bool,
    /// the master volume, controlled via the gui applied at the very end of processing.
    pub master_volume: f32,
    /// the dbap rolloff decibel amount, used to attenuate speaker gains over distances.
    pub dbap_rolloff_db: f64,
    /// the set of sources that are currently soloed. if not empty, only these sounds should play.
    pub soloed: FxHashSet<source::Id>,
    /// a map from audio sound ids to the audio sounds themselves.
    sounds: FxHashMap<sound::Id, ActiveSound>,
    /// a map from speaker ids to the speakers themselves.
    speakers: FxHashMap<speaker::Id, ActiveSpeaker>,

    /// Used for collecting all `sound::Id`s within the sound map into an ordered list.
    ///
    /// Also stores all the unmixed samples for each sound.
    sounds_ordered: Vec<SoundOrdered>,
    /// Used for collecting a `SoundChannel` for every channel in every sound.
    sound_channels: Vec<SoundChannel>,
    /// Used for collecting a `DbapSpeakerInfo` for every speaker reached by every channel in every
    /// sound.
    dbap_speaker_infos: Vec<DbapSpeakerInfo>,

    // /// A map from a speaker's assigned channel to the ID of the speaker.
    // channel_to_speaker: FxHashMap<usize, speaker::Id>,
    /// A buffer for collecting sounds that have been removed due to completing.
    exhausted_sounds: Vec<sound::Id>,

    /// Inter-thread communication channels.
    channels: Channels,

    /// A map that tracks the last calculated buffer's DBAP gain per speaker per sound.
    ///
    /// This allows for linearly interpolating from the speaker gains of the previous buffer to the
    /// gains for the current buffer to avoid clipping.
    dbap_speaker_gains: DbapSpeakerGains,
    /// A buffer to re-use for collecting speakers ready for performing the DBAP calc.
    ///
    /// Only those speakers that are within the `audio::PROXIMITY_LIMIT` will be collected.
    dbap_speakers: Vec<dbap::Speaker>,

}

struct Channels {
    /// Channel for communicating with the audio detection thread.
    detection: detection::Handle,
    /// Channel for communicating active sound info to the GUI.
    gui_audio_monitor_msg_tx: gui::monitor::Sender,
    /// A handle to the soundscape thread - for notifying when a sound is complete.
    soundscape_tx: mpsc::Sender<soundscape::Message>,
    /// A handle to the wav_reader thread - for notifying when a sound has ended.
    wav_reader: source::wav::reader::Handle,
}

/// An iterator yielding all `Sound`s in the model.
pub struct SoundsMut<'a> {
    iter: std::collections::hash_map::IterMut<'a, sound::Id, ActiveSound>,
}

impl<'a> Iterator for SoundsMut<'a> {
    type Item = (&'a sound::Id, &'a mut Sound);
    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next().map(|(id, active)| (id, &mut active.sound))
    }
}

impl Model {
    /// Initialise the `Model`.
    pub fn new(
        frame_count: Arc<AtomicUsize>,
        gui_audio_monitor_msg_tx: gui::monitor::Sender,
        osc_output_msg_tx: mpsc::Sender<osc::output::Message>,
        soundscape_tx: mpsc::Sender<soundscape::Message>,
        wav_reader: source::wav::reader::Handle,
    ) -> Self {
        // Spawn the audio detection thread.
        let detection = detection::spawn(gui_audio_monitor_msg_tx.clone(), osc_output_msg_tx);

        // The currently soloed sources (none by default).
        let soloed = Default::default();

        // A map from audio sound IDs to the audio sounds themselves.
        let sounds = Default::default();

        // A map from speaker IDs to the speakers themselves.
        let speakers = Default::default();

        // Pre-allocate the `sounds_ordered` buffer.
        //
        // This just uses the first sound `Id` for every buffer for now (this will be overwritten
        // at the beginning of every call to `render`).
        let sounds_ordered = (0..MAX_SOUNDS)
            .map(|_| SoundOrdered {
                id: sound::Id::INITIAL,
                unmixed_samples: vec![0.0; FRAMES_PER_BUFFER * 2],
                channels: 0,
            })
            .collect();

        // Pre-allocate a buffer for storing sound channels.
        //
        // We do a rough estimation using `MAX_SOUNDS` with a stereo number of channels.
        let sound_channels = Vec::with_capacity(MAX_SOUNDS * 2);

        // Pre-allocate a buffer for storing dbap_speaker_infos.
        //
        // We do a rough estimation using `MAX_SOUNDS` with a stereo number of channels with the
        // `MAX_CHANNELS` number of speakers.
        let dbap_speaker_infos = Vec::with_capacity(MAX_SOUNDS * 2 * MAX_CHANNELS);

        // A buffer for collecting exhausted `Sound`s.
        let exhausted_sounds = Vec::with_capacity(128);

        // For tracking DBAP speaker gains.
        let dbap_speaker_gains = FxHashMap::default();
        let dbap_speakers = Vec::with_capacity(MAX_CHANNELS);
        // Initialise the master volume to the default value.
        let master_volume = super::DEFAULT_MASTER_VOLUME;

        // Initialise the rolloff to the default value.
        let dbap_rolloff_db = super::DEFAULT_DBAP_ROLLOFF_DB;

        // By default, cpu saving mode is not enabled.
        let cpu_saving_enabled = false;

        let channels = Channels {
            detection,
            gui_audio_monitor_msg_tx,
            soundscape_tx,
            wav_reader,
        };

        Model {
            frame_count,
            cpu_saving_enabled,
            master_volume,
            dbap_rolloff_db,
            soloed,
            sounds,
            sounds_ordered,
            sound_channels,
            dbap_speaker_infos,
            speakers,
            exhausted_sounds,
            channels,
            dbap_speaker_gains,
            dbap_speakers,
        }
    }

    /// Insert an installation for the given `Id`.
    ///
    /// Returns `true` if the installation did not yet exist or false otherwise.
    pub fn insert_installation(&mut self, id: installation::Id, computers: usize) {
        self.channels.detection.add_installation(id, computers);
    }

    /// Remove the installation at the given `Id`.
    ///
    /// Also removes the installation from all speakers that have been assigned to it.
    ///
    /// Returns `true` if the installation was successfully removed.
    ///
    /// Returns `false` if there was no installation for the given `Id`.
    pub fn remove_installation(&mut self, id: &installation::Id) {
        self.channels.detection.remove_installation(*id);

        // Remove the installation from any speakers.
        for speaker in self.speakers.values_mut() {
            speaker.installations.remove(id);
        }

        // Remove the installation from any sounds.
        for sound in self.sounds.values_mut() {
            if let sound::Installations::Set(ref mut set) = sound.installations {
                set.remove(id);
            }
        }
    }

    /// Inserts the speaker and sends an `Add` message to the GUI.
    pub fn insert_speaker(&mut self, id: speaker::Id, speaker: Speaker) -> Option<Speaker> {
        let old_speaker = self.speakers
            .remove(&id)
            .map(|ActiveSpeaker { speaker }| speaker);
        let speaker = ActiveSpeaker { speaker };
        let speaker_msg = gui::SpeakerMessage::Add;
        let msg = gui::AudioMonitorMessage::Speaker(id, speaker_msg);
        self.channels.gui_audio_monitor_msg_tx.send(msg).ok();
        self.speakers.insert(id, speaker);
        old_speaker
    }

    /// Removes the speaker and sends a `Removed` message to the GUI.
    pub fn remove_speaker(&mut self, id: speaker::Id) -> Option<Speaker> {
        let removed = self.speakers
            .remove(&id)
            .map(|ActiveSpeaker { speaker }| speaker);
        if removed.is_some() {
            let speaker_msg = gui::SpeakerMessage::Remove;
            let msg = gui::AudioMonitorMessage::Speaker(id, speaker_msg);
            self.channels.gui_audio_monitor_msg_tx.send(msg).ok();
        }
        removed
    }

    /// Inserts the installation into the speaker with the given `speaker::Id`.
    pub fn insert_speaker_installation(&mut self, id: speaker::Id, inst: installation::Id) -> bool {
        self.speakers
            .get_mut(&id)
            .map(|active| active.speaker.installations.insert(inst))
            .unwrap_or(false)
    }

    /// Removes the installation from the speaker with the given `speaker::Id`.
    pub fn remove_speaker_installation(&mut self, id: speaker::Id, inst: &installation::Id) -> bool {
        self.speakers
            .get_mut(&id)
            .map(|active| active.speaker.installations.remove(inst))
            .unwrap_or(false)
    }

    /// Inserts the sound and sends a `Start` active sound message to the GUI.
    pub fn insert_sound(&mut self, id: sound::Id, sound: ActiveSound) -> Option<ActiveSound> {
        let position = sound.position;
        let channels = sound.channels;
        let source_id = sound.source_id();
        let normalised_progress = sound.normalised_progress();

        // Notify the GUI monitor that a sound has started.
        let sound_msg = gui::ActiveSoundMessage::Start {
            source_id,
            position,
            channels,
            normalised_progress,
        };
        let msg = gui::AudioMonitorMessage::ActiveSound(id, sound_msg);
        self.channels.gui_audio_monitor_msg_tx.send(msg).ok();

        // Notify the detection thread that a new sound has been added.
        self.channels.detection.add_sound(id, channels);

        self.sounds.insert(id, sound)
    }

    /// Update the sound associated with the given Id by applying the given function to it.
    pub fn update_sound<F>(&mut self, id: &sound::Id, update: F) -> bool
    where
        F: FnOnce(&mut Sound),
    {
        match self.sounds.get_mut(id) {
            None => false,
            Some(active) => {
                update(&mut active.sound);
                true
            },
        }
    }

    /// Update all sounds that are produced by the source type with the given `Id`.
    ///
    /// Returns the number of sounds that were updated.
    pub fn update_sounds_with_source<F>(&mut self, id: &source::Id, mut update: F) -> usize
    where
        F: FnMut(&sound::Id, &mut Sound),
    {
        let mut count = 0;
        for (id, sound) in self.sounds_mut().filter(|&(_, ref s)| s.source_id() == *id) {
            update(id, sound);
            count += 1;
        }
        count
    }

    /// Remove all sounds that were spawned via the source with the given `Id`.
    ///
    /// Returns the number of sounds that were updated.
    pub fn remove_sounds_with_source(&mut self, id: &source::Id) -> usize {
        let ids: Vec<_> = self.sounds.iter()
            .filter(|&(_, s)| s.source_id() == *id)
            .map(|(&id, _)| id)
            .collect();
        let count = ids.len();
        for id in ids {
            self.remove_sound(id);
        }
        count
    }

    /// Removes the sound and sends an `End` active sound message to the GUI.
    ///
    /// Also removes the sound from DBAP tracking.
    ///
    /// Returns `false` if the sound did not exist
    pub fn remove_sound(&mut self, id: sound::Id) -> bool {
        let removed = self.sounds.remove(&id);
        if let Some(sound) = removed {
            // Remove the sound from DBAP gain tracking.
            self.dbap_speaker_gains.remove(&id);
            // Notify threads.
            self.channels.notify_sound_end(id, sound);
            true
        } else {
            false
        }
    }

    /// An iterator yielding mutable access to all sounds currently playing.
    pub fn sounds_mut(&mut self) -> SoundsMut {
        let iter = self.sounds.iter_mut();
        SoundsMut { iter }
    }

    /// Clear all data related to a specific audio server project.
    ///
    /// This is called when we switch between projects within the GUI.
    pub fn clear_project_specific_data(&mut self) {
        self.channels.detection.clear_project_specific_data();
        self.frame_count.store(0, atomic::Ordering::Relaxed);
        self.soloed.clear();
        self.speakers.clear();

        let Model { ref mut sounds, ref channels, .. } = *self;
        for (sound_id, sound) in sounds.drain() {
            // Notify threads of sound removal.
            channels.notify_sound_end(sound_id, sound);
        }
    }
}

impl Channels {
    fn notify_sound_end(&self, id: sound::Id, sound: ActiveSound) {
        // GUI thread.
        let sound_msg = gui::ActiveSoundMessage::End { sound };
        let msg = gui::AudioMonitorMessage::ActiveSound(id, sound_msg);
        self.gui_audio_monitor_msg_tx.send(msg).ok();

        // Soundscape thread.
        let update = move |soundscape: &mut soundscape::Model| {
            soundscape.remove_active_sound(&id);
        };
        self.soundscape_tx.send(soundscape::UpdateFn::from(update).into()).ok();

        // Detection thread.
        self.detection.remove_sound(id);

        // WAV reader thread.
        self.wav_reader.end(id);
    }
}

/// A simple linear interpolation function.
///
/// This is used to interpolate between previous and current DBAP speaker gains over the duration
/// of the buffer.
fn lerp(a: f32, b: f32, lerp: f32) -> f32 {
    a + (b - a) * lerp
}

/// The function given to nannou to use for rendering.
pub fn render(mut model: Model, mut buffer: Buffer) -> (Model, Buffer) {
    {
        let Model {
            master_volume,
            cpu_saving_enabled,
            dbap_rolloff_db,
            ref soloed,
            ref mut frame_count,
            ref mut sounds,
            ref mut sounds_ordered,
            ref mut sound_channels,
            ref mut dbap_speaker_infos,
            ref mut exhausted_sounds,
            ref mut speakers,
            ref mut dbap_speaker_gains,
            ref mut dbap_speakers,
            ref channels,
        } = model;

        // Always silence the buffer to begin.
        buffer.iter_mut().for_each(|s| *s = 0.0);

        // Update the map from buffer channels to their speakers.
        //
        // Only track speakers whose channels are valid for the current buffer.
        //
        // TODO: Should probably move this into model for re-use, but its not showing up in
        // profiling.
        let channels_to_speakers: FxHashMap<_, _> = speakers
            .iter()
            .filter_map(|(&id, s)| {
                if s.channel < buffer.channels() {
                    Some((s.channel, id))
                } else {
                    None
                }
            })
            .collect();

        // Retrieve the total number of sounds so we know how long we should slice
        // `sounds_ordered`.
        let num_sounds = sounds.len();

        // Slice only the range that we need from `sounds_ordered`.
        let sounds_ordered = &mut sounds_ordered[..num_sounds];

        // Write the `sound::Id`s from the `sounds` map to the `sounds_ordered` `Vec`.
        //
        // We just need consistency for the rest of the function, the actual order does not matter.
        for (ordered_sound, &sound_id) in sounds_ordered.iter_mut().zip(sounds.keys()) {
            ordered_sound.id = sound_id;
            ordered_sound.unmixed_samples.clear();
        }

        // Clear the channel sounds buffer.
        sound_channels.clear();
        dbap_speaker_infos.clear();

        // For each sound, request `buffer.len()` number of frames and push them to the sound's
        // `unmixed_sounds` buffer.
        for (sound_i, ordered_sound) in sounds_ordered.iter_mut().enumerate() {
            let sound_id = ordered_sound.id;
            let sound = sounds.get_mut(&sound_id).expect("no sound for the given `Id`");

            // Update the ordered sound.
            ordered_sound.channels = sound.channels;

            // Update the GUI with the position of the sound.
            let source_id = sound.source_id();
            let position = sound.position;
            let n_channels = sound.channels;
            let normalised_progress = sound.normalised_progress();
            let update = gui::ActiveSoundMessage::Update {
                source_id,
                position,
                channels: n_channels,
                normalised_progress,
            };
            let msg = gui::AudioMonitorMessage::ActiveSound(sound_id, update);
            channels.gui_audio_monitor_msg_tx.send(msg).ok();

            let ActiveSound {
                ref mut sound,
                ..
            } = *sound;

            // The number of samples to request from the sound for this buffer.
            let num_samples = buffer.len_frames() * sound.channels;

            // Don't play or request samples if paused.
            if !sound.shared.is_playing() {
                continue;
            }

            // Don't play the sound if:
            //
            // - There are no speakers.
            // - The source is muted.
            // - Some other source(s) is/are soloed.
            let play_condition = speakers.is_empty()
                || sound.muted
                || (!soloed.is_empty() && !soloed.contains(&sound.source_id()));
            if play_condition {
                // Pull samples from the signal but do not render them.
                let samples_yielded = sound.signal.samples().take(num_samples).count();
                if samples_yielded < num_samples {
                    exhausted_sounds.push(sound_id);
                }
                continue;
            }

            // Collect the samples from the `Sound`'s `Signal`.
            {
                let mut samples_written = 0;
                for sample in sound.signal.samples().take(num_samples) {
                    let sample = sample * sound.volume;
                    ordered_sound.unmixed_samples.push(sample);
                    samples_written += 1;
                }

                // If CPU saving is not enabled, send the samples to the detector for analysis.
                if !cpu_saving_enabled {
                    let mut detection_buffer = channels.detection.pop_sound_buffer();
                    detection_buffer.extend(ordered_sound.unmixed_samples.iter().cloned());
                    channels.detection.update_sound(sound_id, detection_buffer, n_channels);
                }

                // If we didn't write the expected number of samples, the sound has been exhausted.
                if samples_written < num_samples {
                    exhausted_sounds.push(sound_id);
                    let remaining_silence = (samples_written..num_samples).map(|_| 0.0);
                    ordered_sound.unmixed_samples.extend(remaining_silence);
                }
            }

            // Mix the audio from the signal onto each of the output channels.
            if speakers.is_empty() {
                continue;
            }

            // Get the currently stored DBAP speaker gains for this sound.
            let dbap_speaker_gains = dbap_speaker_gains
                .entry(sound_id)
                .or_insert_with(FxHashMap::default);

            // Collect a `SoundChannel` for every channel in every sound.
            for (sound_channel, channel_point) in sound.channel_points().enumerate() {
                // Update the dbap_speakers buffer with their distances to this sound channel.
                dbap_speakers.clear();

                // Get the DBAP gains for this channel of the sound.
                let dbap_speaker_gains = dbap_speaker_gains
                    .entry(sound_channel)
                    .or_insert_with(FxHashMap::default);

                // Track the range of speaker infos associated with this channel.
                let speaker_infos_start = dbap_speaker_infos.len();

                for channel in 0..buffer.channels() {
                    // Find the speaker for this channel.
                    let speaker_id = match channels_to_speakers.get(&channel) {
                        Some(id) => id,
                        None => continue,
                    };
                    let active = &speakers[speaker_id];
                    let speaker_point = &active.speaker.point;

                    // Get the current gain by performing DBAP calc.
                    let channel_point_f = Point2 {
                        x: channel_point.x.0,
                        y: channel_point.y.0,
                    };
                    let speaker_point_f = Point2 {
                        x: speaker_point.x.0,
                        y: speaker_point.y.0,
                    };

                    // Get the squared distance between the channel and speaker.
                    let distance_2 = dbap::blurred_distance_2(
                        channel_point_f,
                        speaker_point_f,
                        DISTANCE_BLUR,
                    );

                    // If this speaker is not within proximity, skip it.
                    if PROXIMITY_LIMIT_2 < Metres(distance_2) {
                        continue;
                    }

                    // Weight the speaker based on whether or not it is assigned.
                    let weight = speaker::dbap_weight(
                        &sound.installations,
                        &active.speaker.installations,
                    );

                    // TODO: Possibly skip speakers with a weight of 0 (as below)?
                    // Uncertain how this will affect DBAP, but may drastically improve CPU.
                    // if weight == 0.0 {
                    //     continue;
                    // }

                    // Get the previous gain for this channel.
                    let previous_gain = dbap_speaker_gains
                        .get(speaker_id)
                        .map(|&g| g)
                        .unwrap_or(0.0);

                    // Temporarily set the `current_gain` for this `SpeakerInfo` to `0.0`.
                    //
                    // The correct value will be set in the `SpeakerGains` that follow this loop.
                    let current_gain = 0.0;
                    let output_channel = channel;

                    // Create the `DbapSpeakerInfo` relevant to this speaker for the sound channel.
                    let dbap_speaker_info = DbapSpeakerInfo {
                        previous_gain,
                        current_gain,
                        output_channel,
                    };

                    // Create the `dbap::Speaker` so that we may determine the current gain. This
                    // is done following this loop.
                    let speaker = dbap::Speaker { distance: distance_2, weight };
                    dbap_speakers.push(speaker);
                    dbap_speaker_infos.push(dbap_speaker_info);
                }

                // Create the speaker infos range.
                let speaker_infos_end = dbap_speaker_infos.len();
                let speaker_infos_range = speaker_infos_start..speaker_infos_end;

                // If no speakers were found, skip this channel.
                if dbap_speakers.is_empty() {
                    continue;
                }

                // Update the speaker gains.
                let current_gains = dbap::SpeakerGains::new(&dbap_speakers, dbap_rolloff_db);
                for (info_i, current_gain) in speaker_infos_range.clone().zip(current_gains) {
                    dbap_speaker_infos[info_i].current_gain = current_gain as _;
                }

                // Create the `SoundChannel` ready for mixing.
                let sound_channel = SoundChannel {
                    sound_index: sound_i,
                    sound_channel_index: sound_channel,
                    speaker_infos_range,
                };

                // Update the stored `dbap_speaker_gains` map for this sound channel.
                for info_i in sound_channel.speaker_infos_range.clone() {
                    let speaker_info = &dbap_speaker_infos[info_i];
                    let channel = speaker_info.output_channel;
                    let speaker_id = channels_to_speakers[&channel];
                    let current = speaker_info.current_gain;
                    *dbap_speaker_gains.entry(speaker_id).or_insert(current) = current;
                }

                sound_channels.push(sound_channel);
            }
        }

        // Sum the samples for all sound channels onto the output buffer at once.
        //
        // Iterate over each frame and track its index for gain interpolation.
        let frames_len = buffer.len_frames() as f32;
        for (frame_i, frame) in buffer.frames_mut().enumerate() {
            let lerp_amt = frame_i as f32 / frames_len;

            // Loop over each sound channel.
            for sound_channel in sound_channels.iter() {
                let SoundChannel {
                    // The index into the sounds_ordered vec for this channel's sound.
                    sound_index,
                    // The index of the channel within the sound.
                    sound_channel_index,
                    // The index range into the speaker_infos vec for this channel.
                    ref speaker_infos_range,
                } = *sound_channel;

                // Retrieve the unmixed sample for this channel at this frame.
                let sound = &sounds_ordered[sound_index];
                let channel_sample_index = frame_i * sound.channels + sound_channel_index;
                let channel_sample = sound.unmixed_samples[channel_sample_index];

                // Sum this sound channel onto each of the output channels for the nearby speakers.
                for speaker_info in &dbap_speaker_infos[speaker_infos_range.clone()] {
                    let DbapSpeakerInfo {
                        previous_gain,
                        current_gain,
                        output_channel,
                    } = *speaker_info;

                    let speaker_gain = lerp(previous_gain, current_gain, lerp_amt);
                    frame[output_channel] += channel_sample * speaker_gain;
                }
            }
        }

        // Send output buffer to detection thread for analysis.
        let (mut detection_buffer, mut output_info) = channels.detection.pop_output_buffer();
        detection_buffer.extend(buffer.iter().cloned());
        output_info.speakers.extend({
            speakers
                .iter()
                .map(|(&id, speaker)| {
                    let info = detection::SpeakerInfo {
                        channel: speaker.channel,
                        installations: speaker.installations.clone(),
                    };
                    (id, info)
                })
        });
        channels.detection.update_output(detection_buffer, buffer.channels(), output_info);

        // Remove all sounds that have been exhausted.
        for sound_id in exhausted_sounds.drain(..) {
            // Remove the sound from DBAP gain tracking.
            dbap_speaker_gains.remove(&sound_id);
            // Send this with the `End` message to avoid de-allocating on audio thread.
            let sound = sounds.remove(&sound_id).unwrap();
            // Notify the other threads.
            channels.notify_sound_end(sound_id, sound);
        }

        // Apply the master volume.
        for sample in buffer.iter_mut() {
            *sample *= master_volume;
        }

        // Find the peak amplitude and send it via the monitor channel.
        let peak = buffer.iter().fold(0.0, |peak, &s| s.max(peak));
        channels.gui_audio_monitor_msg_tx.send(gui::AudioMonitorMessage::Master { peak }).ok();

        // Step the frame count.
        frame_count.fetch_add(buffer.len_frames(), atomic::Ordering::Relaxed);
    }

    (model, buffer)
}

pub fn channel_point(
    sound_point: Point2<Metres>,
    channel_index: usize,
    total_channels: usize,
    spread: Metres,
    radians: f32,
) -> Point2<Metres> {
    assert!(channel_index < total_channels);
    if total_channels == 1 {
        sound_point
    } else {
        let phase = channel_index as f32 / total_channels as f32;
        let channel_radians_offset = phase * std::f32::consts::PI * 2.0;
        let radians = (radians + channel_radians_offset) as f64;
        let (rel_x, rel_y) = utils::rad_mag_to_x_y(radians, spread.0);
        let x = sound_point.x + Metres(rel_x);
        let y = sound_point.y + Metres(rel_y);
        Point2 { x, y }
    }
}

/// Tests whether or not the given speaker position is within the `PROXIMITY_LIMIT` distance of the
/// given `point` (normally a `Sound`'s channel position).
pub fn speaker_is_in_proximity(point: &Point2<Metres>, speaker: &Point2<Metres>) -> bool {
    let point_f = Point2 {
        x: point.x.0,
        y: point.y.0,
    };
    let speaker_f = Point2 {
        x: speaker.x.0,
        y: speaker.y.0,
    };
    let distance_2 = Metres(point_f.distance2(speaker_f));
    distance_2 < PROXIMITY_LIMIT_2
}
