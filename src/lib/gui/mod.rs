use audio;
use camera::Camera;
use config::Config;
use fxhash::FxHashMap;
use metres::Metres;
use nannou;
use nannou::prelude::*;
use nannou::glium;
use nannou::ui;
use nannou::ui::prelude::*;
use osc;
use osc::input::Log as OscInputLog;
use osc::output::Log as OscOutputLog;
use project::{self, Project};
use soundscape::Soundscape;
use slug::slugify;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::ops::{Deref, DerefMut};
use std::sync::{mpsc, Arc};
use std::sync::atomic::AtomicUsize;
use time_calc::Ms;
use utils::{self, HumanReadableTime, SEC_MS, MIN_MS, HR_MS};

use self::installation_editor::InstallationEditor;
use self::project_editor::ProjectEditor;
use self::soundscape_editor::SoundscapeEditor;
use self::source_editor::{SourceEditor, SourcePreviewMode};
use self::speaker_editor::SpeakerEditor;

mod custom_widget;
pub mod installation_editor;
pub mod control_log;
pub mod master;
pub mod monitor;
pub mod osc_in_log;
pub mod osc_out_log;
pub mod project_editor;
pub mod source_editor;
pub mod soundscape_editor;
pub mod speaker_editor;
mod theme;

type ActiveSoundMap = FxHashMap<audio::sound::Id, ActiveSound>;

/// The structure of the GUI.
///
/// This is the primary state stored on the main thread.
pub struct Model {
    /// The nannou UI state.
    pub ui: Ui,
    /// The currently selected project.
    pub project: Option<(Project, ProjectState)>,
    /// Whether or not the GUI is currently in CPU-saving mode.
    pub cpu_saving_mode: bool,
    /// All images used within the GUI.
    images: Images,
    /// A unique ID for each widget.
    ids: Ids,
    /// Channels for communication with the various threads running on the audio server.
    channels: Channels,
    /// The runtime state of the model.
    state: State,
    /// A unique ID generator to use when spawning new sounds.
    sound_id_gen: audio::sound::IdGenerator,
    /// The latest received audio state.
    audio_monitor: AudioMonitor,
    /// The path to the assets directory path at the time the App started running.
    assets: PathBuf,
}

/// A convenience wrapper that borrows the GUI state necessary for instantiating widgets.
pub struct Gui<'a> {
    ui: UiCell<'a>,
    cpu_saving_mode: bool,
    images: &'a Images,
    ids: &'a mut Ids,
    state: &'a mut State,
    audio_monitor: &'a mut AudioMonitor,
    channels: &'a Channels,
    sound_id_gen: &'a audio::sound::IdGenerator,
    assets: &'a PathBuf,
}

/// GUI state related to a single project.
#[derive(Default)]
pub struct ProjectState {
    /// Runtime state related to the installation editor GUI panel.
    installation_editor: InstallationEditor,
    /// Runtime state related to the source editor GUI panel.
    soundscape_editor: SoundscapeEditor,
    /// Runtime state related to the speaker editor GUI panel.
    speaker_editor: SpeakerEditor,
    /// Runtime state related to the source editor GUI panel.
    source_editor: SourceEditor,
}

/// State available to the GUI during widget instantiation.
pub struct State {
    /// The number of input and output channels available on the default input and output devices.
    audio_channels: AudioChannels,
    /// A log of the most recently received OSC messages for testing/debugging/monitoring.
    osc_in_log: Log<OscInputLog>,
    /// A log of the most recently sent OSC messages for testing/debugging/monitoring.
    osc_out_log: Log<OscOutputLog>,
    /// A log of the most recently received controls for testing/debugging/monitoring.
    control_log: ControlLog,
    /// State related to the project editor.
    project_editor: ProjectEditor,
    /// Whether or not each of the collapsible areas are open within the sidebar.
    is_open: IsOpen,
}

/// The state of each collapsible area in the sidebar.
struct IsOpen {
    project_editor: bool,
    master: bool,
    installation_editor: bool,
    soundscape_editor: bool,
    speaker_editor: bool,
    source_editor: bool,
    side_menu: bool,
    osc_in_log: bool,
    osc_out_log: bool,
    control_log: bool,
}

/// The number of audio input and output channels available on the input and output devices.
struct AudioChannels {
    input: usize,
    output: usize,
}

/// Channels for communication with the various threads running on the audio server.
pub struct Channels {
    pub frame_count: Arc<AtomicUsize>,
    pub osc_in_log_rx: mpsc::Receiver<OscInputLog>,
    pub osc_out_log_rx: mpsc::Receiver<OscOutputLog>,
    pub osc_out_msg_tx: mpsc::Sender<osc::output::Message>,
    pub control_rx: mpsc::Receiver<osc::input::Control>,
    pub soundscape: Soundscape,
    pub wav_reader: audio::source::wav::reader::Handle,
    pub audio_input: audio::input::Stream,
    pub audio_output: audio::output::Stream,
    pub audio_monitor_msg_rx: mpsc::Receiver<AudioMonitorMessage>,
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

struct Log<T> {
    // Newest to oldest is stored front to back respectively.
    deque: VecDeque<T>,
    // The index of the oldest message currently stored in the deque.
    start_index: usize,
    // The max number of messages stored in the log at one time.
    limit: usize,
}

type ControlLog = Log<osc::input::Control>;

// A structure for monitoring the state of the audio thread for visualisation.
#[derive(Default)]
struct AudioMonitor {
    master_peak: f32,
    pub active_sounds: ActiveSoundMap,
    speakers: FxHashMap<audio::speaker::Id, ChannelLevels>,
}

impl AudioMonitor {
    /// Clears all state and resets the last received master peak volume.
    pub fn clear(&mut self) {
        self.master_peak = 0.0;
        self.active_sounds.clear();
        self.speakers.clear();
    }

    /// Clears all invalid sounds and speakers from the monitor.
    ///
    /// - All `ActiveSound`s that have a `source::Id` that cannot be found in the project are
    /// removed.
    /// - All `Speaker`s that have a `speaker::Id` that cannot be found in the project are removed.
    pub fn clear_invalid(&mut self, project: &Project) {
        self.active_sounds.retain(|_, s| project.sources.contains_key(&s.source_id));
        self.speakers.retain(|id, _| project.speakers.contains_key(id));
    }
}

// The state of an active sound.
struct ActiveSound {
    source_id: audio::source::Id,
    position: audio::sound::Position,
    channels: Vec<ChannelLevels>,
    // The normalised progress through the playback of the sound.
    normalised_progress: Option<f64>,
}

// The detected levels for a single channel.
#[derive(Default)]
struct ChannelLevels {
    rms: f32,
    peak: f32,
}

/// A message sent from the audio thread with some audio levels.
pub enum AudioMonitorMessage {
    Master { peak: f32 },
    ActiveSound(audio::sound::Id, ActiveSoundMessage),
    Speaker(audio::speaker::Id, SpeakerMessage),
}

/// A message related to an active sound.
pub enum ActiveSoundMessage {
    Start {
        normalised_progress: Option<f64>,
        source_id: audio::source::Id,
        position: audio::sound::Position,
        channels: usize,
    },
    Update {
        normalised_progress: Option<f64>,
        source_id: audio::source::Id,
        position: audio::sound::Position,
        channels: usize,
    },
    UpdateChannel {
        index: usize,
        rms: f32,
        peak: f32,
    },
    End {
        sound: audio::output::ActiveSound,
    },
}

/// A message related to a speaker.
#[derive(Debug)]
pub enum SpeakerMessage {
    Add,
    Update { rms: f32, peak: f32 },
    Remove,
}

impl Default for IsOpen {
    fn default() -> Self {
        IsOpen {
            side_menu: true,
            project_editor: false,
            master: false,
            installation_editor: false,
            soundscape_editor: false,
            speaker_editor: false,
            source_editor: false,
            osc_in_log: false,
            osc_out_log: false,
            control_log: false,
        }
    }
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
        audio_input_channels: usize,
        audio_output_channels: usize,
    ) -> Self {

        // Load a Nannou UI.
        let mut ui = app.new_ui(window_id)
            .with_theme(theme::construct())
            .build()
            .expect("failed to build `Ui`");

        // The type containing the unique ID for each widget in the GUI.
        let ids = Ids::new(ui.widget_id_generator());

        // Load and insert the fonts to be used.
        let font_path = fonts_directory(assets).join("NotoSans/NotoSans-Regular.ttf");
        ui.fonts_mut()
            .insert_from_file(&font_path)
            .unwrap_or_else(|err| {
                panic!("failed to load font \"{}\": {}", font_path.display(), err)
            });

        // Load and insert the images to be used.
        let floorplan_path = images_directory(assets).join("floorplan.png");
        let floorplan = insert_image(
            &floorplan_path,
            app.window(window_id).unwrap().inner_glium_display(),
            &mut ui.image_map,
        );
        let images = Images { floorplan };

        // Initialise the GUI state.
        let input = audio_input_channels;
        let output = audio_output_channels;
        let audio_channels = AudioChannels { input, output };

        // If there's a default project, attempt to load it.
        let project = Project::load_from_slug(
            &assets,
            &config.selected_project_slug,
            &config.project_default,
        );
        let (project_tuple, state) = if let Some(project) = project {
            project.reset_and_sync_all_threads(&channels);
            let project_state = Default::default();
            let mut state = State::new(&project.config, audio_channels);
            state.project_editor.text_box_name = project.name.clone();
            let project_tuple = Some((project, project_state));
            (project_tuple, state)
        } else {
            let state = State::new(&config.project_default, audio_channels);
            (None, state)
        };

        // Initialise the audio monitor.
        let audio_monitor = Default::default();

        // Whether or not CPU saving mode is enabled.
        let cpu_saving_mode = config.cpu_saving_mode;

        // Notify audio output thread.
        channels
            .audio_output
            .send(move |audio| audio.cpu_saving_enabled = cpu_saving_mode)
            .expect("failed to update cpu saving mode on audio output thread");

        Model {
            ui,
            cpu_saving_mode,
            images,
            state,
            ids,
            channels,
            project: project_tuple,
            sound_id_gen,
            assets: assets.into(),
            audio_monitor,
        }
    }

    /// Update the GUI model.
    ///
    /// - Collect pending OSC and control messages for the logs.
    /// - Instantiate the Ui's widgets.
    pub fn update(&mut self, default_project_config: &project::Config) {
        let Model {
            ref mut ui,
            ref mut ids,
            ref mut project,
            ref mut state,
            ref mut audio_monitor,
            ref mut cpu_saving_mode,
            ref images,
            ref channels,
            ref sound_id_gen,
            ref assets,
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

        // Handle control messages.
        for control in channels.control_rx.try_iter() {
            match &control {
                &osc::input::Control::MasterVolume(osc::input::MasterVolume(volume)) => {
                    // Update local copy.
                    if let Some((ref mut project, _)) = *project {
                        project.master.volume = volume;
                    }

                    // Update the audio output copy.
                    channels
                        .audio_output
                        .send(move |audio| audio.master_volume = volume)
                        .expect("failed to send updated master volume to audio output thread");
                },

                &osc::input::Control::SourceVolume(ref source_volume) => {
                    let osc::input::SourceVolume { ref name, volume } = *source_volume;

                    let project = match *project {
                        None => continue,
                        Some((ref mut proj, _)) => proj,
                    };

                    // Update local copy.
                    let id = match project
                        .state
                        .sources
                        .iter_mut()
                        .find(|&(_, ref s)| &s.name[..] == name)
                    {
                        None => continue,
                        Some((&id, ref mut source)) => {
                            source.volume = volume;
                            id
                        },
                    };

                    // Update the soundscape copy.
                    channels
                        .soundscape
                        .send(move |soundscape| {
                            soundscape.update_source(&id, |source| source.volume = volume);
                        })
                        .expect("failed to send updated source volume to soundscape thread");

                    // Update the audio output copies.
                    channels
                        .audio_output
                        .send(move |audio| {
                            audio.update_sounds_with_source(&id, move |_, sound| {
                                sound.volume = volume;
                            });
                        })
                        .expect("failed to send updated source volume to audio output thread");
                }

                &osc::input::Control::PlaySoundscape => {
                    channels
                        .soundscape
                        .play()
                        .expect("failed to send `Play` message to soundscape thread");
                }

                &osc::input::Control::PauseSoundscape => {
                    channels
                        .soundscape
                        .pause()
                        .expect("failed to send `Pause` message to soundscape thread");
                }
            }

            // Log the message.
            state.control_log.push_msg(control);
        }

        // Update the map of active sounds.
        for msg in channels.audio_monitor_msg_rx.try_iter() {
            match msg {
                AudioMonitorMessage::Master { peak } => {
                    audio_monitor.master_peak = peak;
                },
                AudioMonitorMessage::ActiveSound(id, msg) => match msg {
                    ActiveSoundMessage::Start {
                        source_id,
                        position,
                        channels,
                        normalised_progress,
                    } => {
                        let active_sound = ActiveSound::new(
                            source_id,
                            position,
                            channels,
                            normalised_progress,
                        );
                        audio_monitor.active_sounds.insert(id, active_sound);
                    }
                    ActiveSoundMessage::Update {
                        source_id,
                        position,
                        channels,
                        normalised_progress,
                    } => {
                        let active_sound = audio_monitor
                            .active_sounds
                            .entry(id)
                            .or_insert_with(|| {
                                ActiveSound::new(source_id, position, channels, normalised_progress)
                            });
                        active_sound.position = position;
                        active_sound.normalised_progress = normalised_progress;
                    }
                    ActiveSoundMessage::UpdateChannel { index, rms, peak } => {
                        if let Some(active_sound) = audio_monitor.active_sounds.get_mut(&id) {
                            let mut channel = &mut active_sound.channels[index];
                            channel.rms = rms;
                            channel.peak = peak;
                        }
                    }
                    ActiveSoundMessage::End { sound: _sound } => {
                        audio_monitor.active_sounds.remove(&id);

                        // If the Id of the sound being removed matches the current preview, remove
                        // it.
                        if let Some((_, ref mut project_state)) = *project {
                            match project_state.source_editor.preview.current {
                                Some((SourcePreviewMode::OneShot, s_id)) if id == s_id => {
                                    project_state.source_editor.preview.current = None;
                                }
                                _ => (),
                            }
                        }
                    }
                },
                AudioMonitorMessage::Speaker(id, msg) => match msg {
                    SpeakerMessage::Add => {
                        let speaker = ChannelLevels::default();
                        audio_monitor.speakers.insert(id, speaker);
                    }
                    SpeakerMessage::Update { rms, peak } => {
                        let speaker = ChannelLevels { rms, peak };
                        audio_monitor.speakers.insert(id, speaker);
                    }
                    SpeakerMessage::Remove => {
                        audio_monitor.speakers.remove(&id);
                    }
                },
            }
        }

        // Check that all active sounds are still valid in case the GUI switched the project.
        match *project {
            Some((ref mut project, _)) => audio_monitor.clear_invalid(project),
            None => audio_monitor.clear(),
        }

        // Set the widgets.
        let ui = ui.set_widgets();

        // Check for `Ctrl+S` or `Cmd+S` for saving, or `Ctrl+Space` for cpu saving mode.
        for event in ui.global_input().events().ui() {
            if let ui::event::Ui::Press(_, press) = *event {
                match press.button {
                    ui::event::Button::Keyboard(ui::input::Key::S) => {
                        let save_mod =
                            press.modifiers.contains(ui::input::keyboard::ModifierKey::CTRL)
                            || press.modifiers.contains(ui::input::keyboard::ModifierKey::GUI);
                        if save_mod {
                            if let Some((ref project, _)) = *project {
                                project.save(assets).expect("failed to save project on keyboard shortcut");
                            }
                        }
                    }

                    ui::event::Button::Keyboard(ui::input::Key::Space) => {
                        if press.modifiers.contains(ui::input::keyboard::ModifierKey::CTRL) {
                            *cpu_saving_mode = !*cpu_saving_mode;

                            // Notify audio output thread.
                            let cpu_saving_mode = *cpu_saving_mode;
                            channels
                                .audio_output
                                .send(move |audio| audio.cpu_saving_enabled = cpu_saving_mode)
                                .expect("failed to update cpu saving mode on audio output thread");
                        }
                    }

                    _ => (),
                }
            }
        }

        let mut gui = Gui {
            ui,
            cpu_saving_mode: *cpu_saving_mode,
            ids,
            images,
            state,
            channels,
            sound_id_gen,
            audio_monitor,
            assets,
        };
        set_widgets(&mut gui, project, default_project_config);
    }

    /// Whether or not the GUI currently contains representations of active sounds.
    ///
    /// This is used at the top-level to determine what application loop mode to use.
    ///
    /// NOTE: This was used to determine whether or not the GUI need be re-instantiated on an
    /// update method. However, this seemed to cause some GUI visualisation issues on macos
    /// where the GUI wouldn't show on startup. This has since been fixed and macos now uses a
    /// waiting event loop, however now it is awakened when necessary via the `gui::monitor`
    /// thread and as a result this method remains unused. It is left here as it might become
    /// useful in the future.
    #[allow(dead_code)]
    pub fn is_animating(&self) -> bool {
        !self.audio_monitor.active_sounds.is_empty()
    }

    /// If a project is currently selected, this returns its directory path slug.
    pub fn selected_project_slug(&self) -> Option<String> {
        self.project.as_ref().map(|&(ref project, _)| slugify(&project.name))
    }
}

impl State {
    /// Initialise the `State` and send any loaded speakers and sources to the audio and composer
    /// threads.
    fn new(config: &project::Config, audio_channels: AudioChannels) -> Self {
        let osc_in_log = Log::with_limit(config.osc_input_log_limit);
        let osc_out_log = Log::with_limit(config.osc_output_log_limit);
        let control_log = Log::with_limit(config.control_log_limit);
        let is_open = Default::default();
        let project_editor = ProjectEditor::default();
        State {
            osc_in_log,
            osc_out_log,
            control_log,
            audio_channels,
            project_editor,
            is_open,
        }
    }
}

impl Channels {
    /// Initialise the GUI communication channels.
    pub fn new(
        frame_count: Arc<AtomicUsize>,
        osc_in_log_rx: mpsc::Receiver<OscInputLog>,
        osc_out_log_rx: mpsc::Receiver<OscOutputLog>,
        osc_out_msg_tx: mpsc::Sender<osc::output::Message>,
        control_rx: mpsc::Receiver<osc::input::Control>,
        soundscape: Soundscape,
        wav_reader: audio::source::wav::reader::Handle,
        audio_input: audio::input::Stream,
        audio_output: audio::output::Stream,
        audio_monitor_msg_rx: mpsc::Receiver<AudioMonitorMessage>,
    ) -> Self {
        Channels {
            frame_count,
            osc_in_log_rx,
            osc_out_log_rx,
            osc_out_msg_tx,
            control_rx,
            soundscape,
            wav_reader,
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
        for &OscOutputLog {
            addr,
            ref msg,
            ref error,
            ..
        } in &self.deque
        {
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

impl ControlLog {
    // Format the log in a single string of messages.
    fn format(&self) -> String {
        let mut s = String::new();
        let mut index = self.start_index + self.deque.len();
        for control in &self.deque {
            let line = format!("{}: {:?}\n", index, control);
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

impl ActiveSound {
    fn new(
        source_id: audio::source::Id,
        pos: audio::sound::Position,
        channels: usize,
        normalised_progress: Option<f64>,
    ) -> Self {
        ActiveSound {
            source_id,
            position: pos,
            channels: (0..channels).map(|_| ChannelLevels::default()).collect(),
            normalised_progress,
        }
    }
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
    let rgba_image = nannou::image::open(&path)
        .unwrap_or_else(|err| panic!("failed to load image \"{}\": {}", path.display(), err))
        .to_rgba();
    let (w, h) = rgba_image.dimensions();
    let raw_image =
        glium::texture::RawImage2d::from_raw_rgba_reversed(&rgba_image.into_raw(), (w, h));
    let texture = glium::texture::Texture2d::new(display, raw_image)
        .expect("failed to create texture for imaage");
    ((w as Scalar, h as Scalar), texture)
}

/// Insert the image at the given path into the given `ImageMap`.
///
/// Return its Id and Dimensions in the form of an `Image`.
fn insert_image(path: &Path, display: &glium::Display, image_map: &mut ui::Texture2dMap) -> Image {
    let ((width, height), texture) = load_image(path, display);
    let id = image_map.insert(texture);
    let image = Image { id, width, height };
    image
}

// A unique ID for each widget in the GUI.
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
        // Project settings.
        project_editor,
        project_editor_add,
        project_editor_name,
        project_editor_none,
        project_editor_list,
        project_editor_copy,
        project_editor_save,
        project_editor_remove,
        // Master control settings.
        master,
        master_peak_meter,
        master_volume,
        master_realtime_source_latency,
        master_dbap_rolloff,
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
        // Control Log.
        control_log,
        control_log_text,
        control_log_scrollbar_y,
        control_log_scrollbar_x,
        // Installation Editor.
        installation_editor,
        installation_editor_none,
        installation_editor_list,
        installation_editor_add,
        installation_editor_remove,
        installation_editor_name,
        installation_editor_selected_canvas,
        installation_editor_computer_canvas,
        installation_editor_computer_text,
        installation_editor_computer_number,
        installation_editor_computer_list,
        installation_editor_osc_canvas,
        installation_editor_osc_text,
        installation_editor_osc_ip_text_box,
        installation_editor_osc_address_text_box,
        installation_editor_soundscape_canvas,
        installation_editor_soundscape_text,
        installation_editor_soundscape_simultaneous_sounds_slider,
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
        soundscape_editor,
        soundscape_editor_is_playing,
        soundscape_editor_group_canvas,
        soundscape_editor_group_text,
        soundscape_editor_group_add,
        soundscape_editor_group_none,
        soundscape_editor_group_list,
        soundscape_editor_group_remove,
        soundscape_editor_selected_canvas,
        soundscape_editor_selected_text,
        soundscape_editor_selected_name,
        soundscape_editor_occurrence_rate_text,
        soundscape_editor_occurrence_rate_slider,
        soundscape_editor_simultaneous_sounds_text,
        soundscape_editor_simultaneous_sounds_slider,
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
        source_editor_selected_installations_canvas,
        source_editor_selected_installations_text,
        source_editor_selected_installations_ddl,
        source_editor_selected_installations_list,
        source_editor_selected_installations_remove,
        source_editor_selected_soundscape_canvas,
        source_editor_selected_soundscape_title,
        source_editor_selected_soundscape_occurrence_rate_text,
        source_editor_selected_soundscape_occurrence_rate_slider,
        source_editor_selected_soundscape_simultaneous_sounds_text,
        source_editor_selected_soundscape_simultaneous_sounds_slider,
        source_editor_selected_soundscape_playback_duration_text,
        source_editor_selected_soundscape_playback_duration_slider,
        source_editor_selected_soundscape_attack_duration_text,
        source_editor_selected_soundscape_attack_duration_slider,
        source_editor_selected_soundscape_release_duration_text,
        source_editor_selected_soundscape_release_duration_slider,
        source_editor_selected_soundscape_groups_text,
        source_editor_selected_soundscape_groups_list,
        source_editor_selected_soundscape_movement_text,
        source_editor_selected_soundscape_movement_mode_list,
        source_editor_selected_soundscape_movement_generative_list,
        source_editor_selected_soundscape_movement_fixed_point,
        source_editor_selected_soundscape_movement_agent_max_speed_text,
        source_editor_selected_soundscape_movement_agent_max_speed_slider,
        source_editor_selected_soundscape_movement_agent_max_force_text,
        source_editor_selected_soundscape_movement_agent_max_force_slider,
        source_editor_selected_soundscape_movement_agent_max_rotation_text,
        source_editor_selected_soundscape_movement_agent_max_rotation_slider,
        source_editor_selected_soundscape_movement_agent_directional,
        source_editor_selected_soundscape_movement_ngon_speed_text,
        source_editor_selected_soundscape_movement_ngon_speed_slider,
        source_editor_selected_soundscape_movement_ngon_vertices_text,
        source_editor_selected_soundscape_movement_ngon_vertices_slider,
        source_editor_selected_soundscape_movement_ngon_step_text,
        source_editor_selected_soundscape_movement_ngon_step_slider,
        source_editor_selected_soundscape_movement_ngon_dimensions_text,
        source_editor_selected_soundscape_movement_ngon_width_slider,
        source_editor_selected_soundscape_movement_ngon_height_slider,
        source_editor_selected_soundscape_movement_ngon_radians_text,
        source_editor_selected_soundscape_movement_ngon_radians_slider,
        source_editor_selected_wav_canvas,
        source_editor_selected_wav_text,
        source_editor_selected_wav_data,
        source_editor_selected_wav_loop_toggle,
        source_editor_selected_wav_playback_text,
        source_editor_selected_wav_playback_list,
        source_editor_selected_realtime_canvas,
        source_editor_selected_realtime_text,
        source_editor_selected_realtime_duration,
        source_editor_selected_realtime_start_channel,
        source_editor_selected_realtime_end_channel,
        source_editor_selected_common_canvas,
        source_editor_selected_volume_text,
        source_editor_selected_volume_slider,
        source_editor_selected_solo,
        source_editor_selected_mute,
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
        floorplan_project_name,
        floorplan_speakers[],
        floorplan_speaker_labels[],
        floorplan_sounds[],
        floorplan_channel_to_speaker_lines[],

        // Text drawn in the CPU-saving mode.
        cpu_saving_mode,
        cpu_saving_mode_shortcut,
    }
}

// Begin building a `CollapsibleArea` for the sidebar.
pub fn collapsible_area(
    is_open: bool,
    text: &str,
    side_menu_id: widget::Id,
) -> widget::CollapsibleArea {
    widget::CollapsibleArea::new(is_open, text)
        .w_of(side_menu_id)
        .h(ITEM_HEIGHT)
        .parent(side_menu_id)
}

// Begin building a basic info text block.
pub fn info_text(text: &str) -> widget::Text {
    widget::Text::new(&text)
        .font_size(SMALL_FONT_SIZE)
        .line_spacing(6.0)
}

// A function to simplify the crateion of a label for a hz slider.
pub fn hz_label(hz: f64) -> String {
    match utils::human_readable_hz(hz) {
        (HumanReadableTime::Ms, times_per_ms) => {
            format!("{} per millisecond", times_per_ms.round())
        },
        (HumanReadableTime::Secs, hz) => {
            format!("{} per second", hz.round())
        },
        (HumanReadableTime::Mins, times_per_min) => {
            format!("{} per minute", times_per_min.round())
        },
        (HumanReadableTime::Hrs, times_per_hr) => {
            format!("{} per hour", times_per_hr.round())
        },
        (HumanReadableTime::Days, times_per_day) => {
            format!("{} per day", times_per_day.round())
        },
    }
}

// A function to simplify the creation of a label for a duration slider.
pub fn duration_label(ms: &Ms) -> String {
    // Playback duration.
    match utils::human_readable_ms(ms) {
        (HumanReadableTime::Ms, ms) => {
            format!("{} ms", ms)
        },
        (HumanReadableTime::Secs, secs) => {
            let secs = secs.floor();
            let ms = ms.ms() - (secs * SEC_MS);
            format!("{} secs {} ms", secs, ms)
        },
        (HumanReadableTime::Mins, mins) => {
            let mins = mins.floor();
            let secs = (ms.ms() - (mins * MIN_MS)) / SEC_MS;
            format!("{} mins {} secs", mins, secs)
        },
        (HumanReadableTime::Hrs, hrs) => {
            let hrs = hrs.floor();
            let mins = (ms.ms() - (hrs * HR_MS)) / MIN_MS;
            format!("{} hrs {} mins", hrs, mins)
        },
        (HumanReadableTime::Days, days) => {
            format!("{} days", days)
        },
    }
}

pub const TEXT_PAD: Scalar = 20.0;
pub const ITEM_HEIGHT: Scalar = 30.0;
pub const SMALL_FONT_SIZE: FontSize = 12;
pub const DARK_A: ui::Color = ui::Color::Rgba(0.1, 0.13, 0.15, 1.0);

// Set the widgets in the side menu.
fn set_side_menu_widgets(
    gui: &mut Gui,
    project: &mut Option<(Project, ProjectState)>,
    default_project_config: &project::Config,
) {
    // Project Editor - for adding, saving and removing projects.
    let mut last_area_id = project_editor::set(gui, project, default_project_config);

    // Many of the sidebar widgets can only be displayed if a project is selected.
    if let Some((ref mut project, ref mut project_state)) = *project {
        // Installation Editor - for editing installation-specific data.
        last_area_id = master::set(last_area_id, gui, project);

        // Installation Editor - for editing installation-specific data.
        last_area_id = installation_editor::set(last_area_id, gui, project, project_state);

        // Speaker Editor - for adding, editing and removing speakers.
        last_area_id = speaker_editor::set(last_area_id, gui, project, project_state);

        // Soundscape Editor - for playing/pausing and adding, editing and removing groups.
        last_area_id = soundscape_editor::set(last_area_id, gui, project, project_state);

        // For adding, changing and removing audio sources.
        last_area_id = source_editor::set(last_area_id, gui, project, project_state);

        // The log of received controls.
        last_area_id = control_log::set(last_area_id, gui, project);

        // The log of received OSC messages.
        last_area_id = osc_in_log::set(last_area_id, gui, project);
    }

    // The log of sent OSC messages.
    osc_out_log::set(last_area_id, gui);
}

// Update all widgets in the GUI with the given state.
fn set_widgets(
    gui: &mut Gui,
    project: &mut Option<(Project, ProjectState)>,
    default_project_config: &project::Config,
) {
    let background_color = color::WHITE;

    // The background for the main `UI` window.
    widget::Canvas::new()
        .color(background_color)
        .pad(0.0)
        .parent(gui.window)
        .middle_of(gui.window)
        .wh_of(gui.window)
        .set(gui.ids.background, gui);

    // If the GUI is in CPU saving mode, just draw the text to show how to get back to live mode.
    if gui.cpu_saving_mode {
        widget::Text::new("CPU Saving Mode")
            .middle_of(gui.ids.background)
            .font_size(64)
            .color(color::DARK_CHARCOAL)
            .set(gui.ids.cpu_saving_mode, gui);

        widget::Text::new("Press `Ctrl + Space` to switch back to live mode.")
            .down(24.0)
            .align_middle_x_of(gui.ids.cpu_saving_mode)
            .font_size(24)
            .color(color::DARK_CHARCOAL)
            .set(gui.ids.cpu_saving_mode_shortcut, gui);

        return;
    }

    // A thin menu bar on the left.
    //
    // The menu bar is collapsed by default, and shows three lines at the top.
    // Pressing these three lines opens the menu, revealing a list of options.
    const CLOSED_SIDE_MENU_W: ui::Scalar = 40.0;
    const OPEN_SIDE_MENU_W: ui::Scalar = 300.0;
    const SIDE_MENU_BUTTON_H: ui::Scalar = CLOSED_SIDE_MENU_W;
    let side_menu_is_open = gui.state.is_open.side_menu;
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
        gui.state.is_open.side_menu = !side_menu_is_open;
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
        }
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
        set_side_menu_widgets(gui, project, default_project_config);

        // Set the scrollbar for the side menu.
        widget::Scrollbar::y_axis(gui.ids.side_menu)
            .right_from(gui.ids.side_menu, 0.0)
            .set(gui.ids.side_menu_scrollbar, gui);
    }

    // Only continue if a project is selected.
    let &mut (ref mut project, ref mut project_state) = match *project {
        Some(ref mut p) => p,
        None => {
            // TODO: A text widget saying to add a project.
            return;
        }
    };

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

    let floorplan_pixels_per_metre = project.config.floorplan_pixels_per_metre;
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
    project.state.camera.zoom = (project.state.camera.zoom - total_scroll / 200.0)
        .max(full_scale_w.min(full_scale_h))
        .min(1.0);

    // Move the camera by clicking with the left mouse button and dragging.
    let total_drag = gui.widget_input(gui.ids.floorplan)
        .drags()
        .left()
        .map(|drag| drag.delta_xy)
        .fold([0.0, 0.0], |acc, dt| [acc[0] + dt[0], acc[1] + dt[1]]);
    project.state.camera.position.x -= project.state.camera.scalar_to_metres(total_drag[0]);
    project.state.camera.position.y -= project.state.camera.scalar_to_metres(total_drag[1]);

    // The part of the image visible from the camera.
    let visible_w_m = project.state.camera.scalar_to_metres(floorplan_canvas_w);
    let visible_h_m = project.state.camera.scalar_to_metres(floorplan_canvas_h);

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
    project.state.camera.position.x = project.state
        .camera
        .position
        .x
        .max(min_cam_x_m)
        .min(max_cam_x_m);
    project.state.camera.position.y = project.state
        .camera
        .position
        .y
        .max(min_cam_y_m)
        .min(max_cam_y_m);

    let visible_x = metres_to_floorplan_pixels(project.state.camera.position.x);
    let visible_y = metres_to_floorplan_pixels(project.state.camera.position.y);
    let visible_w = metres_to_floorplan_pixels(visible_w_m);
    let visible_h = metres_to_floorplan_pixels(visible_h_m);
    let visible_rect = ui::Rect::from_xy_dim([visible_x, visible_y], [visible_w, visible_h]);

    // If the left mouse button was clicked on the floorplan, deselect the speaker.
    if gui.widget_input(gui.ids.floorplan)
        .clicks()
        .left()
        .next()
        .is_some()
    {
        project_state.speaker_editor.selected = None;
    }

    // Display the floorplan.
    widget::Image::new(gui.images.floorplan.id)
        .source_rectangle(visible_rect)
        .w_h(floorplan_w, floorplan_h)
        .middle_of(gui.ids.floorplan_canvas)
        .set(gui.ids.floorplan, gui);

    // The name of the project if one is selected.
    widget::Text::new(&project.name)
        .top_left_with_margin_on(gui.ids.floorplan, 20.0)
        .color(ui::color::BLACK)
        .set(gui.ids.floorplan_project_name, gui);

    // Retrieve the absolute xy position of the floorplan as this will be useful for converting
    // absolute GUI values to metres and vice versa.
    let floorplan_xy = gui.rect_of(gui.ids.floorplan).unwrap().xy();

    // Draw the speakers over the floorplan.
    //
    // Display the `gui.state.speaker_editor.speakers` over the floorplan as circles.
    let radius_min_m = project.config.min_speaker_radius_metres;
    let radius_max_m = project.config.max_speaker_radius_metres;
    let radius_min = project.state.camera.metres_to_scalar(radius_min_m);
    let radius_max = project.state.camera.metres_to_scalar(radius_max_m);

    fn x_position_metres_to_floorplan(x: Metres, cam: &Camera) -> Scalar {
        cam.metres_to_scalar(x - cam.position.x)
    }
    fn y_position_metres_to_floorplan(y: Metres, cam: &Camera) -> Scalar {
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

        let Project {
            state: project::State {
                ref camera,
                ref mut speakers,
                ..
            },
            ..
        } = *project;

        // Ensure there are enough IDs available.
        let num_speakers = speakers.len();
        if ids.floorplan_speakers.len() < num_speakers {
            let id_gen = &mut ui.widget_id_generator();
            ids.floorplan_speakers.resize(num_speakers, id_gen);
        }
        if ids.floorplan_speaker_labels.len() < num_speakers {
            let id_gen = &mut ui.widget_id_generator();
            ids.floorplan_speaker_labels.resize(num_speakers, id_gen);
        }

        let sorted_speakers = speaker_editor::sorted_speakers_vec(speakers);
        for (i, speaker_id) in sorted_speakers.into_iter().enumerate() {
            let speaker = speakers.get_mut(&speaker_id).unwrap();
            let widget_id = ids.floorplan_speakers[i];
            let label_widget_id = ids.floorplan_speaker_labels[i];
            let channel = speaker.audio.channel;
            let rms = match audio_monitor.speakers.get(&speaker_id) {
                Some(levels) => levels.rms,
                _ => 0.0,
            };

            let (dragged_x, dragged_y) = ui.widget_input(widget_id)
                .drags()
                .left()
                .fold((0.0, 0.0), |(x, y), drag| {
                    (x + drag.delta_xy[0], y + drag.delta_xy[1])
                });
            let dragged_x_m = camera.scalar_to_metres(dragged_x);
            let dragged_y_m = camera.scalar_to_metres(dragged_y);

            let position = {
                let p = speaker.audio.point;
                let x = p.x + dragged_x_m;
                let y = p.y + dragged_y_m;
                let new_p = Point2 { x, y };
                if p != new_p {
                    // Update the local copy.
                    speaker.audio.point = new_p;

                    // Update the audio copy.
                    let speaker_clone = speaker.audio.clone();
                    channels
                        .audio_output
                        .send(move |audio| {
                            audio.insert_speaker(speaker_id, speaker_clone);
                        })
                        .expect("failed to send updated speaker to audio output thread");

                    // Update the soundscape copy.
                    channels
                        .soundscape
                        .send(move |soundscape| {
                            soundscape.update_speaker(&speaker_id, |s| s.point = new_p);
                        })
                        .expect("failed to send speaker update to soundscape thread");
                }
                new_p
            };

            let (x, y) = position_metres_to_gui(position, camera);

            // Select the speaker if it was pressed.
            if ui.widget_input(widget_id)
                .presses()
                .mouse()
                .left()
                .next()
                .is_some()
            {
                project_state.speaker_editor.selected = Some(i);
            }

            // Give some tactile colour feedback if the speaker is interacted with.
            let color = if Some(i) == project_state.speaker_editor.selected {
                color::BLUE
            } else {
                if channel < state.audio_channels.output {
                    color::DARK_RED
                } else {
                    color::DARK_RED.with_luminance(0.15)
                }
            };
            let color = match ui.widget_input(widget_id).mouse() {
                Some(mouse) => if mouse.buttons.left().is_down() {
                    color.clicked()
                } else {
                    color.highlighted()
                },
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

            // Write the channel number on the speaker.
            let label = format!("{}", channel + 1);
            let font_size = (radius * 0.75) as ui::FontSize;
            widget::Text::new(&label)
                .x_y(x, y)
                .font_size(font_size)
                .graphics_for(widget_id)
                .set(label_widget_id, ui);
        }
    }

    // Draw the currently active sounds over the floorplan.
    let mut speakers_in_proximity = vec![]; // TODO: Move this to where it can be re-used.
    {
        let Gui {
            ref mut ids,
            ref mut state,
            ref mut ui,
            ref channels,
            ref audio_monitor,
            ..
        } = *gui;

        let current = project_state.source_editor.preview.current;
        let point = project_state.source_editor.preview.point;
        let selected = project_state.source_editor.selected;
        let mut channel_amplitudes = [0.0f32; 16];
        for (i, (&sound_id, active_sound)) in audio_monitor.active_sounds.iter().enumerate() {
            // Fill the channel amplitudes.
            for (i, channel) in active_sound.channels.iter().enumerate() {
                channel_amplitudes[i] = channel.rms.powf(0.5); // Emphasise lower amplitudes.
            }

            // TODO: There should be an Id per active sound.
            if ids.floorplan_sounds.len() <= i {
                ids.floorplan_sounds.resize(i + 1, &mut ui.widget_id_generator());
            }
            let sound_widget_id = ids.floorplan_sounds[i];

            // If this is the preview sound it should be draggable and stand out.
            let condition = (current, point, selected);
            let (spread_m, channel_radians, channel_count, position, color) = match condition {
                (Some((_, id)), Some(point), Some(selected_id)) if id == sound_id => {
                    let (spread, channel_radians, channel_count) = {
                        let source = &project.sources[&selected_id];
                        let spread = source.audio.spread;
                        let channel_radians = source.audio.channel_radians;
                        let channel_count = source.audio.channel_count();
                        (spread, channel_radians, channel_count)
                    };

                    // Determine how far the source preview has been dragged, if at all.
                    let (dragged_x, dragged_y) = ui.widget_input(sound_widget_id)
                        .drags()
                        .left()
                        .fold((0.0, 0.0), |(x, y), drag| {
                            (x + drag.delta_xy[0], y + drag.delta_xy[1])
                        });
                    let dragged_x_m = project.state.camera.scalar_to_metres(dragged_x);
                    let dragged_y_m = project.state.camera.scalar_to_metres(dragged_y);

                    // Determine the resulting position after the drag.
                    let position = {
                        let x = point.x + dragged_x_m;
                        let y = point.y + dragged_y_m;
                        let new_p = Point2 { x, y };
                        if point != new_p {
                            // Update the local copy.
                            project_state.source_editor.preview.point = Some(new_p);

                            // Update the output audio thread.
                            channels
                                .audio_output
                                .send(move |audio| {
                                    audio.update_sound(&sound_id, move |s| {
                                        s.position.point = new_p;
                                    });
                                })
                                .expect("failed to send sound position to audio output thread");
                        }

                        audio::sound::Position {
                            point: new_p,
                            radians: 0.0,
                        }
                    };

                    (
                        spread,
                        channel_radians,
                        channel_count,
                        position,
                        color::LIGHT_BLUE,
                    )
                }
                _ => {
                    // Find the source.
                    let (&id, source) = project
                        .sources
                        .iter()
                        .find(|&(&id, _)| id == active_sound.source_id)
                        .expect("No source found for active sound");
                    let spread = source.audio.spread;
                    let channel_radians = source.audio.channel_radians;
                    let channel_count = source.audio.channel_count();
                    let position = active_sound.position;
                    let soloed = &project.state.sources.soloed;
                    let mut color = if source.audio.muted || (!soloed.is_empty() && !soloed.contains(&id)) {
                        color::LIGHT_CHARCOAL
                    } else if soloed.contains(&id) {
                        color::DARK_YELLOW
                    } else {
                        color::DARK_BLUE
                    };

                    // If the source editor is open and this sound is selected, highlight it.
                    if state.is_open.source_editor {
                        if let Some(selected_id) = selected {
                            if project.sources.contains_key(&selected_id) {
                                if selected_id == id {
                                    let luminance = color.luminance();
                                    color = color.with_luminance(luminance.powf(0.5));
                                }
                            }
                        }
                    }

                    (
                        spread,
                        channel_radians,
                        channel_count,
                        position,
                        color,
                    )
                }
            };

            let spread = project.camera.metres_to_scalar(spread_m);
            let side_m = custom_widget::sound::dimension_metres(0.0);
            let side = project.state.camera.metres_to_scalar(side_m);
            let channel_amps = &channel_amplitudes[..channel_count];
            let installations = project.state
                .sources
                .iter()
                .find(|&(&id, _)| id == active_sound.source_id)
                .map(|(_, s)| s.audio.role.clone().into())
                .unwrap_or(audio::sound::Installations::All);

            // Determine the line colour by checking for interactions with the sound.
            let line_color = match ui.widget_input(sound_widget_id).mouse() {
                Some(mouse) => if mouse.buttons.left().is_down() {
                    color.clicked()
                } else {
                    color.highlighted()
                },
                None => color,
            };

            // For each channel in the sound, draw a line to the `closest_speakers` to which it is
            // sending audio.
            let mut line_index = 0;
            for channel in 0..channel_count {
                let point = position.point;
                let radians = position.radians + channel_radians;
                let channel_point_m =
                    audio::output::channel_point(point, channel, channel_count, spread_m, radians);
                let (ch_x, ch_y) = position_metres_to_gui(channel_point_m, &project.camera);
                let channel_amp = channel_amplitudes[channel];
                let speakers = &project.speakers;

                // A function for finding all speakers within proximity of a sound channel.
                fn find_speakers_in_proximity(
                    // The location of the source channel.
                    point: &Point2<Metres>,
                    // Installations that the current sound is applied to.
                    installations: &audio::sound::Installations,
                    // All speakers.
                    speakers: &project::Speakers,
                    // The rolloff attenuation.
                    rolloff_db: f64,
                    // Amp along with the index within the given `Vec`.
                    in_proximity: &mut Vec<(f32, audio::speaker::Id)>,
                ) {
                    if speakers.is_empty() {
                        return;
                    }

                    let (ids, dbap_speakers): (Vec<audio::speaker::Id>, Vec<audio::dbap::Speaker>) = {
                        // The location of the sound.
                        let point_f = Point2 {
                            x: point.x.0,
                            y: point.y.0,
                        };

                        let mut iter = speakers.iter();
                        iter.next()
                            .map(|(&id, speaker)| {
                                // The function used to create the dbap speakers.
                                let dbap_speaker = |speaker: &project::Speaker| -> audio::dbap::Speaker {
                                    let speaker_f = Point2 {
                                        x: speaker.audio.point.x.0,
                                        y: speaker.audio.point.y.0,
                                    };
                                    let distance = audio::dbap::blurred_distance_2(
                                        point_f,
                                        speaker_f,
                                        audio::DISTANCE_BLUR,
                                    );
                                    let weight = audio::speaker::dbap_weight(
                                        installations,
                                        &speaker.audio.installations,
                                    );
                                    audio::dbap::Speaker { distance, weight }
                                };

                                let init = (vec![id], vec![dbap_speaker(speaker)]);
                                iter.fold(init, |(mut ids, mut speakers), (&id, s)| {
                                    ids.push(id);
                                    speakers.push(dbap_speaker(s));
                                    (ids, speakers)
                                })
                            })
                            .unwrap_or_else(Default::default)
                    };
                    let gains = audio::dbap::SpeakerGains::new(&dbap_speakers, rolloff_db);
                    in_proximity.clear();
                    for (i, gain) in gains.enumerate() {
                        let id = ids[i];
                        if audio::output::speaker_is_in_proximity(point, &speakers[&id].audio.point) {
                            in_proximity.push((gain as f32, id));
                        }
                    }
                }

                find_speakers_in_proximity(
                    &channel_point_m,
                    &installations,
                    speakers,
                    project.master.dbap_rolloff_db,
                    &mut speakers_in_proximity,
                );
                let output_channels = state.audio_channels.output;
                for &(amp_scaler, speaker_id) in speakers_in_proximity.iter() {
                    let speaker = &speakers[&speaker_id];
                    if output_channels <= speaker.channel {
                        continue;
                    }
                    const MAX_THICKNESS: Scalar = 16.0;
                    let amp = (channel_amp * amp_scaler).powf(0.75);
                    let thickness = amp as Scalar * MAX_THICKNESS;
                    let speaker_point_m = speaker.point;
                    let (s_x, s_y) = position_metres_to_gui(speaker_point_m, &project.camera);

                    // Ensure there is a unique `Id` for this line.
                    if ids.floorplan_channel_to_speaker_lines.len() <= line_index {
                        let mut gen = ui.widget_id_generator();
                        ids.floorplan_channel_to_speaker_lines
                            .resize(line_index + 1, &mut gen);
                    }

                    let line_id = ids.floorplan_channel_to_speaker_lines[line_index];
                    widget::Line::abs([ch_x, ch_y], [s_x, s_y])
                        .color(line_color.alpha(amp_scaler.powf(0.75)))
                        .depth(1.0)
                        .thickness(thickness)
                        .parent(ids.floorplan)
                        .set(line_id, ui);

                    line_index += 1;
                }
            }

            let (x, y) = position_metres_to_gui(position.point, &project.camera);
            let radians = position.radians as _;
            custom_widget::Sound::new(channel_amps, spread, radians, channel_radians as _)
                .and_then(active_sound.normalised_progress, |w, p| w.progress(p))
                .color(color)
                .x_y(x, y)
                .w_h(side, side)
                .parent(ids.floorplan)
                .set(sound_widget_id, ui);
        }
    }
}
