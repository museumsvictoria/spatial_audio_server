//! A collapsible GUI side-bar area for loading, editing and saving project files.
//!
//! A single project describes all state associated with a particular configuration of the audio
//! server.
//!
//! Collections of parameters include:
//!
//! 1. Master parameters (volume, source latency, DBAP rolloff).
//! 2. Installation state, including soundscape constraints and OSC mappings.
//! 3. Soundscape groups and constraints.
//! 4. Seaker layout.
//! 5. Audio source params and soundscape constraints.

use crate::audio;
use crate::camera::Camera;
use crate::gui;
use crate::installation::{self, Installation};
use crate::master::Master;
use crate::osc;
use crate::soundscape;
use crate::utils;
use fxhash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};
use slug::slugify;
use std::ffi::OsStr;
use std::mem;
use std::ops::{Deref, DerefMut};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::{cmp, fs, io};
use walkdir::WalkDir;

pub mod config;

pub use self::config::Config;

/// The assets sub-directory in which all projects are stored.
const PROJECTS_DIRECTORY_STEM: &'static str = "projects";

/// The file stem of the state of a project.
const STATE_FILE_STEM: &'static str = "state";

/// The extension used for serializing and deserializing project state.
const STATE_EXTENSION: &'static str = "json";

/// The file stem of the config for a project.
const CONFIG_FILE_STEM: &'static str = "config";

/// The extension used for serializing and deserializing project config.
const CONFIG_EXTENSION: &'static str = "json";

/// The name of the directory where the WAVs are stored.
const AUDIO_DIRECTORY_STEM: &'static str = "audio";

/// All state related to a single project including configuration.
///
/// A single project describes a particular configuration of the audio server.
///
/// Collections of parameters include:
#[derive(Debug)]
pub struct Project {
    /// The config file associated with this project.
    pub config: Config,
    /// The state of the project.
    pub state: State,
}

/// All state related to a single project.
///
/// A single project describes a particular configuration of the audio server. Collections of
/// parameters include:
///
/// 1. Master parameters (volume, source latency, DBAP rolloff).
/// 2. Installation state, including soundscape constraints and OSC mappings.
/// 3. Soundscape groups and constraints.
/// 4. Seaker layout.
/// 5. Audio source params and soundscape constraints.
#[derive(Debug, Serialize, Deserialize)]
pub struct State {
    /// The human-readable name of the project.
    ///
    /// This is slugified to create the filename of the project.
    #[serde(default = "default_project_name")]
    pub name: String,
    /// Master state of the project. E.g. volume, DBAP rolloff, latency, etc.
    #[serde(default)]
    pub master: Master,
    /// All installations in the exhibition along with their soundscape constraints.
    #[serde(default = "default_beyond_perception_installations")]
    pub installations: Installations,
    /// All soundscape groups within the exhibition.
    #[serde(default)]
    pub soundscape_groups: SoundscapeGroups,
    /// All speakers within the exhibition.
    #[serde(default)]
    pub speakers: Speakers,
    /// All sources within the exhibition along with the set of currently soloed sources.
    #[serde(default)]
    pub sources: Sources,
    /// The state of the camera over the floorplan.
    #[serde(default)]
    pub camera: Camera,
}

/// A map of all installations within the exhibition to their soundscape constraints.
pub type Installations = FxHashMap<installation::Id, Installation>;

/// A map of all soundscape groups within the exhibition for the project.
pub type SoundscapeGroups = FxHashMap<soundscape::group::Id, SoundscapeGroup>;

/// A map of all speakers within the exhibition for the project.
pub type Speakers = FxHashMap<audio::speaker::Id, Speaker>;

/// A map of all sources within the exhibition for the project.
pub type SourcesMap = FxHashMap<audio::source::Id, Source>;

/// A set of soloed sources.
pub type SoloedSources = FxHashSet<audio::source::Id>;

/// All sources within the exhibition for the project along with the set of soloed sources.
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct Sources {
    /// A map of all sources within the exhibition for the project.
    #[serde(default)]
    pub map: SourcesMap,
    /// The currently soloed sources.
    #[serde(default)]
    pub soloed: SoloedSources,
}

/// State of a single soundscape group within the exhibition associated with a single project.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SoundscapeGroup {
    /// A humand-friendly name for the speaker.
    pub name: String,
    /// Soundscape-specific parameters.
    pub soundscape: soundscape::Group,
}

/// State of a single speaker within the exhibition associated with a single project.
#[derive(Debug, Deserialize, Serialize)]
pub struct Speaker {
    /// A humand-friendly name for the speaker.
    pub name: String,
    /// Audio-related parameters.
    pub audio: audio::Speaker,
}

/// State of a single speaker within the exhibition associated with a single project.
#[derive(Debug, Deserialize, Serialize)]
pub struct Source {
    /// A humand-friendly name for the speaker.
    pub name: String,
    /// Audio-related parameters.
    pub audio: audio::Source,
}

impl State {
    fn default_from_name(name: String) -> Self {
        let master = Default::default();
        let installations = default_beyond_perception_installations();
        let soundscape_groups = Default::default();
        let speakers = Default::default();
        let sources = Default::default();
        let camera = Default::default();
        State {
            name,
            master,
            installations,
            soundscape_groups,
            speakers,
            sources,
            camera,
        }
    }

    /// If all of the installations within `State` are unnamed, name them automatically.
    ///
    /// Returns `true` if installations were renamed, false if not.
    fn auto_name_installations_if_all_unnamed(&mut self) -> bool {
        if self
            .installations
            .values()
            .all(|inst| &inst.name == installation::default::name())
        {
            for (id, installation) in self.installations.iter_mut() {
                let name = match id.0 {
                    0 => "Waves At Work",
                    1 => "Ripples In Spacetime",
                    2 => "Energetic Vibrations - Audio Visualiser",
                    3 => "Energetic Vibrations - Projection Mapping",
                    4 => "Turbulent Encounters",
                    5 => "Cacophony",
                    6 => "Wrapped In Spectrum",
                    7 => "Turret 1",
                    8 => "Turret 2",
                    _ => continue,
                };
                installation.name = name.into();
            }
            true
        } else {
            false
        }
    }
}

impl Project {
    /// Construct the project from its `State` and `Config` parts.
    ///
    /// This is used internally within the `load` and `default` constructors.
    fn from_config_and_state<P>(assets: P, config: Config, mut state: State) -> Self
    where
        P: AsRef<Path>,
    {
        /////////////////////////////////////////////////////////////
        // Update the camera's "floorplan_pixels_per_metre" field. //
        /////////////////////////////////////////////////////////////

        state.camera.floorplan_pixels_per_metre = config.floorplan_pixels_per_metre;

        ////////////////////////////////////////////////////
        // Load any sources that have not yet been loaded //
        ////////////////////////////////////////////////////

        let assets = assets.as_ref();
        let audio_path = assets.join(AUDIO_DIRECTORY_STEM);
        state.auto_name_installations_if_all_unnamed();
        state.sources.remove_invalid_sources(&audio_path);
        state.sources.load_missing_sources(audio_path);
        state.sources.remove_invalid_soloed();

        Project { config, state }
    }

    /// This method clears the state on all threads and re-populates them with the state of the
    /// `Project`. Specifically, this updates the audio input, audio output, osc output and
    /// soundscape threads as necessary.
    ///
    /// This is particularly useful when creating or loading a new project to use as the main
    /// project.
    pub fn reset_and_sync_all_threads(&self, channels: &gui::Channels) {
        // Clear all project state from audio, osc and soundscape thread models.
        channels
            .soundscape
            .send(move |soundscape| soundscape.clear_project_specific_data())
            .expect("failed to send `clear_project_specific_data` message to soundscape thread");
        channels
            .audio_input
            .send(move |audio| audio.clear_project_specific_data())
            .expect("failed to send `clear_project_specific_data` message to audio input thread");
        channels
            .audio_output
            .send(move |audio| audio.clear_project_specific_data())
            .expect("failed to send `clear_project_specific_data` message to audio output thread");
        channels
            .osc_out_msg_tx
            .push(osc::output::Message::ClearProjectSpecificData);

        // TODO: Consider updating config stuff here?

        // Master to audio input and output.
        let master_volume = self.master.volume;
        let dbap_rolloff_db = self.master.dbap_rolloff_db;
        let realtime_source_latency = self.master.realtime_source_latency;
        let proximity_limit_2 = self.master.proximity_limit_2;
        channels
            .audio_output
            .send(move |audio| {
                audio.master_volume = master_volume;
                audio.dbap_rolloff_db = dbap_rolloff_db;
                // Square for efficiency
                audio.proximity_limit_2 = proximity_limit_2;
            })
            .expect("failed to send loaded master volume and dbap rolloff");
        channels
            .soundscape
            .send(move |soundscape| {
                soundscape.realtime_source_latency = realtime_source_latency;
            })
            .expect("failed to send loaded realtime source latency");

        // Installations to soundscape, osc output and audio output.
        for (&id, installation) in self.installations.iter() {
            // Soundscape.
            let clone = installation.soundscape.clone();
            channels
                .soundscape
                .send(move |soundscape| {
                    soundscape.insert_installation(id, clone);
                })
                .expect("failed to send loaded installation soundscape state");

            // OSC output thread.
            for (&computer, addr) in installation.computers.iter() {
                let osc_tx = nannou_osc::sender()
                    .expect("failed to create OSC sender")
                    .connect(&addr.socket)
                    .expect("failed to connect OSC sender");
                let osc_addr = addr.osc_addr.clone();
                let target = osc::output::TargetSource::New(Arc::new(osc_tx));
                let add = osc::output::OscTarget::Add(id, computer, target, osc_addr);
                let msg = osc::output::Message::Osc(add);
                channels.osc_out_msg_tx.push(msg);
            }

            // Audio output thread.
            let computers = installation.computers.len();
            channels
                .audio_output
                .send(move |audio| {
                    audio.insert_installation(id, computers);
                })
                .expect("failed to send loaded installation to audio output thread");
        }

        // Soundscape groups to the soundscape thread.
        for (&id, group) in self.soundscape_groups.iter() {
            let clone = group.soundscape.clone();
            channels
                .soundscape
                .send(move |soundscape| {
                    soundscape.insert_group(id, clone);
                })
                .expect("failed to send soundscape group to soundscape thread");
        }

        // Speakers to the soundscape and audio output threads.
        for (&id, speaker) in self.speakers.iter() {
            let clone = speaker.audio.clone();
            channels
                .audio_output
                .send(move |audio| {
                    audio.insert_speaker(id, clone);
                })
                .expect("failed to send speaker to audio output thread");
            let soundscape_speaker = soundscape::Speaker::from_audio_speaker(&speaker.audio);
            channels
                .soundscape
                .send(move |soundscape| {
                    soundscape.insert_speaker(id, soundscape_speaker);
                })
                .expect("failed to send speaker to soundscape thread");
        }

        // Sources to the audio input and soundscape threads.
        for (&id, source) in self.sources.iter() {
            if let audio::source::Kind::Realtime(ref realtime) = source.kind {
                let clone = realtime.clone();
                channels
                    .audio_input
                    .send(move |audio| {
                        audio.sources.insert(id, clone);
                    })
                    .expect("failed to send source to audio input thread");
            }
            if let Some(clone) = soundscape::Source::from_audio_source(&source) {
                channels
                    .soundscape
                    .send(move |soundscape| {
                        soundscape.insert_source(id, clone);
                    })
                    .expect("failed to send source to soundscape thread");
            }
        }
    }

    /// Create a new project with a unique, default name.
    pub fn new<P>(assets: P, default_config: &Config) -> Self
    where
        P: AsRef<Path>,
    {
        // Get the projects directory.
        let projects_directory = projects_directory(&assets);

        // Create a unique default project name.
        let mut name;
        let mut i = 1;
        loop {
            name = format!("{} {}", default_project_name(), i);
            let slug = slugify(&name);
            let project_directory = projects_directory.join(slug);
            if !project_directory.exists() {
                break;
            } else {
                i += 1;
            }
        }

        // Create the default state.
        let config = default_config.clone();
        let state = State::default_from_name(name);

        Self::from_config_and_state(assets, config, state)
    }

    /// The same as `load`, but loads the project from the given slug rather than the full path.
    ///
    /// Returns `None` if there was no project for the given slug.
    pub fn load_from_slug<P>(assets_path: P, slug: &str, default_config: &Config) -> Option<Self>
    where
        P: AsRef<Path>,
    {
        let projects_directory = projects_directory(&assets_path);
        let project_directory = projects_directory.join(&slug);
        if project_directory.exists() && project_directory.is_dir() {
            Some(Self::load(assets_path, &project_directory, default_config))
        } else {
            None
        }
    }

    /// Load the project from the given project directory path.
    ///
    /// If the project "config.json" does not exist or is invalid, a default config will be used.
    ///
    /// **Panics** if the project "state.json" does not exist or is invalid. However, the method
    /// will attempt to fall back to reasonable default for each field that cannot be deserialized.
    pub fn load<A, P>(assets_path: A, project_directory_path: P, default_config: &Config) -> Self
    where
        A: AsRef<Path>,
        P: AsRef<Path>,
    {
        // Load the configuration json.
        let config_path = project_config_path(&project_directory_path);
        let config: Config =
            utils::load_from_json(&config_path).unwrap_or_else(|_| default_config.clone());

        // Load the state json.
        let state_path = project_state_path(project_directory_path);
        let state: State =
            utils::load_from_json(&state_path).expect("failed to load project state");

        Self::from_config_and_state(assets_path, config, state)
    }

    /// Save the project in its current state.
    ///
    /// `assets` is the path to the projects "assets" directory.
    pub fn save<P>(&self, assets: P) -> io::Result<()>
    where
        P: AsRef<Path>,
    {
        // Create the project directory and all necessary parent directories if missing.
        let project_directory = project_directory_path(&assets, &self.name);
        if !project_directory.exists() || !project_directory.is_dir() {
            fs::create_dir_all(&project_directory)?;
        }

        // Save the configuration json file.
        let config_path = project_config_path(&project_directory);
        if let Err(err) = utils::save_to_json(&config_path, &self.config) {
            eprintln!("failed to save project config.json: {}", err);
        }

        // Save the state json file.
        let state_path = project_state_path(project_directory);
        if let Err(err) = utils::save_to_json(&state_path, &self.state) {
            eprintln!("failed to save project state.json: {}", err);
        }

        Ok(())
    }
}

impl Sources {
    /// Find the next available source ID for the `Sources`.
    pub fn next_id(&self) -> audio::source::Id {
        let next_id = self.map.iter().map(|(&id, _)| id.0).fold(0, cmp::max) + 1;
        audio::source::Id(next_id)
    }

    /// Check for invalid WAV sources
    ///
    /// If there are any ".wav" files in `assets/audio` that have not yet been loaded into the
    /// stored sources, load them as `Wav` kind sources.
    pub fn remove_invalid_sources<P>(&mut self, audio_path: P)
    where
        P: AsRef<Path>,
    {
        remove_invalid_sources(audio_path, self);
    }

    /// Remove all sources from the "soloed" set that no longer exist.
    ///
    /// This is necessary for startup where the soloed file may contain sources that are no longer
    /// valid.
    pub fn remove_invalid_soloed(&mut self) {
        let Sources {
            ref map,
            ref mut soloed,
        } = *self;
        soloed.retain(|id| map.contains_key(id));
    }

    /// Load missing WAV sources.
    ///
    /// If there are any ".wav" files in `assets/audio` that have not yet been loaded into sources,
    /// load them as `Wav` kind sources.
    pub fn load_missing_sources<P>(&mut self, audio_path: P)
    where
        P: AsRef<Path>,
    {
        load_missing_sources(audio_path, self);
    }
}

/// Updates the path from the given new relative path.
///
/// E.g.
///
/// If `path` is "/foo/bar/baz/qux" and `relative` is "/flim/baz" the resulting path will be
/// "/flim/baz/qux".
fn update_path_from_relative<P, R>(path: P, relative: R) -> Option<PathBuf>
where
    P: AsRef<Path>,
    R: AsRef<Path>,
{
    let path = path.as_ref();
    let relative = relative.as_ref();

    let mut components = path.components();
    let relative_stem = relative.file_stem().and_then(|os_str| os_str.to_str());
    components
        .find(|component| match *component {
            Component::Normal(os_str) if os_str.to_str() == relative_stem => true,
            _ => false,
        })
        .map(|_| {
            relative
                .components()
                .chain(components)
                .map(|c| c.as_os_str())
                .collect()
        })
}

#[test]
fn test_update_path_from_relative() {
    let path = Path::new("/foo/bar/baz/qux");
    let relative = Path::new("/flim/baz");
    let expected = Path::new("/flim/baz/qux");
    assert_eq!(
        update_path_from_relative(path, relative),
        Some(PathBuf::from(expected))
    );
}

/// Check for invalid WAV sources.
///
/// If the source path's could not be correctly updated, we attempt to re-attach the path from the
/// `audio` component of the path and onwards.
pub fn remove_invalid_sources<P>(audio_path: P, sources: &mut Sources)
where
    P: AsRef<Path>,
{
    let audio_path = audio_path.as_ref();

    // Check the validity of the WAV source paths.
    //
    // If a path is invalid, check to see if it exists within the given `audio_path`. If so,
    // update the source path. Otherwise, remove it.
    let mut to_remove = vec![];
    for (&id, source) in sources.map.iter_mut() {
        if let audio::source::Kind::Wav(ref mut wav) = source.audio.kind {
            // Check to see that the WAV path contains the `audio` directory in its path.
            //
            // If so, check the path at the new location relative to the audio path.
            //
            // If we can find it, return the new absolute path.
            let new_path = update_path_from_relative(&wav.path, audio_path);

            // Update the wavs path, or remove the source if we couldn't find it.
            if let Some(new_path) = new_path {
                if new_path.exists() {
                    // Reload the WAV file to make sure we have up-to-date info.
                    let mut new_wav = match audio::source::Wav::from_path(new_path.clone()) {
                        Ok(wav) => wav,
                        Err(err) => {
                            eprintln!(
                                "Failed to load wav from path \"{}\": {}. It will be ignored.",
                                new_path.display(),
                                err
                            );
                            continue;
                        }
                    };
                    new_wav.should_loop = wav.should_loop;
                    new_wav.playback = wav.playback;
                    mem::swap(wav, &mut new_wav);
                    continue;
                }
                eprintln!(
                    "Could not find WAV source at \"{}\" or at \"{}\". It will be ignored.",
                    wav.path.display(),
                    new_path.display()
                );
            } else {
                eprintln!(
                    "Could not find WAV source at \"{}\". It will be ignored.",
                    wav.path.display()
                );
            }

            to_remove.push(id);
        }
    }
    for id in to_remove {
        sources.map.remove(&id);
    }
}

/// Load missing WAV sources.
///
/// If there are any ".wav" files in `assets/audio` that have not yet been loaded into sources,
/// load them as `Wav` kind sources.
pub fn load_missing_sources<P>(audio_path: P, sources: &mut Sources)
where
    P: AsRef<Path>,
{
    let audio_path = audio_path.as_ref();

    // If there are any WAVs in `assets/audio/` that we have not yet listed, load them.
    //
    // Ignores all hidden files.
    if audio_path.exists() && audio_path.is_dir() {
        let wav_paths = WalkDir::new(&audio_path)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter_map(|e| {
                let file_name = e.file_name();
                let file_path = Path::new(&file_name);
                if utils::is_file_hidden(&file_path) {
                    return None;
                }
                let ext = file_path
                    .extension()
                    .and_then(OsStr::to_str)
                    .map(str::to_ascii_lowercase);
                match ext.as_ref().map(|e| &e[..]) {
                    Some("wav") | Some("wave") => Some(e.path().to_path_buf()),
                    _ => None,
                }
            });

        // Find the next available ID in case we find new sources.
        let mut next_id = sources.next_id();

        // For each new wav file, create a new source.
        'paths: for path in wav_paths {
            // If we already have this one, continue.
            for s in sources.map.values() {
                match s.audio.kind {
                    audio::source::Kind::Wav(ref wav) => {
                        if wav.path == path {
                            if wav.path == path {
                                continue 'paths;
                            }
                        }
                    }
                    _ => (),
                }
            }
            // Set the name as the file name without the extension.
            let name = match path.file_stem().and_then(OsStr::to_str) {
                Some(name) => name.to_string(),
                None => continue,
            };
            // Load the `Wav`.
            let wav = match audio::source::Wav::from_path(path) {
                Ok(w) => w,
                Err(e) => {
                    eprintln!("Failed to load wav file {:?}: {}", name, e);
                    continue;
                }
            };
            let kind = audio::source::Kind::Wav(wav);
            let role = None;
            let spread = audio::source::default::SPREAD;
            let channel_radians = audio::source::default::CHANNEL_RADIANS;
            let volume = audio::source::default::VOLUME;
            let muted = bool::default();
            let audio = audio::Source {
                kind,
                role,
                spread,
                channel_radians,
                volume,
                muted,
            };
            let source = Source { name, audio };
            sources.map.insert(next_id, source);
            next_id = audio::source::Id(next_id.0 + 1);
        }
    }
}

/// Search for and return the next available installation ID.
pub fn next_installation_id(installations: &Installations) -> installation::Id {
    let next_id = installations.keys().map(|id| id.0).fold(0, cmp::max) + 1;
    installation::Id(next_id)
}

/// Search for and return the next available group ID.
pub fn next_soundscape_group_id(groups: &SoundscapeGroups) -> soundscape::group::Id {
    let next_id = groups.keys().map(|id| id.0).fold(0, cmp::max) + 1;
    soundscape::group::Id(next_id)
}

/// Given the map of speakers, produce the next available unique `Id`.
pub fn next_speaker_id(speakers: &Speakers) -> audio::speaker::Id {
    let next_id = speakers.keys().map(|id| id.0).fold(0, cmp::max) + 1;
    audio::speaker::Id(next_id)
}

/// Given the map of speakers, produce the next available speaker channel index.
///
/// Note: This is a super naiive way of searching however there should never be enough speakers to
/// make it a problem.
pub fn next_available_speaker_channel(speakers: &Speakers) -> usize {
    let mut channel = 0;
    'search: loop {
        for speaker in speakers.values() {
            if channel == speaker.channel {
                channel += 1;
                continue 'search;
            }
        }
        return channel;
    }
}

impl Deref for Project {
    type Target = State;
    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl DerefMut for Project {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state
    }
}

impl Deref for SoundscapeGroup {
    type Target = soundscape::Group;
    fn deref(&self) -> &Self::Target {
        &self.soundscape
    }
}

impl DerefMut for SoundscapeGroup {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.soundscape
    }
}

impl Deref for Speaker {
    type Target = audio::Speaker;
    fn deref(&self) -> &Self::Target {
        &self.audio
    }
}

impl DerefMut for Speaker {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.audio
    }
}

impl Deref for Sources {
    type Target = SourcesMap;
    fn deref(&self) -> &Self::Target {
        &self.map
    }
}

impl DerefMut for Sources {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.map
    }
}

impl Deref for Source {
    type Target = audio::Source;
    fn deref(&self) -> &Self::Target {
        &self.audio
    }
}

impl DerefMut for Source {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.audio
    }
}

/// Create the default map of installations for a project.
pub fn default_beyond_perception_installations() -> Installations {
    installation::BEYOND_PERCEPTION_NAMES
        .iter()
        .enumerate()
        .map(|(i, &name)| {
            let id = installation::Id(i);
            let n_computers =
                installation::beyond_perception_default_num_computers(name).unwrap_or(0);
            let osc_addr = installation::osc_addr_string(name);
            let computers = (0..n_computers)
                .map(|i| {
                    let computer = installation::computer::Id(i);
                    let socket = "127.0.0.1:9002".parse().unwrap();
                    let osc_addr = osc_addr.clone();
                    let addr = installation::computer::Address { socket, osc_addr };
                    (computer, addr)
                })
                .collect();
            let soundscape = Default::default();
            let name = name.into();
            let installation = Installation {
                name,
                computers,
                soundscape,
            };
            (id, installation)
        })
        .collect()
}

/// The default name for a project.
pub fn default_project_name() -> String {
    "My Project".into()
}

/// The path of the "assetes/projects/" directory.
pub fn projects_directory<P>(assets: P) -> PathBuf
where
    P: AsRef<Path>,
{
    assets.as_ref().join(PROJECTS_DIRECTORY_STEM)
}

/// The directory path for a project with the given name.
pub fn project_directory_path<P>(assets: P, name: &str) -> PathBuf
where
    P: AsRef<Path>,
{
    let projects_directory = projects_directory(assets);
    let directory_stem = slugify(name);
    projects_directory.join(directory_stem)
}

/// The file path for the "config.json" file holding all human-friendly config for this project.
pub fn project_config_path<P>(project_directory: P) -> PathBuf
where
    P: AsRef<Path>,
{
    project_directory
        .as_ref()
        .join(CONFIG_FILE_STEM)
        .with_extension(CONFIG_EXTENSION)
}

/// The file path for the "state.json" file holding all state for this project.
pub fn project_state_path<P>(project_directory: P) -> PathBuf
where
    P: AsRef<Path>,
{
    project_directory
        .as_ref()
        .join(STATE_FILE_STEM)
        .with_extension(STATE_EXTENSION)
}

/// Loads the path of every project directory within the `projects/` directory.
pub fn load_project_directories<P>(assets: P) -> io::Result<Vec<PathBuf>>
where
    P: AsRef<Path>,
{
    let projects_directory = projects_directory(assets);
    let paths = fs::read_dir(projects_directory)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter(|p| {
            p.join(STATE_FILE_STEM)
                .with_extension(STATE_EXTENSION)
                .exists()
        })
        .collect();
    Ok(paths)
}
