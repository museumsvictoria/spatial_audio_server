use audio;
use config::Config;
use interaction::Interaction;
use metres::Metres;
use nannou;
use nannou::prelude::*;
use nannou::glium;
use nannou::ui;
use nannou::ui::prelude::*;
use osc;
use osc::input::Log as OscInputLog;
use osc::output::Log as OscOutputLog;
use serde_json;
use soundscape;
use std;
use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::ops::{Deref, DerefMut};
use std::sync::mpsc;

use self::installation_editor::InstallationEditor;
use self::source_editor::{SourceEditor, SourcePreview, SourcePreviewMode, StoredSources};
use self::speaker_editor::{Speaker, SpeakerEditor, StoredSpeakers};

mod custom_widget;
pub mod installation_editor;
pub mod interaction_log;
pub mod osc_in_log;
pub mod osc_out_log;
pub mod source_editor;
pub mod speaker_editor;
mod theme;

// The name of the file where the installation OSC mappings are saved.
const INSTALLATIONS_FILE_STEM: &'static str = "installations";

// The name of the file where the speaker layout is saved.
const SPEAKERS_FILE_STEM: &'static str = "speakers";

// The name of the file where the list of sources is stored.
const SOURCES_FILE_STEM: &'static str = "sources";

// The name of the directory where the WAVs are stored.
const AUDIO_DIRECTORY_NAME: &'static str = "audio";

type ActiveSoundMap = HashMap<audio::sound::Id, ActiveSound>;

pub struct Model {
    pub ui: Ui,
    images: Images,
    fonts: Fonts,
    state: State,
    ids: Ids,
    pub channels: Channels,
    sound_id_gen: audio::sound::IdGenerator,
    audio_monitor: AudioMonitor,
    assets: PathBuf,
}

/// A convenience wrapper that borrows the GUI state necessary for instantiating the widgets.
pub struct Gui<'a> {
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

pub struct State {
    // The loaded config file.
    config: Config,
    // The camera over the 2D floorplan.
    camera: Camera,
    // A log of the most recently received OSC messages for testing/debugging/monitoring.
    osc_in_log: Log<OscInputLog>,
    // A log of the most recently sent OSC messages for testing/debugging/monitoring.
    osc_out_log: Log<OscOutputLog>,
    // A log of the most recently received Interactions for testing/debugging/monitoring.
    interaction_log: InteractionLog,
    installation_editor: InstallationEditor,
    speaker_editor: SpeakerEditor,
    source_editor: SourceEditor,
    max_input_channels: usize,
    // Menu states.
    side_menu_is_open: bool,
    osc_in_log_is_open: bool,
    osc_out_log_is_open: bool,
    interaction_log_is_open: bool,
}

fn installations_path(assets: &Path) -> PathBuf {
    assets.join(Path::new(INSTALLATIONS_FILE_STEM)).with_extension("json")
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
        max_input_channels: usize,
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
        let state = State::new(assets, config, &channels, max_input_channels);

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
        for log in channels.osc_in_log_rx.try_iter() {
            state.osc_in_log.push_msg(log);
        }

        // Collect OSC messages for the OSC log.
        for log in channels.osc_out_log_rx.try_iter() {
            state.osc_out_log.push_msg(log);
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
                installation_editor: InstallationEditor {
                    computer_map,
                    ..
                },
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

        // Save the installation address map.
        let installations_json_string = serde_json::to_string_pretty(&computer_map)
            .expect("failed to serialize installation address map");
        safe_file_save(&installations_path(&assets), &installations_json_string)
            .expect("failed to save installations file");

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
        !self.audio_monitor.active_sounds.is_empty()
    }
}

impl State {
    /// Initialise the `State` and send any loaded speakers and sources to the audio and composer
    /// threads.
    pub fn new(
        assets: &Path,
        config: Config,
        channels: &Channels,
        max_input_channels: usize,
    ) -> Self
    {
        // Load the stored isntallation editor state.
        let computer_map = installation_editor::load_computer_map(&installations_path(assets));

        // Send the loaded OSC installation targets to the osc output thread.
        for (&inst, computers) in &computer_map {
            for (&computer, addr) in computers {
                let osc_tx = nannou::osc::sender()
                    .expect("failed to create OSC sender")
                    .connect(&addr.socket)
                    .expect("failed to connect OSC sender");
                let osc_addr = addr.osc_addr.clone();
                let add = osc::output::OscTarget::Add(inst, computer, osc_tx, osc_addr);
                let msg = osc::output::Message::Osc(add);
                channels.osc_out_msg_tx.send(msg)
                    .expect("failed to send loaded OSC target");
            }
        }

        let installation_editor = InstallationEditor {
            is_open: false,
            selected: None,
            computer_map,
        };

        // Load the existing speaker layout configuration if there is one.
        let StoredSpeakers { speakers, next_id } = StoredSpeakers::load(&speakers_path(assets));

        // Send the loaded speakers to the audio thread.
        for speaker in &speakers {
            let (speaker_id, speaker_clone) = (speaker.id, speaker.audio.clone());
            channels.audio_output.send(move |audio| {
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
        let stored_sources = StoredSources::load(&sources_path(assets), &audio_path);
        let StoredSources { sources, next_id } = stored_sources;

        // Send the realtime sources to the input thread.
        for source in &sources {
            if let audio::source::Kind::Realtime(ref realtime) = source.audio.kind {
                let id = source.id;
                let realtime = realtime.clone();
                channels.audio_input.send(move |audio| {
                    audio.sources.insert(id, realtime);
                }).ok();
            }
        }

        // Send the loaded sources to the composer thread.
        for source in &sources {
            let msg = soundscape::Message::InsertSource(source.id, source.audio.clone());
            channels.soundscape_msg_tx.send(msg).expect("soundscape_msg_tx was closed");
        }

        let preview = SourcePreview {
            current: None,
            point: None,
        };

        let source_editor = SourceEditor {
            is_open: false,
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

        let osc_in_log = Log::with_limit(config.osc_input_log_limit);
        let osc_out_log = Log::with_limit(config.osc_output_log_limit);
        let interaction_log = Log::with_limit(config.interaction_log_limit);

        // State that is specific to the GUI itself.
        State {
            config,
            // TODO: Possibly load camera from file.
            camera,
            installation_editor,
            speaker_editor,
            source_editor,
            osc_in_log,
            osc_out_log,
            interaction_log,
            max_input_channels,
            side_menu_is_open: true,
            osc_in_log_is_open: false,
            osc_out_log_is_open: false,
            interaction_log_is_open: false,
        }
    }
}

pub struct Channels {
    pub osc_in_log_rx: mpsc::Receiver<OscInputLog>,
    pub osc_out_log_rx: mpsc::Receiver<OscOutputLog>,
    pub osc_out_msg_tx: mpsc::Sender<osc::output::Message>,
    pub interaction_rx: mpsc::Receiver<Interaction>,
    pub soundscape_msg_tx: mpsc::Sender<soundscape::Message>,
    /// A handle for communicating with the audio input stream.
    pub audio_input: audio::input::Stream,
    /// A handle for communicating with the audio output stream.
    pub audio_output: audio::output::Stream,
    pub audio_monitor_msg_rx: mpsc::Receiver<AudioMonitorMessage>,
}

impl Channels {
    /// Initialise the GUI communication channels.
    pub fn new(
        osc_in_log_rx: mpsc::Receiver<OscInputLog>,
        osc_out_log_rx: mpsc::Receiver<OscOutputLog>,
        osc_out_msg_tx: mpsc::Sender<osc::output::Message>,
        interaction_rx: mpsc::Receiver<Interaction>,
        soundscape_msg_tx: mpsc::Sender<soundscape::Message>,
        audio_input: audio::input::Stream,
        audio_output: audio::output::Stream,
        audio_monitor_msg_rx: mpsc::Receiver<AudioMonitorMessage>,
    ) -> Self
    {
        Channels {
            osc_in_log_rx,
            osc_out_log_rx,
            osc_out_msg_tx,
            interaction_rx,
            soundscape_msg_tx,
            audio_input,
            audio_output,
            audio_monitor_msg_rx,
        }
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

impl Log<OscInputLog> {
    // Format the log in a single string of messages.
    fn format(&self) -> String {
        let mut s = String::new();
        let mut index = self.start_index + self.deque.len();
        for &OscInputLog { ref addr, ref msg } in &self.deque {
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

impl Log<OscOutputLog> {
    // Format the log in a single string of messages.
    fn format(&self) -> String {
        let mut s = String::new();
        let mut index = self.start_index + self.deque.len();
        for &OscOutputLog { addr, ref msg, ref error, .. } in &self.deque {
            let addr_string = format!("{}: [{}] \"{}\"\n", index, addr, msg.addr);
            s.push_str(&addr_string);

            // Arguments.
            if let Some(ref args) = msg.args {
                s.push_str("    [");

                // Format the `Type` argument into a string.
                // TODO: Perhaps this should be provided by nannou?
                fn format_arg(arg: &nannou::osc::Type) -> String {
                    match arg {
                        &nannou::osc::Type::Float(f) => format!("{:.2}", f),
                        &nannou::osc::Type::Int(i) => format!("{}", i),
                        arg => format!("{:?}", arg),
                    }
                }

                let mut args = args.iter();
                if let Some(first) = args.next() {
                    s.push_str(&format!("{}", format_arg(first)));
                }

                for arg in args {
                    s.push_str(&format!(", {}", format_arg(arg)));
                }

                s.push_str("]\n");
            }

            // Error if any.
            if let Some(ref err) = *error {
                let err_string = format!("  error: {}\n", err);
                s.push_str(&err_string);
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
        side_menu_scrollbar,
        // The menu button at the top of the sidebar.
        side_menu_button,
        side_menu_button_line_top,
        side_menu_button_line_middle,
        side_menu_button_line_bottom,
        // OSC input log.
        osc_in_log,
        osc_in_log_text,
        osc_in_log_scrollbar_y,
        osc_in_log_scrollbar_x,
        // OSC output log.
        osc_out_log,
        osc_out_log_text,
        osc_out_log_scrollbar_y,
        osc_out_log_scrollbar_x,
        // Interaction Log.
        interaction_log,
        interaction_log_text,
        interaction_log_scrollbar_y,
        interaction_log_scrollbar_x,
        // Installation Editor.
        installation_editor,
        installation_editor_list,
        installation_editor_selected_canvas,
        installation_editor_computer_canvas,
        installation_editor_computer_text,
        installation_editor_computer_number,
        installation_editor_computer_list,
        installation_editor_osc_canvas,
        installation_editor_osc_text,
        installation_editor_osc_ip_text_box,
        installation_editor_osc_address_text_box,
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
        speaker_editor_selected_installations_canvas,
        speaker_editor_selected_installations_text,
        speaker_editor_selected_installations_ddl,
        speaker_editor_selected_installations_list,
        speaker_editor_selected_installations_remove,
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
        source_editor_selected_realtime_canvas,
        source_editor_selected_realtime_text,
        source_editor_selected_realtime_duration,
        source_editor_selected_realtime_start_channel,
        source_editor_selected_realtime_end_channel,
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
        floorplan_channel_to_speaker_lines[],
    }
}

// Begin building a `CollapsibleArea` for the sidebar.
pub fn collapsible_area(is_open: bool, text: &str, side_menu_id: widget::Id)
    -> widget::CollapsibleArea
{
    widget::CollapsibleArea::new(is_open, text)
        .w_of(side_menu_id)
        .h(ITEM_HEIGHT)
}

// Begin building a basic info text block.
pub fn info_text(text: &str) -> widget::Text {
    widget::Text::new(&text)
        .font_size(SMALL_FONT_SIZE)
        .line_spacing(6.0)
}

pub const ITEM_HEIGHT: Scalar = 30.0;
pub const SMALL_FONT_SIZE: FontSize = 12;
pub const DARK_A: ui::Color = ui::Color::Rgba(0.1, 0.13, 0.15, 1.0);

// Set the widgets in the side menu.
fn set_side_menu_widgets(gui: &mut Gui) {
    // Installation Editor - for editing installation-specific data.
    let last_area_id = installation_editor::set(gui);

    // Speaker Editor - for adding, editing and removing speakers.
    let last_area_id = speaker_editor::set(last_area_id, gui);

    // For adding, changing and removing audio sources.
    let last_area_id = source_editor::set(last_area_id, gui);

    // The log of received OSC messages.
    let last_area_id = osc_in_log::set(last_area_id, gui);

    // The log of received Interactions.
    let last_area_id = interaction_log::set(last_area_id, gui);

    // The log of sent OSC messages.
    let _last_area_id = osc_out_log::set(last_area_id, gui);
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
    const SIDE_MENU_BUTTON_H: ui::Scalar = CLOSED_SIDE_MENU_W;
    let side_menu_is_open = gui.state.side_menu_is_open;
    let side_menu_w = match side_menu_is_open {
        false => CLOSED_SIDE_MENU_W,
        true => OPEN_SIDE_MENU_W,
    };
    let background_rect = gui.rect_of(gui.ids.background).unwrap();
    let side_menu_h = background_rect.h() - SIDE_MENU_BUTTON_H;

    // The classic three line menu button for opening the side_menu.
    for _click in widget::Button::new()
        .w_h(side_menu_w, SIDE_MENU_BUTTON_H)
        .top_left_of(gui.ids.background)
        .color(color::rgb(0.07, 0.08, 0.09))
        .set(gui.ids.side_menu_button, gui)
    {
        gui.state.side_menu_is_open = !side_menu_is_open;
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

    let side_menu_w_minus_scrollbar = match side_menu_is_open {
        false => side_menu_w,
        true => {
            let scrollbar_w = gui.rect_of(gui.ids.side_menu_scrollbar)
                .map(|r| r.w())
                .unwrap_or(0.0);
            side_menu_w - scrollbar_w
        },
    };

    // The canvas on which all side_menu widgets are placed.
    widget::Canvas::new()
        .w_h(side_menu_w_minus_scrollbar, side_menu_h)
        .bottom_left_of(gui.ids.background)
        .scroll_kids_vertically()
        .pad(0.0)
        .color(color::rgb(0.1, 0.13, 0.15))
        .set(gui.ids.side_menu, gui);

    // Draw the three lines using rectangles.
    fn menu_button_line(menu_button: widget::Id) -> widget::Rectangle {
        let line_h = 2.0;
        let line_w = CLOSED_SIDE_MENU_W / 3.0;
        widget::Rectangle::fill([line_w, line_h])
            .color(color::WHITE)
            .graphics_for(menu_button)
    }

    // If the side_menu is open, set all the side_menu widgets.
    if side_menu_is_open {
        set_side_menu_widgets(gui);

        // Set the scrollbar for the side menu.
        widget::Scrollbar::y_axis(gui.ids.side_menu)
            .right_from(gui.ids.side_menu, 0.0)
            .set(gui.ids.side_menu_scrollbar, gui);
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
                    channels.audio_output.send(move |audio| {
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
    let mut speakers_in_proximity = vec![]; // TODO: Move this to where it can be re-used.
    {
        let Gui { ref mut ids, ref mut state, ref mut ui, ref channels, audio_monitor, .. } = *gui;

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

            // TODO: There should be an Id per active sound.
            let sound_widget_id = ids.floorplan_source_preview;

            // If this is the preview sound it should be draggable and stand out.
            let condition = (current, point, selected);
            let (spread_m, channel_radians, channel_count, position, color) = match condition {
                (Some((_, id)), Some(point), Some(i)) if id == sound_id => {
                    let (spread, channel_radians, channel_count) = {
                        let source = &state.source_editor.sources[i];
                        let spread = source.audio.spread;
                        let channel_radians = source.audio.radians as f64;
                        let channel_count = source.audio.channel_count();
                        (spread, channel_radians, channel_count)
                    };

                    // Determine how far the source preview has been dragged, if at all.
                    let (dragged_x, dragged_y) = ui.widget_input(sound_widget_id)
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
                            channels.audio_output.send(move |audio| {
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

            let spread = state.camera.metres_to_scalar(spread_m);
            let side_m = custom_widget::sound::dimension_metres(0.0);
            let side = state.camera.metres_to_scalar(side_m);
            let channel_amps = &channel_amplitudes[..channel_count];

            // Determine the line colour by checking for interactions with the sound.
            let line_color = match ui.widget_input(sound_widget_id).mouse() {
                Some(mouse) =>
                    if mouse.buttons.left().is_down() { color.clicked() }
                    else { color.highlighted() },
                None => color,
            };

            // For each channel in the sound, draw a line to the `closest_speakers` to which it is
            // sending audio.
            let mut line_index = 0;
            for channel in 0..channel_count {
                let rad = channel_radians as f32;
                let channel_point_m =
                    audio::output::channel_point(position, channel, channel_count, spread_m, rad);
                let (ch_x, ch_y) = position_metres_to_gui(channel_point_m, &state.camera);
                let channel_amp = channel_amplitudes[channel];
                let speakers = &state.speaker_editor.speakers;

                // A function for finding all speakers within proximity of a sound channel.
                fn find_speakers_in_proximity(
                    point: &Point2<Metres>,
                    speakers: &[Speaker],
                    // Amp along with the index within the given `Vec`.
                    in_proximity: &mut Vec<(f32, usize)>,
                ) {
                    let dbap_speakers: Vec<_> = speakers.iter()
                        .map(|speaker| {
                            let speaker = &speaker.audio.point;
                            let point_f = Point2 { x: point.x.0, y: point.y.0 };
                            let speaker_f = Point2 { x: speaker.x.0, y: speaker.y.0 };
                            let distance = point_f.distance(speaker_f);
                            // TODO: Weight the speaker depending on its associated installation.
                            let weight = 1.0;
                            audio::dbap::Speaker { distance, weight }
                        })
                        .collect();

                    let gains = audio::dbap::SpeakerGains::new(&dbap_speakers, audio::ROLLOFF_DB);
                    in_proximity.clear();
                    for (i, gain) in gains.enumerate() {
                        if audio::output::speaker_is_in_proximity(point, &speakers[i].audio.point) {
                            in_proximity.push((gain as f32, i));
                        }
                    }
                }

                find_speakers_in_proximity(&channel_point_m, speakers, &mut speakers_in_proximity);
                for &(amp_scaler, speaker_index) in &speakers_in_proximity {
                    const MAX_THICKNESS: Scalar = 16.0;
                    let amp = channel_amp * amp_scaler;
                    let thickness = amp as Scalar * MAX_THICKNESS;
                    let speaker_point_m = speakers[speaker_index].audio.point;
                    let (s_x, s_y) = position_metres_to_gui(speaker_point_m, &state.camera);

                    // Ensure there is a unique `Id` for this line.
                    if ids.floorplan_channel_to_speaker_lines.len() <= line_index {
                        let mut gen = ui.widget_id_generator();
                        ids.floorplan_channel_to_speaker_lines.resize(line_index+1, &mut gen);
                    }

                    let line_id = ids.floorplan_channel_to_speaker_lines[line_index];
                    widget::Line::abs([ch_x, ch_y], [s_x, s_y])
                        .color(line_color.alpha(amp_scaler))
                        .depth(1.0)
                        .thickness(thickness)
                        .parent(ids.floorplan)
                        .set(line_id, ui);

                    line_index += 1;
                }
            }

            let (x, y) = position_metres_to_gui(position, &state.camera);
            custom_widget::Sound::new(channel_amps, spread, radians, channel_radians)
                .color(color)
                .x_y(x, y)
                .w_h(side, side)
                .parent(ids.floorplan)
                .set(sound_widget_id, ui);
        }
    }
}
