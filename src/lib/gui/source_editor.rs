use audio;
use audio::source::Role;
use audio::source::wav::Playback;
use gui::{collapsible_area, Gui, State};
use gui::{DARK_A, ITEM_HEIGHT, SMALL_FONT_SIZE};
use installation::{self, Installation};
use metres::Metres;
use nannou::prelude::*;
use nannou::ui;
use nannou::ui::prelude::*;
use soundscape;
use std;
use std::ffi::OsStr;
use std::mem;
use std::ops;
use std::path::{Component, Path};
use time_calc::Ms;
use utils;
use walkdir::WalkDir;

pub struct SourceEditor {
    pub is_open: bool,
    pub sources: Vec<Source>,
    // The index of the selected source.
    pub selected: Option<usize>,
    // The next ID to be used for a new source.
    pub next_id: audio::source::Id,
    pub preview: SourcePreview,
}

pub struct SourcePreview {
    pub current: Option<(SourcePreviewMode, audio::sound::Id)>,
    pub point: Option<Point2<Metres>>,
}

#[derive(Copy, Clone, PartialEq)]
pub enum SourcePreviewMode {
    OneShot,
    Continuous,
}

// A GUI view of an audio source.
#[derive(Deserialize, Serialize)]
pub struct Source {
    pub name: String,
    pub audio: audio::Source,
    pub id: audio::source::Id,
}

// A data structure from which sources can be saved/loaded.
#[derive(Deserialize, Serialize)]
pub struct StoredSources {
    #[serde(default)]
    pub sources: Vec<Source>,
    #[serde(default = "first_source_id")]
    pub next_id: audio::source::Id,
}

pub fn first_source_id() -> audio::source::Id {
    audio::source::Id::INITIAL
}

const SOUNDSCAPE_COLOR: ui::Color = ui::color::DARK_RED;
const INTERACTIVE_COLOR: ui::Color = ui::color::DARK_GREEN;
const SCRIBBLES_COLOR: ui::Color = ui::color::DARK_PURPLE;

impl SourceEditor {
    // Returns the next unique source editor.
    fn next_id(&mut self) -> audio::source::Id {
        let next_id = self.next_id;
        self.next_id = audio::source::Id(self.next_id.0.wrapping_add(1));
        next_id
    }
}

impl Default for StoredSources {
    fn default() -> Self {
        StoredSources {
            sources: Vec::new(),
            next_id: audio::source::Id::INITIAL,
        }
    }
}

impl StoredSources {
    /// Load the audio sources from the given path.
    ///
    /// If there are any ".wav" files in `assets/audio` that have not yet been loaded into the
    /// stored sources, load them as `Wav` kind sources.
    ///
    /// If the path is invalid or the JSON can't be read, `StoredSources::new` will be called.
    pub fn load(sources_path: &Path, audio_path: &Path) -> Self {
        let mut stored: StoredSources = utils::load_from_json_or_default(sources_path);

        // Check the validity of the WAV source paths.
        //
        // If a path is invalid, check to see if it exists within the given `audio_path`. If so,
        // update the source path. Otherwise, remove it.
        for i in (0..stored.sources.len()).rev() {
            let mut remove = false;
            if let audio::source::Kind::Wav(ref mut wav) = stored.sources[i].audio.kind {
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

                remove = true;
            }

            if remove {
                // If no valid path was found, remove the source as we can't play it.
                stored.sources.remove(i);
            }
        }

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
                let wav = match audio::source::Wav::from_path(path) {
                    Ok(w) => w,
                    Err(e) => {
                        eprintln!("Failed to load wav file {:?}: {}", name, e);
                        continue;
                    }
                };
                let kind = audio::source::Kind::Wav(wav);
                let role = None;
                let spread = Metres(2.5);
                let radians = 0.0;
                let audio = audio::Source {
                    kind,
                    role,
                    spread,
                    radians,
                };
                let id = stored.next_id;
                let source = Source { name, audio, id };
                stored.sources.push(source);
                stored.next_id = audio::source::Id(stored.next_id.0 + 1);
            }
        }

        // Sort all sources by kind then name.
        stored
            .sources
            .sort_by(|a, b| match (&a.audio.kind, &b.audio.kind) {
                (&audio::source::Kind::Wav(_), &audio::source::Kind::Realtime(_)) => {
                    std::cmp::Ordering::Less
                }
                _ => a.name.cmp(&b.name),
            });
        stored
    }
}

pub fn set(last_area_id: widget::Id, gui: &mut Gui) -> widget::Id {
    let is_open = gui.state.source_editor.is_open;

    const PAD: Scalar = 6.0;
    const TEXT_PAD: Scalar = 20.0;
    const LIST_HEIGHT: Scalar = 140.0;
    const PREVIEW_CANVAS_H: Scalar = 66.0;
    const INSTALLATION_LIST_H: Scalar = ITEM_HEIGHT * 3.0;
    const INSTALLATIONS_CANVAS_H: Scalar =
        PAD + ITEM_HEIGHT * 2.0 + PAD + INSTALLATION_LIST_H + PAD;
    const LOOP_TOGGLE_H: Scalar = ITEM_HEIGHT;
    const PLAYBACK_MODE_H: Scalar = ITEM_HEIGHT;
    const WAV_CANVAS_H: Scalar =
        100.0 + PAD + LOOP_TOGGLE_H + PAD * 4.0 + PLAYBACK_MODE_H + PAD;
    const REALTIME_CANVAS_H: Scalar = 94.0;
    const CHANNEL_LAYOUT_CANVAS_H: Scalar = 200.0;
        PAD + ITEM_HEIGHT * 2.0 + PAD + INSTALLATION_LIST_H + PAD;
    let kind_specific_h = WAV_CANVAS_H.max(REALTIME_CANVAS_H);
    let selected_canvas_h = ITEM_HEIGHT * 2.0 + PAD * 7.0 + PREVIEW_CANVAS_H + kind_specific_h
        + CHANNEL_LAYOUT_CANVAS_H + INSTALLATIONS_CANVAS_H;
    let source_editor_canvas_h = LIST_HEIGHT + ITEM_HEIGHT + selected_canvas_h;

    let (area, event) = collapsible_area(is_open, "Source Editor", gui.ids.side_menu)
        .align_middle_x_of(gui.ids.side_menu)
        .down_from(last_area_id, 0.0)
        .set(gui.ids.source_editor, gui);
    if let Some(event) = event {
        gui.state.source_editor.is_open = event.is_open();
    }

    let area = match area {
        Some(area) => area,
        None => return gui.ids.source_editor,
    };

    // The canvas on which the source editor will be placed.
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
                            }
                            audio::source::Kind::Realtime(ref rt) => (
                                format!(
                                    "[{}-{}CH RT] {}",
                                    rt.channels.start,
                                    rt.channels.end - 1,
                                    source.name
                                ),
                                false,
                            ),
                        }
                    };

                    // Blue if selected, gray otherwise.
                    let color = if selected {
                        color::BLUE
                    } else {
                        color::CHARCOAL
                    };

                    // Use `Button`s for the selectable items.
                    let button = widget::Button::new()
                        .label(&label)
                        .label_font_size(SMALL_FONT_SIZE)
                        .label_x(position::Relative::Place(position::Place::Start(Some(
                            10.0,
                        ))))
                        .color(color);
                    item.set(button, gui);

                    // If the button or any of its children are capturing the mouse, display
                    // the `remove` button.
                    let show_remove_button = !is_wav
                        && gui.global_input()
                            .current
                            .widget_capturing_mouse
                            .map(|id| {
                                id == item.widget_id
                                    || gui.widget_graph()
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
                    }
                }

                // Update the selected source.
                Event::Selection(idx) => {
                    gui.state.source_editor.selected = Some(idx);

                    // If a source was being previewed, stop it.
                    if let Some((_, sound_id)) = gui.state.source_editor.preview.current {
                        gui.channels
                            .audio_output
                            .send(move |audio| {
                                audio.remove_sound(sound_id);
                            })
                            .ok();
                        gui.state.source_editor.preview.current = None;
                    }
                }

                _ => (),
            }
        }

        // The scrollbar for the list.
        if let Some(s) = scrollbar {
            s.set(gui);
        }

        // Remove a source if necessary.
        if let Some(i) = maybe_remove_index {
            if Some(i) == gui.state.source_editor.selected {
                gui.state.source_editor.selected = None;
            }

            // Remove local copy.
            let source = gui.state.source_editor.sources.remove(i);
            let id = source.id;

            // Remove audio input copy.
            gui.channels
                .audio_input
                .send(move |audio| {
                    audio.sources.remove(&id);
                    audio.active_sounds.remove(&id);
                })
                .ok();

            // Remove soundscape copy.
            gui.channels.soundscape.send(move |soundscape| {
                soundscape.remove_source(&id);
            }).expect("soundscape was closed");
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

    // Add a new WAV source.
    if new_wav {
        // Not sure if we want to support this in software yet.
    }

    // Add a new realtime source.
    if new_realtime {
        // Create the Realtime.
        const DEFAULT_CHANNELS: ops::Range<usize> = 0..1;
        const DEFAULT_DURATION: Ms = Ms(3_000.0);
        let channels = DEFAULT_CHANNELS;
        let duration = DEFAULT_DURATION;
        let realtime = audio::source::Realtime { channels, duration };

        // Create the Source.
        let id = gui.state.source_editor.next_id();
        let name = format!("Source {}", id.0);
        let kind = audio::source::Kind::Realtime(realtime.clone());
        let role = Default::default();
        let spread = audio::source::default_spread();
        let radians = Default::default();
        let audio = audio::Source {
            kind,
            role,
            spread,
            radians,
        };
        let source = Source { id, name, audio };

        // Insert the source into the map.
        gui.state.source_editor.sources.push(source);

        // Send the source to the audio input thread.
        gui.channels
            .audio_input
            .send(move |audio| {
                audio.sources.insert(id, realtime);
            })
            .ok();
    }

    let area_rect = gui.rect_of(area.id).unwrap();
    let start = area_rect.y.start;
    let end = start + selected_canvas_h;
    let selected_canvas_y = ui::Range { start, end };

    widget::Canvas::new()
        .pad(PAD)
        .w_of(gui.ids.side_menu)
        .h(selected_canvas_h)
        .y(selected_canvas_y.middle())
        .align_middle_x_of(gui.ids.side_menu)
        .set(gui.ids.source_editor_selected_canvas, gui);

    let selected_canvas_kid_area = gui.kid_area_of(gui.ids.source_editor_selected_canvas)
        .unwrap();

    // If a source is selected, display its info.
    let i = match gui.state.source_editor.selected {
        None => {
            widget::Text::new("No source selected")
                .padded_w_of(area.id, TEXT_PAD)
                .mid_top_with_margin_on(gui.ids.source_editor_selected_canvas, TEXT_PAD)
                .font_size(SMALL_FONT_SIZE)
                .center_justify()
                .set(gui.ids.source_editor_selected_none, gui);
            return area.id;
        }
        Some(i) => i,
    };

    let Gui {
        ref mut ui,
        ref mut ids,
        channels,
        sound_id_gen,
        state:
            &mut State {
                ref camera,
                max_input_channels,
                ref master,
                source_editor:
                    SourceEditor {
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

    // 4 Role Buttons
    let role_button_w = selected_canvas_kid_area.w() / 4.0;
    const NUM_ROLES: usize = 4;
    let (mut events, _) = widget::ListSelect::single(NUM_ROLES)
        .flow_right()
        .item_size(role_button_w)
        .h(ITEM_HEIGHT)
        .align_middle_x_of(ids.source_editor_selected_canvas)
        .down_from(ids.source_editor_selected_name, PAD)
        .set(ids.source_editor_selected_role_list, ui);

    fn int_to_role(i: usize) -> Option<Role> {
        match i {
            1 => Some(Role::Soundscape(Default::default())),
            2 => Some(Role::Interactive),
            3 => Some(Role::Scribbles),
            _ => None,
        }
    }

    fn role_index(role: &Role) -> usize {
        match *role {
            Role::Soundscape(_) => 1,
            Role::Interactive => 2,
            Role::Scribbles => 3,
        }
    }

    fn role_color(role: &Option<Role>) -> ui::Color {
        match *role {
            None => color::DARK_GREY,
            Some(Role::Soundscape(_)) => SOUNDSCAPE_COLOR,
            Some(Role::Interactive) => INTERACTIVE_COLOR,
            Some(Role::Scribbles) => SCRIBBLES_COLOR,
        }
    }

    fn role_label(role: &Option<Role>) -> &'static str {
        match *role {
            None => "NONE",
            Some(Role::Soundscape(_)) => "SCAPE",
            Some(Role::Interactive) => "INTERACT",
            Some(Role::Scribbles) => "SCRIB",
        }
    }

    let selected_role_index = sources[i].audio.role.as_ref().map(role_index).unwrap_or(0);
    let role_selected = |j| j == selected_role_index;

    while let Some(event) = events.next(ui, &role_selected) {
        use self::ui::widget::list_select::Event;
        match event {
            // Instantiate a button for each role.
            Event::Item(item) => {
                let selected = role_selected(item.i);
                let role = int_to_role(item.i);
                let label = role_label(&role);

                // Blue if selected, gray otherwise.
                let color = if selected {
                    role_color(&role)
                } else {
                    color::CHARCOAL
                };

                // Use `Button`s for the selectable items.
                let button = widget::Button::new()
                    .label(&label)
                    .label_font_size(SMALL_FONT_SIZE)
                    .color(color);
                item.set(button, ui);
            }

            // Update the selected role.
            Event::Selection(idx) => {
                let source = &mut sources[i];
                let id = source.id;
                let new_role = int_to_role(idx);
                let old_role = mem::replace(&mut source.audio.role, new_role.clone());
                match (old_role, new_role) {
                    // Don't do anything if the selection has stayed on soundscape.
                    (Some(Role::Soundscape(_)), Some(Role::Soundscape(_))) => (),

                    // If the source became a soundscape source, send it to the soundscape thread.
                    (_, Some(Role::Soundscape(_))) => {
                        let soundscape_source = soundscape::Source::from_audio_source(&source.audio)
                            .expect("source did not have soundscape role");
                        channels.soundscape.send(move |soundscape| {
                            soundscape.insert_source(id, soundscape_source);
                        }).expect("soundscape was closed");
                    },

                    // If it is no longer a soundscape, remove it from the soundscape thread.
                    (Some(Role::Soundscape(_)), _) => {
                        channels.soundscape.send(move |soundscape| {
                            soundscape.remove_source(&id);
                        }).expect("soundscape was closed");
                    },

                    _ => (),
                }
            }

            _ => (),
        }
    }

    // TODO: Show a role-specific canvas of options. E.g. `SCAPE` should show an installation set.
    let mut last_canvas_id = ids.source_editor_selected_role_list;
    match sources[i].audio.role.clone() {
        // For soundscape sounds, allow the user to select installations.
        Some(Role::Soundscape(audio::source::Soundscape { mut installations })) => {
            // A canvas on which installation selection widgets are instantiated.
            widget::Canvas::new()
                .kid_area_w_of(ids.source_editor_selected_canvas)
                .h(INSTALLATIONS_CANVAS_H)
                .align_middle_x_of(ids.source_editor_selected_canvas)
                .parent(ids.source_editor_selected_canvas)
                .down(PAD)
                .pad(PAD)
                .color(color::CHARCOAL)
                .set(ids.source_editor_selected_installations_canvas, ui);
            last_canvas_id = ids.source_editor_selected_installations_canvas;

            // A header for the installations editing area.
            widget::Text::new("Installations")
                .top_left_of(ids.source_editor_selected_installations_canvas)
                .font_size(SMALL_FONT_SIZE)
                .set(ids.source_editor_selected_installations_text, ui);

            // A dropdownlist for assigning installations to the source.
            //
            // Only show installations that aren't yet assigned.
            let installations_vec = installation::ALL
                .iter()
                .filter(|inst| !installations.contains(inst))
                .cloned()
                .collect::<Vec<_>>();
            let installation_strs = installations_vec
                .iter()
                .map(Installation::display_str)
                .collect::<Vec<_>>();
            for index in widget::DropDownList::new(&installation_strs, None)
                .align_middle_x_of(ids.source_editor_selected_installations_canvas)
                .down_from(ids.source_editor_selected_installations_text, PAD * 2.0)
                .h(ITEM_HEIGHT)
                .kid_area_w_of(ids.source_editor_selected_installations_canvas)
                .label("ADD INSTALLATION")
                .label_font_size(SMALL_FONT_SIZE)
                .set(ids.source_editor_selected_installations_ddl, ui)
            {
                let installation = installations_vec[index];
                installations.insert(installation);

                // Update the local copy.
                let source = &mut sources[i];
                if let Some(Role::Soundscape(ref mut soundscape)) = source.audio.role {
                    soundscape.installations.insert(installation);
                }

                // Update the soundscape copy.
                let id = source.id;
                channels.soundscape.send(move |soundscape| {
                    soundscape.update_source(&id, |source| {
                        source.installations.insert(installation);
                    });
                }).expect("soundscape channel closed");

                // Update sounds
                channels.audio_output.send(move |audio| {
                    for (_, sound) in audio.sounds_mut().filter(|&(_, ref s)| s.source_id() == id) {
                        if let audio::sound::Installations::Set(ref mut set) = sound.installations {
                            set.insert(installation);
                        }
                    }
                }).ok();
            }

            // A scrollable list showing each of the assigned installations.
            let mut selected_installations = installations.iter().cloned().collect::<Vec<_>>();
            selected_installations.sort_by(|a, b| a.display_str().cmp(b.display_str()));
            let (mut items, scrollbar) = widget::List::flow_down(selected_installations.len())
                .item_size(ITEM_HEIGHT)
                .h(INSTALLATION_LIST_H)
                .kid_area_w_of(ids.source_editor_selected_installations_canvas)
                .align_middle_x_of(ids.source_editor_selected_installations_canvas)
                .down_from(ids.source_editor_selected_installations_ddl, PAD)
                .scrollbar_next_to()
                .scrollbar_color(color::LIGHT_CHARCOAL)
                .set(ids.source_editor_selected_installations_list, ui);
            let mut maybe_remove_index = None;
            while let Some(item) = items.next(ui) {
                let inst = selected_installations[item.i];
                let label = inst.display_str();

                // Use `Button`s for the selectable items.
                let button = widget::Button::new()
                    .label(&label)
                    .label_font_size(SMALL_FONT_SIZE)
                    .label_x(position::Relative::Place(position::Place::Start(Some(
                        10.0,
                    ))));
                item.set(button, ui);

                // If the button or any of its children are capturing the mouse, display
                // the `remove` button.
                let show_remove_button = ui.global_input()
                    .current
                    .widget_capturing_mouse
                    .map(|id| {
                        id == item.widget_id
                            || ui.widget_graph()
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
                    .set(ids.source_editor_selected_installations_remove, ui)
                    .was_clicked()
                {
                    maybe_remove_index = Some(item.i);
                }
            }

            // The scrollbar for the list.
            if let Some(scrollbar) = scrollbar {
                scrollbar.set(ui);
            }

            // If some installation was clicked for removal, remove it.
            if let Some(inst) = maybe_remove_index.map(|i| selected_installations[i]) {
                let source = &mut sources[i];
                let id = source.id;

                // Remove the local copy.
                if let Some(Role::Soundscape(ref mut soundscape)) = source.audio.role {
                    soundscape.installations.remove(&inst);
                }

                // Remove the soundscape copy.
                channels.soundscape.send(move |soundscape| {
                    soundscape.update_source(&id, move |source| {
                        source.installations.remove(&inst);
                    });
                }).expect("soundscape channel closed");

                // Remove the installation from sounds driven by this source on the output stream.
                channels.audio_output.send(move |audio| {
                    for (_, sound) in audio.sounds_mut().filter(|&(_, ref s)| s.source_id() == id) {
                        if let audio::sound::Installations::Set(ref mut set) = sound.installations {
                            set.remove(&inst);
                        }
                    }
                }).ok();
            }
        },

        // For interactive sounds, allow the user specify the location.NOTE: Option - just work
        // this sound out from the location of the speakers?
        Some(Role::Interactive) => {
        },

        // For scribbles, allow a specific location from which the speaking appears.
        Some(Role::Scribbles) => {
        },

        // If it has no role, no specific stuff to be done.
        None => (),
    }

    // Preview options.
    widget::Canvas::new()
        .mid_left_of(ids.source_editor_selected_canvas)
        .down_from(last_canvas_id, PAD)
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

    fn update_mode(
        new_mode: SourcePreviewMode,
        channels: &super::Channels,
        sound_id_gen: &audio::sound::IdGenerator,
        camera: &super::Camera,
        source: &Source,
        preview: &mut SourcePreview,
        realtime_source_latency: &Ms,
    ) {
        loop {
            match preview.current {
                // If a preview exists, remove it.
                Some((mode, sound_id)) => {
                    channels
                        .audio_output
                        .send(move |audio| {
                            audio.remove_sound(sound_id);
                        })
                        .ok();

                    preview.current = None;
                    if mode != new_mode {
                        continue;
                    }
                }

                // Otherwise set the preview mode to one-shot.
                None => {
                    let sound_id = sound_id_gen.generate_next();
                    preview.current = Some((new_mode, sound_id));
                    // Set the preview position to the centre of the camera if not yet set.
                    if preview.point.is_none() {
                        preview.point = Some(camera.position);
                    }
                    // Send the selected source to the audio thread for playback.
                    let should_cycle = match new_mode {
                        SourcePreviewMode::OneShot => false,
                        SourcePreviewMode::Continuous => true,
                    };
                    let _handle = audio::sound::spawn_from_source(
                        sound_id,
                        source.id,
                        &source.audio,
                        preview.point.unwrap(),
                        should_cycle,
                        &channels.audio_input,
                        &channels.audio_output,
                        *realtime_source_latency,
                    );
                }
            }
            break;
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
        update_mode(
            SourcePreviewMode::OneShot,
            channels,
            sound_id_gen,
            camera,
            &sources[i],
            preview,
            &master.params.realtime_source_latency,
        );
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
        update_mode(
            SourcePreviewMode::Continuous,
            channels,
            sound_id_gen,
            camera,
            &sources[i],
            preview,
            &master.params.realtime_source_latency,
        );
    }

    // Kind-specific data.
    let source_id = sources[i].id;
    let (kind_canvas_id, num_channels) = match sources[i].audio.kind {
        audio::source::Kind::Wav(ref mut wav) => {
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
            let data = format!(
                "{}\nChannels: {}\nSample Rate: {}\n{}",
                file_line, wav.channels, wav.sample_hz, duration_line
            );
            widget::Text::new(&data)
                .font_size(SMALL_FONT_SIZE)
                .align_left_of(ids.source_editor_selected_wav_text)
                .down(PAD)
                .line_spacing(PAD)
                .set(ids.source_editor_selected_wav_data, ui);

            // A `Toggle` for whether or not the WAV should loop.
            let label = if wav.should_loop { "Looping: ON" } else { "Looping: OFF" };
            let canvas_kid_area = ui.kid_area_of(ids.source_editor_selected_wav_canvas).unwrap();
            for new_loop in widget::Toggle::new(wav.should_loop)
                .color(color::LIGHT_CHARCOAL)
                .label(label)
                .label_font_size(SMALL_FONT_SIZE)
                .down(PAD * 2.0)
                .h(LOOP_TOGGLE_H)
                .w(canvas_kid_area.w())
                .align_middle_x_of(ids.source_editor_selected_wav_canvas)
                .set(ids.source_editor_selected_wav_loop_toggle, ui)
            {
                // Update the local copy.
                wav.should_loop = new_loop;

                // Update the soundscape thread copy.
                channels.soundscape.send(move |soundscape| {
                    soundscape.update_source(&source_id, |source| {
                        if let audio::source::Kind::Wav(ref mut wav) = source.kind {
                            wav.should_loop = new_loop;
                        }
                    });
                }).ok();

                // TODO: On the audio output thread, swap out any sounds that use this WAV source
                // with a looping version.
            }

            // The playback mode selection.
            widget::Text::new("Playback Mode")
                .font_size(SMALL_FONT_SIZE)
                .down(PAD)
                .align_left_of(ids.source_editor_selected_wav_text)
                .set(ids.source_editor_selected_wav_playback_text, ui);
            let item_w = canvas_kid_area.w() * 0.5;
            let n_items = audio::source::wav::NUM_PLAYBACK_OPTIONS;
            let (mut events, _scrollbar) = widget::ListSelect::single(n_items)
                .flow_right()
                .item_size(item_w)
                .down(PAD * 2.0)
                .h(PLAYBACK_MODE_H)
                .w(canvas_kid_area.w())
                .mid_bottom_of(ids.source_editor_selected_wav_canvas)
                .set(ids.source_editor_selected_wav_playback_list, ui);

            fn playback_from_index(i: usize) -> Option<Playback> {
                match i {
                    0 => Some(Playback::Retrigger),
                    1 => Some(Playback::Continuous),
                    _ => None,
                }
            }

            fn index_from_playback(playback: &Playback) -> usize {
                match *playback {
                    Playback::Retrigger => 0,
                    Playback::Continuous => 1,
                }
            }

            fn playback_label(playback: &Playback) -> &str {
                match *playback {
                    Playback::Retrigger => "Retrigger",
                    Playback::Continuous => "Continuous",
                }
            }

            let selected_index = index_from_playback(&wav.playback);
            while let Some(event) = events.next(ui, |i| i == selected_index) {
                use self::ui::widget::list_select::Event;
                match event {
                    // Instantiate a button for each source.
                    Event::Item(item) => {
                        let selected = item.i == selected_index;
                        let playback = playback_from_index(item.i)
                            .expect("no playback mode for index");
                        let label = playback_label(&playback);

                        // Blue if selected, gray otherwise.
                        let color = if selected {
                            color::LIGHT_CHARCOAL
                        } else {
                            DARK_A
                        };

                        let button = widget::Button::new()
                            .label(label)
                            .label_font_size(SMALL_FONT_SIZE)
                            .color(color);
                        item.set(button, ui);
                    },
                    // If a selection has occurred.
                    Event::Selection(new_index) => {
                        let new_playback = playback_from_index(new_index)
                            .expect("no playback mode for index");

                        // Update the local copy.
                        wav.playback = new_playback;

                        // Update the soundscape copy.
                        channels.soundscape.send(move |soundscape| {
                            soundscape.update_source(&source_id, |source| {
                                if let audio::source::Kind::Wav(ref mut wav) = source.kind {
                                    wav.playback = new_playback;
                                }
                            });
                        }).ok();

                        // Update all audio thread copies.
                        channels.audio_output.send(move |audio| {
                            audio.update_sounds_with_source(&source_id, move |_, sound| {
                                if let audio::source::Signal::Wav { ref mut playback, .. } = sound.signal {
                                    *playback = new_playback;
                                }
                            });
                        }).ok();
                    },
                    _ => (),
                }
            }

            (ids.source_editor_selected_wav_canvas, wav.channels)
        }
        audio::source::Kind::Realtime(ref mut realtime) => {
            // Instantiate a small canvas for displaying wav-specific stuff.
            widget::Canvas::new()
                .down_from(ids.source_editor_preview_canvas, PAD)
                .parent(ids.source_editor_selected_canvas)
                .w(selected_canvas_kid_area.w())
                .color(color::CHARCOAL)
                .h(REALTIME_CANVAS_H)
                .pad(PAD)
                .set(ids.source_editor_selected_realtime_canvas, ui);

            // Display the immutable WAV data.
            widget::Text::new("REALTIME DATA")
                .font_size(SMALL_FONT_SIZE)
                .top_left_of(ids.source_editor_selected_realtime_canvas)
                .set(ids.source_editor_selected_realtime_text, ui);

            // A small macro to simplify updating each of the local, soundscape and audio input
            // stream support.
            //
            // We use a macro so that a unique `FnOnce` is generated for each unique call. This way
            // we can avoid the `update_fn` requiring `FnMut` which adds some ownership
            // constraints.
            macro_rules! update_realtime {
                ($update_fn:expr) => {
                    $update_fn(realtime);

                    // Update the audio input thread copy.
                    channels.audio_input.send(move |audio| {
                        if let Some(realtime) = audio.sources.get_mut(&source_id) {
                            $update_fn(realtime);
                        }
                    }).ok();

                    // Update the soundscape thread copy.
                    channels.soundscape.send(move |soundscape| {
                        soundscape.update_source(&source_id, |source| {
                            if let audio::source::Kind::Realtime(ref mut realtime) = source.kind {
                                $update_fn(realtime);
                            }
                        });
                    }).expect("soundscape was closed");
                };
            }

            // Playback duration.
            const SEC_MS: f32 = 1_000.0;
            const MIN_MS: f32 = SEC_MS * 60.0;
            const HR_MS: f32 = MIN_MS * 60.0;
            const DAY_MS: f32 = HR_MS * 24.0;
            let min = 0.0;
            let max = HR_MS;
            let ms = realtime.duration.ms() as f32;
            let label = if ms < SEC_MS {
                format!("{:.2} ms", ms)
            } else if ms < MIN_MS {
                let secs = (ms / SEC_MS) as u32;
                let ms = ms - (secs as f32 * SEC_MS);
                format!("{} secs {:.2} ms", secs, ms)
            } else if ms < HR_MS {
                let mins = (ms / MIN_MS) as u32;
                let secs = (ms - (mins as f32 * MIN_MS)) / SEC_MS;
                format!("{} mins {:.2} secs", mins, secs)
            } else if ms < DAY_MS {
                let hrs = (ms / HR_MS) as u32;
                let mins = (ms - (hrs as f32 * HR_MS)) / MIN_MS;
                format!("{} hrs {:.2} mins", hrs, mins)
            } else {
                let days = ms / DAY_MS;
                format!("{:.2} days", days)
            };
            for new_ms in widget::Slider::new(ms, min, max)
                .label(&format!("Duration: {}", label))
                .label_font_size(SMALL_FONT_SIZE)
                .kid_area_w_of(ids.source_editor_selected_realtime_canvas)
                .h(ITEM_HEIGHT)
                .down(PAD)
                .skew(10.0)
                .set(ids.source_editor_selected_realtime_duration, ui)
            {
                // Update the local copy.
                let new_duration = Ms(new_ms as _);
                update_realtime!(|realtime: &mut audio::source::Realtime| realtime.duration = new_duration);
            }

            // Starting channel index (to the left).
            let start_channel_indices = 0..realtime.channels.end;
            let start_channel_labels = start_channel_indices
                .clone()
                .map(|ch| format!("Start Channel: {}", ch))
                .collect::<Vec<_>>();
            let selected_start = Some(realtime.channels.start as usize);
            let channel_w = ui.kid_area_of(ids.source_editor_selected_realtime_canvas)
                .unwrap()
                .w() / 2.0 - PAD / 2.0;
            for new_start in widget::DropDownList::new(&start_channel_labels, selected_start)
                .down(PAD)
                .align_left()
                .label("Start Channel")
                .label_font_size(SMALL_FONT_SIZE)
                .scrollbar_on_top()
                .max_visible_items(5)
                .w(channel_w)
                .h(ITEM_HEIGHT)
                .set(ids.source_editor_selected_realtime_start_channel, ui)
            {
                // Update the local copy.
                update_realtime!(|rt: &mut audio::source::Realtime| rt.channels.start = new_start);
            }

            // End channel index (to the right).
            let mut end_channel_indices = realtime.channels.start..max_input_channels;
            let end_channel_labels = end_channel_indices
                .clone()
                .map(|ch| format!("End Channel: {}", ch))
                .collect::<Vec<_>>();
            let selected_end =
                Some((realtime.channels.end - (realtime.channels.start + 1)) as usize);
            for new_end in widget::DropDownList::new(&end_channel_labels, selected_end)
                .right(PAD)
                .align_top()
                .label("End Channel")
                .label_font_size(SMALL_FONT_SIZE)
                .scrollbar_on_top()
                .max_visible_items(5)
                .w(channel_w)
                .h(ITEM_HEIGHT)
                .set(ids.source_editor_selected_realtime_end_channel, ui)
            {
                let new_end = end_channel_indices.nth(new_end).unwrap() + 1;
                // Update the local copy.
                update_realtime!(|rt: &mut audio::source::Realtime| rt.channels.end = new_end);
            }

            (
                ids.source_editor_selected_realtime_canvas,
                realtime.channels.len(),
            )
        }
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

    let channel_layout_kid_area = ui.kid_area_of(ids.source_editor_selected_channel_layout_canvas)
        .unwrap();
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
        let spread_m = Metres(spread as _);

        // Update the local copy.
        sources[i].audio.spread = spread_m;

        // Update soundscape copy if it's there.
        let id = sources[i].id;
        channels.soundscape.send(move |soundscape| {
            soundscape.update_source(&id, |source| source.spread = spread_m);
        }).expect("soundscape was closed");

        // Update the audio output copies.
        channels.audio_output.send(move |audio| {
            audio.update_sounds_with_source(&id, move |_, sound| {
                sound.spread = spread_m;
            });
        }).ok();
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

        // Update the local copy.
        sources[i].audio.radians = rotation;

        // Update the soundscape copy.
        let id = sources[i].id;
        channels.soundscape.send(move |soundscape| {
            soundscape.update_source(&id, move |source| source.radians = rotation);
        }).expect("soundscape was closed");

        // Update the audio output copies.
        channels.audio_output.send(move |audio| {
            for (_, sound) in audio.sounds_mut().filter(|&(_, ref s)| s.source_id() == id) {
                sound.radians = rotation;
            }
        }).ok();
    }

    // The field over which the channel layout will be visualised.
    let spread_rect = ui.rect_of(ids.source_editor_selected_channel_layout_spread)
        .unwrap();
    let layout_top = spread_rect.bottom() - PAD;
    let layout_bottom = channel_layout_kid_area.bottom();
    let layout_h = layout_top - layout_bottom;
    const CHANNEL_CIRCLE_RADIUS: Scalar = PAD * 2.0;
    let field_h = layout_h - CHANNEL_CIRCLE_RADIUS * 2.0;
    let field_radius = field_h / 2.0;
    widget::Circle::fill(field_radius)
        .color(DARK_A)
        .down_from(
            ids.source_editor_selected_channel_layout_spread,
            PAD + CHANNEL_CIRCLE_RADIUS,
        )
        .align_middle_x_of(ids.source_editor_selected_channel_layout_canvas)
        .set(ids.source_editor_selected_channel_layout_field, ui);

    // Circle demonstrating the actual spread distance of the channels relative to min/max.
    let min_spread_circle_radius = field_radius / 2.0;
    let spread_circle_radius = ui::utils::map_range(
        spread,
        MIN_SPREAD,
        MAX_SPREAD,
        min_spread_circle_radius,
        field_radius,
    );
    widget::Circle::outline(spread_circle_radius)
        .color(color::DARK_BLUE)
        .middle_of(ids.source_editor_selected_channel_layout_field)
        .set(ids.source_editor_selected_channel_layout_spread_circle, ui);

    // A circle for each channel along the edge of the `spread_circle`.
    if ids.source_editor_selected_channel_layout_channels.len() < num_channels {
        let id_gen = &mut ui.widget_id_generator();
        ids.source_editor_selected_channel_layout_channels
            .resize(num_channels, id_gen);
    }
    if ids.source_editor_selected_channel_layout_channel_labels
        .len() < num_channels
    {
        let id_gen = &mut ui.widget_id_generator();
        ids.source_editor_selected_channel_layout_channel_labels
            .resize(num_channels, id_gen);
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
            .x_y_relative_to(
                ids.source_editor_selected_channel_layout_spread_circle,
                x,
                y,
            )
            .parent(ids.source_editor_selected_channel_layout_spread_circle)
            .set(id, ui);

        // The label showing the channel number (starting from 1).
        let label_id = ids.source_editor_selected_channel_layout_channel_labels[i];
        let label = format!("{}", i + 1);
        widget::Text::new(&label)
            .middle_of(id)
            .y_relative_to(id, SMALL_FONT_SIZE as Scalar * 0.13)
            .font_size(SMALL_FONT_SIZE)
            .set(label_id, ui);
    }

    area.id
}
