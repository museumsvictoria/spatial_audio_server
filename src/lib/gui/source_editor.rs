use audio;
use gui::{collapsible_area, Gui, State};
use gui::{ITEM_HEIGHT, SMALL_FONT_SIZE, DARK_A};
use metres::Metres;
use nannou::prelude::*;
use nannou::ui;
use nannou::ui::prelude::*;
use serde_json;
use soundscape;
use std;
use std::ffi::OsStr;
use std::fs::File;
use std::path::Path;

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
    #[serde(default = "Vec::new")]
    pub sources: Vec<Source>,
    #[serde(default = "first_source_id")]
    pub next_id: audio::source::Id,
}

pub fn first_source_id() -> audio::source::Id {
    audio::source::Id::INITIAL
}

const SOUNDSCAPE_COLOR: ui::Color = ui::color::DARK_RED;
const INSTALLATION_COLOR: ui::Color = ui::color::DARK_GREEN;
const SCRIBBLES_COLOR: ui::Color = ui::color::DARK_PURPLE;

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
    pub fn load(sources_path: &Path, audio_path: &Path) -> Self {
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

pub fn set(last_area_id: widget::Id, gui: &mut Gui) -> widget::Id {
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
                    }
                },

                // Update the selected source.
                Event::Selection(idx) => {
                    gui.state.source_editor.selected = Some(idx);

                    // If a source was being previewed, stop it.
                    if let Some((_, sound_id)) = gui.state.source_editor.preview.current {
                        gui.channels.audio.send(move |audio| {
                            audio.remove_sound(sound_id);
                        }).ok();
                        gui.state.source_editor.preview.current = None;
                    }
                },

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
    let i = match gui.state.source_editor.selected {
        None => {
            widget::Text::new("No source selected")
                .padded_w_of(area.id, TEXT_PAD)
                .mid_top_with_margin_on(gui.ids.source_editor_selected_canvas, TEXT_PAD)
                .font_size(SMALL_FONT_SIZE)
                .center_justify()
                .set(gui.ids.source_editor_selected_none, gui);
            return area.id;
        },
        Some(i) => i,
    };

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
                let msg = soundscape::Message::UpdateSource(source.id, source.audio.clone());
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

    fn update_mode(
        new_mode: SourcePreviewMode,
        channels: &super::Channels,
        sound_id_gen: &audio::sound::IdGenerator,
        camera: &super::Camera,
        source: &Source,
        preview: &mut SourcePreview,
    ) {
        loop {
            match preview.current {
                // If a preview exists, remove it.
                Some((mode, sound_id)) => {
                    channels.audio.send(move |audio| {
                        audio.remove_sound(sound_id);
                    }).ok();

                    preview.current = None;
                    if mode != new_mode {
                        continue
                    }
                },

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
                    let sound = sound_from_source(
                        source.id,
                        &source.audio,
                        preview.point.unwrap(),
                        should_cycle,
                    );
                    channels.audio.send(move |audio| {
                        audio.insert_sound(sound_id, sound.into());
                    }).ok();
                },
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
        update_mode(SourcePreviewMode::OneShot, channels, sound_id_gen, camera, &sources[i], preview);
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
        update_mode(SourcePreviewMode::Continuous, channels, sound_id_gen, camera, &sources[i], preview);
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
        let msg = soundscape::Message::UpdateSource(sources[i].id, sources[i].audio.clone());
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
        let msg = soundscape::Message::UpdateSource(sources[i].id, sources[i].audio.clone());
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

    area.id
}
