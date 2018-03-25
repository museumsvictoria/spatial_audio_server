//! The audio render function implementation.
//!
//! The render function is passed to `nannou::App`'s build output stream method and describes how
//! audio should be rendered to the output.

use audio::{PROXIMITY_LIMIT_2, Sound, Speaker, MAX_CHANNELS, ROLLOFF_DB};
use audio::detector::{EnvDetector, Fft, FftDetector, FFT_WINDOW_LEN};
use audio::{dbap, sound, speaker};
use audio::fft;
use gui;
use installation::{self, Installation};
use metres::Metres;
use nannou;
use nannou::audio::Buffer;
use nannou::math::{MetricSpace, Point2};
use osc;
use rustfft::num_complex::Complex;
use rustfft::num_traits::Zero;
use std;
use std::collections::HashMap;
use std::sync::mpsc;

/// Simplified type alias for the nannou audio output stream used by the audio server.
pub type Stream = nannou::audio::Stream<Model>;

/// A sound that is currently active on the audio thread.
pub struct ActiveSound {
    sound: Sound,
    channel_detectors: Box<[EnvDetector]>,
}

pub struct ActiveSpeaker {
    speaker: Speaker,
    env_detector: EnvDetector,
    fft_detector: FftDetector,
}

impl ActiveSound {
    /// Create a new `ActiveSound`.
    pub fn new(sound: Sound) -> Self {
        let channel_detectors = (0..sound.channels)
            .map(|_| EnvDetector::new())
            .collect::<Vec<_>>()
            .into_boxed_slice();
        ActiveSound {
            sound,
            channel_detectors,
        }
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
    // /// A map from a speaker's assigned channel to the ID of the speaker.
    // channel_to_speaker: HashMap<usize, speaker::Id>,
    /// A buffer for collecting the speakers within proximity of the sound's position.
    unmixed_samples: Vec<f32>,
    /// A buffer for collecting sounds that have been removed due to completing.
    exhausted_sounds: Vec<sound::Id>,
    /// Channel for communicating active sound info to the GUI.
    gui_audio_monitor_msg_tx: mpsc::SyncSender<gui::AudioMonitorMessage>,
    /// Channel for sending sound analysis data to the OSC output thread.
    osc_output_msg_tx: mpsc::Sender<osc::output::Message>,
    /// An analysis per installation to re-use for sending to the OSC output thread.
    installation_analyses: HashMap<Installation, Vec<SpeakerAnalysis>>,
    /// A buffer to re-use for DBAP speaker calculations.
    ///
    /// The index of the speaker is its channel.
    dbap_speakers: Vec<dbap::Speaker>,
    /// A buffer to re-use for storing the gain for each speaker produced by DBAP.
    dbap_speaker_gains: Vec<f32>,
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
        gui_audio_monitor_msg_tx: mpsc::SyncSender<gui::AudioMonitorMessage>,
        osc_output_msg_tx: mpsc::Sender<osc::output::Message>,
    ) -> Self {
        // A map from audio sound IDs to the audio sounds themselves.
        let sounds: HashMap<sound::Id, ActiveSound> = HashMap::with_capacity(1024);

        // A map from speaker IDs to the speakers themselves.
        let speakers: HashMap<speaker::Id, ActiveSpeaker> = HashMap::with_capacity(MAX_CHANNELS);

        // A buffer for collecting frames from `Sound`s that have not yet been mixed and written.
        let unmixed_samples = vec![0.0; 1024];

        // A buffer for collecting exhausted `Sound`s.
        let exhausted_sounds = Vec::with_capacity(128);

        // A map from installations to audio analysis frames that can be re-used.
        let installation_analyses = installation::ALL
            .iter()
            .map(|&inst| (inst, Vec::with_capacity(MAX_CHANNELS)))
            .collect();

        // A buffer to re-use for DBAP speaker calculations.
        let dbap_speakers = Vec::with_capacity(MAX_CHANNELS);

        // A buffer to re-use for storing gains produced by DBAP.
        let dbap_speaker_gains = Vec::with_capacity(MAX_CHANNELS);

        // The FFT to re-use by each of the `Detector`s.
        let in_window = [Complex::<f32>::zero(); FFT_WINDOW_LEN];
        let out_window = [Complex::<f32>::zero(); FFT_WINDOW_LEN];
        let fft = Fft::new(in_window, out_window);

        // A buffer for retrieving the frequency amplitudes from the `fft`.
        let fft_frequency_amplitudes_2 = Box::new([0.0; FFT_WINDOW_LEN / 2]);

        Model {
            sounds,
            speakers,
            unmixed_samples,
            exhausted_sounds,
            installation_analyses,
            gui_audio_monitor_msg_tx,
            osc_output_msg_tx,
            dbap_speakers,
            dbap_speaker_gains,
            fft,
            fft_frequency_amplitudes_2,
        }
    }

    /// Inserts the speaker and sends an `Add` message to the GUI.
    pub fn insert_speaker(&mut self, id: speaker::Id, speaker: Speaker) -> Option<Speaker> {
        // Re-use the old detectors if there are any.
        let (env_detector, fft_detector, old_speaker) = match self.speakers.remove(&id) {
            None => (EnvDetector::new(), FftDetector::new(), None),
            Some(ActiveSpeaker {
                speaker,
                env_detector,
                fft_detector,
            }) => (env_detector, fft_detector, Some(speaker)),
        };

        let speaker = ActiveSpeaker {
            speaker,
            env_detector,
            fft_detector,
        };
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

    /// Inserts the installation into the speaker with the given `speaker::Id`.
    pub fn insert_speaker_installation(&mut self, id: speaker::Id, inst: Installation) -> bool {
        self.speakers
            .get_mut(&id)
            .map(|active| active.speaker.installations.insert(inst))
            .unwrap_or(false)
    }

    /// Removes the installation from the speaker with the given `speaker::Id`.
    pub fn remove_speaker_installation(&mut self, id: speaker::Id, inst: &Installation) -> bool {
        self.speakers
            .get_mut(&id)
            .map(|active| active.speaker.installations.remove(inst))
            .unwrap_or(false)
    }

    /// Inserts the sound and sends an `Start` active sound message to the GUI.
    pub fn insert_sound(&mut self, id: sound::Id, sound: ActiveSound) -> Option<ActiveSound> {
        let position = sound.sound.point;
        let channels = sound.sound.channels;
        let source_id = sound.sound.source_id();
        let sound_msg = gui::ActiveSoundMessage::Start {
            source_id,
            position,
            channels,
        };
        let msg = gui::AudioMonitorMessage::ActiveSound(id, sound_msg);
        self.gui_audio_monitor_msg_tx.try_send(msg).ok();
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

    /// An iterator yielding mutable access to all sounds currently playing.
    pub fn sounds_mut(&mut self) -> SoundsMut {
        let iter = self.sounds.iter_mut();
        SoundsMut { iter }
    }
}

/// The function given to nannou to use for rendering.
pub fn render(mut model: Model, mut buffer: Buffer) -> (Model, Buffer) {
    {
        let Model {
            ref mut sounds,
            ref mut unmixed_samples,
            ref mut exhausted_sounds,
            ref mut installation_analyses,
            ref mut speakers,
            ref mut dbap_speakers,
            ref mut dbap_speaker_gains,
            ref gui_audio_monitor_msg_tx,
            ref osc_output_msg_tx,
            ref mut fft,
            ref mut fft_frequency_amplitudes_2,
        } = model;

        // Always silence the buffer to begin.
        for sample in buffer.iter_mut() {
            *sample = 0.0;
        }

        // For each sound, request `buffer.len()` number of frames and sum them onto the
        // relevant output channels.
        for (&sound_id, active_sound) in sounds.iter_mut() {
            let ActiveSound {
                ref mut sound,
                ref mut channel_detectors,
            } = *active_sound;

            // Don't play it if paused.
            if !sound.shared.is_playing() {
                continue;
            }

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
                for (index, env_detector) in channel_detectors.iter().enumerate() {
                    let (rms, peak) = env_detector.current();
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

                // Update the dbap_speakers buffer with their distances to this sound channel.
                dbap_speakers.clear();
                for channel in 0..buffer.channels() {
                    // Find the speaker for this channel.
                    // TODO: Could speed this up by maintaining a map from channels to speaker IDs.
                    if let Some(active) = speakers.values().find(|s| s.speaker.channel == channel) {
                        let point_f = Point2 {
                            x: channel_point.x.0,
                            y: channel_point.y.0,
                        };
                        let speaker = &active.speaker.point;
                        let speaker_f = Point2 {
                            x: speaker.x.0,
                            y: speaker.y.0,
                        };
                        let distance = point_f.distance(speaker_f);
                        // Weight the speaker depending on its associated installations.
                        let weight = match sound.installations {
                            sound::Installations::All => 1.0,
                            sound::Installations::Set(ref set) => {
                                match set.intersection(&active.speaker.installations).next() {
                                    Some(_) => 1.0,
                                    None => 0.0,
                                }
                            },
                        };
                        dbap_speakers.push(dbap::Speaker { distance, weight });
                    }
                }

                // Update the speaker gains.
                dbap_speaker_gains.clear();
                let gains = dbap::SpeakerGains::new(&dbap_speakers, ROLLOFF_DB);
                dbap_speaker_gains.extend(gains.map(|f| f as f32));

                // For every frame in the buffer, mix the unmixed sample.
                let mut sample_index = i;
                for frame in buffer.frames_mut() {
                    let channel_sample = unmixed_samples[sample_index];
                    for (channel, &gain) in dbap_speaker_gains.iter().enumerate() {
                        // Only write to the channels that will be read by the audio device.
                        if let Some(sample) = frame.get_mut(channel) {
                            *sample += channel_sample * gain;
                        }
                    }
                    sample_index += sound.channels;
                }
            }
        }

        // For each speaker, feed its amplitude into its detectors.
        let n_channels = buffer.channels();
        let mut sum_peak = 0.0;
        let mut sum_rms = 0.0;
        let mut sum_lmh = [0.0; 3];
        let mut sum_fft_8_band = [0.0; 8];
        for (&id, active) in speakers.iter_mut() {
            let mut channel_i = active.speaker.channel;
            if channel_i >= n_channels {
                continue;
            }
            let ActiveSpeaker {
                ref mut env_detector,
                ref mut fft_detector,
                ..
            } = *active;
            for frame in buffer.frames() {
                let sample = frame[channel_i];
                env_detector.next(sample);
                fft_detector.push(sample);
            }

            // The current env and fft detector states.
            let (rms, peak) = env_detector.current();
            fft_detector.calc_fft(fft, &mut fft_frequency_amplitudes_2[..]);
            let (l_2, m_2, h_2) = fft::lmh(&fft_frequency_amplitudes_2[..]);
            let mut fft_8_bins_2 = [0.0; 8];
            fft::mel_bins(&fft_frequency_amplitudes_2[..], &mut fft_8_bins_2);

            // Send the detector state for the speaker to the GUI.
            let speaker_msg = gui::SpeakerMessage::Update { rms, peak };
            let msg = gui::AudioMonitorMessage::Speaker(id, speaker_msg);
            gui_audio_monitor_msg_tx.try_send(msg).ok();

            // Sum the rms and peak.
            for installation in &active.speaker.installations {
                let speakers = match installation_analyses.get_mut(&installation) {
                    None => continue,
                    Some(speakers) => speakers,
                };
                sum_peak += peak;
                sum_rms += rms;
                for (sum, amp_2) in sum_lmh.iter_mut().zip(&[l_2, m_2, h_2]) {
                    *sum += amp_2.sqrt() / (FFT_WINDOW_LEN / 2) as f32;
                }
                for (sum, amp_2) in sum_fft_8_band.iter_mut().zip(&fft_8_bins_2) {
                    *sum += amp_2.sqrt() / (FFT_WINDOW_LEN / 2) as f32;
                }
                let analysis = SpeakerAnalysis {
                    peak,
                    rms,
                    index: channel_i,
                };
                speakers.push(analysis);
            }
        }

        // Send the collected analysis to the OSC output thread.
        for (&installation, speakers) in installation_analyses.iter_mut() {
            if speakers.is_empty() {
                continue;
            }
            speakers.sort_by(|a, b| a.index.cmp(&b.index));
            let len_f = speakers.len() as f32;
            let avg_peak = sum_peak / len_f;
            let avg_rms = sum_rms / len_f;
            let avg_lmh = [sum_lmh[0] / len_f, sum_lmh[1] / len_f, sum_lmh[2] / len_f];
            let mut avg_8_band = [0.0; 8];
            for (avg, &sum) in avg_8_band.iter_mut().zip(&sum_fft_8_band) {
                *avg = sum / len_f;
            }
            let avg_fft = osc::output::FftData {
                lmh: avg_lmh,
                bins: avg_8_band,
            };
            let speakers = speakers
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
            let msg = osc::output::Message::Audio(installation, data);
            osc_output_msg_tx.send(msg).ok();
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
) -> Point2<Metres> {
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
