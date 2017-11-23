use audio;
use composer;
use config::Config;
use interaction::Interaction;
use metres::Metres;
use nannou;
use nannou::prelude::*;
use nannou::glium;
use nannou::osc;
use nannou::ui;
use nannou::ui::prelude::*;
use serde_json;
use std;
use std::collections::{HashMap, VecDeque};
use std::ffi::OsStr;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::net::SocketAddr;
use std::ops::{Deref, DerefMut};
use std::sync::mpsc;

mod custom_widget;
mod theme;

type ActiveSoundMap = HashMap<audio::sound::Id, ActiveSound>;

pub struct Model {
    pub ui: Ui,
    images: Images,
    fonts: Fonts,
    state: State,
    ids: Ids,
    channels: Channels,
    sound_id_gen: audio::sound::IdGenerator,
    audio_monitor: AudioMonitor,
    assets: PathBuf,
}

/// A convenience wrapper that borrows the GUI state necessary for instantiating the widgets.
struct Gui<'a> {
    ui: UiCell<'a>,
    /// The images used throughout the GUI.
    images: &'a Images,
    fonts: &'a Fonts,
    ids: &'a mut Ids,
    state: &'a mut State,
    audio_monitor: &'a AudioMonitor,
    channels: &'a Channels,
    sound_id_gen: &'a audio::sound::IdGenerator,
}

struct State {
    // The loaded config file.
    config: Config,
    // The camera over the 2D floorplan.
    camera: Camera,
    // A log of the most recently received OSC messages for testing/debugging/monitoring.
    osc_log: OscLog,
    // A log of the most recently received Interactions for testing/debugging/monitoring.
    interaction_log: InteractionLog,
    speaker_editor: SpeakerEditor,
    source_editor: SourceEditor,
    // Menu states.
    side_menu_is_open: bool,
    osc_log_is_open: bool,
    interaction_log_is_open: bool,
}

fn speakers_path(assets: &Path) -> PathBuf {
    assets.join(Path::new(SPEAKERS_FILE_STEM)).with_extension("json")
}

fn sources_path(assets: &Path) -> PathBuf {
    assets.join(Path::new(SOURCES_FILE_STEM)).with_extension("json")
}

impl Model {
    /// Initialise the GUI model.
    pub fn new(
        assets: &Path,
        config: Config,
        app: &App,
        window_id: WindowId,
        channels: Channels,
        sound_id_gen: audio::sound::IdGenerator,
    ) -> Self {
        let mut ui = app.new_ui(window_id).with_theme(theme::construct()).build().unwrap();

        // The type containing the unique ID for each widget in the GUI.
        let ids = Ids::new(ui.widget_id_generator());

        // Load and insert the fonts to be used.
        let font_path = fonts_directory(assets).join("NotoSans/NotoSans-Regular.ttf");
        let notosans_regular = ui.fonts_mut().insert_from_file(font_path).unwrap();
        let fonts = Fonts { notosans_regular };

        // Load and insert the images to be used.
        let floorplan_path = images_directory(assets).join("floorplan.png");
        let floorplan = insert_image(&floorplan_path,
                                     app.window(window_id).unwrap().inner_glium_display(),
                                     &mut ui.image_map);
        let images = Images { floorplan };

        // Initialise the GUI state.
        let state = State::new(assets, config, &channels);

        // Initialise the audio monitor.
        let active_sounds = HashMap::new();
        let speakers = HashMap::new();
        let audio_monitor = AudioMonitor { active_sounds, speakers };

        Model {
            ui,
            images,
            fonts,
            state,
            ids,
            channels,
            sound_id_gen,
            assets: assets.into(),
            audio_monitor,
        }
    }

    /// Update the GUI model.
    ///
    /// - Collect pending OSC and interaction messages for the logs.
    /// - Instantiate the Ui's widgets.
    pub fn update(&mut self) {
        let Model {
            ref mut ui,
            ref mut ids,
            ref mut state,
            ref mut audio_monitor,
            ref images,
            ref fonts,
            ref channels,
            ref sound_id_gen,
            ..
        } = *self;

        // Collect OSC messages for the OSC log.
        for (addr, msg) in channels.osc_msg_rx.try_iter() {
            state.osc_log.push_msg((addr, msg));
        }

        // Collect interactions for the interaction log.
        for interaction in channels.interaction_rx.try_iter() {
            state.interaction_log.push_msg(interaction);
        }

        // Update the map of active sounds.
        for msg in channels.audio_monitor_msg_rx.try_iter() {
            match msg {
                AudioMonitorMessage::ActiveSound(id, msg) => match msg {
                    ActiveSoundMessage::Start { source_id, position, channels } => {
                        let channels = (0..channels)
                            .map(|_| ChannelLevels { rms: 0.0, peak: 0.0 })
                            .collect();
                        let mut active_sound = ActiveSound { source_id, position, channels };
                        audio_monitor.active_sounds.insert(id, active_sound);
                    },
                    ActiveSoundMessage::Update { position } => {
                        let active_sound = audio_monitor.active_sounds.get_mut(&id).unwrap();
                        active_sound.position = position;
                    },
                    ActiveSoundMessage::UpdateChannel { index, rms, peak } => {
                        let active_sound = audio_monitor.active_sounds.get_mut(&id).unwrap();
                        let mut channel = &mut active_sound.channels[index];
                        channel.rms = rms;
                        channel.peak = peak;
                    },
                    ActiveSoundMessage::End => {
                        audio_monitor.active_sounds.remove(&id);

                        // If the Id of the sound being removed matches the current preview, remove
                        // it.
                        match state.source_editor.preview.current {
                            Some((SourcePreviewMode::OneShot, s_id)) if id == s_id => {
                                state.source_editor.preview.current = None;
                            },
                            _ => (),
                        }
                    },
                },
                AudioMonitorMessage::Speaker(id, msg) => match msg {
                    SpeakerMessage::Add => {
                        let speaker = ChannelLevels { rms: 0.0, peak: 0.0 };
                        audio_monitor.speakers.insert(id, speaker);
                    },
                    SpeakerMessage::Update { rms, peak } => {
                        let speaker = ChannelLevels { rms, peak };
                        audio_monitor.speakers.insert(id, speaker);
                    },
                    SpeakerMessage::Remove => {
                        audio_monitor.speakers.remove(&id);
                    },
                },
            }
        }

        let ui = ui.set_widgets();
        let mut gui = Gui { ui, ids, images, fonts, state, channels, sound_id_gen, audio_monitor };
        set_widgets(&mut gui);
    }

    /// Save the speaker configuration and audio sources on exit.
    pub fn exit(self) {
        // Saves the file to a temporary file before removing the original to reduce the chance
        // of losing data in the case that something goes wrong during saving.
        fn safe_file_save(path: &Path, content: &str) -> Result<(), std::io::Error> {
            let temp_path = path.with_extension("tmp");

            // If the temp file exists, remove it.
            if temp_path.exists() {
                std::fs::remove_file(&temp_path)?;
            }

            // Create the directory if it doesn't exist.
            if let Some(directory) = path.parent() {
                if !directory.exists() {
                    std::fs::create_dir_all(&temp_path)?;
                }
            }

            // Write the temp file.
            let mut file = File::create(&temp_path)?;
            file.write(content.as_bytes())?;

            // If there's already a file at `path`, remove it.
            if path.exists() {
                std::fs::remove_file(&path)?;
            }

            // Rename the temp file to the original path name.
            std::fs::rename(temp_path, path)?;

            Ok(())
        }

        // Destructure the GUI state for serializing.
        let Model {
            state: State {
                speaker_editor: SpeakerEditor {
                    speakers,
                    next_id: next_speaker_id,
                    ..
                },
                source_editor: SourceEditor {
                    sources,
                    next_id: next_source_id,
                    ..
                },
                ..
            },
            assets,
            ..
        } = self;

        // Save the speaker configuration.
        let speakers_json_string = {
            let next_id = next_speaker_id;
            let stored_speakers = StoredSpeakers { speakers, next_id };
            serde_json::to_string_pretty(&stored_speakers)
                .expect("failed to serialize speaker layout")
        };
        safe_file_save(&speakers_path(&assets), &speakers_json_string)
            .expect("failed to save speakers file");

        // Save the list of audio sources.
        let sources_json_string = {
            let next_id = next_source_id;
            let stored_sources = StoredSources { sources, next_id };
            serde_json::to_string_pretty(&stored_sources)
                .expect("failed to serialize sources")
        };
        safe_file_save(&sources_path(&assets), &sources_json_string)
            .expect("failed to save sources file");
    }

    /// Whether or not the GUI currently contains representations of active sounds.
    ///
    /// This is used at the top-level to determine what application loop mode to use.
    pub fn is_animating(&self) -> bool {
        !self.audio_monitor.active_sounds.is_empty() || !self.audio_monitor.speakers.is_empty()
    }
}

impl State {
    /// Initialise the `State` and send any loaded speakers and sources to the audio and composer
    /// threads.
    pub fn new(assets: &Path, config: Config, channels: &Channels) -> Self {
        // Load the existing speaker layout configuration if there is one.
        let StoredSpeakers { speakers, next_id } = StoredSpeakers::load(&speakers_path(assets));

        // Send the loaded speakers to the audio thread.
        for speaker in &speakers {
            let (speaker_id, speaker_clone) = (speaker.id, speaker.audio.clone());
            channels.audio.send(move |audio| {
                audio.insert_speaker(speaker_id, speaker_clone);
            }).ok();
        }

        let speaker_editor = SpeakerEditor {
            is_open: false,
            selected: None,
            speakers,
            next_id,
        };

        // Load the existing sound sources if there are some.
        let audio_path = assets.join(Path::new(AUDIO_DIRECTORY_NAME));
        let StoredSources { sources, next_id } = StoredSources::load(&sources_path(assets), &audio_path);

        // Send the loaded sources to the composer thread.
        for source in &sources {
            let msg = composer::Message::UpdateSource(source.id, source.audio.clone());
            channels.composer_msg_tx.send(msg).expect("composer_msg_tx was closed");
        }

        let preview = SourcePreview {
            current: None,
            point: None,
        };

        let source_editor = SourceEditor {
            is_open: true,
            selected: None,
            next_id,
            sources,
            preview,
        };

        let camera = Camera {
            floorplan_pixels_per_metre: config.floorplan_pixels_per_metre,
            position: Point2 { x: Metres(0.0), y: Metres(0.0) },
            zoom: 0.0,
        };

        let osc_log = Log::with_limit(config.osc_log_limit);
        let interaction_log = Log::with_limit(config.interaction_log_limit);

        // State that is specific to the GUI itself.
        State {
            config,
            // TODO: Possibly load camera from file.
            camera,
            speaker_editor,
            source_editor,
            osc_log,
            interaction_log,
            side_menu_is_open: true,
            osc_log_is_open: false,
            interaction_log_is_open: false,
        }
    }
}

pub struct Channels {
    pub osc_msg_rx: mpsc::Receiver<(SocketAddr, osc::Message)>,
    pub interaction_rx: mpsc::Receiver<Interaction>,
    pub composer_msg_tx: mpsc::Sender<composer::Message>,
    /// A handle for communicating with the audio output stream.
    pub audio: audio::OutputStream,
    pub audio_monitor_msg_rx: mpsc::Receiver<AudioMonitorMessage>,
}

impl Channels {
    /// Initialise the GUI communication channels.
    pub fn new(
        osc_msg_rx: mpsc::Receiver<(SocketAddr, osc::Message)>,
        interaction_rx: mpsc::Receiver<Interaction>,
        composer_msg_tx: mpsc::Sender<composer::Message>,
        audio: audio::OutputStream,
        audio_monitor_msg_rx: mpsc::Receiver<AudioMonitorMessage>,
    ) -> Self
    {
        Channels {
            osc_msg_rx,
            interaction_rx,
            composer_msg_tx,
            audio,
            audio_monitor_msg_rx,
        }
    }
}

struct SpeakerEditor {
    is_open: bool,
    // The list of speaker outputs.
    speakers: Vec<Speaker>,
    // The index of the selected speaker.
    selected: Option<usize>,
    // The next ID to be used for a new speaker.
    next_id: audio::speaker::Id,
}

#[derive(Deserialize, Serialize)]
struct Speaker {
    // Speaker state shared with the audio thread.
    audio: audio::Speaker,
    name: String,
    id: audio::speaker::Id,
}

// A data structure from which the speaker layout can be saved and loaded.
#[derive(Deserialize, Serialize)]
struct StoredSpeakers {
    #[serde(default = "first_speaker_id")]
    next_id: audio::speaker::Id,
    #[serde(default = "Vec::new")]
    speakers: Vec<Speaker>,
}

// The name of the file where the speaker layout is saved.
const SPEAKERS_FILE_STEM: &'static str = "speakers";

// The name of the file where the list of sources is stored.
const SOURCES_FILE_STEM: &'static str = "sources";

// The name of the directory where the WAVs are stored.
const AUDIO_DIRECTORY_NAME: &'static str = "audio";

const SOUNDSCAPE_COLOR: ui::Color = ui::color::DARK_RED;
const INSTALLATION_COLOR: ui::Color = ui::color::DARK_GREEN;
const SCRIBBLES_COLOR: ui::Color = ui::color::DARK_PURPLE;

fn first_speaker_id() -> audio::speaker::Id {
    audio::speaker::Id(0)
}

impl StoredSpeakers {
    fn new() -> Self {
        StoredSpeakers {
            speakers: Vec::new(),
            next_id: first_speaker_id(),
        }
    }

    /// Load the stored speakers from the given path.
    ///
    /// If the path is invalid or the JSON can't be read, `StoredSpeakers::new` will be called.
    fn load(path: &Path) -> Self {
        File::open(&path)
            .ok()
            .and_then(|f| serde_json::from_reader(f).ok())
            .unwrap_or_else(StoredSpeakers::new)
    }
}

struct SourceEditor {
    is_open: bool,
    sources: Vec<Source>,
    // The index of the selected source.
    selected: Option<usize>,
    // The next ID to be used for a new source.
    next_id: audio::source::Id,
    preview: SourcePreview,
}

struct SourcePreview {
    current: Option<(SourcePreviewMode, audio::sound::Id)>,
    point: Option<Point2<Metres>>,
}

#[derive(Copy, Clone)]
enum SourcePreviewMode {
    OneShot,
    Continuous,
}

// A GUI view of an audio source.
#[derive(Deserialize, Serialize)]
struct Source {
    name: String,
    audio: audio::Source,
    id: audio::source::Id,
}

// A data structure from which sources can be saved/loaded.
#[derive(Deserialize, Serialize)]
struct StoredSources {
    #[serde(default = "Vec::new")]
    sources: Vec<Source>,
    #[serde(default = "first_source_id")]
    next_id: audio::source::Id,
}

fn first_source_id() -> audio::source::Id {
    audio::source::Id::INITIAL
}

impl StoredSources {
    fn new() -> Self {
        StoredSources {
            next_id: audio::source::Id::INITIAL,
            sources: Vec::new(),
        }
    }

    /// Load the audio sources from the given path.
    ///
    /// If there are any ".wav" files in `assets/audio` that have not yet been loaded into the
    /// stored sources, load them as `Wav` kind sources.
    ///
    /// If the path is invalid or the JSON can't be read, `StoredSources::new` will be called.
    fn load(sources_path: &Path, audio_path: &Path) -> Self {
        let mut stored = File::open(&sources_path)
            .ok()
            .and_then(|f| serde_json::from_reader(f).ok())
            .unwrap_or_else(StoredSources::new);

        // If there are any WAVs in `assets/audio/` that we have not yet listed, load them.
        if audio_path.exists() && audio_path.is_dir() {
            let wav_paths = std::fs::read_dir(&audio_path)
                .expect("failed to read audio directory")
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().ok().map(|e| e.is_file()).unwrap_or(false))
                .filter_map(|e| {
                    let file_name = e.file_name();
                    let file_path = Path::new(&file_name);
                    let ext = file_path.extension()
                        .and_then(OsStr::to_str)
                        .map(std::ascii::AsciiExt::to_ascii_lowercase);
                    match ext.as_ref().map(|e| &e[..]) {
                        Some("wav") | Some("wave") => Some(audio_path.join(file_path)),
                        _ => None,
                    }
                });

            // For each new wav file, create a new source.
            'paths: for path in wav_paths {
                // If we already have this one, continue.
                for s in &stored.sources {
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
                let wav = match audio::Wav::from_path(path) {
                    Ok(w) => w,
                    Err(e) => {
                        println!("Failed to load wav file {:?}: {}", name, e);
                        continue;
                    },
                };
                let kind = audio::source::Kind::Wav(wav);
                let role = None;
                let spread = Metres(2.5);
                let radians = 0.0;
                let audio = audio::Source { kind, role, spread, radians };
                let id = stored.next_id;
                let source = Source { name, audio, id };
                stored.sources.push(source);
                stored.next_id = audio::source::Id(stored.next_id.0 + 1);
            }
        }

        // Sort all sources by name.
        stored.sources.sort_by(|a, b| a.name.cmp(&b.name));
        stored
    }
}

impl<'a> Deref for Gui<'a> {
    type Target = UiCell<'a>;
    fn deref(&self) -> &Self::Target {
        &self.ui
    }
}

impl<'a> DerefMut for Gui<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.ui
    }
}

#[derive(Clone, Copy, Debug)]
struct Image {
    id: ui::image::Id,
    width: Scalar,
    height: Scalar,
}

#[derive(Debug)]
struct Images {
    floorplan: Image,
}

#[derive(Debug)]
struct Fonts {
    notosans_regular: text::font::Id,
}

// A 2D camera used to navigate around the floorplan visualisation.
#[derive(Debug)]
struct Camera {
    // The number of floorplan pixels per metre.
    floorplan_pixels_per_metre: f64,
    // The position of the camera over the floorplan.
    //
    // [0.0, 0.0] - the centre of the floorplan.
    position: Point2<Metres>,
    // The higher the zoom, the closer the floorplan appears.
    //
    // The zoom can be multiplied by a distance in metres to get the equivalent distance as a GUI
    // scalar value.
    //
    // 1.0 - Original resolution.
    // 0.5 - 50% view.
    zoom: Scalar,
}

impl Camera {
    /// Convert from metres to the GUI scalar value.
    fn metres_to_scalar(&self, Metres(metres): Metres) -> Scalar {
        self.zoom * metres * self.floorplan_pixels_per_metre
    }

    /// Convert from the GUI scalar value to metres.
    fn scalar_to_metres(&self, scalar: Scalar) -> Metres {
        Metres((scalar / self.zoom) / self.floorplan_pixels_per_metre)
    }
}

struct Log<T> {
    // Newest to oldest is stored front to back respectively.
    deque: VecDeque<T>,
    // The index of the oldest message currently stored in the deque.
    start_index: usize,
    // The max number of messages stored in the log at one time.
    limit: usize,
}

type OscLog = Log<(SocketAddr, osc::Message)>;
type InteractionLog = Log<Interaction>;

impl<T> Log<T> {
    // Construct an OscLog that stores the given max number of messages.
    fn with_limit(limit: usize) -> Self {
        Log {
            deque: VecDeque::new(),
            start_index: 0,
            limit,
        }
    }

    // Push a new OSC message to the log.
    fn push_msg(&mut self, msg: T) {
        self.deque.push_front(msg);
        while self.deque.len() > self.limit {
            self.deque.pop_back();
            self.start_index += 1;
        }
    }
}

impl OscLog {
    // Format the log in a single string of messages.
    fn format(&self) -> String {
        let mut s = String::new();
        let mut index = self.start_index + self.deque.len();
        for &(ref addr, ref msg) in &self.deque {
            let addr_string = format!("{}: [{}{}]\n", index, addr, msg.addr);
            s.push_str(&addr_string);

            // Arguments.
            if let Some(ref args) = msg.args {
                for arg in args {
                    s.push_str(&format!("    {:?}\n", arg));
                }
            }

            index -= 1;
        }
        s
    }
}

impl InteractionLog {
    // Format the log in a single string of messages.
    fn format(&self) -> String {
        let mut s = String::new();
        let mut index = self.start_index + self.deque.len();
        for &interaction in &self.deque {
            let line = format!("{}: {:?}\n", index, interaction);
            s.push_str(&line);
            index -= 1;
        }
        s
    }
}

impl<T> Deref for Log<T> {
    type Target = VecDeque<T>;
    fn deref(&self) -> &Self::Target {
        &self.deque
    }
}


// A structure for monitoring the state of the audio thread for visualisation.
struct AudioMonitor {
    active_sounds: ActiveSoundMap,
    speakers: HashMap<audio::speaker::Id, ChannelLevels>,
}

// The state of an active sound.
struct ActiveSound {
    source_id: audio::source::Id,
    position: Point2<Metres>,
    channels: Vec<ChannelLevels>,
}

// The detected levels for a single channel.
struct ChannelLevels {
    rms: f32,
    peak: f32,
}

/// A message sent from the audio thread with some audio levels.
pub enum AudioMonitorMessage {
    ActiveSound(audio::sound::Id, ActiveSoundMessage),
    Speaker(audio::speaker::Id, SpeakerMessage),
}

/// A message related to an active sound.
pub enum ActiveSoundMessage {
    Start {
        source_id: audio::source::Id,
        position: Point2<Metres>,
        channels: usize,
    },
    Update {
        position: Point2<Metres>,
    },
    UpdateChannel {
        index: usize,
        rms: f32,
        peak: f32,
    },
    End,
}

/// A message related to a speaker.
pub enum SpeakerMessage {
    Add,
    Update {
        rms: f32,
        peak: f32,
    },
    Remove,
}

/// The directory in which all fonts are stored.
fn fonts_directory(assets: &Path) -> PathBuf {
    assets.join("fonts")
}

/// The directory in which all images are stored.
fn images_directory(assets: &Path) -> PathBuf {
    assets.join("images")
}

/// Load the image at the given path into a texture.
///
/// Returns the dimensions of the image alongside the texture.
fn load_image(
    path: &Path,
    display: &glium::Display,
) -> ((Scalar, Scalar), glium::texture::Texture2d) {
    let rgba_image = nannou::image::open(&path).unwrap().to_rgba();
    let (w, h) = rgba_image.dimensions();
    let raw_image =
        glium::texture::RawImage2d::from_raw_rgba_reversed(&rgba_image.into_raw(), (w, h));
    let texture = glium::texture::Texture2d::new(display, raw_image).unwrap();
    ((w as Scalar, h as Scalar), texture)
}

/// Insert the image at the given path into the given `ImageMap`.
///
/// Return its Id and Dimensions in the form of an `Image`.
fn insert_image(
    path: &Path,
    display: &glium::Display,
    image_map: &mut ui::Texture2dMap,
) -> Image {
    let ((width, height), texture) = load_image(path, display);
    let id = image_map.insert(texture);
    let image = Image { id, width, height };
    image
}

// A unique ID foor each widget in the GUI.
widget_ids! {
    pub struct Ids {
        // The backdrop for all widgets.
        background,

        // The canvas for the menu to the left of the GUI.
        side_menu,
        // The menu button at the top of the sidebar.
        side_menu_button,
        side_menu_button_line_top,
        side_menu_button_line_middle,
        side_menu_button_line_bottom,
        // OSC Log.
        osc_log,
        osc_log_text,
        osc_log_scrollbar_y,
        osc_log_scrollbar_x,
        // Interaction Log.
        interaction_log,
        interaction_log_text,
        interaction_log_scrollbar_y,
        interaction_log_scrollbar_x,
        // Speaker Editor.
        speaker_editor,
        speaker_editor_no_speakers,
        speaker_editor_list,
        speaker_editor_add,
        speaker_editor_remove,
        speaker_editor_selected_canvas,
        speaker_editor_selected_none,
        speaker_editor_selected_name,
        speaker_editor_selected_channel,
        speaker_editor_selected_position,
        // Audio Sources.
        source_editor,
        source_editor_no_sources,
        source_editor_list,
        source_editor_add_wav,
        source_editor_add_realtime,
        source_editor_remove,
        source_editor_selected_canvas,
        source_editor_selected_none,
        source_editor_selected_name,
        source_editor_selected_role_list,
        source_editor_selected_wav_canvas,
        source_editor_selected_wav_text,
        source_editor_selected_wav_data,
        source_editor_selected_channel_layout_canvas,
        source_editor_selected_channel_layout_text,
        source_editor_selected_channel_layout_spread,
        source_editor_selected_channel_layout_rotation,
        source_editor_selected_channel_layout_field,
        source_editor_selected_channel_layout_spread_circle,
        source_editor_selected_channel_layout_channels[],
        source_editor_selected_channel_layout_channel_labels[],
        source_editor_preview_canvas,
        source_editor_preview_text,
        source_editor_preview_one_shot,
        source_editor_preview_continuous,

        // The floorplan image and the canvas on which it is placed.
        floorplan_canvas,
        floorplan,
        floorplan_speakers[],
        floorplan_source_preview,
    }
}

// Begin building a `CollapsibleArea` for the sidebar.
fn collapsible_area(is_open: bool, text: &str, side_menu_id: widget::Id)
    -> widget::CollapsibleArea
{
    widget::CollapsibleArea::new(is_open, text)
        .w_of(side_menu_id)
        .h(ITEM_HEIGHT)
}

// Begin building a basic info text block.
fn info_text(text: &str) -> widget::Text {
    widget::Text::new(&text)
        .font_size(SMALL_FONT_SIZE)
        .line_spacing(6.0)
}

const ITEM_HEIGHT: Scalar = 30.0;
const SMALL_FONT_SIZE: FontSize = 12;
const DARK_A: ui::Color = ui::Color::Rgba(0.1, 0.13, 0.15, 1.0);

// Instantiate the sidebar speaker editor widgets.
fn set_speaker_editor(gui: &mut Gui) -> widget::Id {
    let is_open = gui.state.speaker_editor.is_open;
    const LIST_HEIGHT: Scalar = 140.0;
    const PAD: Scalar = 6.0;
    const TEXT_PAD: Scalar = 20.0;

    const SELECTED_CANVAS_H: Scalar = ITEM_HEIGHT * 2.0 + PAD * 3.0;
    let speaker_editor_canvas_h = LIST_HEIGHT + ITEM_HEIGHT + SELECTED_CANVAS_H;

    let (area, event) = collapsible_area(is_open, "Speaker Editor", gui.ids.side_menu)
        .align_middle_x_of(gui.ids.side_menu)
        .down_from(gui.ids.side_menu_button, 0.0)
        .set(gui.ids.speaker_editor, gui);
    if let Some(event) = event {
        gui.state.speaker_editor.is_open = event.is_open();
    }

    if let Some(area) = area {
        // The canvas on which the log will be placed.
        let canvas = widget::Canvas::new()
            .scroll_kids()
            .pad(0.0)
            .h(speaker_editor_canvas_h);
        area.set(canvas, gui);

        // If there are no speakers, display a message saying how to add some.
        if gui.state.speaker_editor.speakers.is_empty() {
            widget::Text::new("Add some speaker outputs with the `+` button")
                .padded_w_of(area.id, TEXT_PAD)
                .mid_top_with_margin_on(area.id, TEXT_PAD)
                .font_size(SMALL_FONT_SIZE)
                .center_justify()
                .set(gui.ids.speaker_editor_no_speakers, gui);

        // Otherwise display the speaker list.
        } else {
            let num_items = gui.state.speaker_editor.speakers.len();
            let (mut events, scrollbar) = widget::ListSelect::single(num_items)
                .item_size(ITEM_HEIGHT)
                .h(LIST_HEIGHT)
                .align_middle_x_of(area.id)
                .align_top_of(area.id)
                .scrollbar_next_to()
                .scrollbar_color(color::LIGHT_CHARCOAL)
                .set(gui.ids.speaker_editor_list, gui);

            // If a speaker was removed, process it after the whole list is instantiated to avoid
            // invalid indices.
            let mut maybe_remove_index = None;

            while let Some(event) = events.next(gui, |i| gui.state.speaker_editor.selected == Some(i)) {
                use self::ui::widget::list_select::Event;
                match event {

                    // Instantiate a button for each speaker.
                    Event::Item(item) => {
                        let selected = gui.state.speaker_editor.selected == Some(item.i);
                        let label = {
                            let speaker = &gui.state.speaker_editor.speakers[item.i];
                            let channel = speaker.audio.channel;
                            let position = speaker.audio.point;
                            let label = format!("{} - CH {} - ({}mx, {}my)",
                                                speaker.name, channel,
                                                (position.x.0 * 100.0).trunc() / 100.0,
                                                (position.y.0 * 100.0).trunc() / 100.0);
                            label
                        };

                        // Blue if selected, gray otherwise.
                        let color = if selected { color::BLUE } else { color::CHARCOAL };

                        // Use `Button`s for the selectable items.
                        let button = widget::Button::new()
                            .label(&label)
                            .label_font_size(SMALL_FONT_SIZE)
                            .label_x(position::Relative::Place(position::Place::Start(Some(10.0))))
                            .color(color);
                        item.set(button, gui);

                        // If the button or any of its children are capturing the mouse, display
                        // the `remove` button.
                        let show_remove_button = gui.global_input().current.widget_capturing_mouse
                            .map(|id| {
                                id == item.widget_id ||
                                gui.widget_graph()
                                    .does_recursive_depth_edge_exist(item.widget_id, id)
                            })
                            .unwrap_or(false);

                        if !show_remove_button {
                            continue;
                        }

                        if widget::Button::new()
                            .label("X")
                            .label_font_size(SMALL_FONT_SIZE)
                            .color(color::DARK_RED.alpha(0.5))
                            .w_h(ITEM_HEIGHT, ITEM_HEIGHT)
                            .align_right_of(item.widget_id)
                            .align_middle_y_of(item.widget_id)
                            .parent(item.widget_id)
                            .set(gui.ids.speaker_editor_remove, gui)
                            .was_clicked()
                        {
                            maybe_remove_index = Some(item.i);
                            if selected {
                                gui.state.speaker_editor.selected = None;
                            }
                        }
                    },

                    // Update the selected speaker.
                    Event::Selection(idx) => gui.state.speaker_editor.selected = Some(idx),

                    _ => (),
                }
            }

            // The scrollbar for the list.
            if let Some(s) = scrollbar { s.set(gui); }

            // Remove a speaker if necessary.
            if let Some(i) = maybe_remove_index {
                let speaker = gui.state.speaker_editor.speakers.remove(i);
                gui.channels.audio.send(move |audio| {
                    audio.remove_speaker(speaker.id);
                }).ok();
            }
        }

        // Only display the `add_speaker` button if there are less than `max` num channels.
        let show_add_button = gui.state.speaker_editor.speakers.len() < audio::MAX_CHANNELS;

        if show_add_button {
            let plus_size = (ITEM_HEIGHT * 0.66) as FontSize;
            if widget::Button::new()
                .color(DARK_A)
                .label("+")
                .label_font_size(plus_size)
                .align_middle_x_of(area.id)
                .mid_top_with_margin_on(area.id, LIST_HEIGHT)
                .w_of(area.id)
                .parent(area.id)
                .set(gui.ids.speaker_editor_add, gui)
                .was_clicked()
            {
                let id = gui.state.speaker_editor.next_id;
                let name = format!("S{}", id.0);
                let channel = {
                    // Search for the next available channel starting from 0.
                    //
                    // Note: This is a super naiive way of searching however there should never
                    // be enough speakers to make it a problem.
                    let mut channel = 0;
                    'search: loop {
                        for speaker in &gui.state.speaker_editor.speakers {
                            if channel == speaker.audio.channel {
                                channel += 1;
                                continue 'search;
                            }
                        }
                        break channel;
                    }
                };
                let audio = audio::Speaker {
                    point: gui.state.camera.position,
                    channel: channel,
                };
                let speaker = Speaker { id, name, audio };

                let (speaker_id, speaker_clone) = (speaker.id, speaker.audio.clone());
                gui.channels.audio.send(move |audio| {
                    audio.insert_speaker(speaker_id, speaker_clone);
                }).ok();
                gui.state.speaker_editor.speakers.push(speaker);
                gui.state.speaker_editor.next_id = audio::speaker::Id(id.0.wrapping_add(1));
                gui.state.speaker_editor.selected = Some(gui.state.speaker_editor.speakers.len() - 1);
            }
        }

        let area_rect = gui.rect_of(area.id).unwrap();
        let start = area_rect.y.start;
        let end = start + SELECTED_CANVAS_H;
        let selected_canvas_y = ui::Range { start, end };

        widget::Canvas::new()
            .pad(PAD)
            .w_of(gui.ids.side_menu)
            .h(SELECTED_CANVAS_H)
            .y(selected_canvas_y.middle())
            .align_middle_x_of(gui.ids.side_menu)
            .set(gui.ids.speaker_editor_selected_canvas, gui);

        // If a speaker is selected, display its info.
        if let Some(i) = gui.state.speaker_editor.selected {
            let Gui { ref mut state, ref mut ui, ref ids, channels, .. } = *gui;
            let SpeakerEditor { ref mut speakers, .. } = state.speaker_editor;

            for event in widget::TextBox::new(&speakers[i].name)
                .mid_top_of(ids.speaker_editor_selected_canvas)
                .kid_area_w_of(ids.speaker_editor_selected_canvas)
                .parent(gui.ids.speaker_editor_selected_canvas)
                .h(ITEM_HEIGHT)
                .color(DARK_A)
                .font_size(SMALL_FONT_SIZE)
                .set(ids.speaker_editor_selected_name, ui)
            {
                if let widget::text_box::Event::Update(string) = event {
                    speakers[i].name = string;
                }
            }

            let channel_vec: Vec<String> = (0..audio::MAX_CHANNELS)
                .map(|ch| {
                    speakers
                        .iter()
                        .enumerate()
                        .find(|&(ix, s)| i != ix && s.audio.channel == ch)
                        .map(|(_ix, s)| format!("CH {} (swap with {})", ch, &s.name))
                        .unwrap_or_else(|| format!("CH {}", ch))
                })
                .collect();
            let selected = speakers[i].audio.channel;

            for new_index in widget::DropDownList::new(&channel_vec, Some(selected))
                .down_from(ids.speaker_editor_selected_name, PAD)
                .align_middle_x_of(ids.side_menu)
                .kid_area_w_of(ids.speaker_editor_selected_canvas)
                .h(ITEM_HEIGHT)
                .parent(ids.speaker_editor_selected_canvas)
                .scrollbar_on_top()
                .max_visible_items(5)
                .color(DARK_A)
                .border_color(color::LIGHT_CHARCOAL)
                .label_font_size(SMALL_FONT_SIZE)
                .set(ids.speaker_editor_selected_channel, ui)
            {
                speakers[i].audio.channel = new_index;
                let (speaker_id, speaker_clone) = (speakers[i].id, speakers[i].audio.clone());
                channels.audio.send(move |audio| {
                    audio.insert_speaker(speaker_id, speaker_clone);
                }).ok();

                // If an existing speaker was assigned to `index`, swap it with the original
                // selection.
                let maybe_index = speakers.iter()
                    .enumerate()
                    .find(|&(ix, s)| i != ix && s.audio.channel == new_index)
                    .map(|(ix, _)| ix);
                if let Some(ix) = maybe_index {
                    speakers[ix].audio.channel = selected;
                    let (speaker_id, speaker_clone) = (speakers[ix].id, speakers[ix].audio.clone());
                    channels.audio.send(move |audio| {
                        audio.insert_speaker(speaker_id, speaker_clone);
                    }).ok();
                }
            }

        // Otherwise no speaker is selected.
        } else {
            widget::Text::new("No speaker selected")
                .padded_w_of(area.id, TEXT_PAD)
                .mid_top_with_margin_on(gui.ids.speaker_editor_selected_canvas, TEXT_PAD)
                .font_size(SMALL_FONT_SIZE)
                .center_justify()
                .set(gui.ids.speaker_editor_selected_none, gui);
        }

        area.id
    } else {
        gui.ids.speaker_editor
    }
}

fn set_source_editor(last_area_id: widget::Id, gui: &mut Gui) -> widget::Id {
    let is_open = gui.state.source_editor.is_open;
    const LIST_HEIGHT: Scalar = 140.0;
    const PAD: Scalar = 6.0;
    const TEXT_PAD: Scalar = 20.0;

    // 200.0 is just some magic, temp, extra height.
    const PREVIEW_CANVAS_H: Scalar = 66.0;
    const WAV_CANVAS_H: Scalar = 100.0;
    const CHANNEL_LAYOUT_CANVAS_H: Scalar = 200.0;
    const SELECTED_CANVAS_H: Scalar = ITEM_HEIGHT * 2.0 + PAD * 6.0 + PREVIEW_CANVAS_H + WAV_CANVAS_H + CHANNEL_LAYOUT_CANVAS_H;
    let source_editor_canvas_h = LIST_HEIGHT + ITEM_HEIGHT + SELECTED_CANVAS_H;

    let (area, event) = collapsible_area(is_open, "Source Editor", gui.ids.side_menu)
        .align_middle_x_of(gui.ids.side_menu)
        .down_from(last_area_id, 0.0)
        .set(gui.ids.source_editor, gui);
    if let Some(event) = event {
        gui.state.source_editor.is_open = event.is_open();
    }

    if let Some(area) = area {
        // The canvas on which the log will be placed.
        let canvas = widget::Canvas::new()
            .scroll_kids()
            .pad(0.0)
            .h(source_editor_canvas_h);
        area.set(canvas, gui);

        // If there are no sources, display a message saying how to add some.
        if gui.state.source_editor.sources.is_empty() {
            widget::Text::new("Add some source outputs with the `+` button")
                .padded_w_of(area.id, TEXT_PAD)
                .mid_top_with_margin_on(area.id, TEXT_PAD)
                .font_size(SMALL_FONT_SIZE)
                .center_justify()
                .set(gui.ids.source_editor_no_sources, gui);

        // Otherwise display the source list.
        } else {
            let num_items = gui.state.source_editor.sources.len();
            let (mut events, scrollbar) = widget::ListSelect::single(num_items)
                .item_size(ITEM_HEIGHT)
                .h(LIST_HEIGHT)
                .align_middle_x_of(area.id)
                .align_top_of(area.id)
                .scrollbar_next_to()
                .scrollbar_color(color::LIGHT_CHARCOAL)
                .set(gui.ids.source_editor_list, gui);

            // If a source was removed, process it after the whole list is instantiated to avoid
            // invalid indices.
            let mut maybe_remove_index = None;

            while let Some(event) = events.next(gui, |i| gui.state.source_editor.selected == Some(i)) {
                use self::ui::widget::list_select::Event;
                match event {

                    // Instantiate a button for each source.
                    Event::Item(item) => {
                        let selected = gui.state.source_editor.selected == Some(item.i);
                        let (label, is_wav) = {
                            let source = &gui.state.source_editor.sources[item.i];
                            match source.audio.kind {
                                audio::source::Kind::Wav(ref wav) => {
                                    (format!("[{}CH WAV] {}", wav.channels, source.name), true)
                                },
                                audio::source::Kind::Realtime(ref rt) => {
                                    (format!("[{}CH RT] {}", rt.channels, source.name), false)
                                },
                            }
                        };

                        // Blue if selected, gray otherwise.
                        let color = if selected { color::BLUE } else { color::CHARCOAL };

                        // Use `Button`s for the selectable items.
                        let button = widget::Button::new()
                            .label(&label)
                            .label_font_size(SMALL_FONT_SIZE)
                            .label_x(position::Relative::Place(position::Place::Start(Some(10.0))))
                            .color(color);
                        item.set(button, gui);

                        // If the button or any of its children are capturing the mouse, display
                        // the `remove` button.
                        let show_remove_button = !is_wav &&
                            gui.global_input().current.widget_capturing_mouse
                                .map(|id| {
                                    id == item.widget_id ||
                                    gui.widget_graph()
                                        .does_recursive_depth_edge_exist(item.widget_id, id)
                                })
                                .unwrap_or(false);

                        if !show_remove_button {
                            continue;
                        }

                        if widget::Button::new()
                            .label("X")
                            .label_font_size(SMALL_FONT_SIZE)
                            .color(color::DARK_RED.alpha(0.5))
                            .w_h(ITEM_HEIGHT, ITEM_HEIGHT)
                            .align_right_of(item.widget_id)
                            .align_middle_y_of(item.widget_id)
                            .parent(item.widget_id)
                            .set(gui.ids.source_editor_remove, gui)
                            .was_clicked()
                        {
                            maybe_remove_index = Some(item.i);
                            if selected {
                                gui.state.source_editor.selected = None;
                            }
                        }
                    },

                    // Update the selected source.
                    Event::Selection(idx) => {
                        // If a source was being previewed, stop it.
                        if let Some((_, sound_id)) = gui.state.source_editor.preview.current {
                            gui.channels.audio.send(move |audio| {
                                audio.remove_sound(sound_id);
                            }).ok();

                            gui.state.source_editor.preview.current = None;
                        }

                        gui.state.source_editor.selected = Some(idx);

                    },

                    _ => (),
                }
            }

            // The scrollbar for the list.
            if let Some(s) = scrollbar { s.set(gui); }

            // Remove a source if necessary.
            if let Some(_i) = maybe_remove_index {
                unimplemented!();
            }
        }

        let plus_button_w = gui.rect_of(area.id).unwrap().w() / 2.0;
        let plus_button = || -> widget::Button<widget::button::Flat> {
            widget::Button::new()
                .color(DARK_A)
                .w(plus_button_w)
                .label_font_size(SMALL_FONT_SIZE)
                .parent(area.id)
                .mid_top_with_margin_on(area.id, LIST_HEIGHT)
        };

        let new_wav = plus_button()
            .label("+ WAV")
            .align_left_of(area.id)
            .set(gui.ids.source_editor_add_wav, gui)
            .was_clicked();

        let new_realtime = plus_button()
            .label("+ Realtime")
            .align_right_of(area.id)
            .set(gui.ids.source_editor_add_realtime, gui)
            .was_clicked();

        if new_wav || new_realtime {
            // let id = gui.state.speaker_editor.next_id;
            // let name = format!("S{}", id.0);
            // let channel = {
            //     // Search for the next available channel starting from 0.
            //     //
            //     // Note: This is a super naiive way of searching however there should never
            //     // be enough speakers to make it a problem.
            //     let mut channel = 0;
            //     'search: loop {
            //         for speaker in &gui.state.speaker_editor.speakers {
            //             if channel == speaker.audio.channel.load(atomic::Ordering::Relaxed) {
            //                 channel += 1;
            //                 continue 'search;
            //             }
            //         }
            //         break channel;
            //     }
            // };
            // let audio = Arc::new(audio::Speaker {
            //     point: Atomic::new(gui.state.camera.position),
            //     channel: AtomicUsize::new(channel),
            // });
            // let speaker = Speaker { id, name, audio };

            // gui.state.speaker_editor.audio_msg_tx
            //     .send(audio::Message::AddSpeaker(speaker.id, speaker.audio.clone()))
            //     .expect("audio_msg_tx was closed");
            // gui.state.speaker_editor.speakers.push(speaker);
            // gui.state.speaker_editor.next_id = audio::speaker::Id(id.0.wrapping_add(1));
            // gui.state.speaker_editor.selected = Some(gui.state.speaker_editor.speakers.len() - 1);
        }

        let area_rect = gui.rect_of(area.id).unwrap();
        let start = area_rect.y.start;
        let end = start + SELECTED_CANVAS_H;
        let selected_canvas_y = ui::Range { start, end };

        widget::Canvas::new()
            .pad(PAD)
            .w_of(gui.ids.side_menu)
            .h(SELECTED_CANVAS_H)
            .y(selected_canvas_y.middle())
            .align_middle_x_of(gui.ids.side_menu)
            .set(gui.ids.source_editor_selected_canvas, gui);

        let selected_canvas_kid_area = gui.kid_area_of(gui.ids.source_editor_selected_canvas).unwrap();

        // If a source is selected, display its info.
        if let Some(i) = gui.state.source_editor.selected {
            let Gui {
                ref mut ui,
                ref mut ids,
                channels,
                sound_id_gen,
                state: &mut State {
                    ref camera,
                    source_editor: SourceEditor {
                        ref mut sources,
                        ref mut preview,
                        ..
                    },
                    ..
                },
                ..
            } = *gui;

            for event in widget::TextBox::new(&sources[i].name)
                .mid_top_of(ids.source_editor_selected_canvas)
                .kid_area_w_of(ids.source_editor_selected_canvas)
                .w(selected_canvas_kid_area.w())
                .parent(ids.source_editor_selected_canvas)
                .h(ITEM_HEIGHT)
                .color(DARK_A)
                .font_size(SMALL_FONT_SIZE)
                .set(ids.source_editor_selected_name, ui)
            {
                if let widget::text_box::Event::Update(string) = event {
                    sources[i].name = string;
                }
            }

            // TODO: 4 Role Buttons
            let role_button_w = selected_canvas_kid_area.w() / 4.0;
            const NUM_ROLES: usize = 4;
            let (mut events, _) = widget::ListSelect::single(NUM_ROLES)
                .flow_right()
                .item_size(role_button_w)
                .h(ITEM_HEIGHT)
                .align_middle_x_of(ids.source_editor_selected_canvas)
                .down_from(ids.source_editor_selected_name, PAD)
                .set(ids.source_editor_selected_role_list, ui);

            fn int_to_role(i: usize) -> Option<audio::source::Role> {
                match i {
                    1 => Some(audio::source::Role::Soundscape),
                    2 => Some(audio::source::Role::Installation),
                    3 => Some(audio::source::Role::Scribbles),
                    _ => None
                }
            }

            fn role_color(role: Option<audio::source::Role>) -> ui::Color {
                match role {
                    None => color::DARK_GREY,
                    Some(audio::source::Role::Soundscape) => SOUNDSCAPE_COLOR,
                    Some(audio::source::Role::Installation) => INSTALLATION_COLOR,
                    Some(audio::source::Role::Scribbles) => SCRIBBLES_COLOR,
                }
            }

            fn role_label(role: Option<audio::source::Role>) -> &'static str {
                match role {
                    None => "NONE",
                    Some(audio::source::Role::Soundscape) => "SCAPE",
                    Some(audio::source::Role::Installation) => "INST",
                    Some(audio::source::Role::Scribbles) => "SCRIB",
                }
            }

            let selected_role = sources[i].audio.role;
            let role_selected = |j| int_to_role(j) == selected_role;

            while let Some(event) = events.next(ui, |j| role_selected(j)) {
                use self::ui::widget::list_select::Event;
                match event {

                    // Instantiate a button for each role.
                    Event::Item(item) => {
                        let selected = role_selected(item.i);
                        let role = int_to_role(item.i);
                        let label = role_label(role);

                        // Blue if selected, gray otherwise.
                        let color = if selected { role_color(role) } else { color::CHARCOAL };

                        // Use `Button`s for the selectable items.
                        let button = widget::Button::new()
                            .label(&label)
                            .label_font_size(SMALL_FONT_SIZE)
                            .color(color);
                        item.set(button, ui);
                    },

                    // Update the selected role.
                    Event::Selection(idx) => {
                        let source = &mut sources[i];
                        source.audio.role = int_to_role(idx);
                        let msg = composer::Message::UpdateSource(source.id, source.audio.clone());
                        channels.composer_msg_tx.send(msg).expect("composer_msg_tx was closed");
                    },

                    _ => (),
                }
            }

            // Preview options.
            widget::Canvas::new()
                .mid_left_of(ids.source_editor_selected_canvas)
                .down(PAD)
                .parent(ids.source_editor_selected_canvas)
                .color(color::CHARCOAL)
                .w(selected_canvas_kid_area.w())
                .h(PREVIEW_CANVAS_H)
                .pad(PAD)
                .set(ids.source_editor_preview_canvas, ui);

            // PREVIEW header..
            widget::Text::new("PREVIEW")
                .font_size(SMALL_FONT_SIZE)
                .top_left_of(ids.source_editor_preview_canvas)
                .set(ids.source_editor_preview_text, ui);

            let preview_kid_area = ui.kid_area_of(ids.source_editor_preview_canvas).unwrap();
            let button_w = preview_kid_area.w() / 2.0 - PAD / 2.0;

            fn sound_from_source(
                source_id: audio::source::Id,
                source: &audio::Source,
                point: Point2<Metres>,
                should_cycle: bool,
            ) -> audio::Sound {
                match source.kind {
                    audio::source::Kind::Wav(ref wav) => {
                        // The wave signal iterator.
                        let signal = match should_cycle {
                            false => audio::wav::stream_signal(&wav.path).unwrap(),
                            true => audio::wav::stream_signal_cycled(&wav.path).unwrap(),
                        };

                        audio::Sound {
                            source_id: source_id,
                            channels: wav.channels,
                            signal: signal,
                            point: point,
                            spread: source.spread,
                            radians: source.radians,
                        }
                    },
                    audio::source::Kind::Realtime(ref _realtime) => {
                        unimplemented!();
                    },
                }
            }

            if widget::Button::new()
                .bottom_left_of(ids.source_editor_preview_canvas)
                .label("One Shot")
                .label_font_size(SMALL_FONT_SIZE)
                .w(button_w)
                .color(match preview.current {
                    Some((SourcePreviewMode::OneShot, _)) => color::BLUE,
                    _ => color::DARK_CHARCOAL,
                })
                .set(ids.source_editor_preview_one_shot, ui)
                .was_clicked()
            {
                loop {
                    match preview.current {
                        Some((mode, sound_id)) => {
                            channels.audio.send(move |audio| {
                                audio.remove_sound(sound_id);
                            }).ok();

                            preview.current = None;
                            match mode {
                                SourcePreviewMode::Continuous => continue,
                                SourcePreviewMode::OneShot => break,
                            }
                        },
                        None => {
                            // Set the preview mode to one-shot.
                            let sound_id = sound_id_gen.generate_next();
                            preview.current = Some((SourcePreviewMode::OneShot, sound_id));
                            // Set the preview position to the centre of the camera if not yet set.
                            if preview.point.is_none() {
                                preview.point = Some(camera.position);
                            }
                            // Send the selected source to the audio thread for playback.
                            let should_cycle = false;
                            let sound = sound_from_source(sources[i].id,
                                                          &sources[i].audio,
                                                          preview.point.unwrap(),
                                                          should_cycle);
                            channels.audio.send(move |audio| {
                                audio.insert_sound(sound_id, sound.into());
                            }).ok();

                            break;
                        },
                    }
                }
            }

            if widget::Button::new()
                .bottom_right_of(ids.source_editor_preview_canvas)
                .label("Continuous")
                .label_font_size(SMALL_FONT_SIZE)
                .w(button_w)
                .color(match preview.current {
                    Some((SourcePreviewMode::Continuous, _)) => color::BLUE,
                    _ => color::DARK_CHARCOAL,
                })
                .set(ids.source_editor_preview_continuous, ui)
                .was_clicked()
            {
                loop {
                    match preview.current {
                        Some((mode, sound_id)) => {
                            channels.audio.send(move |audio| {
                                audio.remove_sound(sound_id);
                            }).ok();
                            preview.current = None;
                            match mode {
                                SourcePreviewMode::OneShot => continue,
                                SourcePreviewMode::Continuous => break,
                            }
                        },
                        None => {
                            // Set the preview mode to one-shot.
                            let sound_id = sound_id_gen.generate_next();
                            preview.current = Some((SourcePreviewMode::Continuous, sound_id));
                            // Set the preview position to the centre of the camera if not yet set.
                            if preview.point.is_none() {
                                preview.point = Some(camera.position);
                            }
                            // Send the selected source to the audio thread for playback.
                            // TODO: This should loop somehow.
                            let should_cycle = true;
                            let sound = sound_from_source(sources[i].id,
                                                          &sources[i].audio,
                                                          preview.point.unwrap(),
                                                          should_cycle);
                            channels.audio.send(move |audio| {
                                audio.insert_sound(sound_id, sound.into());
                            }).ok();
                            break;
                        },
                    }
                }
            }

            // Kind-specific data.
            let (kind_canvas_id, num_channels) = match sources[i].audio.kind {
                audio::source::Kind::Wav(ref wav) => {

                    // Instantiate a small canvas for displaying wav-specific stuff.
                    widget::Canvas::new()
                        .down_from(ids.source_editor_preview_canvas, PAD)
                        .parent(ids.source_editor_selected_canvas)
                        .w(selected_canvas_kid_area.w())
                        .color(color::CHARCOAL)
                        .h(WAV_CANVAS_H)
                        .pad(PAD)
                        .set(ids.source_editor_selected_wav_canvas, ui);

                    // Display the immutable WAV data.
                    widget::Text::new("WAV DATA")
                        .font_size(SMALL_FONT_SIZE)
                        .top_left_of(ids.source_editor_selected_wav_canvas)
                        .set(ids.source_editor_selected_wav_text, ui);
                    let duration_ms = wav.duration_ms();
                    let duration_line = if duration_ms.ms() > 1_000.0 {
                        format!("Duration: {:.4} seconds", duration_ms.ms() / 1_000.0)
                    } else {
                        format!("Duration: {:.4} milliseconds", duration_ms.ms())
                    };
                    let file_line = format!("File: {}", wav.path.file_name().unwrap().to_str().unwrap());
                    let data = format!("{}\nChannels: {}\nSample Rate: {}\n{}",
                                       file_line, wav.channels, wav.sample_hz, duration_line);
                    widget::Text::new(&data)
                        .font_size(SMALL_FONT_SIZE)
                        .align_left_of(ids.source_editor_selected_wav_text)
                        .down(PAD)
                        .line_spacing(PAD)
                        .set(ids.source_editor_selected_wav_data, ui);

                    (ids.source_editor_selected_wav_canvas, wav.channels)
                },
                audio::source::Kind::Realtime(ref _realtime) => {
                    unreachable!();
                },
            };

            // Channel layout widgets.
            widget::Canvas::new()
                .down_from(kind_canvas_id, PAD)
                .h(CHANNEL_LAYOUT_CANVAS_H)
                .w(selected_canvas_kid_area.w())
                .pad(PAD)
                .parent(ids.source_editor_selected_canvas)
                .color(color::CHARCOAL)
                .set(ids.source_editor_selected_channel_layout_canvas, ui);

            // Display the immutable WAV data.
            widget::Text::new("CHANNEL LAYOUT")
                .font_size(SMALL_FONT_SIZE)
                .top_left_of(ids.source_editor_selected_channel_layout_canvas)
                .set(ids.source_editor_selected_channel_layout_text, ui);

            let channel_layout_kid_area = ui.kid_area_of(ids.source_editor_selected_channel_layout_canvas).unwrap();
            let slider_w = channel_layout_kid_area.w() / 2.0 - PAD / 2.0;

            let slider = |value, min, max| {
                widget::Slider::new(value, min, max)
                    .label_font_size(SMALL_FONT_SIZE)
                    .w(slider_w)
            };

            // Slider for controlling how far apart speakers should be spread.
            const MIN_SPREAD: f32 = 0.0;
            const MAX_SPREAD: f32 = 10.0;
            let mut spread = sources[i].audio.spread.0 as f32;
            let label = format!("Spread: {:.2} metres", spread);
            for new_spread in slider(spread, MIN_SPREAD, MAX_SPREAD)
                .skew(2.0)
                .label(&label)
                .mid_left_of(ids.source_editor_selected_channel_layout_canvas)
                .down(PAD * 1.5)
                .set(ids.source_editor_selected_channel_layout_spread, ui)
            {
                spread = new_spread;
                sources[i].audio.spread = Metres(spread as _);
                let msg = composer::Message::UpdateSource(sources[i].id, sources[i].audio.clone());
                channels.composer_msg_tx.send(msg).expect("composer_msg_tx was closed");
            }

            // Slider for controlling how channels should be rotated.
            const MIN_RADIANS: f32 = 0.0;
            const MAX_RADIANS: f32 = std::f32::consts::PI * 2.0;
            let mut rotation = sources[i].audio.radians;
            let label = format!("Rotate: {:.2} radians", rotation);
            for new_rotation in slider(rotation, MIN_RADIANS, MAX_RADIANS)
                .label(&label)
                .mid_right_of(ids.source_editor_selected_channel_layout_canvas)
                .align_middle_y_of(ids.source_editor_selected_channel_layout_spread)
                .set(ids.source_editor_selected_channel_layout_rotation, ui)
            {
                rotation = new_rotation;
                sources[i].audio.radians = rotation;
                let msg = composer::Message::UpdateSource(sources[i].id, sources[i].audio.clone());
                channels.composer_msg_tx.send(msg).expect("composer_msg_tx was closed");
            }

            // The field over which the channel layout will be visualised.
            let spread_rect = ui.rect_of(ids.source_editor_selected_channel_layout_spread).unwrap();
            let layout_top = spread_rect.bottom() - PAD;
            let layout_bottom = channel_layout_kid_area.bottom();
            let layout_h = layout_top - layout_bottom;
            const CHANNEL_CIRCLE_RADIUS: Scalar = PAD * 2.0;
            let field_h = layout_h - CHANNEL_CIRCLE_RADIUS * 2.0;
            let field_radius = field_h / 2.0;
            widget::Circle::fill(field_radius)
                .color(DARK_A)
                .down_from(ids.source_editor_selected_channel_layout_spread, PAD + CHANNEL_CIRCLE_RADIUS)
                .align_middle_x_of(ids.source_editor_selected_channel_layout_canvas)
                .set(ids.source_editor_selected_channel_layout_field, ui);

            // Circle demonstrating the actual spread distance of the channels relative to min/max.
            let min_spread_circle_radius = field_radius / 2.0;
            let spread_circle_radius = ui::utils::map_range(spread,
                                                            MIN_SPREAD, MAX_SPREAD,
                                                            min_spread_circle_radius, field_radius);
            widget::Circle::outline(spread_circle_radius)
                .color(color::DARK_BLUE)
                .middle_of(ids.source_editor_selected_channel_layout_field)
                .set(ids.source_editor_selected_channel_layout_spread_circle, ui);

            // A circle for each channel along the edge of the `spread_circle`.
            if ids.source_editor_selected_channel_layout_channels.len() < num_channels {
                let id_gen = &mut ui.widget_id_generator();
                ids.source_editor_selected_channel_layout_channels.resize(num_channels, id_gen);
            }
            if ids.source_editor_selected_channel_layout_channel_labels.len() < num_channels {
                let id_gen = &mut ui.widget_id_generator();
                ids.source_editor_selected_channel_layout_channel_labels.resize(num_channels, id_gen);
            }
            for i in 0..num_channels {

                // The channel circle.
                let id = ids.source_editor_selected_channel_layout_channels[i];
                let (x, y) = if num_channels == 1 {
                    (0.0, 0.0)
                } else {
                    let phase = i as f32 / num_channels as f32;
                    let default_radians = phase * MAX_RADIANS;
                    let radians = (rotation + default_radians) as Scalar;
                    let x = -radians.cos() * spread_circle_radius;
                    let y = radians.sin() * spread_circle_radius;
                    (x, y)
                };
                widget::Circle::fill(CHANNEL_CIRCLE_RADIUS)
                    .color(color::BLUE)
                    .x_y_relative_to(ids.source_editor_selected_channel_layout_spread_circle, x, y)
                    .parent(ids.source_editor_selected_channel_layout_spread_circle)
                    .set(id, ui);

                // The label showing the channel number (starting from 1).
                let label_id = ids.source_editor_selected_channel_layout_channel_labels[i];
                let label = format!("{}", i+1);
                widget::Text::new(&label)
                    .middle_of(id)
                    .y_relative_to(id, SMALL_FONT_SIZE as Scalar * 0.13)
                    .font_size(SMALL_FONT_SIZE)
                    .set(label_id, ui);
            }

        // Otherwise no source is selected.
        } else {
            widget::Text::new("No source selected")
                .padded_w_of(area.id, TEXT_PAD)
                .mid_top_with_margin_on(gui.ids.source_editor_selected_canvas, TEXT_PAD)
                .font_size(SMALL_FONT_SIZE)
                .center_justify()
                .set(gui.ids.source_editor_selected_none, gui);
        }

        area.id
    } else {
        gui.ids.source_editor
    }
}

fn set_osc_log(last_area_id: widget::Id, gui: &mut Gui) -> widget::Id {
    let is_open = gui.state.osc_log_is_open;
    let log_canvas_h = 200.0;
    let (area, event) = collapsible_area(is_open, "OSC Input Log", gui.ids.side_menu)
        .align_middle_x_of(gui.ids.side_menu)
        .down_from(last_area_id, 0.0)
        .set(gui.ids.osc_log, gui);
    if let Some(event) = event {
        gui.state.osc_log_is_open = event.is_open();
    }
    if let Some(area) = area {

        // The canvas on which the log will be placed.
        let canvas = widget::Canvas::new()
            .scroll_kids()
            .pad(10.0)
            .h(log_canvas_h);
        area.set(canvas, gui);

        // The text widget used to display the log.
        let log_string = match gui.state.osc_log.len() {
            0 => format!("No messages received yet.\nListening on port {}...",
                         gui.state.config.osc_input_port),
            _ => gui.state.osc_log.format(),
        };
        info_text(&log_string)
            .top_left_of(area.id)
            .kid_area_w_of(area.id)
            .set(gui.ids.osc_log_text, gui);

        // Scrollbars.
        widget::Scrollbar::y_axis(area.id)
            .color(color::LIGHT_CHARCOAL)
            .auto_hide(false)
            .set(gui.ids.osc_log_scrollbar_y, gui);
        widget::Scrollbar::x_axis(area.id)
            .color(color::LIGHT_CHARCOAL)
            .auto_hide(true)
            .set(gui.ids.osc_log_scrollbar_x, gui);

        area.id
    } else {
        gui.ids.osc_log
    }
}

fn set_interaction_log(last_area_id: widget::Id, gui: &mut Gui) -> widget::Id {
    let is_open = gui.state.interaction_log_is_open;
    let log_canvas_h = 200.0;
    let (area, event) = collapsible_area(is_open, "Interaction Log", gui.ids.side_menu)
        .align_middle_x_of(gui.ids.side_menu)
        .down_from(last_area_id, 0.0)
        .set(gui.ids.interaction_log, gui);
    if let Some(event) = event {
        gui.state.interaction_log_is_open = event.is_open();
    }

    if let Some(area) = area {
        // The canvas on which the log will be placed.
        let canvas = widget::Canvas::new()
            .scroll_kids()
            .pad(10.0)
            .h(log_canvas_h);
        area.set(canvas, gui);

        // The text widget used to display the log.
        let log_string = match gui.state.interaction_log.len() {
            0 => format!("No interactions received yet.\nListening on port {}...",
                         gui.state.config.osc_input_port),
            _ => gui.state.interaction_log.format(),
        };
        info_text(&log_string)
            .top_left_of(area.id)
            .kid_area_w_of(area.id)
            .set(gui.ids.interaction_log_text, gui);

        // Scrollbars.
        widget::Scrollbar::y_axis(area.id)
            .color(color::LIGHT_CHARCOAL)
            .auto_hide(false)
            .set(gui.ids.interaction_log_scrollbar_y, gui);
        widget::Scrollbar::x_axis(area.id)
            .color(color::LIGHT_CHARCOAL)
            .auto_hide(true)
            .set(gui.ids.interaction_log_scrollbar_x, gui);

        area.id
    } else {
        gui.ids.interaction_log
    }
}

// Set the widgets in the side menu.
fn set_side_menu_widgets(gui: &mut Gui) {

    // Speaker Editor - for adding, editing and removing speakers.
    let last_area_id = set_speaker_editor(gui);

    // For adding, changing and removing audio sources.
    let last_area_id = set_source_editor(last_area_id, gui);

    // The log of received OSC messages.
    let last_area_id = set_osc_log(last_area_id, gui);

    // The log of received Interactions.
    let _last_area_id = set_interaction_log(last_area_id, gui);
}

// Update all widgets in the GUI with the given state.
fn set_widgets(gui: &mut Gui) {

    let background_color = color::WHITE;

    // The background for the main `UI` window.
    widget::Canvas::new()
        .color(background_color)
        .pad(0.0)
        .parent(gui.window)
        .middle_of(gui.window)
        .wh_of(gui.window)
        .set(gui.ids.background, gui);

    // A thin menu bar on the left.
    //
    // The menu bar is collapsed by default, and shows three lines at the top.
    // Pressing these three lines opens the menu, revealing a list of options.
    const CLOSED_SIDE_MENU_W: ui::Scalar = 40.0;
    const OPEN_SIDE_MENU_W: ui::Scalar = 300.0;
    let side_menu_is_open = gui.state.side_menu_is_open;
    let side_menu_w = match side_menu_is_open {
        false => CLOSED_SIDE_MENU_W,
        true => OPEN_SIDE_MENU_W,
    };

    // The canvas on which all side_menu widgets are placed.
    widget::Canvas::new()
        .w(side_menu_w)
        .h_of(gui.ids.background)
        .mid_left_of(gui.ids.background)
        .pad(0.0)
        .color(color::rgb(0.1, 0.13, 0.15))
        .set(gui.ids.side_menu, gui);

    // The classic three line menu button for opening the side_menu.
    for _click in widget::Button::new()
        .w_h(side_menu_w, CLOSED_SIDE_MENU_W)
        .mid_top_of(gui.ids.side_menu)
        //.color(color::BLACK)
        .color(color::rgb(0.07, 0.08, 0.09))
        .set(gui.ids.side_menu_button, gui)
    {
        gui.state.side_menu_is_open = !side_menu_is_open;
    }

    // Draw the three lines using rectangles.
    fn menu_button_line(menu_button: widget::Id) -> widget::Rectangle {
        let line_h = 2.0;
        let line_w = CLOSED_SIDE_MENU_W / 3.0;
        widget::Rectangle::fill([line_w, line_h])
            .color(color::WHITE)
            .graphics_for(menu_button)
    }

    let margin = CLOSED_SIDE_MENU_W / 3.0;
    menu_button_line(gui.ids.side_menu_button)
        .mid_top_with_margin_on(gui.ids.side_menu_button, margin)
        .set(gui.ids.side_menu_button_line_top, gui);
    menu_button_line(gui.ids.side_menu_button)
        .middle_of(gui.ids.side_menu_button)
        .set(gui.ids.side_menu_button_line_middle, gui);
    menu_button_line(gui.ids.side_menu_button)
        .mid_bottom_with_margin_on(gui.ids.side_menu_button, margin)
        .set(gui.ids.side_menu_button_line_bottom, gui);

    // If the side_menu is open, set all the side_menu widgets.
    if side_menu_is_open {
        set_side_menu_widgets(gui);
    }

    // The canvas on which the floorplan will be displayed.
    let background_rect = gui.rect_of(gui.ids.background).unwrap();
    let floorplan_canvas_w = background_rect.w() - side_menu_w;
    let floorplan_canvas_h = background_rect.h();
    widget::Canvas::new()
        .w_h(floorplan_canvas_w, floorplan_canvas_h)
        .h_of(gui.ids.background)
        .color(color::WHITE)
        .align_right_of(gui.ids.background)
        .align_middle_y_of(gui.ids.background)
        .crop_kids()
        .set(gui.ids.floorplan_canvas, gui);

    let floorplan_pixels_per_metre = gui.state.camera.floorplan_pixels_per_metre;
    let metres_from_floorplan_pixels = |px| Metres(px / floorplan_pixels_per_metre);
    let metres_to_floorplan_pixels = |Metres(m)| m * floorplan_pixels_per_metre;

    let floorplan_w_metres = metres_from_floorplan_pixels(gui.images.floorplan.width);
    let floorplan_h_metres = metres_from_floorplan_pixels(gui.images.floorplan.height);

    // The amount which the image must be scaled to fill the floorplan_canvas while preserving
    // aspect ratio.
    let full_scale_w = floorplan_canvas_w / gui.images.floorplan.width;
    let full_scale_h = floorplan_canvas_h / gui.images.floorplan.height;
    let floorplan_w = full_scale_w * gui.images.floorplan.width;
    let floorplan_h = full_scale_h * gui.images.floorplan.height;

    // If the floorplan was scrolled, adjust the camera zoom.
    let total_scroll = gui.widget_input(gui.ids.floorplan)
        .scrolls()
        .fold(0.0, |acc, scroll| acc + scroll.y);
    gui.state.camera.zoom = (gui.state.camera.zoom - total_scroll / 200.0)
        .max(full_scale_w.min(full_scale_h))
        .min(1.0);

    // Move the camera by clicking with the left mouse button and dragging.
    let total_drag = gui.widget_input(gui.ids.floorplan)
        .drags()
        .left()
        .map(|drag| drag.delta_xy)
        .fold([0.0, 0.0], |acc, dt| [acc[0] + dt[0], acc[1] + dt[1]]);
    gui.state.camera.position.x -= gui.state.camera.scalar_to_metres(total_drag[0]);
    gui.state.camera.position.y -= gui.state.camera.scalar_to_metres(total_drag[1]);

    // The part of the image visible from the camera.
    let visible_w_m = gui.state.camera.scalar_to_metres(floorplan_canvas_w);
    let visible_h_m = gui.state.camera.scalar_to_metres(floorplan_canvas_h);

    // Clamp the camera's position so it doesn't go out of bounds.
    let invisible_w_m = floorplan_w_metres - visible_w_m;
    let invisible_h_m = floorplan_h_metres - visible_h_m;
    let half_invisible_w_m = invisible_w_m * 0.5;
    let half_invisible_h_m = invisible_h_m * 0.5;
    let centre_x_m = floorplan_w_metres * 0.5;
    let centre_y_m = floorplan_h_metres * 0.5;
    let min_cam_x_m = centre_x_m - half_invisible_w_m;
    let max_cam_x_m = centre_x_m + half_invisible_w_m;
    let min_cam_y_m = centre_y_m - half_invisible_h_m;
    let max_cam_y_m = centre_y_m + half_invisible_h_m;
    gui.state.camera.position.x = gui.state.camera.position.x.max(min_cam_x_m).min(max_cam_x_m);
    gui.state.camera.position.y = gui.state.camera.position.y.max(min_cam_y_m).min(max_cam_y_m);

    let visible_x = metres_to_floorplan_pixels(gui.state.camera.position.x);
    let visible_y = metres_to_floorplan_pixels(gui.state.camera.position.y);
    let visible_w = metres_to_floorplan_pixels(visible_w_m);
    let visible_h = metres_to_floorplan_pixels(visible_h_m);
    let visible_rect = ui::Rect::from_xy_dim([visible_x, visible_y], [visible_w, visible_h]);

    // If the left mouse button was clicked on the floorplan, deselect the speaker.
    if gui.widget_input(gui.ids.floorplan).clicks().left().next().is_some() {
        gui.state.speaker_editor.selected = None;
    }

    // Display the floorplan.
    widget::Image::new(gui.images.floorplan.id)
        .source_rectangle(visible_rect)
        .w_h(floorplan_w, floorplan_h)
        .middle_of(gui.ids.floorplan_canvas)
        .set(gui.ids.floorplan, gui);

    // Retrieve the absolute xy position of the floorplan as this will be useful for converting
    // absolute GUI values to metres and vice versa.
    let floorplan_xy = gui.rect_of(gui.ids.floorplan).unwrap().xy();

    // Draw the speakers over the floorplan.
    //
    // Display the `gui.state.speaker_editor.speakers` over the floorplan as circles.
    let radius_min_m = gui.state.config.min_speaker_radius_metres;
    let radius_max_m = gui.state.config.max_speaker_radius_metres;
    let radius_min = gui.state.camera.metres_to_scalar(radius_min_m);
    let radius_max = gui.state.camera.metres_to_scalar(radius_max_m);

    fn x_position_metres_to_floorplan (x: Metres, cam: &Camera) -> Scalar {
        cam.metres_to_scalar(x - cam.position.x)
    }
    fn y_position_metres_to_floorplan (y: Metres, cam: &Camera) -> Scalar {
        cam.metres_to_scalar(y - cam.position.y)
    }

    // Convert the given position in metres to a gui Scalar position relative to the middle of the
    // floorplan.
    fn position_metres_to_floorplan(p: Point2<Metres>, cam: &Camera) -> (Scalar, Scalar) {
        let x = x_position_metres_to_floorplan(p.x, cam);
        let y = y_position_metres_to_floorplan(p.y, cam);
        (x, y)
    };

    // Convert the given position in metres to an absolute GUI scalar position.
    let position_metres_to_gui = |p: Point2<Metres>, cam: &Camera| -> (Scalar, Scalar) {
        let (x, y) = position_metres_to_floorplan(p, cam);
        (floorplan_xy[0] + x, floorplan_xy[1] + y)
    };

    // // Convert the given absolute GUI position to a position in metres.
    // let position_gui_to_metres = |p: [Scalar; 2], cam: &Camera| -> Point2<Metres> {
    //     let (floorplan_x, floorplan_y) = (p[0] - floorplan_xy[0], p[1] - floorplan_xy[1]);
    //     let x = cam.scalar_to_metres(floorplan_x);
    //     let y = cam.scalar_to_metres(floorplan_y);
    //     Point2 { x, y }
    // };

    {
        let Gui {
            ref mut ids,
            ref mut state,
            ref mut ui,
            ref channels,
            ref audio_monitor,
            ..
        } = *gui;

        // Ensure there are enough IDs available.
        let num_speakers = state.speaker_editor.speakers.len();
        if ids.floorplan_speakers.len() < num_speakers {
            let id_gen = &mut ui.widget_id_generator();
            ids.floorplan_speakers.resize(num_speakers, id_gen);
        }

        for i in 0..state.speaker_editor.speakers.len() {
            let widget_id = ids.floorplan_speakers[i];
            let speaker_id = state.speaker_editor.speakers[i].id;
            let rms = match audio_monitor.speakers.get(&speaker_id) {
                Some(levels) => levels.rms,
                _ => 0.0,
            };

            let (dragged_x, dragged_y) = ui.widget_input(widget_id)
                .drags()
                .left()
                .fold((0.0, 0.0), |(x, y), drag| (x + drag.delta_xy[0], y + drag.delta_xy[1]));
            let dragged_x_m = state.camera.scalar_to_metres(dragged_x);
            let dragged_y_m = state.camera.scalar_to_metres(dragged_y);

            let position = {
                let SpeakerEditor { ref mut speakers, .. } = state.speaker_editor;
                let p = speakers[i].audio.point;
                let x = p.x + dragged_x_m;
                let y = p.y + dragged_y_m;
                let new_p = Point2 { x, y };
                if p != new_p {
                    speakers[i].audio.point = new_p;
                    let (speaker_id, speaker_clone) = (speakers[i].id, speakers[i].audio.clone());
                    channels.audio.send(move |audio| {
                        audio.insert_speaker(speaker_id, speaker_clone);
                    }).ok();
                }
                new_p
            };

            let (x, y) = position_metres_to_gui(position, &state.camera);

            // Select the speaker if it was pressed.
            if ui.widget_input(widget_id)
                .presses()
                .mouse()
                .left()
                .next()
                .is_some()
            {
                state.speaker_editor.selected = Some(i);
            }

            // Give some tactile colour feedback if the speaker is interacted with.
            let color = if Some(i) == state.speaker_editor.selected { color::BLUE } else { color::DARK_RED };
            let color = match ui.widget_input(widget_id).mouse() {
                Some(mouse) =>
                    if mouse.buttons.left().is_down() { color.clicked() }
                    else { color.highlighted() },
                None => color,
            };

            // Feed the RMS into the speaker's radius.
            let radius = radius_min + (radius_max - radius_min) * rms.powf(0.5) as f64;

            // Display a circle for the speaker.
            widget::Circle::fill(radius)
                .x_y(x, y)
                .parent(ids.floorplan)
                .color(color)
                .set(widget_id, ui);
        }
    }

    // Draw the currently active sounds over the floorplan.
    {
        let Gui { ref ids, ref mut state, ref mut ui, ref channels, audio_monitor, .. } = *gui;

        let current = state.source_editor.preview.current;
        let point = state.source_editor.preview.point;
        let selected = state.source_editor.selected;
        let mut channel_amplitudes = [0.0f32; 16];
        for (&sound_id, active_sound) in &audio_monitor.active_sounds {
            let radians = 0.0;

            // Fill the channel amplitudes.
            for (i, channel) in active_sound.channels.iter().enumerate() {
                channel_amplitudes[i] = channel.rms.powf(0.5); // Emphasise lower amplitudes.
            }

            // If this is the preview sound it should be draggable and stand out.
            let condition = (current, point, selected);
            let (spread, channel_radians, channel_count, position, color) = match condition {
                (Some((_, id)), Some(point), Some(i)) if id == sound_id => {
                    let (spread, channel_radians, channel_count) = {
                        let source = &state.source_editor.sources[i];
                        let spread = source.audio.spread;
                        let channel_radians = source.audio.radians as f64;
                        let channel_count = source.audio.channel_count();
                        (spread, channel_radians, channel_count)
                    };

                    // Determine how far the source preview has been dragged, if at all.
                    let (dragged_x, dragged_y) = ui.widget_input(ids.floorplan_source_preview)
                        .drags()
                        .left()
                        .fold((0.0, 0.0), |(x, y), drag| (x + drag.delta_xy[0], y + drag.delta_xy[1]));
                    let dragged_x_m = state.camera.scalar_to_metres(dragged_x);
                    let dragged_y_m = state.camera.scalar_to_metres(dragged_y);

                    // Determine the resulting position after the drag.
                    let position = {
                        let x = point.x + dragged_x_m;
                        let y = point.y + dragged_y_m;
                        let new_p = Point2 { x, y };
                        if point != new_p {
                            state.source_editor.preview.point = Some(new_p);
                            channels.audio.send(move |audio| {
                                if let Some(sound) = audio.sound_mut(&sound_id) {
                                    sound.point = new_p;
                                }
                            }).ok();
                        }
                        new_p
                    };

                    (spread, channel_radians, channel_count, position, color::LIGHT_BLUE)
                },
                _ => {
                    // Find the source.
                    let source = state.source_editor.sources
                        .iter()
                        .find(|s| s.id == active_sound.source_id)
                        .expect("No source found for active sound");
                    let spread = source.audio.spread;
                    let channel_radians = source.audio.radians as f64;
                    let channel_count = source.audio.channel_count();
                    let position = active_sound.position;
                    (spread, channel_radians, channel_count, position, color::DARK_BLUE)
                },
            };

            let spread = state.camera.metres_to_scalar(spread);
            let side_m = custom_widget::sound::dimension_metres(0.0);
            let side = state.camera.metres_to_scalar(side_m);

            let (x, y) = position_metres_to_gui(position, &state.camera);

            let channel_amps = &channel_amplitudes[..channel_count];
            custom_widget::Sound::new(channel_amps, spread, radians, channel_radians)
                .color(color)
                .x_y(x, y)
                .w_h(side, side)
                .parent(ids.floorplan)
                .set(ids.floorplan_source_preview, ui);
        }
    }

}
