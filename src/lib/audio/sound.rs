use audio::{input, output, source, Source, SAMPLE_RATE};
use installation::Installation;
use metres::Metres;
use nannou::math::Point2;
use std::collections::HashSet;
use std::sync::{mpsc, Arc, Mutex};
use std::sync::atomic::{self, AtomicBool};
use time_calc::Ms;

/// `Sound`s can be thought of as a stack of three primary components:
///
/// 1. **Source**: for generating audio data (via oscillator, wave, audio input, etc).
/// 2. **Pre-spatial effects processing**: E.g. fades.
/// 3. **Spatial Output**: maps the sound from a position in space to the output channels.
pub struct Sound {
    // State shared with the handles.
    pub shared: Arc<Shared>,
    // The number of channels yielded by the `Sound`.
    pub channels: usize,
    // Includes the source and pre-spatial effects.
    //
    // The signal is unique in that channels are interleaved rather than presented side-by-side in
    // the `Frame` type itself. This allows having a dynamic number of channels.
    //
    // The sound is "complete" when the signal returns `None` and will be removed from the map on
    // the audio thread.
    //
    // TODO: This could potentially just be an actual type? `sound::Signal` that matched on
    // the source kind, stored its own stack of effects, etc?
    pub signal: source::Signal,
    // The location of the sound within the space.
    pub point: Point2<Metres>,
    pub spread: Metres,
    pub radians: f32,
    // Installations in which this sound can be played.
    pub installations: Installations,
}

/// A handle to a currently playing sound.
#[derive(Clone, Debug)]
pub struct Handle {
    shared: Arc<Shared>,
}

/// A handle to the necessary source-specific data.
#[derive(Debug)]
pub enum SourceHandle {
    Wav,
    Realtime {
        is_capturing: Arc<AtomicBool>,
    },
}

// State shared between multiple handles to a single sound.
#[derive(Debug)]
pub struct Shared {
    is_playing: AtomicBool,
    source_id: source::Id,
    id: Id,
    source: SourceHandle,
}

impl Shared {
    /// Whether or not the soundscape is currently playing.
    pub fn is_playing(&self) -> bool {
        self.is_playing.load(atomic::Ordering::Relaxed)
    }
}

impl Handle {
    /// Whether or not the soundscape is currently playing.
    pub fn is_playing(&self) -> bool {
        self.shared.is_playing()
    }

    /// Pauses the soundscape playback.
    ///
    /// Returns `false` if it was already paused.
    pub fn pause(&self) -> bool {
        let result = !self.is_playing() != false;
        if let SourceHandle::Realtime { ref is_capturing } = self.shared.source {
            is_capturing.store(false, atomic::Ordering::Relaxed);
        }
        self.shared.is_playing.store(false, atomic::Ordering::Relaxed);
        result
    }

    /// Plays the soundscape.
    ///
    /// Returns `false` if the it was already playing.
    pub fn play(&self) -> bool {
        let result = self.is_playing() != true;
        if let SourceHandle::Realtime { ref is_capturing } = self.shared.source {
            is_capturing.store(true, atomic::Ordering::Relaxed);
        }
        self.shared.is_playing.store(true, atomic::Ordering::Relaxed);
        result
    }

    /// The ID of the source used to generate this sound.
    pub fn source_id(&self) -> source::Id {
        self.shared.source_id
    }

    /// The ID of the sound associated with this handle.
    pub fn id(&self) -> Id {
        self.shared.id
    }
}

/// Creates a sound from the given `Source` and send it to the output stream.
///
/// If the sound is a realtime source, send the source end to the input stream.
pub fn spawn_from_source(
    id: Id,
    source_id: source::Id,
    source: &Source,
    initial_position: Point2<Metres>,
    continuous_preview: bool,
    input_stream: &input::Stream,
    output_stream: &output::Stream,
    latency: Ms,
) -> Handle
{
    let installations = source.role.clone().into();
    match source.kind {
        source::Kind::Wav(ref wav) => {
            spawn_from_wav(
                id,
                source_id,
                wav,
                source.spread,
                source.radians,
                installations,
                initial_position,
                continuous_preview,
                output_stream,
            )

        },
        source::Kind::Realtime(ref realtime) => {
            spawn_from_realtime(
                id,
                source_id,
                realtime,
                source.spread,
                source.radians,
                installations,
                initial_position,
                continuous_preview,
                input_stream,
                output_stream,
                latency,
            )
        },
    }
}

/// Creates a sound from the given `source::Wav` and send it to the output audio stream.
pub fn spawn_from_wav(
    id: Id,
    source_id: source::Id,
    wav: &source::Wav,
    spread: Metres,
    radians: f32,
    installations: Installations,
    initial_position: Point2<Metres>,
    continuous_preview: bool,
    audio_output: &output::Stream,
) -> Handle
{
    // The wave samples iterator.
    let samples = match wav.should_loop || continuous_preview {
        false => source::wav::SampleStream::from_path(&wav.path).unwrap().into(),
        true => source::wav::SampleStream::from_path(&wav.path).unwrap().cycle().into(),
    };

    // The source signal.
    let playback = wav.playback.clone();
    let signal = source::Signal::Wav { samples, playback };

    // Initialise the sound playing.
    let is_playing = AtomicBool::new(true);

    // State shared between the handles to the sound.
    let shared = Arc::new(Shared {
        is_playing,
        source_id,
        id,
        source: SourceHandle::Wav,
    });

    // The sound.
    let sound = Sound {
        shared: shared.clone(),
        channels: wav.channels,
        signal,
        point: initial_position,
        spread,
        radians,
        installations,
    };

    // Create the handle to the sound.
    let handle = Handle {
        shared,
    };

    // The output stream active sound.
    let output_active_sound = sound.into();

    // Send the active sound to the audio input thread.
    audio_output
        .send(move |audio| {
            audio.insert_sound(id, output_active_sound);
        })
        .ok();

    handle
}

/// Creates a sound from the given `source::Realtime` and send it to the output audio stream.
///
/// Also spawns the `input::ActiveSound` on the input audio stream.
pub fn spawn_from_realtime(
    id: Id,
    source_id: source::Id,
    realtime: &source::Realtime,
    spread: Metres,
    radians: f32,
    installations: Installations,
    initial_position: Point2<Metres>,
    continuous_preview: bool,
    audio_input: &input::Stream,
    audio_output: &output::Stream,
    latency: Ms,
) -> Handle {
    // The duration of the sound so that the realtime thread knows when to stop serving samples.
    let duration = if continuous_preview {
        input::Duration::Infinite
    } else {
        let frames = realtime.duration.samples(SAMPLE_RATE as _);
        input::Duration::Frames(frames as _)
    };

    // Add some latency in case input and output streams aren't synced.
    let n_channels = realtime.channels.len();
    let delay_frames = latency.samples(SAMPLE_RATE as _);
    let delay_samples = delay_frames as usize * n_channels;
    let sync_channel_len = delay_samples * 2;

    // The buffer used to send samples from audio input stream to audio output stream.
    let (sample_tx, sample_rx) = mpsc::sync_channel(sync_channel_len);

    // Insert the silence for the delay.
    for _ in 0..delay_samples {
        sample_tx.send(0.0).ok();
    }

    // The signal from which the sound will draw samples.
    let samples = source::realtime::Signal { sample_rx };
    let signal = source::Signal::Realtime { samples };

    // Initialise the sound playing.
    let is_playing = AtomicBool::new(true);
    let is_capturing = Arc::new(AtomicBool::new(true));

    // Create the `ActiveSound` for the input stream.
    let input_active_sound = input::ActiveSound {
        duration,
        sample_tx,
        is_capturing: is_capturing.clone(),
    };

    // State shared between the handles to a realtime sound.
    let source_handle = SourceHandle::Realtime {
        is_capturing,
    };

    // State shared between the handles to the sound.
    let shared = Arc::new(Shared {
        is_playing,
        source_id,
        id,
        source: source_handle,
    });

    // Create the sound.
    let sound = Sound {
        shared: shared.clone(),
        channels: n_channels,
        signal,
        point: initial_position,
        spread,
        radians,
        installations,
    };

    // Create the handle to the sound.
    let handle = Handle {
        shared,
    };

    // The output stream active sound.
    let output_active_sound = sound.into();

    // Send the active sound to the audio input thread.
    audio_input
        .send(move |audio| {
            audio
                .active_sounds
                .entry(source_id)
                .or_insert_with(Vec::new)
                .push(input_active_sound);
        })
        .ok();

    // Send the active sound to the audio input thread.
    audio_output
        .send(move |audio| {
            audio.insert_sound(id, output_active_sound);
        })
        .ok();

    handle
}

impl Sound {
    /// The ID of the source used to generate this sound.
    pub fn source_id(&self) -> source::Id {
        self.shared.source_id
    }

    /// The ID of the sound associated with this handle.
    pub fn id(&self) -> Id {
        self.shared.id
    }
}

#[derive(Debug)]
pub enum Installations {
    All,
    Set(HashSet<Installation>),
}

impl From<Option<source::Role>> for Installations {
    fn from(role: Option<source::Role>) -> Self {
        match role {
            None => Installations::All,
            Some(role) => role.into(),
        }
    }
}

impl From<source::Role> for Installations {
    fn from(role: source::Role) -> Self {
        match role {
            source::Role::Soundscape(s) => Installations::Set(s.installations),
            _ => Installations::All,
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct Id(u64);

impl Id {
    const INITIAL: Self = Id(0);
}

/// A threadsafe unique `Id` generator for sharing between the `Composer` and `GUI` threads.
#[derive(Clone)]
pub struct IdGenerator {
    next: Arc<Mutex<Id>>,
}

impl IdGenerator {
    pub fn new() -> Self {
        IdGenerator {
            next: Arc::new(Mutex::new(Id::INITIAL)),
        }
    }

    pub fn generate_next(&self) -> Id {
        let mut next = self.next.lock().unwrap();
        let id = *next;
        *next = Id(id.0.wrapping_add(1));
        id
    }
}
