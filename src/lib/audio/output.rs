//! The audio render function implementation.
//!
//! The render function is passed to `nannou::App`'s build output stream method and describes how
//! audio should be rendered to the output.

use audio::{DISTANCE_BLUR, PROXIMITY_LIMIT_2, Sound, Speaker, MAX_CHANNELS};
use audio::detector::{EnvDetector, Fft, FftDetector, FFT_WINDOW_LEN};
use audio::{dbap, source, sound, speaker};
use audio::fft;
use fxhash::{FxHashMap, FxHashSet};
use gui;
use installation;
use metres::Metres;
use nannou;
use nannou::audio::Buffer;
use nannou::math::{MetricSpace, Point2};
use osc;
use rustfft::num_complex::Complex;
use rustfft::num_traits::Zero;
use soundscape;
use std;
use std::ops::{Deref, DerefMut};
use std::sync::mpsc;
use time_calc::Samples;
use utils;

/// Simplified type alias for the nannou audio output stream used by the audio server.
pub type Stream = nannou::audio::Stream<Model>;

/// A sound that is currently active on the audio thread.
pub struct ActiveSound {
    sound: Sound,
    channel_detectors: Box<[EnvDetector]>,
    total_duration_frames: Option<Samples>,
}

pub struct ActiveSpeaker {
    speaker: Speaker,
    env_detector: EnvDetector,
}

pub struct Installation {
    /// A buffer that is resized to match the size of the `buffer` fed to the `render` function.
    ///
    /// This is re-used between calls to `render` to sum all audio from each speaker channel that
    /// is assigned to the installation. The audio data is then used to perform the necessary FFT
    /// calculations for the installation, ready to be sent over OSC for visualisation.
    ///
    /// This is silenced at the beginning of each call to `render`.
    buffer: Vec<f32>,
    /// The peak and RMS for each speaker in the installation.
    speaker_analyses: Vec<SpeakerAnalysis>,
    /// The detector used for incrementally calculating the FFT.
    fft_detector: FftDetector,
    /// The number of computers assigned to the installation to which audio data will be sent.
    ///
    /// If this value is `0`, the FFT calculation is not performed in order to save CPU.
    computers: usize,
}

impl ActiveSound {
    /// Create a new `ActiveSound`.
    pub fn new(sound: Sound) -> Self {
        let channel_detectors = (0..sound.channels)
            .map(|_| EnvDetector::new())
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let total_duration_frames = sound.signal.remaining_frames();
        ActiveSound {
            sound,
            channel_detectors,
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

struct SpeakerAnalysis {
    rms: f32,
    peak: f32,
    index: usize,
}

/// State that lives on the audio thread.
pub struct Model {
    /// the total number of frames written since the model was created or the project was switched.
    ///
    /// this is used for synchronising `continuous` wavs to the audio timeline with sample-perfect
    /// accuracy.
    pub frame_count: u64,
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
    // /// A map from a speaker's assigned channel to the ID of the speaker.
    // channel_to_speaker: FxHashMap<usize, speaker::Id>,
    /// A buffer for collecting the speakers within proximity of the sound's position.
    unmixed_samples: Vec<f32>,
    /// A buffer for collecting sounds that have been removed due to completing.
    exhausted_sounds: Vec<sound::Id>,
    /// Channel for communicating active sound info to the GUI.
    gui_audio_monitor_msg_tx: gui::monitor::Sender,
    /// Channel for sending sound analysis data to the OSC output thread.
    osc_output_msg_tx: mpsc::Sender<osc::output::Message>,
    /// A handle to the soundscape thread - for notifying when a sound is complete.
    soundscape_tx: mpsc::Sender<soundscape::Message>,
    /// Data related to a single installation necessary for the audio output thread.
    installations: FxHashMap<installation::Id, Installation>,
    /// A buffer to re-use for DBAP speaker calculations.
    ///
    /// The index of the speaker is its channel.
    dbap_speakers: Vec<dbap::Speaker>,
    /// A buffer to re-use for storing the gain for each speaker produced by DBAP.
    dbap_speaker_gains: Vec<f32>,
    /// The FFT planner used to prepare the FFT calculations and share data between them.
    fft_planner: fft::Planner,
    /// The FFT to re-use by each of the `Detector`s.
    fft: Fft,
    /// A buffer for retrieving the frequency amplitudes from the `fft`.
    fft_frequency_amplitudes_2: Box<[f32; FFT_WINDOW_LEN / 2]>,
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
        gui_audio_monitor_msg_tx: gui::monitor::Sender,
        osc_output_msg_tx: mpsc::Sender<osc::output::Message>,
        soundscape_tx: mpsc::Sender<soundscape::Message>,
    ) -> Self {
        // The currently soloed sources (none by default).
        let soloed = Default::default();

        // A map from audio sound IDs to the audio sounds themselves.
        let sounds = Default::default();

        // A map from speaker IDs to the speakers themselves.
        let speakers = Default::default();

        // A buffer for collecting frames from `Sound`s that have not yet been mixed and written.
        let unmixed_samples = vec![0.0; 1024];

        // A buffer for collecting exhausted `Sound`s.
        let exhausted_sounds = Vec::with_capacity(128);

        // A map from installations to their audio data and speaker analyses that can be re-used.
        let installations = Default::default();

        // A buffer to re-use for DBAP speaker calculations.
        let dbap_speakers = Vec::with_capacity(MAX_CHANNELS);

        // A buffer to re-use for storing gains produced by DBAP.
        let dbap_speaker_gains = Vec::with_capacity(MAX_CHANNELS);

        // The FFT to re-use by each of the `Detector`s.
        let in_window = [Complex::<f32>::zero(); FFT_WINDOW_LEN];
        let out_window = [Complex::<f32>::zero(); FFT_WINDOW_LEN];
        let fft = Fft::new(in_window, out_window);
        let inverse = false;
        let fft_planner = fft::Planner::new(inverse);

        // A buffer for retrieving the frequency amplitudes from the `fft`.
        let fft_frequency_amplitudes_2 = Box::new([0.0; FFT_WINDOW_LEN / 2]);

        // Initialise the master volume to the default value.
        let master_volume = super::DEFAULT_MASTER_VOLUME;

        // Initialise the rolloff to the default value.
        let dbap_rolloff_db = super::DEFAULT_DBAP_ROLLOFF_DB;

        // Initialise the frame count.
        let frame_count = 0;

        Model {
            frame_count,
            master_volume,
            dbap_rolloff_db,
            soloed,
            sounds,
            speakers,
            unmixed_samples,
            exhausted_sounds,
            installations,
            gui_audio_monitor_msg_tx,
            osc_output_msg_tx,
            soundscape_tx,
            dbap_speakers,
            dbap_speaker_gains,
            fft,
            fft_planner,
            fft_frequency_amplitudes_2,
        }
    }

    /// Insert an installation for the given `Id`.
    ///
    /// Returns `true` if the installation did not yet exist or false otherwise.
    pub fn insert_installation(&mut self, id: installation::Id, computers: usize) -> bool {
        let speaker_analyses = Vec::with_capacity(MAX_CHANNELS);
        let buffer = Vec::with_capacity(1024);
        let fft_detector = FftDetector::new();
        let installation = Installation {
            speaker_analyses,
            buffer,
            fft_detector,
            computers,
        };
        self.installations.insert(id, installation).is_none()
    }

    /// Remove the installation at the given `Id`.
    ///
    /// Also removes the installation from all speakers that have been assigned to it.
    ///
    /// Returns `true` if the installation was successfully removed.
    ///
    /// Returns `false` if there was no installation for the given `Id`.
    pub fn remove_installation(&mut self, id: &installation::Id) -> bool {
        for speaker in self.speakers.values_mut() {
            speaker.installations.remove(id);
        }
        for sound in self.sounds.values_mut() {
            if let sound::Installations::Set(ref mut set) = sound.installations {
                set.remove(id);
            }
        }
        self.installations.remove(id).is_some()
    }

    /// Inserts the speaker and sends an `Add` message to the GUI.
    pub fn insert_speaker(&mut self, id: speaker::Id, speaker: Speaker) -> Option<Speaker> {
        // Re-use the old detectors if there are any.
        let (env_detector, old_speaker) = match self.speakers.remove(&id) {
            None => (EnvDetector::new(), None),
            Some(ActiveSpeaker { speaker, env_detector, }) => (env_detector, Some(speaker)),
        };

        let speaker = ActiveSpeaker { speaker, env_detector };
        let speaker_msg = gui::SpeakerMessage::Add;
        let msg = gui::AudioMonitorMessage::Speaker(id, speaker_msg);
        self.gui_audio_monitor_msg_tx.send(msg).ok();
        self.speakers.insert(id, speaker);
        old_speaker
    }

    /// Removes the speaker and sens a `Removed` message to the GUI.
    pub fn remove_speaker(&mut self, id: speaker::Id) -> Option<Speaker> {
        let removed = self.speakers.remove(&id);
        if removed.is_some() {
            let speaker_msg = gui::SpeakerMessage::Remove;
            let msg = gui::AudioMonitorMessage::Speaker(id, speaker_msg);
            self.gui_audio_monitor_msg_tx.send(msg).ok();
        }
        removed.map(|ActiveSpeaker { speaker, .. }| speaker)
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
        let sound_msg = gui::ActiveSoundMessage::Start {
            source_id,
            position,
            channels,
            normalised_progress,
        };
        let msg = gui::AudioMonitorMessage::ActiveSound(id, sound_msg);
        self.gui_audio_monitor_msg_tx.send(msg).ok();
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

    /// Removes the sound and sends an `End` active sound message to the GUI.
    ///
    /// Returns `false` if the sound did not exist
    pub fn remove_sound(&mut self, id: sound::Id) -> bool {
        let removed = self.sounds.remove(&id);
        if let Some(sound) = removed {
            // Notify the gui.
            let sound_msg = gui::ActiveSoundMessage::End { sound };
            let msg = gui::AudioMonitorMessage::ActiveSound(id, sound_msg);
            self.gui_audio_monitor_msg_tx.send(msg).ok();

            // Notify the soundscape thread.
            let update = move |soundscape: &mut soundscape::Model| {
                soundscape.remove_active_sound(&id);
            };
            self.soundscape_tx.send(soundscape::UpdateFn::from(update).into()).ok();
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
        self.frame_count = 0;
        self.soloed.clear();
        self.speakers.clear();
        self.installations.clear();

        let Model { ref mut sounds, ref gui_audio_monitor_msg_tx, .. } = *self;
        for (sound_id, sound) in sounds.drain() {
            // Send signal of completion back to GUI thread.
            let sound_msg = gui::ActiveSoundMessage::End { sound };
            let msg = gui::AudioMonitorMessage::ActiveSound(sound_id, sound_msg);
            gui_audio_monitor_msg_tx.send(msg).ok();
        }
    }
}

/// The function given to nannou to use for rendering.
pub fn render(mut model: Model, mut buffer: Buffer) -> (Model, Buffer) {
    {
        let Model {
            master_volume,
            dbap_rolloff_db,
            ref soloed,
            ref mut frame_count,
            ref mut sounds,
            ref mut unmixed_samples,
            ref mut exhausted_sounds,
            ref mut installations,
            ref mut speakers,
            ref mut dbap_speakers,
            ref mut dbap_speaker_gains,
            ref gui_audio_monitor_msg_tx,
            ref osc_output_msg_tx,
            ref soundscape_tx,
            ref mut fft,
            ref mut fft_planner,
            ref mut fft_frequency_amplitudes_2,
        } = model;

        // Always silence the buffer to begin.
        buffer.iter_mut().for_each(|s| *s = 0.0);

        // Clear the analyses.
        for installation in installations.values_mut() {
            installation.speaker_analyses.clear();
            installation.buffer.resize(buffer.len_frames(), 0.0);
            installation.buffer.iter_mut().for_each(|s| *s = 0.0);
        }

        // For each sound, request `buffer.len()` number of frames and sum them onto the
        // relevant output channels.
        for (&sound_id, sound) in sounds.iter_mut() {
            // Update the GUI with the position of the sound.
            let source_id = sound.source_id();
            let position = sound.position;
            let channels = sound.channels;
            let normalised_progress = sound.normalised_progress();
            let update = gui::ActiveSoundMessage::Update {
                source_id,
                position,
                channels,
                normalised_progress,
            };
            let msg = gui::AudioMonitorMessage::ActiveSound(sound_id, update);
            gui_audio_monitor_msg_tx.send(msg).ok();

            let ActiveSound {
                ref mut sound,
                ref mut channel_detectors,
                ..
            } = *sound;

            // Don't play or request samples if paused.
            if !sound.shared.is_playing() {
                continue;
            }

            // The number of samples to request from the sound for this buffer.
            let num_samples = buffer.len_frames() * sound.channels;

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

            // If the source is a `Continuous` WAV, ensure it is seeked to the correct position.
            if let source::SignalKind::Wav { ref playback, ref mut samples } = sound.signal.kind {
                if let source::wav::Playback::Continuous = *playback {
                    if let Err(err) = samples.seek(*frame_count) {
                        eprintln!("failed to seek file for continuous WAV source: {}", err);
                        continue;
                    }
                }
            }

            // Clear the unmixed samples, ready to collect the new ones.
            unmixed_samples.clear();
            {
                let mut samples_written = 0;
                for sample in sound.signal.samples().take(num_samples) {
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
                for (index, env_detector) in channel_detectors.iter().enumerate() {
                    let (rms, peak) = env_detector.current();
                    let sound_msg = gui::ActiveSoundMessage::UpdateChannel { index, rms, peak };
                    let msg = gui::AudioMonitorMessage::ActiveSound(sound_id, sound_msg);
                    gui_audio_monitor_msg_tx.send(msg).ok();
                }
            }

            // Mix the audio from the signal onto each of the output channels.
            if speakers.is_empty() {
                continue;
            }

            for (i, channel_point) in sound.channel_points().enumerate() {
                // Update the dbap_speakers buffer with their distances to this sound channel.
                dbap_speakers.clear();
                for channel in 0..buffer.channels() {
                    // Find the speaker for this channel.
                    // TODO: Could speed this up by maintaining a map from channels to speaker IDs.
                    if let Some(active) = speakers.values().find(|s| s.speaker.channel == channel) {
                        let channel_point_f = Point2 {
                            x: channel_point.x.0,
                            y: channel_point.y.0,
                        };
                        let speaker = &active.speaker.point;
                        let speaker_f = Point2 {
                            x: speaker.x.0,
                            y: speaker.y.0,
                        };
                        let distance = dbap::blurred_distance_2(channel_point_f, speaker_f, DISTANCE_BLUR);
                        let weight = speaker::dbap_weight(&sound.installations, &active.speaker.installations);
                        dbap_speakers.push(dbap::Speaker { distance, weight });
                    }
                }

                // If no speakers were found, skip this channel.
                if dbap_speakers.is_empty() {
                    continue;
                }

                // Update the speaker gains.
                dbap_speaker_gains.clear();
                let gains = dbap::SpeakerGains::new(&dbap_speakers, dbap_rolloff_db);
                dbap_speaker_gains.extend(gains.map(|f| f as f32));

                // For every frame in the buffer, mix the unmixed sample.
                let mut sample_index = i;
                for frame in buffer.frames_mut() {
                    let channel_sample = unmixed_samples[sample_index];
                    for (channel, &speaker_gain) in dbap_speaker_gains.iter().enumerate() {
                        // Only write to the channels that will be read by the audio device.
                        if let Some(sample) = frame.get_mut(channel) {
                            *sample += channel_sample * speaker_gain * sound.volume;
                        }
                    }
                    sample_index += sound.channels;
                }
            }
        }

        // For each speaker, feed its amplitude into its detectors.
        let n_channels = buffer.channels();
        for (&id, active) in speakers.iter_mut() {
            let channel_i = active.speaker.channel;
            if channel_i >= n_channels {
                continue;
            }
            let ActiveSpeaker {
                ref mut env_detector,
                ..
            } = *active;

            // Update the envelope detector.
            for frame in buffer.frames() {
                let sample = frame[channel_i];
                env_detector.next(sample);
            }

            // The current env and fft detector states.
            let (rms, peak) = env_detector.current();

            // Send the detector state for the speaker to the GUI.
            let speaker_msg = gui::SpeakerMessage::Update { rms, peak };
            let msg = gui::AudioMonitorMessage::Speaker(id, speaker_msg);
            gui_audio_monitor_msg_tx.send(msg).ok();

            // Sum raw audio data for FFTs.
            for id in &active.speaker.installations {
                let installation = match installations.get_mut(&id) {
                    None => continue,
                    Some(installation) => installation,
                };

                // If the installation has no computers, skip it.
                if installation.computers == 0 {
                    continue;
                }

                let index = channel_i;
                let analysis = SpeakerAnalysis { peak, rms, index };
                installation.speaker_analyses.push(analysis);

                // Sum the audio data for the speaker onto its associated installation buffers.
                for (i, frame) in buffer.frames().enumerate() {
                    installation.buffer[i] += frame[channel_i];
                }
            }
        }

        // Send the collected analysis to the OSC output thread.
        for (&id, installation) in installations.iter_mut() {
            // If there are no speakers, skip the installation.
            if speakers.is_empty() {
                continue;
            }

            // If the installation has no computers, there's no point analysing audio.
            if installation.computers == 0 {
                continue;
            }

            // Retrieve the audio buffer so we can perform FFT.
            let avg_fft: osc::output::FftData = {
                let Installation {
                    ref buffer,
                    ref speaker_analyses,
                    ref mut fft_detector,
                    ..
                } = *installation;

                let n_speakers = speaker_analyses.len();

                // Feed the buffer into the FFT detector, normalised for the number of speakers in
                // the installation..
                for &sample in buffer {
                    // TODO: This division might be better on lmh and bins.
                    fft_detector.push(sample / n_speakers as f32);
                }

                // Perform the FFT.
                fft_detector.calc_fft(fft_planner, fft, &mut fft_frequency_amplitudes_2[..]);

                // Retrieve the LMH representation.
                let (l_2, m_2, h_2) = fft::lmh(&fft_frequency_amplitudes_2[..]);
                let mut lmh = [0.0; 3];
                for (amp, amp_2) in lmh.iter_mut().zip(&[l_2, m_2, h_2]) {
                    *amp = amp_2.sqrt() / (FFT_WINDOW_LEN / 2) as f32;
                }

                // Retrieve the 8-bin representation.
                let mut bins_2 = [0.0; 8];
                fft::mel_bins(&fft_frequency_amplitudes_2[..], &mut bins_2);
                let mut bins = [0.0; 8];
                for (amp, amp_2) in bins.iter_mut().zip(&bins_2) {
                    *amp = amp_2.sqrt() / (FFT_WINDOW_LEN / 2) as f32;
                }

                // Prepare the osc output message.
                let fft_data = osc::output::FftData { lmh, bins };
                fft_data
            };

            // Find the average peak and RMS across all speakers in the installation.
            let (avg_peak, avg_rms) = {
                let n_speakers_f = installation.speaker_analyses.len() as f32;
                let mut iter = installation.speaker_analyses.iter();
                iter.next()
                    .map(|s| {
                        let init = (s.peak, s.rms);
                        iter.fold(init, |(acc_p, acc_r), s| (acc_p + s.peak, acc_r + s.rms))
                    })
                    .map(|(sum_p, sum_r)| (sum_p / n_speakers_f, sum_r / n_speakers_f))
                    .unwrap_or((0.0, 0.0))
            };

            // Sort the speakers by channel index as the OSC output thread assumes that speakers
            // are in order of index.
            installation.speaker_analyses.sort_by(|a, b| a.index.cmp(&b.index));
            let speakers = installation.speaker_analyses
                .drain(..)
                .map(|s| osc::output::Speaker {
                    rms: s.rms,
                    peak: s.peak,
                })
                .collect();
            let data = osc::output::AudioFrameData {
                avg_peak,
                avg_rms,
                avg_fft,
                speakers,
            };
            let msg = osc::output::Message::Audio(id, data);
            osc_output_msg_tx.send(msg).ok();
        }

        // Remove all sounds that have been exhausted.
        for sound_id in exhausted_sounds.drain(..) {
            // Possibly send this with the `End` message to avoid de-allocating on audio thread.
            let sound = sounds.remove(&sound_id).unwrap();

            // Send signal of completion back to GUI thread.
            let sound_msg = gui::ActiveSoundMessage::End { sound };
            let msg = gui::AudioMonitorMessage::ActiveSound(sound_id, sound_msg);
            gui_audio_monitor_msg_tx.send(msg).ok();

            // Notify the soundscape thread.
            let update = move |soundscape: &mut soundscape::Model| {
                soundscape.remove_active_sound(&sound_id);
            };
            soundscape_tx.send(soundscape::UpdateFn::from(update).into()).ok();
        }

        // Apply the master volume.
        for sample in buffer.iter_mut() {
            *sample *= master_volume;
        }

        // Find the peak amplitude and send it via the monitor channel.
        let peak = buffer.iter().fold(0.0, |peak, &s| s.max(peak));
        gui_audio_monitor_msg_tx.send(gui::AudioMonitorMessage::Master { peak }).ok();

        // Step the frame count.
        *frame_count += buffer.len_frames() as u64;
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
