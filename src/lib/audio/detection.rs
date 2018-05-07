//! The thread on which all audio detection on the output buffer is performed.
//!
//! Performs the following detection:
//!
//! - RMS and Peak per Sound.
//! - RMS and Peak per Speaker channel.
//! - FFT and avg RMS and Peak per installation.

use audio::{MAX_CHANNELS, FRAMES_PER_BUFFER};
use audio::{fft, sound, speaker};
use audio::detector::{EnvDetector, Fft, FftDetector, FFT_WINDOW_LEN};
use crossbeam::sync::SegQueue;
use fxhash::{FxHashMap, FxHashSet};
use gui;
use installation;
use osc;
use rustfft::num_complex::Complex;
use rustfft::num_traits::Zero;
use std::ops;
use std::sync::{Arc, Mutex};
use std::{thread, time};

/// The number of buffers per active sound that the detection thread will attempt to maintain.
const BUFFERS_PER_SOUND: usize = 3;

/// The type used for queueing messages for processing by the detection thread.
type MessageQueue = Arc<SegQueue<Message>>;

/// The type used for queueing buffers for re-use.
type BufferQueue = Arc<SegQueue<Vec<f32>>>;

/// The type used for queueing output buffers and their relevant analysis info for re-use.
type OutputBufferQueue = Arc<SegQueue<(Vec<f32>, OutputInfo)>>;

/// The type used for mapping `sound::Id`s to their relevant detection state.
type Sounds = FxHashMap<sound::Id, Sound>;

/// Tye type used for mapping `speaker::Id`s to their relevant detection state.
type Speakers = FxHashMap<speaker::Id, Speaker>;

/// A map from `installation::Id`s to their relevant detection state.
type Installations = FxHashMap<installation::Id, Installation>;

/// Information relevant to analysis of the output buffer.
#[derive(Default)]
pub struct OutputInfo {
    pub speakers: FxHashMap<speaker::Id, SpeakerInfo>,
}

/// Information relevant to output analysis for a single speaker.
pub struct SpeakerInfo {
    pub channel: usize,
    pub installations: FxHashSet<installation::Id>,
}

/// Detection state relevant to a single sound.
struct Sound {
    channel_detectors: Box<[EnvDetector]>,
}

/// Detection state relevant to a single speaker.
struct Speaker {
    env_detector: EnvDetector,
}

/// Analysis related to a single speaker.
struct SpeakerAnalysis {
    rms: f32,
    peak: f32,
    index: usize,
}

/// Detection state relevant to a single installation.
struct Installation {
    /// Used for collecting the sum of all channels in the installation.
    summed_samples_of_all_channels: Vec<f32>,
    /// The peak and RMS for each speaker in the installation.
    speaker_analyses: Vec<SpeakerAnalysis>,
    /// The detector used for incrementally calculating the FFT.
    fft_detector: FftDetector,
    /// The number of computers assigned to the installation to which audio data will be sent.
    ///
    /// If this value is `0`, the FFT calculation is not performed in order to save CPU.
    computers: usize,
}

/// Buffer of samples for analysis, received by the detection thread.
struct Buffer {
    samples: Vec<f32>,
    channels: usize,
}

/// Unisgned integer type used to represent a number of channels.
type Channels = usize;

/// The number of computers assigned to an installation.
type Computers = usize;

/// Messages received by the detection thread.
enum Message {
    AddSound(sound::Id, Channels),
    UpdateSound(sound::Id, Buffer),
    RemoveSound(sound::Id),

    AddInstallation(installation::Id, Computers),
    RemoveInstallation(installation::Id),

    Output(Buffer, OutputInfo),

    CpuSavingEnabled(bool),
    ClearProjectSpecificData,

    Exit,
}

/// A handle for communicating with the detection thread.
pub struct Handle {
    tx: MessageQueue,
    thread: Arc<Mutex<Option<thread::JoinHandle<()>>>>,
    sound_buffer_rx: BufferQueue,
    output_buffer_rx: OutputBufferQueue,
}

/// State stored on the detection thread.
pub struct Model {
    sounds: Sounds,
    speakers: Speakers,
    installations: Installations,

    /// If CPU saving is enabled, don't run the envelope detectors.
    cpu_saving_enabled: bool,

    gui_audio_monitor_msg_tx: gui::monitor::Sender,
    osc_output_msg_tx: osc::output::Tx,
    sound_buffer_tx: BufferQueue,
    output_buffer_tx: OutputBufferQueue,

    /// The current number of active sound buffers that are being cycled between the detection and
    /// audio output threads. This should always be at least `BUFFERS_PER_SOUND * sounds.len()`.
    num_active_sound_buffers: usize,

    /// The FFT planner used to prepare the FFT calculations and share data between them.
    fft_planner: fft::Planner,
    /// The FFT to re-use by each of the `Detector`s.
    fft: Fft,
    /// A buffer for retrieving the frequency amplitudes from the `fft`.
    fft_frequency_amplitudes_2: Box<[f32; FFT_WINDOW_LEN / 2]>,
}

impl OutputInfo {
    fn clear(&mut self) {
        self.speakers.clear();
    }
}

impl Buffer {
    /// Constructor for a buffer ready for analysis.
    pub fn new(samples: Vec<f32>, channels: usize) -> Self {
        assert_eq!(samples.len() % channels, 0);
        Buffer { samples, channels }
    }
}

impl Handle {
    /// Send the detection thread a message that indicates a new sound has begun and that the
    /// detectors should be prepared.
    pub fn add_sound(&self, id: sound::Id, channels: usize) {
        let msg = Message::AddSound(id, channels);
        self.tx.push(msg);
    }

    /// Update the detection state for the sound with the given `Id`.
    pub fn update_sound(&self, id: sound::Id, samples: Vec<f32>, channels: usize) {
        let buffer = Buffer::new(samples, channels);
        let msg = Message::UpdateSound(id, buffer);
        self.tx.push(msg);
    }

    /// Removes the detection state for the sound with the given Id.
    pub fn remove_sound(&self, id: sound::Id) {
        let msg = Message::RemoveSound(id);
        self.tx.push(msg);
    }

    /// Add an installation and initialize the necessary detection state.
    pub fn add_installation(&self, id: installation::Id, computers: usize) {
        let msg = Message::AddInstallation(id, computers);
        self.tx.push(msg);
    }

    /// Remove the detection state for the given installation.
    pub fn remove_installation(&self, id: installation::Id) {
        let msg = Message::RemoveInstallation(id);
        self.tx.push(msg);
    }

    /// Update the output analysis.
    pub fn update_output(&self, samples: Vec<f32>, channels: usize, info: OutputInfo) {
        let buffer = Buffer::new(samples, channels);
        let msg = Message::Output(buffer, info);
        self.tx.push(msg);
    }

    /// Tell the detection thread whether or not cpu mode is enabled or disabled.
    pub fn cpu_saving_enabled(&self, enabled: bool) {
        let msg = Message::CpuSavingEnabled(enabled);
        self.tx.push(msg);
    }

    /// Pop the next available sound buffer for use off the queue.
    pub fn pop_sound_buffer(&self) -> Vec<f32> {
        let mut buffer = self.sound_buffer_rx.try_pop().unwrap_or_else(Vec::new);
        buffer.clear();
        buffer
    }

    /// Pop the next available output buffer for use off the queue.
    pub fn pop_output_buffer(&self) -> (Vec<f32>, OutputInfo) {
        let (mut buffer, mut info) = self.output_buffer_rx.try_pop()
            .unwrap_or_else(Default::default);
        buffer.clear();
        info.clear();
        (buffer, info)
    }

    /// Clear all project-specific state on the detection thread.
    pub fn clear_project_specific_data(&self) {
        let msg = Message::ClearProjectSpecificData;
        self.tx.push(msg);
    }

    /// Stops the wav reader thread and returns the raw handle to its thread.
    ///
    /// This is called automatically when the handle is dropped.
    fn exit(&self) -> Option<thread::JoinHandle<()>> {
        self.tx.push(Message::Exit);
        self.thread.lock().unwrap().take()
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.exit();
    }
}

impl Model {
    /// Initialiise the state for the detection thread.
    pub fn new(
        gui_audio_monitor_msg_tx: gui::monitor::Sender,
        osc_output_msg_tx: osc::output::Tx,
        sound_buffer_tx: BufferQueue,
        output_buffer_tx: OutputBufferQueue,
    ) -> Self {
        let sounds = Sounds::default();
        let speakers = Speakers::default();
        let installations = Installations::default();
        let num_active_sound_buffers = 0;

        // The FFT to re-use by each of the `Detector`s.
        let in_window = [Complex::<f32>::zero(); FFT_WINDOW_LEN];
        let out_window = [Complex::<f32>::zero(); FFT_WINDOW_LEN];
        let fft = Fft::new(in_window, out_window);
        let inverse = false;
        let fft_planner = fft::Planner::new(inverse);

        // A buffer for retrieving the frequency amplitudes from the `fft`.
        let fft_frequency_amplitudes_2 = Box::new([0.0; FFT_WINDOW_LEN / 2]);

        // CPU saving mode is disabled by default.
        let cpu_saving_enabled = false;

        Model {
            sounds,
            speakers,
            installations,
            cpu_saving_enabled,
            gui_audio_monitor_msg_tx,
            osc_output_msg_tx,
            sound_buffer_tx,
            output_buffer_tx,
            num_active_sound_buffers,
            fft_planner,
            fft,
            fft_frequency_amplitudes_2,
        }
    }
}

/// Create a slice of channel detectors from the given number of channels.
fn new_channel_detectors(channels: usize) -> Box<[EnvDetector]> {
    (0..channels)
        .map(|_| EnvDetector::new())
        .collect::<Vec<_>>()
        .into_boxed_slice()
}

/// Add the given sound to the `Sounds` map.
fn new_sound(channels: usize) -> Sound {
    let channel_detectors = new_channel_detectors(channels);
    Sound { channel_detectors }
}

/// Spawn the audio detection thread, returning a handle that may be used for communication.
pub fn spawn(
    gui_audio_monitor_msg_tx: gui::monitor::Sender,
    osc_output_msg_tx: osc::output::Tx,
) -> Handle {
    let queue = Arc::new(SegQueue::new());
    let tx = queue.clone();
    let rx = queue;

    let sound_buffer_queue = Arc::new(SegQueue::new());
    let sound_buffer_tx = sound_buffer_queue.clone();
    let sound_buffer_rx = sound_buffer_queue;

    let output_buffer_queue = Arc::new(SegQueue::new());
    let output_buffer_tx = output_buffer_queue.clone();
    let output_buffer_rx = output_buffer_queue;

    let thread = thread::Builder::new()
        .name("audio_detection".into())
        .spawn(move || {
            run(gui_audio_monitor_msg_tx, osc_output_msg_tx, rx, sound_buffer_tx, output_buffer_tx);
        })
        .unwrap();
    let thread = Arc::new(Mutex::new(Some(thread)));

    Handle { tx, thread, sound_buffer_rx, output_buffer_rx }
}

/// The main loop for the detection thread.
fn run(
    gui_audio_monitor_msg_tx: gui::monitor::Sender,
    osc_output_msg_tx: osc::output::Tx,
    rx: MessageQueue,
    sound_buffer_tx: BufferQueue,
    output_buffer_tx: OutputBufferQueue,
) {
    let mut model = Model::new(
        gui_audio_monitor_msg_tx,
        osc_output_msg_tx,
        sound_buffer_tx,
        output_buffer_tx,
    );

    // Pre-prepare a bunch of sound buffers before the loop kicks off.
    const EST_NUM_SOUNDS: usize = 128;
    const SOUND_BUFFERS_TO_PREPARE: usize = EST_NUM_SOUNDS * BUFFERS_PER_SOUND;
    model.num_active_sound_buffers = SOUND_BUFFERS_TO_PREPARE;
    for _ in 0..SOUND_BUFFERS_TO_PREPARE {
        let buffer = Vec::with_capacity(FRAMES_PER_BUFFER * 2);
        model.sound_buffer_tx.push(buffer);
    }

    // Pre-prepare some output buffers.
    const OUTPUT_BUFFERS_TO_PREPARE: usize = 3;
    for _ in 0..OUTPUT_BUFFERS_TO_PREPARE {
        let buffer = Vec::with_capacity(FRAMES_PER_BUFFER * MAX_CHANNELS);
        let info = Default::default();
        model.output_buffer_tx.push((buffer, info));
    }

    // Begin the loop.
    loop {
        let msg = match rx.try_pop() {
            // If there are no messages waiting, sleep for a tiny bit to avoid rinsing cpu.
            None => {
                thread::sleep(time::Duration::from_millis(1));
                continue;
            },
            Some(msg) => msg,
        };

        match msg {
            // Insert the new sound into the map.
            Message::AddSound(sound_id, channels) => {

                let sound = new_sound(channels);
                model.sounds.insert(sound_id, sound);

                // Check whether or not we should add more buffers to the cycle.
                let min_num_sound_buffers = BUFFERS_PER_SOUND * model.sounds.len();
                if model.num_active_sound_buffers < min_num_sound_buffers {
                    for _ in model.num_active_sound_buffers..min_num_sound_buffers {
                        let samples_len = channels * FRAMES_PER_BUFFER;
                        let buffer = Vec::with_capacity(samples_len);
                        model.sound_buffer_tx.push(buffer);
                        model.num_active_sound_buffers += 1;
                    }
                }
            },

            // Update the detection for the sound with the given `Id`.
            Message::UpdateSound(sound_id, buffer) => {
                let Model {
                    ref mut sounds,
                    ref sound_buffer_tx,
                    ref gui_audio_monitor_msg_tx,
                    ..
                } = model;

                // Retrieve the sound from the map.
                let sound = match sounds.get_mut(&sound_id) {
                    None => {
                        let Buffer { samples, .. } = buffer;
                        sound_buffer_tx.push(samples);
                        continue;
                    },
                    Some(sound) => sound,
                };

                // Ensure that the channels match the channel detectors.
                if buffer.channels != sound.channel_detectors.len() {
                    sound.channel_detectors = new_channel_detectors(buffer.channels);
                }

                // Update the channel detectors.
                for (i, &sample) in buffer.iter().enumerate() {
                    let channel = i % buffer.channels;
                    sound.channel_detectors[channel].next(sample);
                }

                // Queue the buffer for re-use.
                let Buffer { samples, .. } = buffer;
                sound_buffer_tx.push(samples);

                // Send the audio detection state off to the GUI monitoring.
                for (index, env_detector) in sound.channel_detectors.iter().enumerate() {
                    let (rms, peak) = env_detector.current();
                    let sound_msg = gui::ActiveSoundMessage::UpdateChannel { index, rms, peak };
                    let msg = gui::AudioMonitorMessage::ActiveSound(sound_id, sound_msg);
                    gui_audio_monitor_msg_tx.push(msg);
                }
            },

            // The sound has ended and its state should be removed.
            Message::RemoveSound(sound_id) => {
                model.sounds.remove(&sound_id);
            },

            // Initialise detection state for the given installation if it does not already exit.
            Message::AddInstallation(installation_id, computers) => {
                let installation = model
                    .installations
                    .entry(installation_id)
                    .or_insert_with(|| {
                        let speaker_analyses = Vec::with_capacity(MAX_CHANNELS);
                        let summed_samples_of_all_channels = Vec::with_capacity(FRAMES_PER_BUFFER);
                        let fft_detector = FftDetector::new();
                        Installation {
                            speaker_analyses,
                            summed_samples_of_all_channels,
                            fft_detector,
                            computers,
                        }
                    });
                installation.computers = computers;
            },

            // Remove the given detection state for the given installation.
            Message::RemoveInstallation(installation_id) => {
                model.installations.remove(&installation_id);
            },

            // Perform analysis for the output buffer.
            //
            // 1. Update speaker peak and RMS detectors and send result to GUI.
            // 2. Update installation buffers with latest samples ready for FFT.
            // 3. Perform FFT on installations.
            // 4. Send analysis data on to OSC output.
            Message::Output(buffer, info) => {
                let Model {
                    ref mut speakers,
                    ref mut installations,
                    ref mut fft,
                    ref mut fft_planner,
                    ref mut fft_frequency_amplitudes_2,
                    ref gui_audio_monitor_msg_tx,
                    ref osc_output_msg_tx,
                    ref output_buffer_tx,
                    cpu_saving_enabled,
                    ..
                } = model;

                let Buffer { samples, channels } = buffer;
                let OutputInfo { speakers: speaker_infos } = info;

                // The number of frames in the buffer.
                let len_frames = samples.len() / channels;

                // Initialise the detection state for each installation.
                for installation in installations.values_mut() {
                    installation.speaker_analyses.clear();
                    installation.summed_samples_of_all_channels.resize(len_frames, 0.0);
                    installation.summed_samples_of_all_channels.iter_mut().for_each(|s| *s = 0.0);
                }

                // For each speaker, feed its amplitude into its detectors.
                for (&id, speaker) in &speaker_infos {
                    // Skip speakers that are out of range of the buffer.
                    if channels <= speaker.channel {
                        continue;
                    }

                    // Retrieve the detector state for the speaker.
                    let state = speakers
                        .entry(id)
                        .or_insert_with(|| Speaker { env_detector: EnvDetector::new() });

                    // Only update the envelope detector if CPU saving is not enabled.
                    let mut rms = 0.0;
                    let mut peak = 0.0;
                    if !cpu_saving_enabled {
                        // Update the envelope detector.
                        for frame in samples.chunks(channels) {
                            let sample = frame[speaker.channel];
                            state.env_detector.next(sample);
                        }

                        // The current env detector states.
                        let (current_rms, current_peak) = state.env_detector.current();
                        rms = current_rms;
                        peak = current_peak;

                        // Send the detector state for this speaker to the GUI.
                        let speaker_msg = gui::SpeakerMessage::Update { rms, peak };
                        let msg = gui::AudioMonitorMessage::Speaker(id, speaker_msg);
                        gui_audio_monitor_msg_tx.push(msg);
                    }

                    // Sum the data from this speaker onto the buffers all of its assigned installations.
                    for installation_id in &speaker.installations {
                        // Retrieve the detection state for this installation.
                        let installation = match installations.get_mut(installation_id) {
                            None => continue,
                            Some(installation) => installation,
                        };

                        // If the installation has no computers, skip it.
                        if installation.computers == 0 {
                            continue;
                        }

                        // Insert the speaker analysis for this speaker into the installation.
                        let index = speaker.channel;
                        let analysis = SpeakerAnalysis { peak, rms, index };
                        installation.speaker_analyses.push(analysis);

                        // Sum the audio data for the speaker onto its associated installation buffers.
                        installation
                            .summed_samples_of_all_channels
                            .iter_mut()
                            .zip(samples.chunks(channels))
                            .for_each(|(sample, frame)| {
                                *sample = frame[speaker.channel];
                            });
                    }
                }

                // Perform FFT and send collected analysis to OSC output thread.
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
                            ref summed_samples_of_all_channels,
                            ref speaker_analyses,
                            ref mut fft_detector,
                            ..
                        } = *installation;

                        let n_speakers = speaker_analyses.len();

                        // Feed the buffer into the FFT detector, normalised for the number of
                        // speakers in the installation.
                        for &sample in summed_samples_of_all_channels {
                            // TODO: This division might be more efficient on lmh and bins but not
                            // certain that it is correct/transitive.
                            fft_detector.push(sample / n_speakers as f32);
                        }

                        // Perform the FFT.
                        fft_detector.calc_fft(
                            fft_planner,
                            fft,
                            &mut fft_frequency_amplitudes_2[..],
                        );

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

                    // Sort the speakers by channel index as the OSC output thread assumes that
                    // speakers are in order of index.
                    installation.speaker_analyses.sort_by(|a, b| a.index.cmp(&b.index));

                    // Collect the speaker data.
                    let speakers = installation.speaker_analyses
                        .drain(..)
                        .map(|s| osc::output::Speaker {
                            rms: s.rms,
                            peak: s.peak,
                        })
                        .collect();

                    // Prepare the data for OSC.
                    let data = osc::output::AudioFrameData {
                        avg_peak,
                        avg_rms,
                        avg_fft,
                        speakers,
                    };

                    // Send to the OSC thread.
                    let msg = osc::output::Message::Audio(id, data);
                    osc_output_msg_tx.push(msg);
                }

                // Send buffer and output info back to audio thread for re-use.
                let info = OutputInfo { speakers: speaker_infos };
                output_buffer_tx.push((samples, info));
            },

            // Clear all project-specific detection data.
            Message::ClearProjectSpecificData => {
                model.sounds.clear();
                model.speakers.clear();
                model.installations.clear();
            },

            Message::CpuSavingEnabled(enabled) => {
                model.cpu_saving_enabled = enabled;
            }

            // Exit the loop as the app has exited.
            Message::Exit => {
                break
            },
        }
    }
}

impl ops::Deref for Buffer {
    type Target = [f32];
    fn deref(&self) -> &Self::Target {
        &self.samples
    }
}

impl ops::DerefMut for Buffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.samples
    }
}
