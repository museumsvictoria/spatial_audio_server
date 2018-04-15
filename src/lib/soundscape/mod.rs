use audio;
use installation::{self, Installation};
use metres::Metres;
use mindtree_utils::noise_walk;
use nannou;
use nannou::prelude::*;
use nannou::rand::{Rng, SeedableRng, XorShiftRng};
use std::cmp;
use std::collections::{HashMap, HashSet};
use std::ops;
use std::sync::atomic::AtomicBool;
use std::sync::{atomic, mpsc, Arc, Mutex};
use std::thread;
use std::time;
use time_calc::Ms;
use utils::{self, duration_to_secs, Range, Seed};

pub use self::group::Group;
pub use self::movement::Movement;
use self::movement::BoundingRect;

pub mod group;
pub mod movement;

const TICK_RATE_MS: u64 = 16;

type Installations = HashMap<Installation, installation::Soundscape>;
type Groups = HashMap<group::Id, Group>;
type Sources = HashMap<audio::source::Id, Source>;
type Speakers = HashMap<audio::speaker::Id, Speaker>;
type GroupsLastUsed = HashMap<group::Id, time::Instant>;
type SourcesLastUsed = HashMap<audio::source::Id, time::Instant>;
type InstallationAreas = HashMap<Installation, movement::Area>;
type InstallationSpeakers = HashMap<Installation, Vec<audio::speaker::Id>>;
type ActiveSounds = HashMap<audio::sound::Id, ActiveSound>;
type ActiveSoundPositions = HashMap<audio::sound::Id, ActiveSoundPosition>;
type ActiveSoundsPerInstallation = HashMap<Installation, Vec<audio::sound::Id>>;
type TargetSoundsPerInstallation = HashMap<Installation, usize>;

/// The kinds of messages received by the soundscape thread.
pub enum Message {
    /// Updates to the soundscape state from other threads.
    Update(UpdateFn),
    /// Steps forward the soundscape.
    Tick(Tick),
    /// Play all active sounds.
    Play,
    /// Pause all active sounds.
    Pause,
    /// Stop running the soundscape and exit.
    Exit,
}

#[derive(Copy, Clone, Debug)]
pub struct Tick {
    instant: time::Instant,
    /// The time that accumulated since the last tick occurred only while playback was enabled.
    since_last_tick: time::Duration,
    /// The total duration over which the soundscape has played.
    ///
    /// This does not increase when the stream is paused.
    playback_duration: time::Duration,
}

/// The update function applied to a source.
///
/// This is a workaround for the current inability to call a `Box<FnOnce>`
pub struct UpdateFn {
    function: Box<FnMut(&mut Model) + Send>,
}

/// The handle to the soundscape that can be used and shared amonth the main thread.
#[derive(Clone)]
pub struct Soundscape {
    tx: mpsc::Sender<Message>,
    /// Keep the thread handle in an `Option` so we can take it from the mutex upon exit.
    thread: Arc<Mutex<Option<thread::JoinHandle<()>>>>,
    /// Whether or not the soundscape is currently playing.
    is_playing: Arc<AtomicBool>,
}

/// Data related to a single speaker that is relevant to the soundscape.
#[derive(Clone, Debug)]
pub struct Speaker {
    /// The position of the speaker in metres.
    pub point: Point2<Metres>,
    /// All installations assigned to the speaker.
    pub installations: HashSet<Installation>,
}

/// Properties of an audio source that are relevant to the soundscape thread.
pub struct Source {
    pub constraints: audio::source::Soundscape,
    pub kind: audio::source::Kind,
    pub spread: Metres,
    pub channel_radians: f32,
    pub volume: f32,
    pub muted: bool,
    /// The time at which the source was last used to create a sound.
    pub last_sound_created: Option<time::Instant>,
}

/// Represents a currently active sound spawned by the soundscape thread.
pub struct ActiveSound {
    /// The installation for which this sound was initially spawned.
    pub initial_installation: Installation,
    /// State related to active sound's assigned movement.
    pub movement: Movement,
    /// The handle associated with this sound.
    handle: audio::sound::Handle,
}

// The current positioning of an active sound.
struct ActiveSoundPosition {
    // The source from which this active sound was produced.
    source_id: audio::source::Id,
    // The current location of the active sound.
    position: audio::sound::Position,
}

/// The model containing all state running on the soundscape thread.
pub struct Model {
    /// The latency applied to realtime sounds when spawned.
    pub realtime_source_latency: Ms,
    /// The soundscape's deterministic source of randomness.
    seed: Seed,
    /// How long the soundscape has been actively playing (in an un-paused state).
    ///
    /// This is updated upon each `Tick`.
    playback_duration: time::Duration,
    /// All installations within the exhibition.
    installations: Installations,
    /// Constraints for collections of sources.
    groups: Groups,
    /// All sources available to the soundscape for producing audio.
    sources: Sources,
    /// All speakers within the exhibition.
    speakers: Speakers,
    /// The moment at which each `Group` was last used to spawn a sound.
    groups_last_used: GroupsLastUsed,
    /// The moment at which each `Source` was last used to spawn a sound.
    sources_last_used: SourcesLastUsed,

    /// All sounds currently being played that were spawned by the soundscape thread.
    active_sounds: ActiveSounds,

    /// Tracks the speakers assignned to each installation. Updated at the beginning of each tick.
    installation_speakers: InstallationSpeakers,
    /// This tracks the bounding area for each installation at the beginning of each tick.
    installation_areas: InstallationAreas,
    /// A handle for submitting new sounds to the input stream.
    audio_input_stream: audio::input::Stream,
    /// A handle for submitting new sounds to the output stream.
    audio_output_stream: audio::output::Stream,
    /// For generating unique IDs for each new sound.
    sound_id_gen: audio::sound::IdGenerator,
    // A handle to the ticker thread.
    tick_thread: thread::JoinHandle<()>,
}

impl ActiveSound {
    /// The current location and orientation of the active sound.
    pub fn position(&self) -> audio::sound::Position {
        self.movement.position()
    }

    /// A simplified view of the active sound's position.
    fn active_sound_position(&self) -> ActiveSoundPosition {
        let position = self.position();
        let source_id = self.handle.source_id();
        ActiveSoundPosition {
            source_id,
            position
        }
    }
}

impl Speaker {
    pub fn from_audio_speaker(s: &audio::Speaker) -> Self {
        Speaker {
            point: s.point,
            installations: s.installations.clone(),
        }
    }
}

impl Source {
    /// Create a `soundscape::Source` from an `audio::Source`.
    ///
    /// Returns `None` if the given audio source does not have the `Soundscape` role.
    pub fn from_audio_source(source: &audio::Source) -> Option<Self> {
        let constraints = match source.role {
            Some(audio::source::Role::Soundscape(ref source)) => source.clone(),
            _ => return None,
        };
        let kind = source.kind.clone();
        let spread = source.spread;
        let channel_radians = source.channel_radians;
        let volume = source.volume;
        let muted = source.muted;
        let last_sound_created = None;
        Some(Source {
            constraints,
            kind,
            spread,
            channel_radians,
            volume,
            muted,
            last_sound_created,
        })
    }

    /// Create an `audio::Source`, used for creating `Sound`s.
    pub fn to_audio_source(&self) -> audio::Source {
        let kind = self.kind.clone();
        let role = Some(audio::source::Role::Soundscape(self.constraints.clone()));
        let spread = self.spread;
        let channel_radians = self.channel_radians;
        let volume = self.volume;
        let muted = self.muted;
        audio::Source {
            kind,
            role,
            spread,
            channel_radians,
            volume,
            muted,
        }
    }
}

impl Soundscape {
    /// Send a `FnOnce(&mut Model)` function to update the soundscape thread model.
    pub fn send<F>(&self, update: F) -> Result<(), mpsc::SendError<()>>
    where
        F: FnOnce(&mut Model) + Send + 'static,
    {
        let update = UpdateFn::from(update);
        let msg = Message::Update(update);
        if let Err(mpsc::SendError(_)) = self.tx.send(msg) {
            return Err(mpsc::SendError(()));
        }
        Ok(())
    }

    /// Whether or not the soundscape is currently playing.
    pub fn is_playing(&self) -> bool {
        self.is_playing.load(atomic::Ordering::Relaxed)
    }

    /// Pauses the soundscape playback.
    ///
    /// Returns `false` if it was already paused.
    pub fn pause(&self) -> Result<bool, mpsc::SendError<()>> {
        let result = !self.is_playing() != false;
        let msg = Message::Pause;
        self.is_playing.store(false, atomic::Ordering::Relaxed);
        self.tx
            .send(msg)
            .map(|_| result)
            .map_err(|_| mpsc::SendError(()))
    }

    /// Plays the soundscape.
    pub fn play(&self) -> Result<bool, mpsc::SendError<()>> {
        let result = self.is_playing() != true;
        let msg = Message::Play;
        self.is_playing.store(true, atomic::Ordering::Relaxed);
        self.tx
            .send(msg)
            .map(|_| result)
            .map_err(|_| mpsc::SendError(()))
    }

    /// Stops the soundscape thread and returns the raw handle to its thread.
    pub fn exit(self) -> Option<thread::JoinHandle<()>> {
        self.tx.send(Message::Exit).ok();
        self.thread.lock().unwrap().take()
    }
}

impl Model {
    /// Insert a new installation.
    pub fn insert_installation(
        &mut self,
        installation: Installation,
        state: installation::Soundscape,
    ) -> Option<installation::Soundscape> {
        self.installations.insert(installation, state)
    }

    /// Update the given installation's state.
    ///
    /// Returns `false` if the installation was not there.
    pub fn update_installation<F>(&mut self, installation: &Installation, update: F) -> bool
    where
        F: FnOnce(&mut installation::Soundscape),
    {
        match self.installations.get_mut(installation) {
            None => false,
            Some(i) => {
                update(i);
                true
            }
        }
    }

    /// Insert a new soundscape group.
    pub fn insert_group(&mut self, id: group::Id, group: Group) -> Option<Group> {
        self.groups.insert(id, group)
    }

    /// Updates the group with the given function.
    ///
    /// Returns `false` if the group wasn't there.
    pub fn update_group<F>(&mut self, id: &group::Id, update: F) -> bool
    where
        F: FnOnce(&mut Group),
    {
        match self.groups.get_mut(id) {
            None => false,
            Some(s) => {
                update(s);
                true
            }
        }
    }

    /// Remove the given soundscape group.
    pub fn remove_group(&mut self, id: &group::Id) -> Option<Group> {
        self.groups.remove(id)
    }

    /// Insert a speaker into the inner map.
    pub fn insert_speaker(&mut self, id: audio::speaker::Id, speaker: Speaker) -> Option<Speaker> {
        self.speakers.insert(id, speaker)
    }

    /// Updates the speaker with the given function.
    ///
    /// Returns `false` if the speaker wasn't there.
    pub fn update_speaker<F>(&mut self, id: &audio::speaker::Id, update: F) -> bool
    where
        F: FnOnce(&mut Speaker),
    {
        match self.speakers.get_mut(id) {
            None => false,
            Some(s) => {
                update(s);
                true
            }
        }
    }

    /// Remove a speaker from the inner hashmap.
    pub fn remove_speaker(&mut self, id: &audio::speaker::Id) -> Option<Speaker> {
        self.speakers.remove(id)
    }

    /// Insert a source into the inner hashmap.
    pub fn insert_source(&mut self, id: audio::source::Id, source: Source) -> Option<Source> {
        self.sources.insert(id, source)
    }

    /// Updates the source with the given function.
    ///
    /// Returns `false` if the source wasn't there.
    pub fn update_source<F>(&mut self, id: &audio::source::Id, update: F) -> bool
    where
        F: FnOnce(&mut Source),
    {
        match self.sources.get_mut(id) {
            None => false,
            Some(s) => {
                update(s);
                true
            }
        }
    }

    /// Updates the given source and all related sounds for the given new set of audio source
    /// movement constraints.
    pub fn update_source_movement(
        &mut self,
        source_id: &audio::source::Id,
        movement: &audio::source::Movement,
    ) {
        // First, update the source at the given `Id`.
        let clone = movement.clone();
        self.update_source(source_id, |source| source.movement = clone);

        // Now update all active sounds that use this source.
        let Model {
            seed,
            ref playback_duration,
            ref sources,
            ref speakers,
            ref installations,
            ref mut active_sounds,
            ..
        } = *self;

        // Collect the necessary data for generating a `Movement` instance from the constraints.
        let mut installation_speakers = InstallationSpeakers::default();
        update_installation_speakers(speakers, &mut installation_speakers);
        let mut installation_areas = InstallationAreas::default();
        update_installation_areas(speakers, &installation_speakers, &mut installation_areas);
        let mut target_sounds_per_installation = TargetSoundsPerInstallation::default();
        update_target_sounds_per_installation(
            seed,
            playback_duration,
            installations,
            &mut target_sounds_per_installation,
        );
        let active_sound_positions = active_sound_positions(active_sounds);

        // Collect a map of all new movements.
        for (sound_id, sound) in active_sound_positions {
            // Only update sounds that use this source.
            if sound.source_id != *source_id {
                continue;
            }

            // Find the installation assigned to this sound.
            match closest_assigned_installation(&sound, sources, &installation_areas) {
                None => continue,
                Some(installation) => {
                    // Generate the movement.
                    let movement = generate_movement(
                        *source_id,
                        sources,
                        installation,
                        installations,
                        &installation_areas,
                        &target_sounds_per_installation,
                        &active_sounds,
                    );
                    // Update the sound.
                    active_sounds.get_mut(&sound_id).unwrap().movement = movement;
                },
            }
        }
    }

    /// Remove a source from the inner hashmap.
    pub fn remove_source(&mut self, id: &audio::source::Id) -> Option<Source> {
        self.active_sounds
            .retain(|_, s| *id != s.handle.source_id());
        self.sources.remove(id)
    }

    /// Remove an active sound from the hashmap.
    pub fn remove_active_sound(&mut self, id: &audio::sound::Id) -> Option<ActiveSound> {
        self.active_sounds.remove(id)
    }

    /// Update the state of all active sounds spawned via the source with the given `Id`.
    ///
    /// Returns the number of active sounds that were updated.
    pub fn update_active_sounds_with_source<F>(
        &mut self,
        source_id: audio::source::Id,
        mut update: F,
    ) -> usize
    where
        F: FnMut(&audio::sound::Id, &mut ActiveSound),
    {
        let mut count = 0;
        for (id, sound) in self.active_sounds.iter_mut() {
            if sound.handle.source_id() == source_id {
                update(&id, sound);
                count += 1;
            }
        }
        count
    }
}

impl UpdateFn {
    // Consume self and call the update function with the given source.
    fn call(mut self, model: &mut Model) {
        (self.function)(model)
    }
}

impl From<UpdateFn> for Message {
    fn from(f: UpdateFn) -> Self {
        Message::Update(f)
    }
}

impl<F> From<F> for UpdateFn
where
    F: FnOnce(&mut Model) + Send + 'static,
{
    fn from(f: F) -> Self {
        let mut f_opt = Some(f);
        let fn_mut = move |source: &mut Model| {
            if let Some(f) = f_opt.take() {
                f(source);
            }
        };
        UpdateFn {
            function: Box::new(fn_mut) as _,
        }
    }
}

impl ops::Deref for Source {
    type Target = audio::source::Soundscape;
    fn deref(&self) -> &Self::Target {
        &self.constraints
    }
}

impl ops::DerefMut for Source {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.constraints
    }
}

impl ops::Deref for ActiveSound {
    type Target = audio::sound::Handle;
    fn deref(&self) -> &Self::Target {
        &self.handle
    }
}

impl ops::DerefMut for ActiveSound {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.handle
    }
}

/// Spawn the "soundscape" thread and return a handle to it.
///
/// The role of the soundscape thread is as follows:
///
/// 1. Decide when to introduce new sounds based on the properties of the currently playing sounds.
/// 2. Compose `Sound`s from a stack of `Source` -> `[Effect]`.
/// 3. Compose the path of travel through the space (including rotations for multi-channel sounds).
/// 4. Send the `Sound`s to the audio thread and accompanying monitoring stuff to the GUI thread
///    (for tracking positions, RMS, etc).
pub fn spawn(
    seed: Seed,
    tx: mpsc::Sender<Message>,
    rx: mpsc::Receiver<Message>,
    audio_input_stream: audio::input::Stream,
    audio_output_stream: audio::output::Stream,
    sound_id_gen: audio::sound::IdGenerator,
) -> Soundscape {
    let is_playing = Arc::new(AtomicBool::new(true));

    // Spawn a thread to generate and send ticks.
    let tick_tx = tx.clone();
    let tick_is_playing = is_playing.clone();
    let tick_thread = thread::Builder::new()
        .name("soundscape_ticker".into())
        .spawn(move || {
            let mut last = time::Instant::now();
            let mut playback_duration = time::Duration::from_secs(0);
            loop {
                thread::sleep(time::Duration::from_millis(TICK_RATE_MS));
                let instant = time::Instant::now();
                let since_last_tick = instant.duration_since(last);
                last = instant;
                if !tick_is_playing.load(atomic::Ordering::Relaxed) {
                    continue;
                }
                playback_duration += since_last_tick;
                let tick = Tick {
                    instant,
                    since_last_tick,
                    playback_duration,
                };
                if tick_tx.send(Message::Tick(tick)).is_err() {
                    break;
                }
            }
        })
        .unwrap();

    // The model maintaining state between messages.
    let realtime_source_latency = audio::DEFAULT_REALTIME_SOURCE_LATENCY;
    let playback_duration = time::Duration::from_secs(0);
    let installations = Default::default();
    let groups = Default::default();
    let sources = Default::default();
    let speakers = Default::default();
    let active_sounds = Default::default();
    let installation_speakers = Default::default();
    let installation_areas = Default::default();
    let groups_last_used = Default::default();
    let sources_last_used = Default::default();
    let model = Model {
        realtime_source_latency,
        seed,
        playback_duration,
        installations,
        groups,
        sources,
        speakers,
        active_sounds,
        groups_last_used,
        sources_last_used,
        installation_speakers,
        installation_areas,
        audio_input_stream,
        audio_output_stream,
        sound_id_gen,
        tick_thread,
    };

    // Spawn the soundscape thread.
    let thread = thread::Builder::new()
        .name("soundscape".into())
        .spawn(move || run(model, rx))
        .unwrap();
    let thread = Arc::new(Mutex::new(Some(thread)));
    Soundscape {
        tx,
        thread,
        is_playing,
    }
}

// A blocking function that is run on the unique soundscape thread (called by spawn).
fn run(mut model: Model, msg_rx: mpsc::Receiver<Message>) {
    // Wait for messages.
    for msg in msg_rx {
        match msg {
            // An update from another thread.
            Message::Update(update) => update.call(&mut model),

            // Break from the loop and finish the thread.
            Message::Exit => break,

            // Step forward the state of the soundscape.
            Message::Tick(t) => tick(&mut model, t),

            // Play all active sounds.
            Message::Play => {
                for sound in model.active_sounds.values() {
                    sound.handle.play();
                }
            }

            // Pause all active sounds.
            Message::Pause => {
                for sound in model.active_sounds.values() {
                    sound.handle.pause();
                }
            }
        }
    }
}

// Convert a map of active sounds to a map of data only relevant to their positions.
fn active_sound_positions(active_sounds: &ActiveSounds) -> ActiveSoundPositions {
    active_sounds.iter()
        .map(|(&id, sound)| (id, sound.active_sound_position()))
        .collect()
}

// Find the sound's closest assigned installation.
//
// Returns `None` if:
//
// - installation_areas is empty
// - no installations are assigned
// - there are no sources for the sound's current source `Id`
fn closest_assigned_installation(
    sound: &ActiveSoundPosition,
    sources: &Sources,
    installation_areas: &InstallationAreas,
) -> Option<Installation>
{
    if let Some(source) = sources.get(&sound.source_id) {
        let sound_point = Point2 {
            x: sound.position.x.0,
            y: sound.position.y.0,
        };
        let mut distances = source
            .constraints
            .installations
            .iter()
            .filter_map(|&i| installation_areas.get(&i).map(|a| (i, a)))
            .map(|(i, a)| {
                let centroid = Point2 {
                    x: a.centroid.x.0,
                    y: a.centroid.y.0,
                };
                (i, sound_point.distance2(centroid))
            });
        if let Some((i, dist)) = distances.next() {
            let (closest_installation, _) = distances.fold(
                (i, dist),
                |(ia, min), (ib, dist)| {
                    if dist < min {
                        (ib, dist)
                    } else {
                        (ia, min)
                    }
                },
            );
            return Some(closest_installation);
        }
    }
    None
}

// Group the active sounds via the installation they are currently closest to.
fn active_sounds_per_installation(
    active_sound_positions: &ActiveSoundPositions,
    sources: &Sources,
    installation_areas: &InstallationAreas,
) -> ActiveSoundsPerInstallation
{
    let mut active_sounds_per_installation = ActiveSoundsPerInstallation::default();
    for (&sound_id, sound) in active_sound_positions.iter() {
        if let Some(inst) = closest_assigned_installation(sound, sources, installation_areas) {
            active_sounds_per_installation
                .entry(inst)
                .or_insert_with(Vec::new)
                .push(sound_id);
        }
    }
    active_sounds_per_installation
}

// Collect the necessary data for each installation relevant to the agent.
fn agent_installation_data(
    source_id: audio::source::Id,
    sources: &Sources,
    installations: &Installations,
    installation_areas: &InstallationAreas,
    target_sounds_per_installation: &TargetSoundsPerInstallation,
    active_sound_positions: &ActiveSoundPositions,
) -> movement::agent::InstallationDataMap
{
    // We can't find installation data if there is no source for the given id.
    let source = match sources.get(&source_id) {
        None => {
            eprintln!("`agent_installation_data`: no source found for given `source::Id`");
            return Default::default();
        },
        Some(s) => s,
    };

    // Group the sounds by the installations that they are closest to.
    let active_sounds_per_installation =
        active_sounds_per_installation(active_sound_positions, sources, installation_areas);

    // For each assigned installation, collect the necessary installation data.
    //
    // Installations that have no assigned speakers (and in turn no area) are discluded.
    source
        .installations
        .iter()
        .filter_map(|inst| {
            let area = match installation_areas.get(inst) {
                None => return None,
                Some(area) => area.clone()
            };
            let range = &installations[inst].simultaneous_sounds;
            let current_num_sounds = active_sounds_per_installation
                .get(inst)
                .map(|sounds| {
                    sounds
                        .iter()
                        .filter(|s| active_sound_positions[s].source_id == source_id)
                        .count()
                })
                .unwrap_or(0);
            let target_num_sounds = target_sounds_per_installation[inst];
            let num_sounds_needed_to_reach_target =
                target_num_sounds as i32 - current_num_sounds as i32;
            let num_sounds_needed = if current_num_sounds < range.min {
                range.min - current_num_sounds
            } else {
                0
            };
            let num_available_sounds = if current_num_sounds < range.max {
                range.max - current_num_sounds
            } else {
                0
            };
            let data = movement::agent::InstallationData {
                area,
                num_sounds_needed_to_reach_target,
                num_sounds_needed,
                num_available_sounds,
            };
            Some((*inst, data))
        })
        .collect()
}

// Generate a movement for some source within some given installation.
fn generate_movement(
    source_id: audio::source::Id,
    sources: &Sources,
    installation: Installation,
    installations: &Installations,
    installation_areas: &InstallationAreas,
    target_sounds_per_installation: &TargetSoundsPerInstallation,
    active_sounds: &ActiveSounds,
) -> Movement {
    match sources[&source_id].movement {
        audio::source::Movement::Fixed(ref pos) => {
            let area = installation_areas
                .get(&installation)
                .expect("no area for the given installation");
            let x = area.bounding_rect.left + area.bounding_rect.width() * pos.x;
            let y = area.bounding_rect.bottom + area.bounding_rect.height() * pos.y;
            let point = pt2(x, y);
            let radians = 0.0;
            let position = audio::sound::Position { point, radians };
            Movement::Fixed(position)
        },
        audio::source::Movement::Generative(ref gen) => match *gen {
            audio::source::movement::Generative::Agent(ref agent) => {
                let mut rng = nannou::rand::thread_rng();
                // TODO: Should these be skewed?
                let r = &agent.max_speed;
                let max_speed = map_range(rng.gen(), 0f64, 1.0, r.min, r.max);
                let r = &agent.max_force;
                let max_force = map_range(rng.gen(), 0f64, 1.0, r.min, r.max);
                let active_sound_positions = active_sound_positions(active_sounds);
                let installation_data = agent_installation_data(
                    source_id,
                    sources,
                    installations,
                    installation_areas,
                    &target_sounds_per_installation,
                    &active_sound_positions,
                );
                let agent = movement::Agent::generate(
                    rng,
                    installation,
                    &installation_data,
                    max_speed,
                    max_force,
                );
                let generative = movement::Generative::Agent(agent);
                let movement = Movement::Generative(generative);
                movement
            },

            audio::source::movement::Generative::Ngon(ref ngon) => {
                let mut rng = nannou::rand::thread_rng();
                // TODO: Should these be skewed?
                let r = &ngon.vertices;
                let vertices = map_range(rng.gen(), 0f64, 1.0, r.min, r.max);
                let r = &ngon.nth;
                let nth = map_range(rng.gen(), 0f64, 1.0, r.min, r.max);
                let r = &ngon.speed;
                let speed = map_range(rng.gen(), 0f64, 1.0, r.min, r.max);
                let r = &ngon.radians_offset;
                let radians_offset = map_range(rng.gen(), 0f64, 1.0, r.min, r.max);
                let bounding_rect = &installation_areas[&installation].bounding_rect;
                let ngon = movement::Ngon::new(
                    vertices,
                    nth,
                    ngon.normalised_dimensions,
                    radians_offset,
                    speed,
                    bounding_rect,
                );
                let generative = movement::Generative::Ngon(ngon);
                let movement = Movement::Generative(generative);
                movement
            }
        },
    }
}

// A unique, constant seed associated with the installation.
fn installation_seed(installation: &Installation) -> [u32; 4] {
    // Convert the installation to its integer representation.
    let u = installation.to_u32();
    let seed = [u; 4];
    seed
}

// Update the map from installations to speakers.
fn update_installation_speakers(
    speakers: &Speakers,
    installation_speakers: &mut InstallationSpeakers,
) {
    for speakers in installation_speakers.values_mut() {
        speakers.clear();
    }
    for (&id, speaker) in speakers {
        for &installation in &speaker.installations {
            installation_speakers
                .entry(installation)
                .or_insert_with(Default::default)
                .push(id);
        }
    }
}

// Update the map from installations to their areas.
//
// An installations `Area` is determined via the assigned speaker locations.
fn update_installation_areas(
    speakers: &Speakers,
    installation_speakers: &InstallationSpeakers,
    installation_areas: &mut InstallationAreas,
) {
    installation_areas.clear();
    for (&installation, installation_speakers) in installation_speakers {
        let speaker_points = || installation_speakers.iter().map(|id| speakers[id].point);
        let bounding_rect = match BoundingRect::from_points(speaker_points()) {
            None => continue,
            Some(rect) => rect,
        };
        let centroid = match nannou::geom::centroid(speaker_points().map(|p| pt2(p.x.0, p.y.0))) {
            None => continue,
            Some(p) => pt2(Metres(p.x), Metres(p.y)),
        };
        let area = movement::Area {
            bounding_rect,
            centroid,
        };
        installation_areas.insert(installation, area);
    }
}

// Determine the target number of sounds for the given installation.
//
// We can determine this in a purely functional manner by using the playback duration as the phase
// for a noise_walk signal.
fn installation_target_sounds(
    seed: Seed,
    playback_duration: &time::Duration,
    installation: &Installation,
    constraints: &installation::Soundscape,
) -> usize {
    let playback_secs = duration_to_secs(playback_duration);
    // Update the target number of sounds very slowly. Say, once every 5 minutes.
    let hr_secs = 1.0 * 60.0 * 60.0;
    let hz = 1.0 / hr_secs;
    // Offset the phase using the `Installation` as a unique seed.
    let mut noise_walk_seed = utils::add_seeds(&seed, &installation_seed(&installation));
    if noise_walk_seed == [0, 0, 0, 0] {
        noise_walk_seed[0] = 1;
    }
    let mut rng = XorShiftRng::from_seed(noise_walk_seed);
    let phase_offset: f64 = rng.gen();
    let phase = phase_offset + playback_secs * hz;
    // Amplify the noise_walk slightly so that it occasionally reaches min and max.
    let amp = (noise_walk(phase) * 1.5).min(1.0).max(-1.0);
    let normalised_amp = amp * 0.5 + 0.5;
    let range = &constraints.simultaneous_sounds;
    let range_diff = range.max - range.min;
    (range.min as f64 + normalised_amp * range_diff as f64) as usize
}

// Determine the target number of sounds per installation.
//
// We can determine this in a purely functional manner by using the playback duration as the phase
// for a noise_walk signal.
fn update_target_sounds_per_installation(
    seed: Seed,
    playback_duration: &time::Duration,
    installations: &Installations,
    target_sounds_per_installation: &mut TargetSoundsPerInstallation,
) {
    target_sounds_per_installation.clear();
    for (installation, constraints) in installations {
        let target_num_sounds = installation_target_sounds(
            seed,
            playback_duration,
            installation,
            constraints,
        );
        target_sounds_per_installation.insert(*installation, target_num_sounds);
    }
}

// Called each time the soundscape thread receives a tick.
fn tick(model: &mut Model, tick: Tick) {
    let Model {
        realtime_source_latency,
        seed,
        ref mut playback_duration,
        ref installations,
        ref groups,
        ref speakers,
        ref sources,
        ref mut groups_last_used,
        ref mut sources_last_used,
        ref mut active_sounds,
        ref mut installation_speakers,
        ref mut installation_areas,
        ref mut sound_id_gen,
        ref audio_input_stream,
        ref audio_output_stream,
        ..
    } = *model;

    // Update the playback duration so far.
    *playback_duration = tick.playback_duration;

    // Update the map from installations to speakers.
    update_installation_speakers(speakers, installation_speakers);

    // Create the map from installations to their areas.
    //
    // An installations `Area` is determined via the assigned speaker locations.
    update_installation_areas(speakers, installation_speakers, installation_areas);

    // Determine the target number of sounds per installation.
    //
    // We can determine this in a purely functional manner by using the playback duration as the
    // phase for a noise_walk signal.
    let mut target_sounds_per_installation: TargetSoundsPerInstallation = HashMap::default();
    update_target_sounds_per_installation(
        seed,
        &tick.playback_duration,
        installations,
        &mut target_sounds_per_installation,
    );

    // Update the movement of each active sound.
    {
        let mut rng = nannou::rand::thread_rng();
        let active_sound_positions = active_sound_positions(active_sounds);
        for (&sound_id, sound) in active_sounds.iter_mut() {
            let initial_installation_area = installation_areas.get(&sound.initial_installation);
            match sound.movement {
                Movement::Fixed(_) => (),
                Movement::Generative(ref mut generative) => match *generative {
                    movement::Generative::Agent(ref mut agent) => {
                        let source_id = sound.handle.source_id();
                        let installation_data = agent_installation_data(
                            source_id,
                            sources,
                            installations,
                            installation_areas,
                            &target_sounds_per_installation,
                            &active_sound_positions,
                        );
                        agent.update(&mut rng, &tick.since_last_tick, &installation_data);
                    },
                    movement::Generative::Ngon(ref mut ngon) => {
                        if let Some(area) = initial_installation_area {
                            ngon.update(&tick.since_last_tick, &area.bounding_rect);
                        }
                    },
                },
            }

            // Update the position of the sounds on the audio thread.
            //
            // The audio thread will then notify the GUI of the new position upon the next rendered
            // buffer.
            let position = sound.position();
            audio_output_stream
                .send(move |audio| {
                    audio.update_sound(&sound_id, move |sound| {
                        sound.position = position;
                    });
                })
                .ok();
        }
    }

    // For each installation, check the number of sounds that are playing.
    //
    // Sound/Installation associations are determined by finding the installation's centroid that
    // is closest to each sound (as long as that installation is one of those assigned to the
    // sound's source).
    let active_sounds_per_installation = {
        let active_sound_positions = active_sound_positions(active_sounds);
        active_sounds_per_installation(&active_sound_positions, sources, installation_areas)
    };

    // Determine how many sounds to add (if any) by finding the difference between the target
    // number and actual number.
    for (installation, &num_target_sounds) in &target_sounds_per_installation {
        let num_active_sounds = match active_sounds_per_installation.get(installation) {
            None => 0,
            Some(sounds) => sounds.len(),
        };
        let sounds_to_add = if num_target_sounds > num_active_sounds {
            num_target_sounds - num_active_sounds
        } else {
            // If there are no sounds to add, move on to the next installation.
            continue;
        };

        // The movement area associated with this installation.
        //
        // If there is no area, there is nowhere we can safely place sounds so we continue.
        let installation_area = match installation_areas.get(installation) {
            Some(area) => area,
            None => continue,
        };

        #[derive(Debug)]
        struct Suitability {
            // The number of sounds needed to reach the minimum number of sounds for the group.
            //
            // A positive value here should be a heavily weight the probability of using sources
            // from this group to add sounds.
            num_sounds_needed: usize,
            // The number of sounds that may be added from this group.
            //
            // This will always be either equal to or greater than the `num_sounds_needed`
            num_available_sounds: usize,
            // Suitability related to the timing of playback.
            //
            // This is `None` if the sound has never been played.
            timing: Option<Timing>,
            // Used to reset the  `duration` properties once a sound from this group is used.
            occurrence_rate_interval: Range<Ms>,
        }

        #[derive(Debug)]
        struct Timing {
            // The duration since the minimum occurrence rate interval.
            duration_since_min_interval: Ms,
            // The duration until time will have exceeded the max occurrence rate
            duration_until_sound_needed: Ms,
        }

        impl Suitability {
            // This is called when the group or source that owns these suitability parameters have
            // been used as a source for a new sound. Using this we can incrementally update the
            // list of
            //
            // Returns `true` if the source or group that owns these suitability parameters should
            // be removed from the list of available groups/sounds. This will always be the case if
            // the group or source has some non-zero occurrence rate interval.
            fn update_for_used_sound(&mut self) -> bool {
                self.num_sounds_needed = self.num_sounds_needed.saturating_sub(1);
                self.num_available_sounds = self.num_available_sounds.saturating_sub(1);
                let timing = Timing {
                    duration_since_min_interval: -self.occurrence_rate_interval.min,
                    duration_until_sound_needed: self.occurrence_rate_interval.max,
                };
                self.timing = Some(timing);
                if self.occurrence_rate_interval.min > Ms(0.0) {
                    true
                } else {
                    false
                }
            }
        }

        #[derive(Debug)]
        struct AvailableGroup {
            // The unique Id associated with this group.
            id: group::Id,
            // Parameters describing the group's availablility.
            suitability: Suitability,
        }

        // Collect available groups of sounds (based on occurrence rate and simultaneous sounds).
        let mut available_groups: Vec<AvailableGroup> = groups
            .iter()
            .filter_map(|(group_id, group)| {
                // The number of active sounds in this installation sourced from this group.
                let num_active_sounds = active_sounds_per_installation
                    .get(installation)
                    .map(|installation_active_sounds| {
                        installation_active_sounds
                            .iter()
                            .map(|id| &active_sounds[id])
                            .filter(|sound| {
                                let source_id = sound.source_id();
                                let source = match sources.get(&source_id) {
                                    None => return false,
                                    Some(s) => s,
                                };
                                source.groups.contains(group_id)
                            })
                            .count()
                    })
                    .unwrap_or(0);

                // If there are no available sounds, skip this group.
                let num_available_sounds = if group.simultaneous_sounds.max > num_active_sounds {
                    group.simultaneous_sounds.max - num_active_sounds
                } else {
                    return None;
                };

                let num_sounds_needed = if group.simultaneous_sounds.min > num_active_sounds {
                    group.simultaneous_sounds.min - num_active_sounds
                } else {
                    0
                };

                // Find the duration since the last time a sound was spawned using a source from
                // this group.
                let timing = if let Some(&last_used) = groups_last_used.get(group_id) {
                    let duration_since_last: time::Duration =
                        tick.instant.duration_since(last_used);
                    let duration_since_last_ms =
                        Ms(duration_to_secs(&duration_since_last) * 1_000.0);
                    let duration_since_min_interval =
                        if duration_since_last_ms > group.occurrence_rate.min {
                            duration_since_last_ms - group.occurrence_rate.min
                        } else {
                            return None;
                        };
                    let duration_until_sound_needed =
                        group.occurrence_rate.max - duration_since_last_ms;
                    Some(Timing {
                        duration_since_min_interval,
                        duration_until_sound_needed,
                    })
                } else {
                    None
                };

                let occurrence_rate_interval = group.occurrence_rate;
                let suitability = Suitability {
                    occurrence_rate_interval,
                    num_sounds_needed,
                    num_available_sounds,
                    timing,
                };

                Some(AvailableGroup {
                    id: *group_id,
                    suitability,
                })
            })
            .collect();

        // If there are no available groups, go to the next installation.
        if available_groups.is_empty() {
            continue;
        }

        // Order the two sets or properties by their suitability for use as the next sound.
        fn suitability(a: &Suitability, b: &Suitability) -> cmp::Ordering {
            match b.num_sounds_needed.cmp(&a.num_sounds_needed) {
                cmp::Ordering::Equal => match (&a.timing, &b.timing) {
                    (&None, &Some(_)) => cmp::Ordering::Less,
                    (&Some(_), &None) => cmp::Ordering::Greater,
                    (&None, &None) => cmp::Ordering::Equal,
                    (&Some(ref a), &Some(ref b)) => a.duration_until_sound_needed
                        .partial_cmp(&b.duration_until_sound_needed)
                        .expect("could not compare `duration_until_sound_needed`"),
                },
                ord => ord,
            }
        }

        // Sort the groups by:
        //
        // 1. The number of sounds needed
        // 2. The duration until a sound is needed to beat the occurrence rate.
        available_groups.sort_by(|a, b| suitability(&a.suitability, &b.suitability));

        // Find a source from the available groups for each sound that is to be added.
        //
        // Each time a sound is added the available group from which it was sourced should be
        // updated and the vec should be re-sorted.
        for _ in 0..sounds_to_add {
            {
                // The active sounds for the installation.
                let installation_active_sounds =
                    match active_sounds_per_installation.get(installation) {
                        None => &[],
                        Some(s) => &s[..],
                    };

                // Retrieve the front group if there is still one.
                let group_index: usize = match available_groups.is_empty() {
                    true => continue,
                    false => {
                        let num_equal = utils::count_equal(&available_groups, |a, b| {
                            suitability(&a.suitability, &b.suitability)
                        });
                        nannou::rand::thread_rng().gen_range(0, num_equal)
                    }
                };

                #[derive(Debug)]
                struct AvailableSource {
                    // The unique Id associated with this group.
                    id: audio::source::Id,
                    // Params that describe the suitability of the source for use with a sound.
                    suitability: Suitability,
                    // Ranges used to trigger playback.
                    playback_duration: Range<Ms>,
                    attack_duration: Range<Ms>,
                    release_duration: Range<Ms>,
                }

                // Find all available sources for the front group.
                let mut available_sources: Vec<AvailableSource> = sources
                    .iter()
                    .filter_map(|(source_id, source)| {
                        // We only want sources from the current group.
                        if !source.groups.contains(&available_groups[group_index].id) {
                            return None;
                        }

                        // How many instances of this sound are already playing.
                        let num_sounds = installation_active_sounds
                            .iter()
                            .map(|id| &active_sounds[id])
                            .filter(|s| s.source_id() == *source_id)
                            .count();

                        // If there are no available sounds, skip this group.
                        let num_available_sounds = if source.simultaneous_sounds.max > num_sounds {
                            source.simultaneous_sounds.max - num_sounds
                        } else {
                            return None;
                        };

                        // Determine the number of this sound that is required to reach the minimum.
                        let num_sounds_needed = if source.simultaneous_sounds.min > num_sounds {
                            source.simultaneous_sounds.min - num_sounds
                        } else {
                            0
                        };

                        // TODO: Find the duration since the last time a sound was spawned using a source
                        // from this group.
                        let timing = if let Some(&last_use) = sources_last_used.get(source_id) {
                            let duration_since_last = tick.instant.duration_since(last_use);
                            let duration_since_last_ms =
                                Ms(duration_to_secs(&duration_since_last) * 1_000.0);
                            let duration_since_min_interval =
                                if duration_since_last_ms > source.occurrence_rate.min {
                                    duration_since_last_ms - source.occurrence_rate.min
                                } else {
                                    return None;
                                };
                            let duration_until_sound_needed =
                                source.occurrence_rate.max - duration_since_last_ms;
                            Some(Timing {
                                duration_since_min_interval,
                                duration_until_sound_needed,
                            })
                        } else {
                            None
                        };

                        let occurrence_rate_interval = source.occurrence_rate;
                        let suitability = Suitability {
                            occurrence_rate_interval,
                            num_sounds_needed,
                            num_available_sounds,
                            timing,
                        };

                        Some(AvailableSource {
                            id: *source_id,
                            suitability,
                            playback_duration: source.playback_duration,
                            attack_duration: source.attack_duration,
                            release_duration: source.release_duration,
                        })
                    })
                    .collect();

                // If there are no available sources for this group, continue to the next.
                if available_sources.is_empty() {
                    continue;
                }

                // Sort the groups by:
                //
                // 1. The number of sounds needed
                // 2. The duration until a sound is needed to beat the occurrence rate.
                available_sources.sort_by(|a, b| suitability(&a.suitability, &b.suitability));

                // The index of the source.
                let source_index: usize = {
                    let num_equal = utils::count_equal(&available_sources, |a, b| {
                        suitability(&a.suitability, &b.suitability)
                    });
                    nannou::rand::thread_rng().gen_range(0, num_equal)
                };

                // Pick one of the most suitable sources.
                {
                    //let source = find_equally_suitable(available_sources.iter().map(|s| &s.suitability));
                    let source = &available_sources[source_index];

                    // TODO: Determine the initial position of the sound based on:
                    //
                    // 1. Installation for which we're triggereing a sound.
                    // 2. Movement properties and constraints of the source and group.
                    let initial_position = {
                        let mut rng = nannou::rand::thread_rng();
                        let left: bool = rng.gen();
                        let x_mag: f64 = rng.gen();
                        let x = match left {
                            true => {
                                Metres(x_mag)
                                    * (installation_area.centroid.x
                                        - installation_area.bounding_rect.left)
                                    + installation_area.centroid.x
                            }
                            false => {
                                Metres(x_mag)
                                    * (installation_area.centroid.x
                                        - installation_area.bounding_rect.right)
                                    + installation_area.centroid.x
                            }
                        };
                        let down: bool = rng.gen();
                        let y_mag: f64 = rng.gen();
                        let y = match down {
                            true => {
                                Metres(y_mag)
                                    * (installation_area.centroid.y
                                        - installation_area.bounding_rect.bottom)
                                    + installation_area.centroid.y
                            }
                            false => {
                                Metres(y_mag)
                                    * (installation_area.centroid.y
                                        - installation_area.bounding_rect.top)
                                    + installation_area.centroid.y
                            }
                        };
                        let point = Point2 { x, y };
                        let radians = rng.gen::<f32>() * 2.0 * ::std::f32::consts::PI;
                        audio::sound::Position { point, radians }
                    };

                    // Generate the attack and release durations based on their source ranges.
                    let mut rng = nannou::rand::thread_rng();
                    let attack_duration_frames =
                        audio::source::random_playback_duration(&mut rng, source.attack_duration)
                            .to_samples(audio::SAMPLE_RATE);
                    let release_duration_frames =
                        audio::source::random_playback_duration(&mut rng, source.release_duration)
                            .to_samples(audio::SAMPLE_RATE);
                    let duration_frames =
                        audio::source::random_playback_duration(&mut rng, source.playback_duration)
                            .to_samples(audio::SAMPLE_RATE);

                    // This is not a continuous preview (this is only used for GUI sounds).
                    let continuous_preview = false;

                    // Choose a movement type based on the source's assigned options.
                    let movement = generate_movement(
                        source.id,
                        &sources,
                        *installation,
                        installations,
                        installation_areas,
                        &target_sounds_per_installation,
                        &active_sounds,
                    );

                    // Spawn the sound from this source
                    let audio_source = sources[&source.id].to_audio_source();
                    let source_id = source.id;
                    let sound_id = sound_id_gen.generate_next();
                    let sound = audio::sound::spawn_from_source(
                        sound_id,
                        source_id,
                        &audio_source,
                        initial_position,
                        attack_duration_frames,
                        release_duration_frames,
                        continuous_preview,
                        Some(duration_frames),
                        audio_input_stream,
                        audio_output_stream,
                        realtime_source_latency,
                    );

                    // Track the time at which the group and source were last used.
                    groups_last_used.insert(available_groups[group_index].id, tick.instant);
                    sources_last_used.insert(source_id, tick.instant);

                    // Create the active sound for out use.
                    let active_sound = ActiveSound {
                        initial_installation: *installation,
                        handle: sound,
                        movement,
                    };

                    // Store the new active sound.
                    active_sounds.insert(sound_id, active_sound);
                }

                // Update the `AvailableGroup` and `AvailableSource` for this source.
                if available_groups[group_index]
                    .suitability
                    .update_for_used_sound()
                {
                    available_groups.remove(group_index);
                }
                if available_sources[source_index]
                    .suitability
                    .update_for_used_sound()
                {
                    available_sources.remove(source_index);
                }
            }

            // Re-sort the `available_groups` now that their suitability has been updated.
            available_groups.sort_by(|a, b| suitability(&a.suitability, &b.suitability));
        }
    }
}
