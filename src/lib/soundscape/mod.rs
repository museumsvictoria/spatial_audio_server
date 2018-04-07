use audio;
use installation::{self, Installation};
use metres::Metres;
use mindtree_utils::noise_walk;
use nannou;
use nannou::rand::{Rng, SeedableRng, XorShiftRng};
use nannou::prelude::*;
use std::cmp;
use std::collections::{HashMap, HashSet};
use std::ops;
use std::sync::{atomic, mpsc, Arc, Mutex};
use std::sync::atomic::AtomicBool;
use std::thread;
use std::time;
use time_calc::Ms;
use utils::{self, Range, Seed};

pub use self::group::Group;
use self::movement::BoundingRect;

pub mod group;
mod movement;

const TICK_RATE_MS: u64 = 16;

// The kinds of messages received by the soundscape thread.
pub enum Message {
    // Updates to the soundscape state from other threads.
    Update(UpdateFn),
    // Steps forward the soundscape.
    Tick(Tick),
    // Play all active sounds.
    Play,
    // Pause all active sounds.
    Pause,
    // Stop running the soundscape and exit.
    Exit,
}

#[derive(Copy, Clone, Debug)]
pub struct Tick {
    instant: time::Instant,
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
    /// The time at which the source was last used to create a sound.
    pub last_sound_created: Option<time::Instant>,
}

/// Represents a currently active sound spawned by the soundscape thread.
pub struct ActiveSound {
    /// The handle associated with this sound.
    handle: audio::sound::Handle,
    /// The installation for which this sound was triggered.
    initial_installation: Installation,

    // movement: fn(Tick) -> audio::sound::Position,
    // TODO: We can probably remove this as we can always get them from `movement` in a purely
    // functional manner?
    /// The current location and orientation of the sound.
    position: audio::sound::Position,
}

/// The model containing all state running on the soundscape thread.
pub struct Model {
    /// The latency applied to realtime sounds when spawned.
    pub realtime_source_latency: Ms,
    /// The soundscape's deterministic source of randomness.
    seed: Seed,
    /// All installations within the exhibition.
    installations: HashMap<Installation, installation::Soundscape>,
    /// Constraints for collections of sources.
    groups: HashMap<group::Id, Group>,
    /// All sources available to the soundscape for producing audio.
    sources: HashMap<audio::source::Id, Source>,
    /// All speakers within the exhibition.
    speakers: HashMap<audio::speaker::Id, Speaker>,
    /// The moment at which each `Group` was last used to spawn a sound.
    groups_last_used: HashMap<group::Id, time::Instant>,
    /// The moment at which each `Source` was last used to spawn a sound.
    sources_last_used: HashMap<audio::source::Id, time::Instant>,

    /// All sounds currently being played that were spawned by the soundscape thread.
    active_sounds: HashMap<audio::sound::Id, ActiveSound>,

    /// Tracks the speakers assignned to each installation. Updated at the beginning of each tick.
    installation_speakers: HashMap<Installation, Vec<audio::speaker::Id>>,
    /// This tracks the bounding area for each installation at the beginning of each tick.
    installation_areas: HashMap<Installation, movement::Area>,
    /// A handle for submitting new sounds to the input stream.
    audio_input_stream: audio::input::Stream,
    /// A handle for submitting new sounds to the output stream.
    audio_output_stream: audio::output::Stream,
    /// For generating unique IDs for each new sound.
    sound_id_gen: audio::sound::IdGenerator,
    // A handle to the ticker thread.
    tick_thread: thread::JoinHandle<()>,
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
        let last_sound_created = None;
        Some(Source {
            constraints,
            kind,
            spread,
            channel_radians,
            last_sound_created,
        })
    }

    /// Create an `audio::Source`, used for creating `Sound`s.
    pub fn to_audio_source(&self) -> audio::Source {
        let kind = self.kind.clone();
        let role = Some(audio::source::Role::Soundscape(self.constraints.clone()));
        let spread = self.spread;
        let channel_radians = self.channel_radians;
        audio::Source {
            kind,
            role,
            spread,
            channel_radians,
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
        self.tx.send(msg).map(|_| result).map_err(|_| mpsc::SendError(()))
    }

    /// Plays the soundscape.
    pub fn play(&self) -> Result<bool, mpsc::SendError<()>> {
        let result = self.is_playing() != true;
        let msg = Message::Play;
        self.is_playing.store(true, atomic::Ordering::Relaxed);
        self.tx.send(msg).map(|_| result).map_err(|_| mpsc::SendError(()))
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
    ) -> Option<installation::Soundscape>
    {
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
            },
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
            },
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
            },
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
            },
        }
    }

    /// Remove a source from the inner hashmap.
    pub fn remove_source(&mut self, id: &audio::source::Id) -> Option<Source> {
        self.active_sounds.retain(|_, s| *id != s.handle.source_id());
        self.sources.remove(id)
    }

    /// Remove an active sound from the hashmap.
    pub fn remove_active_sound(&mut self, id: &audio::sound::Id) -> Option<ActiveSound> {
        self.active_sounds.remove(id)
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
        .stack_size(512) // 512 bytes - a tiny stack for a tiny job.
        .spawn(move || {
            let mut last = time::Instant::now();
            let mut playback_duration = time::Duration::from_secs(0);
            loop {
                thread::sleep(time::Duration::from_millis(TICK_RATE_MS));
                let instant = time::Instant::now();
                let since_last_tick = instant.duration_since(last);
                if !tick_is_playing.load(atomic::Ordering::Relaxed) {
                    continue;
                }
                playback_duration += since_last_tick;
                let tick = Tick { instant, since_last_tick, playback_duration };
                if tick_tx.send(Message::Tick(tick)).is_err() {
                    break;
                }
                last = instant;
            }
        })
        .unwrap();

    // The model maintaining state between messages.
    let realtime_source_latency = audio::DEFAULT_REALTIME_SOURCE_LATENCY;
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
    Soundscape { tx, thread, is_playing }
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
            },

            // Pause all active sounds.
            Message::Pause => {
                for sound in model.active_sounds.values() {
                    sound.handle.pause();
                }
            }
        }
    }
}

// Called each time the soundscape thread receives a tick.
fn tick(model: &mut Model, tick: Tick) {
    let Model {
        realtime_source_latency,
        seed,
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

    println!("SOUNDSCAPE TICK");

    // Update the map from installations to speakers.
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

    // Create the map from installations to their areas.
    //
    // An installations `Area` is determined via the assigned speaker locations.
    installation_areas.clear();
    for (&installation, installation_speakers) in installation_speakers {
        let mut iter = installation_speakers.iter();
        let first_id = match iter.next() {
            None => continue,
            Some(first) => first,
        };
        let init = BoundingRect::from_point(speakers[first_id].point);
        let bounding_rect = iter.fold(init, |b, id| b.with_point(speakers[id].point));
        let centroid = {
            let points = installation_speakers.iter()
                .map(|id| speakers[id].point)
                .map(|p| Point2 { x: p.x.0, y: p.y.0 });
            nannou::geom::centroid(points)
                .map(|p| Point2 { x: Metres(p.x), y: Metres(p.y) })
                .unwrap()
        };
        let area = movement::Area { bounding_rect, centroid };
        installation_areas.insert(installation, area);
    }

    // TODO: Update the movement of each active sound.


    // For each installation, check the number of sounds that are playing.
    //
    // Sound/Installation associations are determined by finding the installation's centroid that
    // is closest to each sound (as long as that installation is one of those assigned to the
    // sound's source).
    let mut active_sounds_per_installation: HashMap<Installation, Vec<audio::sound::Id>> = HashMap::default();
    for s in active_sounds.values() {
        let source_id = s.handle.source_id();
        if let Some(source) = sources.get(&source_id) {
            let sound_point = Point2 { x: s.position.x.0, y: s.position.y.0 };
            let mut distances = source
                .constraints
                .installations
                .iter()
                .filter_map(|&i| installation_areas.get(&i).map(|a| (i, a)))
                .map(|(i, a)| {
                    let centroid = Point2 { x: a.centroid.x.0, y: a.centroid.y.0 };
                    (i, sound_point.distance2(centroid))
                });
            if let Some((i, dist)) = distances.next() {
                let (closest_installation, _) = distances.fold((i, dist), |(ia, min), (ib, dist)| {
                    if dist < min {
                        (ib, dist)
                    } else {
                        (ia, min)
                    }
                });
                active_sounds_per_installation
                    .entry(closest_installation)
                    .or_insert_with(Vec::new)
                    .push(s.id());
            }
        }
    }

    // A unique, constant seed associated with the installation.
    fn installation_seed(installation: &Installation) -> [u32; 4] {
        // Convert the installation to its integer representation.
        let u = installation.to_u32();
        let seed = [u; 4];
        seed
    }

    // Determine the target number of sounds per installation.
    //
    // We can determine this in a purely functional manner by using the playback duration as the
    // phase for a noise_walk signal.
    let mut target_sounds_per_installation: HashMap<Installation, usize> = HashMap::default();
    for (&installation, constraints) in installations {
        let target_num_sounds = {
            let playback_secs = duration_to_secs(&tick.playback_duration);
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
            let amp = noise_walk(phase);
            let normalised_amp = amp * 0.5 + 0.5;
            let range = &constraints.simultaneous_sounds;
            let range_diff = range.max - range.min;
            (range.min as f64 + normalised_amp * range_diff as f64) as usize
        };
        target_sounds_per_installation.insert(installation, target_num_sounds);
    }

    // Determine how many sounds to add (if any) by finding the difference between the target
    // number and actual number.
    for (installation, &num_target_sounds) in &target_sounds_per_installation {
        println!("\tInstallation: {:?}", installation);
        println!("\t\tnum_target_sounds: {}", num_target_sounds);
        let num_active_sounds = match active_sounds_per_installation.get(installation) {
            None => 0,
            Some(sounds) => sounds.len(),
        };
        println!("\t\tnum_active_sounds: {}", num_active_sounds);
        let sounds_to_add = if num_target_sounds > num_active_sounds {
            num_target_sounds - num_active_sounds
        } else {
            // If there are no sounds to add, move on to the next installation.
            continue;
        };
        println!("\t\tsounds_to_add: {}", sounds_to_add);

        // The movement area associated with this installation.
        //
        // If there is no area, there is nowhere we can safely place sounds so we continue.
        let installation_area = match installation_areas.get(installation) {
            Some(area) => area,
            None => continue,
        };

        println!("\t\tarea.rect: {:?}", installation_area.bounding_rect);
        println!("\t\tarea.centroid: {:?}", installation_area.centroid);

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
            .filter_map(|(group_id, group) | {
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
                    let duration_since_last: time::Duration = tick.instant.duration_since(last_used);
                    let duration_since_last_ms = Ms(duration_to_secs(&duration_since_last) * 1_000.0);
                    let duration_since_min_interval = if duration_since_last_ms > group.occurrence_rate.min {
                        duration_since_last_ms - group.occurrence_rate.min
                    } else {
                        return None;
                    };
                    let duration_until_sound_needed = group.occurrence_rate.max - duration_since_last_ms;
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

        println!("\t\tavailable groups: {:?}", available_groups.len());
        for g in &available_groups {
            println!("\t\t\t{:?}", g);
        }

        // If there are no available groups, go to the next installation.
        if available_groups.is_empty() {
            continue;
        }

        // Order the two sets or properties by their suitability for use as the next sound.
        fn suitability(a: &Suitability, b: &Suitability) -> cmp::Ordering {
            match b.num_sounds_needed.cmp(&a.num_sounds_needed) {
                cmp::Ordering::Equal => {
                    match (&a.timing, &b.timing) {
                        (&None, &Some(_)) => cmp::Ordering::Less,
                        (&Some(_), &None) => cmp::Ordering::Greater,
                        (&None, &None) => cmp::Ordering::Equal,
                        (&Some(ref a), &Some(ref b)) => {
                            a.duration_until_sound_needed
                                .partial_cmp(&b.duration_until_sound_needed)
                                .expect("could not compare `duration_until_sound_needed`")
                        },
                    }
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
            println!("\t\tAdding Sounds...");
            {
                // The active sounds for the installation.
                let installation_active_sounds = match active_sounds_per_installation.get(installation) {
                    None => &[],
                    Some(s) => &s[..],
                };

                // Retrieve the front group if there is still one.
                let group_index: usize = match available_groups.is_empty() {
                    true => continue,
                    false => {
                        let num_equal = utils::count_equal(
                            &available_groups,
                            |a, b| suitability(&a.suitability, &b.suitability),
                        );
                        nannou::rand::thread_rng().gen_range(0, num_equal)
                    },
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
                            let duration_since_last_ms = Ms(duration_to_secs(&duration_since_last) * 1_000.0);
                            let duration_since_min_interval = if duration_since_last_ms > source.occurrence_rate.min {
                                duration_since_last_ms - source.occurrence_rate.min
                            } else {
                                return None;
                            };
                            let duration_until_sound_needed = source.occurrence_rate.max - duration_since_last_ms;
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

                println!("\t\t\tavailable sources: {:?}", available_sources.len());
                for s in &available_sources {
                    println!("\t\t\t\t{:?}", s);
                }

                // Sort the groups by:
                //
                // 1. The number of sounds needed
                // 2. The duration until a sound is needed to beat the occurrence rate.
                available_sources.sort_by(|a, b| suitability(&a.suitability, &b.suitability));

                // The index of the source.
                let source_index: usize = {
                    let num_equal = utils::count_equal(
                        &available_sources,
                        |a, b| suitability(&a.suitability, &b.suitability),
                    );
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
                                    * (installation_area.centroid.x - installation_area.bounding_rect.left)
                                    + installation_area.centroid.x
                            }
                            false => {
                                Metres(x_mag)
                                    * (installation_area.centroid.x - installation_area.bounding_rect.right)
                                    + installation_area.centroid.x
                            }
                        };
                        let down: bool = rng.gen();
                        let y_mag: f64 = rng.gen();
                        let y = match down {
                            true => {
                                Metres(y_mag)
                                    * (installation_area.centroid.y - installation_area.bounding_rect.bottom)
                                    + installation_area.centroid.y
                            },
                            false => {
                                Metres(y_mag)
                                    * (installation_area.centroid.y - installation_area.bounding_rect.top)
                                    + installation_area.centroid.y
                            },
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

                    println!("\t\t\t\tid:       {:?}", source.id);
                    println!("\t\t\t\tposition: {:?}", initial_position);
                    println!("\t\t\t\tradians:  {:?}", initial_position);
                    println!("\t\t\t\tattack:   {:?}", attack_duration_frames);
                    println!("\t\t\t\trelease:  {:?}", release_duration_frames);
                    println!("\t\t\t\tduration: {:?}", duration_frames);

                    // This is not a continuous preview (this is only used for GUI sounds).
                    let continuous_preview = false;

                    // Spawn the sound from this source now.
                    let sound_id = sound_id_gen.generate_next();
                    let source_id = source.id;
                    let audio_source = sources[&source.id].to_audio_source();
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
                        handle: sound,
                        initial_installation: *installation,
                        position: initial_position,
                    };

                    // Store the new active sound.
                    active_sounds.insert(sound_id, active_sound);
                }

                // Update the `AvailableGroup` and `AvailableSource` for this source.
                if available_groups[group_index].suitability.update_for_used_sound() {
                    available_groups.remove(group_index);
                }
                if available_sources[source_index].suitability.update_for_used_sound() {
                    available_sources.remove(source_index);
                }
            }

            // Re-sort the `available_groups` now that their suitability has been updated.
            available_groups.sort_by(|a, b| suitability(&a.suitability, &b.suitability));
        }
    }
}

fn duration_to_secs(d: &time::Duration) -> f64 {
    d.as_secs() as f64 + d.subsec_nanos() as f64 * 1e-9
}
