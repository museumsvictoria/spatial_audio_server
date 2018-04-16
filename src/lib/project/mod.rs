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
//!
//! The directory hieararchy looks as follows:
//!
//! assets/
//! |-audio/
//! |-images/
//! |-projects/
//! | |-projects.toml
//! | |-awesome-project/
//! | |-foo-project/
//! | |-sweeeet/
//! |   |-audio/
//! |   |-images/
//! |-project-backups/
//!

use audio;
use gui;
use installation::{self, Installation};
use master::{self, Master};
use nannou;
use osc;
use slug::slugify;
use soundscape;
use std::{cmp, fs, io};
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use utils;

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
const CONFIG_EXTENSION: &'static str = "toml";

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
    #[serde(default = "default_installations")]
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
pub type SoundscapeGroups = FxHashMap<soundscape::group::Id, Group>;

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

impl Project {
    /// Load the project from the given project directory path.
    ///
    /// If the project "config.toml" does not exist or is invalid, a default config will be used.
    ///
    /// **Panics** if the project "state.json" does not exist or is invalid. However, the method
    /// will attempt to fall back to reasonable default for each field that cannot be deserialized.
    pub fn load<A, P>(
        assets_path: A,
        project_directory_path: P,
        default_config: &Config,
        channels: &gui::Channels,
    ) -> Self
    where
        A: AsRef<Path>,
        P: AsRef<Path>,
    {
        // Load the configuration toml.
        let config_path = project_config_path(&project_directory_path);
        let config = utils::load_from_toml(&config_path)
            .unwrap_or_else(|_| default_config.clone());

        // Load the state json.
        let state_path = project_state_path(project_directory_path);
        let mut state = utils::load_from_json(&state)
            .expect("failed to load project state");

        /////////////////////////////////////////////////////////////
        // Update the camera's "floorplan_pixels_per_metre" field. //
        /////////////////////////////////////////////////////////////

        state.camera.floorplan_pixels_per_metre = config.floorplan_pixels_per_metre;

        ////////////////////////////////////////////////////
        // Load any sources that have not yet been loaded //
        ////////////////////////////////////////////////////

        let assets = assets_path.as_ref();
        let audio_path = assets.join(AUDIO_DIRECTORY_STEM);
        state.sources.remove_invalid_sources(&audio_path);
        state.sources.load_missing_sources(audio_path);
        state.sources.remove_invalid_soloed();

        ////////////////////////////////////////////////////////////////
        // Update the audio, OSC and soundscape threads as necessary. //
        ////////////////////////////////////////////////////////////////

        // TODO: Clear all project state from audio, osc and soundscape thread models.
        unimplemented!();

        // Master to audio input and output.
        let master_volume = state.master_volume;
        let dbap_rolloff_db = state.dbap_rolloff_db;
        let realtime_source_latency = state.realtime_source_latency;
        channels
            .audio_output
            .send(move |audio| {
                audio.master_volume = master_volume;
                audio.dbap_rolloff_db = dbap_rolloff_db;
            })
            .expect("failed to send loaded master volume and dbap rolloff");
        channels
            .soundscape
            .send(move |soundscape| {
                soundscape.realtime_source_latency = realtime_source_latency;
            })
            .expect("failed to send loaded realtime source latency");

        // Installations to soundscape and osc output.
        for (id, installation) in state.installations.iter() {
            let clone = installation.soundscape.clone();
            channels
                .soundscape
                .send(move |soundscape| {
                    soundscape.insert_installation(id, clone);
                })
                .expect("failed to send loaded installation soundscape state");
            for (computer, addr) in installation.computers.iter() {
                let osc_tx = nannou::osc::sender()
                    .expect("failed to create OSC sender")
                    .connect(&addr.socket)
                    .expect("failed to connect OSC sender");
                let osc_addr = addr.osc_addr.clone();
                let add = osc::output::OscTarget::Add(id, computer, osc_tx, osc_addr);
                let msg = osc::output::Message::Osc(add);
                channels
                    .osc_out_msg_tx
                    .send(msg)
                    .expect("failed to send loaded OSC target");
            }
        }

        // Soundscape groups to the soundscape thread.
        for (&id, group) in state.soundscape_groups.iter() {
            let clone = group.soundscape.clone();
            channels
                .soundscape
                .send(move |soundscape| {
                    soundscape.insert_group(id, clone);
                })
                .ok();
        }

        // Speakers to the soundscape and audio output threads.
        for (&id, speaker) in state.speakers.iter() {
            let clone = speaker.audio.clone();
            channels
                .audio_output
                .send(move |audio| {
                    audio.insert_speaker(id, clone);
                })
                .ok();
            let soundscape_speaker = soundscape::Speaker::from_audio_speaker(&speaker.audio);
            channels
                .soundscape
                .send(move |soundscape| {
                    soundscape.insert_speaker(id, soundscape_speaker);
                })
                .ok();
        }

        // Sources to the audio input and soundscape threads.
        for (&id, source) in state.sources.iter() {
            if let audio::source::Kind::Realtime(ref realtime) = source.kind {
                let clone = realtime.clone();
                channels
                    .audio_input
                    .send(move |audio| {
                        audio.sources.insert(id, clone);
                    })
                    .ok();
            }
            if let Some(clone) = soundscape::Source::from_audio_source(&source) {
                channels
                    .soundscape
                    .send(move |soundscape| {
                        soundscape.insert_source(id, clone);
                    })
                    .ok();
            }
        }

        Project { config, state }
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

        // Save the configuration toml file.
        let config_path = project_config_path(&project_directory);
        if let Err(err) = utils::save_to_toml(&config_path, &self.config) {
            eprintln!("failed to save project config.toml: {}", err);
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
        let Sources { ref map, ref mut soloed } = *self;
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

/// Check for invalid WAV sources
///
/// If there are any ".wav" files in `assets/audio` that have not yet been loaded into the
/// stored sources, load them as `Wav` kind sources.
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
            // If the path is valid, continue.
            if wav.path.exists() {
                continue;
            }

            // If the path doesn't exist, check to see if it contains the audio path stem.
            //
            // If so, check the path at the new location relative to the audio path.
            //
            // If we can find it, return the new absolute path.
            let new_path: Option<std::path::PathBuf> = {
                let mut components = wav.path.components();
                let audio_path_stem = audio_path.file_stem().and_then(|os_str| os_str.to_str());
                components
                    .find(|component| match *component {
                        Component::Normal(os_str) if os_str.to_str() == audio_path_stem => true,
                        _ => false,
                    })
                    .map(|_| {
                        audio_path.components()
                            .chain(components)
                            .map(|c| c.as_os_str())
                            .collect()
                    })
            };

            // Update the wavs path, or remove the source if we couldn't find it.
            if let Some(new_path) = new_path {
                if new_path.exists() {
                    wav.path = new_path;
                    continue;
                }
                eprintln!("Could not find WAV source at \"{}\" or at \"{}\". It will be ignored",
                          wav.path.display(),
                          new_path.display());
            } else {
                eprintln!("Could not find WAV source at \"{}\". It will be ignored.",
                          wav.path.display());
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
    if audio_path.exists() && audio_path.is_dir() {
        let wav_paths = WalkDir::new(&audio_path)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter_map(|e| {
                let file_name = e.file_name();
                let file_path = Path::new(&file_name);
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
                    audio::source::Kind::Wav(ref wav) => if wav.path == path {
                        continue 'paths;
                    },
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
            next_id = audio::source::Id(id.0 + 1);
        }
    }
}

/// Given the map of speakers, produce the next available unique `Id`.
pub fn next_speaker_id(speakers: &Speakers) -> audio::speaker::Id {
    let next_id = speakers
        .keys()
        .map(|id| id.0)
        .fold(0, cmp::max) + 1;
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
pub fn default_installations() -> Installations {
    installation::ALL
        .iter()
        .map(|&id| {
            let computers = (0..id.default_num_computers())
                .map(|i| {
                    let computer = installation::computer::Id(i);
                    let socket = "127.0.0.1:9002".parse().unwrap();
                    let osc_addr_base = id.default_osc_addr_str().to_string();
                    let osc_addr = format!("/{}/{}", osc_addr_base, i);
                    let addr = installation::computer::Address { socket, osc_addr };
                    (computer, addr)
                })
                .collect();
            let soundscape = Default::default();
            let installation = Installation { computers, soundscape };
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

/// The file path for the "config.toml" file holding all human-friendly config for this project.
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
        .filter(|p| p.join(STATE_FILE_STEM).with_extension(STATE_EXTENSION).exists())
        .collect();
    Ok(path)
}
