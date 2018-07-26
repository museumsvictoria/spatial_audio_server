use audio;
use audio::source::Role;
use audio::source::wav::Playback;
use gui::{collapsible_area, duration_label, hz_label, Gui, ProjectState, State};
use gui::{DARK_A, ITEM_HEIGHT, SMALL_FONT_SIZE};
use metres::Metres;
use nannou::prelude::*;
use nannou::ui;
use nannou::ui::prelude::*;
use project::{self, Project};
use soundscape;
use std::{self, cmp, mem, ops};
use std::sync::atomic;
use time_calc::{Ms, Samples};
use utils;

/// Runtime state related to the source editor GUI panel.
#[derive(Debug, Default)]
pub struct SourceEditor {
    /// The Id of the currently selected source.
    pub selected: Option<audio::source::Id>,
    /// The source currently being previewed via the source editor GUI.
    pub preview: SourcePreview,
}

/// A source currently being previewed.
#[derive(Debug, Default)]
pub struct SourcePreview {
    pub current: Option<(SourcePreviewMode, audio::sound::Id)>,
    pub point: Option<Point2<Metres>>,
}

/// The mode of source preview.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum SourcePreviewMode {
    OneShot,
    Continuous,
}

/// Sort sources by kind and then name when displaying in the list.
fn source_display_order(a: &project::Source, b: &project::Source) -> cmp::Ordering {
    match (&a.kind, &b.kind) {
        (&audio::source::Kind::Wav(_), &audio::source::Kind::Realtime(_)) => {
            cmp::Ordering::Less
        }
        _ => a.name.cmp(&b.name),
    }
}

const SOUNDSCAPE_COLOR: ui::Color = ui::color::DARK_RED;
const INTERACTIVE_COLOR: ui::Color = ui::color::DARK_GREEN;
const SCRIBBLES_COLOR: ui::Color = ui::color::DARK_PURPLE;

pub fn set(
    last_area_id: widget::Id,
    gui: &mut Gui,
    project: &mut Project,
    project_state: &mut ProjectState,
) -> widget::Id {

    let Gui {
        ref mut ui,
        ref mut ids,
        ref mut audio_monitor,
        channels,
        sound_id_gen,
        state:
            &mut State {
                ref mut is_open,
                ref audio_channels,
                ..
            },
        ..
    } = *gui;

    let Project {
        state: project::State {
            ref camera,
            ref master,
            ref soundscape_groups,
            ref installations,
            ref mut sources,
            ..
        },
        ..
    } = *project;

    let ProjectState {
        ref mut source_editor,
        ..
    } = *project_state;

    const PAD: Scalar = 6.0;
    const TEXT_PAD: Scalar = 20.0;
    const LIST_HEIGHT: Scalar = 140.0;
    const PREVIEW_CANVAS_H: Scalar = 66.0;
    const INSTALLATION_LIST_H: Scalar = ITEM_HEIGHT * 3.0;
    const INSTALLATIONS_CANVAS_H: Scalar =
        PAD + ITEM_HEIGHT * 2.0 + PAD + INSTALLATION_LIST_H + PAD;
    const SLIDER_H: Scalar = ITEM_HEIGHT;
    const SOUNDSCAPE_GROUP_LIST_H: Scalar = ITEM_HEIGHT * 3.0;
    const BUTTON_H: Scalar = ITEM_HEIGHT;
    const SOUNDSCAPE_CANVAS_H: Scalar = PAD + TEXT_PAD + PAD
        + TEXT_PAD + PAD + SLIDER_H + PAD
        + TEXT_PAD + PAD + SLIDER_H + PAD
        + TEXT_PAD + PAD + SLIDER_H + PAD
        + TEXT_PAD + PAD + SLIDER_H + PAD
        + TEXT_PAD + PAD + SLIDER_H + PAD
        + TEXT_PAD + PAD * 3.5 + SOUNDSCAPE_GROUP_LIST_H + PAD
        + TEXT_PAD + PAD * 2.0 + BUTTON_H + PAD + BUTTON_H + PAD
        + TEXT_PAD + PAD * 2.0 + SLIDER_H + PAD
        + TEXT_PAD + PAD * 2.0 + SLIDER_H + PAD
        + TEXT_PAD + PAD * 2.0 + SLIDER_H + PAD
        + TEXT_PAD + PAD * 2.0 + SLIDER_H * 2.0 + PAD
        + TEXT_PAD + PAD * 2.0 + SLIDER_H + PAD;
    const LOOP_TOGGLE_H: Scalar = ITEM_HEIGHT;
    const PLAYBACK_MODE_H: Scalar = ITEM_HEIGHT;
    const WAV_CANVAS_H: Scalar =
        100.0 + PAD + LOOP_TOGGLE_H + PAD * 4.0 + PLAYBACK_MODE_H + PAD;
    const REALTIME_CANVAS_H: Scalar = 94.0;
    const CHANNEL_LAYOUT_H: Scalar = 200.0;
    const COMMON_CANVAS_H: Scalar = TEXT_PAD + PAD + SLIDER_H + PAD + CHANNEL_LAYOUT_H;
    let kind_specific_h = WAV_CANVAS_H.max(REALTIME_CANVAS_H);
    let selected_canvas_h = ITEM_HEIGHT * 2.0 + PAD * 7.0 + PREVIEW_CANVAS_H + kind_specific_h
        + COMMON_CANVAS_H + INSTALLATIONS_CANVAS_H + PAD + SOUNDSCAPE_CANVAS_H;
    let source_editor_canvas_h = LIST_HEIGHT + ITEM_HEIGHT + selected_canvas_h;

    let (area, event) = collapsible_area(is_open.source_editor, "Source Editor", ids.side_menu)
        .align_middle_x_of(ids.side_menu)
        .down_from(last_area_id, 0.0)
        .set(ids.source_editor, ui);
    if let Some(event) = event {
        is_open.source_editor = event.is_open();
    }

    let area = match area {
        Some(area) => area,
        None => return ids.source_editor,
    };

    // The canvas on which the source editor will be placed.
    let canvas = widget::Canvas::new()
        .pad(0.0)
        .h(source_editor_canvas_h);
    area.set(canvas, ui);

    // Convert the given map into a sorted list of source Ids.
    fn sorted_sources_vec(sources: &project::SourcesMap) -> Vec<audio::source::Id> {
        let mut sources_vec: Vec<_> = sources.keys().cloned().collect();
        sources_vec.sort_by(|a, b| source_display_order(&sources[a], &sources[b]));
        sources_vec
    }

    // Convert the map of sources into a vec sourted for display.
    //
    // TODO: Possibly store this within source_editor for re-use.
    let mut sources_vec = sorted_sources_vec(sources);

    // If there are no sources, display a message saying how to add some.
    if sources.is_empty() {
        widget::Text::new("Add some source outputs with the `+` button")
            .padded_w_of(area.id, TEXT_PAD)
            .mid_top_with_margin_on(area.id, TEXT_PAD)
            .font_size(SMALL_FONT_SIZE)
            .center_justify()
            .set(ids.source_editor_no_sources, ui);

    // Otherwise display the source list.
    } else {
        let num_items = sources.len();
        let (mut events, scrollbar) = widget::ListSelect::single(num_items)
            .item_size(ITEM_HEIGHT)
            .h(LIST_HEIGHT)
            .align_middle_x_of(area.id)
            .align_top_of(area.id)
            .scrollbar_next_to()
            .scrollbar_color(color::LIGHT_CHARCOAL)
            .set(ids.source_editor_list, ui);

        // If a source was removed, process it after the whole list is instantiated to avoid
        // invalid indices.
        let mut maybe_remove_index = None;
        let selected_id = source_editor.selected;
        let selected_index = sources_vec.iter()
            .position(|&id| Some(id) == selected_id)
            .unwrap_or(0);

        while let Some(event) = events.next(ui, |i| i == selected_index) {
            use self::ui::widget::list_select::Event;
            match event {
                // Instantiate a button for each source.
                Event::Item(item) => {
                    let selected = selected_index == item.i;
                    let id = sources_vec[item.i];
                    let (label, is_wav) = {
                        let source = &sources[&id];
                        match source.audio.kind {
                            audio::source::Kind::Wav(ref wav) => {
                                (format!("[{}CH WAV] {}", wav.channels, source.name), true)
                            }
                            audio::source::Kind::Realtime(ref rt) => (
                                format!(
                                    "[{}-{}CH RT] {}",
                                    rt.channels.start + 1,
                                    (rt.channels.end - 1) + 1,
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
                    item.set(button, ui);

                    // If the button or any of its children are capturing the mouse, display
                    // the `remove` button.
                    let show_remove_button = !is_wav
                        && ui.global_input()
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
                        .set(ids.source_editor_remove, ui)
                        .was_clicked()
                    {
                        maybe_remove_index = Some(item.i);
                    }
                }

                // Update the selected source.
                Event::Selection(idx) => {
                    let id = sources_vec[idx];
                    source_editor.selected = Some(id);

                    // If a source was being previewed, stop it.
                    if let Some((_, sound_id)) = source_editor.preview.current {
                        channels
                            .audio_output
                            .send(move |audio| {
                                audio.remove_sound(sound_id);
                            })
                            .expect("failed to remove previewed sound from audio output thread");
                        source_editor.preview.current = None;
                    }
                }

                _ => (),
            }
        }

        // The scrollbar for the list.
        if let Some(s) = scrollbar {
            s.set(ui);
        }

        // Remove a source if necessary.
        if let Some(i) = maybe_remove_index {
            let remove_id = sources_vec.remove(i);
            if let Some(id) = source_editor.selected {
                if remove_id == id {
                    source_editor.selected = None;
                }
            }

            // Remove any monitored sounds using this source ID.
            audio_monitor.active_sounds.retain(|_, s| s.source_id != remove_id);

            // Remove the local copy.
            sources.remove(&remove_id);

            // Remove audio input copy.
            channels
                .audio_input
                .send(move |audio| {
                    audio.sources.remove(&remove_id);
                    audio.active_sounds.remove(&remove_id);
                })
                .expect("failed to remove source from audio input thread");

            // Remove soundscape copy.
            channels
                .soundscape
                .send(move |soundscape| {
                    soundscape.remove_source(&remove_id);
                })
                .expect("failed to remove source from soundscape thread");
        }
    }

    let plus_button_w = ui.rect_of(area.id).unwrap().w() / 2.0;
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
        .set(ids.source_editor_add_wav, ui)
        .was_clicked();

    let new_realtime = plus_button()
        .label("+ Realtime")
        .align_right_of(area.id)
        .set(ids.source_editor_add_realtime, ui)
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
        let n_channels = DEFAULT_CHANNELS;
        let duration = DEFAULT_DURATION;
        let realtime = audio::source::Realtime {
            channels: n_channels,
            duration,
        };

        // Create the Source.
        let id = sources.next_id();
        let name = format!("Source {}", id.0);
        let kind = audio::source::Kind::Realtime(realtime.clone());
        let role = Default::default();
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
        let source = project::Source { name, audio };

        // Insert the source into the map.
        sources.insert(id, source);

        // Send the source to the audio input thread.
        channels
            .audio_input
            .send(move |audio| {
                audio.sources.insert(id, realtime);
            })
            .expect("failed to send new source to audio input thread");
    }

    let area_rect = ui.rect_of(area.id).unwrap();
    let start = area_rect.y.start;
    let end = start + selected_canvas_h;
    let selected_canvas_y = ui::Range { start, end };

    widget::Canvas::new()
        .pad(PAD)
        .w_of(ids.side_menu)
        .h(selected_canvas_h)
        .y(selected_canvas_y.middle())
        .align_middle_x_of(ids.side_menu)
        .parent(area.id)
        .set(ids.source_editor_selected_canvas, ui);

    let selected_canvas_kid_area = ui.kid_area_of(ids.source_editor_selected_canvas)
        .unwrap();

    // If a source is selected, display its info.
    let id = match source_editor.selected {
        None => {
            widget::Text::new("No source selected")
                .padded_w_of(area.id, TEXT_PAD)
                .mid_top_with_margin_on(ids.source_editor_selected_canvas, TEXT_PAD)
                .font_size(SMALL_FONT_SIZE)
                .center_justify()
                .set(ids.source_editor_selected_none, ui);
            return area.id;
        }
        Some(id) => id,
    };

    for event in widget::TextBox::new(&sources[&id].name)
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
            sources.get_mut(&id).unwrap().name = string;
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

    let selected_role_index = sources[&id].audio.role.as_ref().map(role_index).unwrap_or(0);
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
                let source = sources.get_mut(&id).unwrap();
                let new_role = int_to_role(idx);
                let old_role = mem::replace(&mut source.audio.role, new_role.clone());
                match (old_role, new_role) {
                    // Don't do anything if the selection has stayed on soundscape.
                    (Some(Role::Soundscape(_)), Some(Role::Soundscape(_))) => (),

                    // If the source became a soundscape source, send it to the soundscape thread.
                    (_, Some(Role::Soundscape(_))) => {
                        let soundscape_source = soundscape::Source::from_audio_source(&source.audio)
                            .expect("source did not have soundscape role");
                        channels
                            .soundscape
                            .send(move |soundscape| {
                                soundscape.insert_source(id, soundscape_source);
                            })
                            .expect("failed to send soundscape source to soundscape thread");
                    },

                    // If it is no longer a soundscape.
                    (Some(Role::Soundscape(_)), _) => {
                        // Remove the source from the soundscape.
                        channels
                            .soundscape
                            .send(move |soundscape| {
                                soundscape.remove_source(&id);
                            })
                            .expect("failed to remove soundscape source from soundscape thread");

                        // Remove all sounds with this source from the audio output thread.
                        channels
                            .audio_output
                            .send(move |audio| {
                                audio.remove_sounds_with_source(&id);
                            })
                            .expect("failed to remove soundscape source sounds from audio output thread");
                    },

                    _ => (),
                }
            }

            _ => (),
        }
    }

    // Preview options.
    widget::Canvas::new()
        .mid_left_of(ids.source_editor_selected_canvas)
        .down_from(ids.source_editor_selected_role_list, PAD)
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
        source_id: audio::source::Id,
        source: &project::Source,
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
                        .expect("failed to remove sound from audio output thread");

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

                    // No attack or release for previews.
                    let attack_duration = Samples(0);
                    let release_duration = Samples(0);
                    let max_duration = None;
                    let position = audio::sound::Position {
                        point: preview.point.unwrap(),
                        radians: 0.0,
                    };

                    // When previewing sounds, remove the role so they play back through all
                    // speakers.
                    let mut audio = source.audio.clone();
                    audio.role = None;

                    let _handle = audio::sound::spawn_from_source(
                        sound_id,
                        source_id,
                        &audio,
                        position,
                        attack_duration,
                        release_duration,
                        should_cycle,
                        max_duration,
                        channels.frame_count.load(atomic::Ordering::Relaxed) as _,
                        &channels.wav_reader,
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
        .color(match source_editor.preview.current {
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
            id,
            &sources[&id],
            &mut source_editor.preview,
            &master.realtime_source_latency,
        );
    }

    if widget::Button::new()
        .bottom_right_of(ids.source_editor_preview_canvas)
        .label("Continuous")
        .label_font_size(SMALL_FONT_SIZE)
        .w(button_w)
        .color(match source_editor.preview.current {
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
            id,
            &sources[&id],
            &mut source_editor.preview,
            &master.realtime_source_latency,
        );
    }

    // Kind-specific data.
    let (kind_canvas_id, num_channels) = match sources.get_mut(&id).unwrap().audio.kind {
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
                channels
                    .soundscape
                    .send(move |soundscape| {
                        soundscape.update_source(&id, |source| {
                            if let audio::source::Kind::Wav(ref mut wav) = source.kind {
                                wav.should_loop = new_loop;
                            }
                        });
                    })
                    .expect("failed to send source should_loop toggle to soundscape thread");

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
                        channels
                            .soundscape
                            .send(move |soundscape| {
                                soundscape.update_source(&id, |source| {
                                    if let audio::source::Kind::Wav(ref mut wav) = source.kind {
                                        wav.playback = new_playback;
                                    }
                                });
                            })
                            .expect("failed to send source playback mode to soundscape thread");

                        // Update all audio thread copies.
                        channels
                            .audio_output
                            .send(move |audio| {
                                audio.update_sounds_with_source(&id, move |_, sound| {
                                    if let audio::source::SignalKind::Wav { ref mut playback, .. } = sound.signal.kind {
                                        *playback = new_playback;
                                    }
                                });
                            })
                            .expect("failed to send source playback mode to audio output thread");
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
                    channels
                        .audio_input
                        .send(move |audio| {
                            if let Some(realtime) = audio.sources.get_mut(&id) {
                                $update_fn(realtime);
                            }
                        })
                        .expect("failed to send realtime source update to audio input thread");

                    // Update the soundscape thread copy.
                    channels
                        .soundscape
                        .send(move |soundscape| {
                            soundscape.update_source(&id, |source| {
                                if let audio::source::Kind::Realtime(ref mut realtime) = source.kind {
                                    $update_fn(realtime);
                                }
                            });
                        })
                        .expect("failed to send realtime source update to soundscape thread");
                };
            }

            // Maximum playback duration.
            //
            // This represents:
            //
            // - The duration over which a source previewed via "One Shot" will play.
            // - The maximum playback duration of a soundscape sound using this source.
            let label = duration_label(&realtime.duration);
            let min = 0.0;
            let max = utils::HR_MS;
            for new_ms in widget::Slider::new(realtime.duration.ms(), min, max)
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
                .map(|ch| format!("Start Channel: {}", ch + 1))
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
            let mut end_channel_indices = realtime.channels.start..audio_channels.input;
            let end_channel_labels = end_channel_indices
                .clone()
                .map(|ch| format!("End Channel: {}", ch + 1))
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
        .h(COMMON_CANVAS_H)
        .w(selected_canvas_kid_area.w())
        .pad(PAD)
        .parent(ids.source_editor_selected_canvas)
        .color(color::CHARCOAL)
        .set(ids.source_editor_selected_common_canvas, ui);

    // Display the volume slider.
    widget::Text::new("VOLUME")
        .font_size(SMALL_FONT_SIZE)
        .top_left_of(ids.source_editor_selected_common_canvas)
        .set(ids.source_editor_selected_volume_text, ui);

    let volume = sources[&id].volume;
    let label = format!("{:.3}", volume);
    for new_volume in widget::Slider::new(volume, 0.0, 1.0)
        .label(&label)
        .label_font_size(SMALL_FONT_SIZE)
        .kid_area_w_of(ids.source_editor_selected_common_canvas)
        .h(SLIDER_H)
        .align_left()
        .down(PAD * 1.5)
        .color(color::DARK_GREEN)
        .set(ids.source_editor_selected_volume_slider, ui)
    {
        // Update the local copy.
        sources.get_mut(&id).unwrap().volume = new_volume;

        // Update the soundscape copy.
        channels
            .soundscape
            .send(move |soundscape| {
                soundscape.update_source(&id, |source| source.volume = new_volume);
            })
            .expect("failed to send source volume update to soundscape thread");

        // Update the audio output copies.
        channels
            .audio_output
            .send(move |audio| {
                audio.update_sounds_with_source(&id, move |_, sound| {
                    sound.volume = new_volume;
                });
            })
            .expect("failed to send source volume update to audio output thread");
    }

    // Buttons for solo and mute behaviour.
    let channel_layout_kid_area = ui.kid_area_of(ids.source_editor_selected_common_canvas)
        .unwrap();
    let button_w = channel_layout_kid_area.w() / 2.0 - PAD / 2.0;
    let toggle = |value: bool| widget::Toggle::new(value)
        .w(button_w)
        .h(ITEM_HEIGHT)
        .label_font_size(SMALL_FONT_SIZE);

    // Solo button.
    let solo = sources.soloed.contains(&id);
    for new_solo in toggle(solo)
        .label("SOLO")
        .align_left()
        .down(PAD)
        .color(color::DARK_YELLOW)
        .set(ids.source_editor_selected_solo, ui)
    {
        // If the CTRL key was down, unsolo all other sources.
        if ui.global_input().current.modifiers.contains(ui::input::keyboard::ModifierKey::CTRL) {
            // Update local copy.
            sources.soloed.clear();

            // Update audio output copy.
            channels
                .audio_output
                .send(move |audio| {
                    audio.soloed.clear();
                })
                .expect("failed to send message for clearing soloed sources to audio output thread");
        }

        // Update local copy.
        if new_solo {
            sources.soloed.insert(id);
        } else {
            sources.soloed.remove(&id);
        }

        // Update audio output copy.
        channels
            .audio_output
            .send(move |audio| {
                if new_solo {
                    audio.soloed.insert(id);
                } else {
                    audio.soloed.remove(&id);
                }
            })
            .expect("failed to send soloed sources update to audio output thread");
    }

    // Mute button.
    for new_mute in toggle(sources[&id].muted)
        .label("MUTE")
        .align_top()
        .right(PAD)
        .color(color::BLUE)
        .set(ids.source_editor_selected_mute, ui)
    {
        // Update local copy.
        sources.get_mut(&id).unwrap().muted = new_mute;

        // Update soundscape copy.
        channels
            .soundscape
            .send(move |soundscape| {
                soundscape.update_source(&id, |source| source.muted = new_mute);
            })
            .expect("failed to send muted sources update to soundscape thread");

        // Update audio output copy.
        channels
            .audio_output
            .send(move |audio| {
                audio.update_sounds_with_source(&id, move |_, sound| {
                    sound.muted = new_mute;
                });
            })
            .expect("failed to send muted sources update to audio output thread");
    }

    // Display the channel layout.
    widget::Text::new("CHANNEL LAYOUT")
        .font_size(SMALL_FONT_SIZE)
        .mid_left_of(ids.source_editor_selected_common_canvas)
        .down(PAD * 1.5)
        .set(ids.source_editor_selected_channel_layout_text, ui);

    let slider_w = button_w;
    let slider = |value, min, max| {
        widget::Slider::new(value, min, max)
            .label_font_size(SMALL_FONT_SIZE)
            .w(slider_w)
    };

    // Slider for controlling how far apart speakers should be spread.
    const MIN_SPREAD: f32 = 0.0;
    const MAX_SPREAD: f32 = 10.0;
    let mut spread = sources[&id].audio.spread.0 as f32;
    let label = format!("Spread: {:.2} metres", spread);
    for new_spread in slider(spread, MIN_SPREAD, MAX_SPREAD)
        .skew(2.0)
        .label(&label)
        .mid_left_of(ids.source_editor_selected_common_canvas)
        .down(PAD * 1.5)
        .set(ids.source_editor_selected_channel_layout_spread, ui)
    {
        spread = new_spread;
        let spread_m = Metres(spread as _);

        // Update the local copy.
        sources.get_mut(&id).unwrap().audio.spread = spread_m;

        // Update soundscape copy if it's there.
        channels
            .soundscape
            .send(move |soundscape| {
                soundscape.update_source(&id, |source| source.spread = spread_m);
            })
            .expect("failed to send source channel spread to soundscape thread");

        // Update the audio output copies.
        channels
            .audio_output
            .send(move |audio| {
                audio.update_sounds_with_source(&id, move |_, sound| {
                    sound.spread = spread_m;
                });
            })
            .expect("failed to send source channel spread to audio output thread");
    }

    // Slider for controlling how channels should be rotated.
    const MIN_RADIANS: f32 = 0.0;
    const MAX_RADIANS: f32 = std::f32::consts::PI * 2.0;
    let mut channel_radians = sources[&id].audio.channel_radians;
    let label = format!("Rotate: {:.2} radians", channel_radians);
    for new_channel_radians in slider(channel_radians, MIN_RADIANS, MAX_RADIANS)
        .label(&label)
        .mid_right_of(ids.source_editor_selected_common_canvas)
        .align_middle_y_of(ids.source_editor_selected_channel_layout_spread)
        .set(ids.source_editor_selected_channel_layout_rotation, ui)
    {
        channel_radians = new_channel_radians;

        // Update the local copy.
        sources.get_mut(&id).unwrap().audio.channel_radians = channel_radians;

        // Update the soundscape copy.
        channels
            .soundscape
            .send(move |soundscape| {
                soundscape.update_source(&id, move |source| {
                    source.channel_radians = channel_radians;
                });
            })
            .expect("failed to send source channel radians to soundscape thread");

        // Update the audio output copies.
        channels
            .audio_output
            .send(move |audio| {
                for (_, sound) in audio.sounds_mut().filter(|&(_, ref s)| s.source_id() == id) {
                    sound.channel_radians = channel_radians;
                }
            })
            .expect("failed to send source channel radians to audio output thread");
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
        .align_middle_x_of(ids.source_editor_selected_common_canvas)
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
            let channel_radians_offset = phase * MAX_RADIANS;
            let radians = (channel_radians + channel_radians_offset) as Scalar;
            let (x, y) = utils::rad_mag_to_x_y(radians, spread_circle_radius);
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

    ///////////////////
    // Role-specific //
    ///////////////////

    match sources.get_mut(&id).unwrap().audio.role.clone() {
        // For soundscape sounds, allow the user to select installations.
        Some(Role::Soundscape(soundscape)) => {
            // Destructure the soundscape roll to its fields.
            let audio::source::Soundscape {
                installations: mut source_installations,
                groups,
                occurrence_rate,
                simultaneous_sounds,
                playback_duration,
                attack_duration,
                release_duration,
                movement,
            } = soundscape;

            // A canvas on which installation selection widgets are instantiated.
            widget::Canvas::new()
                .kid_area_w_of(ids.source_editor_selected_canvas)
                .h(INSTALLATIONS_CANVAS_H)
                .align_middle_x_of(ids.source_editor_selected_canvas)
                .parent(ids.source_editor_selected_canvas)
                .down_from(ids.source_editor_selected_common_canvas, PAD)
                .pad(PAD)
                .color(color::CHARCOAL)
                .set(ids.source_editor_selected_installations_canvas, ui);

            // A header for the installations editing area.
            widget::Text::new("Installations")
                .top_left_of(ids.source_editor_selected_installations_canvas)
                .font_size(SMALL_FONT_SIZE)
                .set(ids.source_editor_selected_installations_text, ui);

            // A dropdownlist for assigning installations to the source.
            //
            // Only show installations that aren't yet assigned.
            let installations_vec = installations
                .keys()
                .filter(|inst| !source_installations.contains(inst))
                .cloned()
                .collect::<Vec<_>>();
            let installation_strs = installations_vec
                .iter()
                .map(|inst_id| &installations[inst_id].name)
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
                source_installations.insert(installation);

                // Update the local copy.
                let source = sources.get_mut(&id).unwrap();
                if let Some(Role::Soundscape(ref mut soundscape)) = source.audio.role {
                    soundscape.installations.insert(installation);
                }

                // Update the soundscape copy.
                channels
                    .soundscape
                    .send(move |soundscape| {
                        soundscape.update_source(&id, |source| {
                            source.installations.insert(installation);
                        });
                    })
                    .expect("failed to send assigned installation to source on soundscape thread");

                // Update sounds
                channels
                    .audio_output
                    .send(move |audio| {
                        for (_, sound) in audio.sounds_mut().filter(|&(_, ref s)| s.source_id() == id) {
                            if let audio::sound::Installations::Set(ref mut set) = sound.installations {
                                set.insert(installation);
                            }
                        }
                    })
                    .expect("failed to send assigned installation to sounds on audio output thread");
            }

            // A scrollable list showing each of the assigned installations.
            let mut selected_installations = source_installations.iter().cloned().collect::<Vec<_>>();
            selected_installations.sort_by(|a, b| {
                installations[&a].name.cmp(&installations[&b].name)
            });
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
                let label = &installations[&inst].name;

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
                let source = sources.get_mut(&id).unwrap();

                // Remove the local copy.
                if let Some(Role::Soundscape(ref mut soundscape)) = source.audio.role {
                    soundscape.installations.remove(&inst);
                }

                // Remove the soundscape copy.
                channels
                    .soundscape
                    .send(move |soundscape| {
                        soundscape.update_source(&id, move |source| {
                            source.installations.remove(&inst);
                        });
                    })
                    .expect("failed to remove installation from source on soundscape thread");

                // Remove the installation from sounds driven by this source on the output stream.
                channels
                    .audio_output
                    .send(move |audio| {
                        for (_, sound) in audio.sounds_mut().filter(|&(_, ref s)| s.source_id() == id) {
                            if let audio::sound::Installations::Set(ref mut set) = sound.installations {
                                set.remove(&inst);
                            }
                        }
                    })
                    .expect("failed to remove installation from source on audio output thread");
            }

            ////////////////////////////
            // SOUNDSCAPE CONSTRAINTS //
            ////////////////////////////
            //
            // TODO:
            // 1. Occurrence Rate
            // 2. Simultaneous Playback
            // 3. Duration of playback
            // 4. Assigned Groups

            // A canvas on which installation selection widgets are instantiated.
            widget::Canvas::new()
                .h(SOUNDSCAPE_CANVAS_H)
                .kid_area_w_of(ids.source_editor_selected_canvas)
                .align_middle_x_of(ids.source_editor_selected_canvas)
                .down_from(ids.source_editor_selected_installations_canvas, PAD)
                .parent(ids.source_editor_selected_canvas)
                .pad(PAD)
                .color(color::CHARCOAL)
                .set(ids.source_editor_selected_soundscape_canvas, ui);

            // A header for the installations editing area.
            widget::Text::new("SOUNDSCAPE CONSTRAINTS")
                .top_left_of(ids.source_editor_selected_soundscape_canvas)
                .font_size(SMALL_FONT_SIZE)
                .set(ids.source_editor_selected_soundscape_title, ui);

            // Shorthand for expecting a soundscape value within a source.
            fn expect_soundscape_mut<'a>(
                sources: &'a mut project::SourcesMap,
                id: &audio::source::Id,
            ) -> &'a mut audio::source::Soundscape
            {
                sources
                    .get_mut(id)
                    .unwrap()
                    .audio
                    .role
                    .as_mut()
                    .expect("no role was assigned")
                    .soundscape_mut()
                    .expect("role was not soundscape")
            }

            /////////////////////
            // Occurrence Rate //
            /////////////////////

            widget::Text::new("Occurrence Rate")
                .align_left()
                .down(PAD * 2.0)
                .font_size(SMALL_FONT_SIZE)
                .set(ids.source_editor_selected_soundscape_occurrence_rate_text, ui);

            // A range slider for constraining the occurrence rate.
            let max_hz = utils::ms_interval_to_hz(occurrence_rate.min);
            let min_hz = utils::ms_interval_to_hz(occurrence_rate.max);
            let min_hz_label = hz_label(min_hz);
            let max_hz_label = hz_label(max_hz);
            let label = format!("{} to {}", min_hz_label, max_hz_label);
            let total_min_hz = utils::ms_interval_to_hz(Ms(utils::DAY_MS));
            let total_max_hz = utils::ms_interval_to_hz(Ms(1.0));

            let range_slider = |start, end, min, max| {
                widget::RangeSlider::new(start, end, min, max)
                    .kid_area_w_of(ids.source_editor_selected_soundscape_canvas)
                    .h(SLIDER_H)
                    .label_font_size(SMALL_FONT_SIZE)
                    .color(ui::color::LIGHT_CHARCOAL)
            };

            for (edge, value) in range_slider(min_hz, max_hz, total_min_hz, total_max_hz)
                .skew(0.1)
                .align_left()
                .label(&label)
                .down(PAD * 2.0)
                .set(ids.source_editor_selected_soundscape_occurrence_rate_slider, ui)
            {
                let hz = {
                    let (unit, times_per_unit) = utils::human_readable_hz(value as _);
                    unit.times_per_unit_to_hz(times_per_unit.round())
                };

                // Update the local copy.
                let new_rate = {
                    let soundscape = expect_soundscape_mut(sources, &id);
                    match edge {
                        widget::range_slider::Edge::Start => {
                            let ms = utils::hz_to_ms_interval(hz);
                            soundscape.occurrence_rate.max = ms;
                        },
                        widget::range_slider::Edge::End => {
                            let ms = utils::hz_to_ms_interval(hz);
                            soundscape.occurrence_rate.min = ms;
                        }
                    }
                    soundscape.occurrence_rate
                };

                // Update the soundscape copy.
                channels
                    .soundscape
                    .send(move |soundscape| {
                        soundscape.update_source(&id, |source| {
                            source.occurrence_rate = new_rate;
                        });
                    })
                    .expect("failed to send updated source occurrence rate to soundscape thread");
            }

            /////////////////////////
            // Simultaneous Sounds //
            /////////////////////////

            widget::Text::new("Simultaneous Sounds")
                .align_left()
                .down(PAD * 2.0)
                .font_size(SMALL_FONT_SIZE)
                .set(ids.source_editor_selected_soundscape_simultaneous_sounds_text, ui);

            let range = simultaneous_sounds;
            let label = format!("{} to {} sounds at once", range.min, range.max);
            let total_min_num = 0.0;
            let total_max_num = 10.0;
            let min = range.min as f64;
            let max = range.max as f64;
            for (edge, value) in range_slider(min, max, total_min_num, total_max_num)
                .skew(0.5)
                .align_left()
                .label(&label)
                .down(PAD * 2.0)
                .set(ids.source_editor_selected_soundscape_simultaneous_sounds_slider, ui)
            {
                let num = value as _;

                // Update the local copy.
                let new_num = {
                    let soundscape = expect_soundscape_mut(sources, &id);
                    match edge {
                        widget::range_slider::Edge::Start => {
                            soundscape.simultaneous_sounds.min = num;
                        },
                        widget::range_slider::Edge::End => {
                            soundscape.simultaneous_sounds.max = num;
                        }
                    }
                    soundscape.simultaneous_sounds
                };

                // Update the soundscape copy.
                channels
                    .soundscape
                    .send(move |soundscape| {
                        soundscape.update_source(&id, |source| {
                            source.simultaneous_sounds = new_num;
                        });
                    })
                    .expect("failed to send source simultaenous sounds to soundscape thread");
            }

            ///////////////////////
            // Playback Duration //
            ///////////////////////

            widget::Text::new("Playback Duration")
                .align_left()
                .down(PAD * 2.0)
                .font_size(SMALL_FONT_SIZE)
                .set(ids.source_editor_selected_soundscape_playback_duration_text, ui);

            // The max duration depends on the kind of source:
            //
            // - If it is a non-looping WAV, then the max duration is the length of the WAV.
            // - If it is a looping WAV or a realtime source the max is some arbitrary limit.
            let skew = sources[&id].kind.playback_duration_skew();
            let max_duration = match sources[&id].kind {
                audio::source::Kind::Realtime(ref realtime) => realtime.duration,
                audio::source::Kind::Wav(ref wav) => match wav.should_loop {
                    true => audio::source::MAX_PLAYBACK_DURATION,
                    false => wav.duration.to_ms(audio::SAMPLE_RATE),
                }
            };
            let min_duration = Ms(0.0);
            let min_duration_ms = min_duration.ms();
            let max_duration_ms = max_duration.ms();
            let range = playback_duration;
            let label = format!("{} to {}", duration_label(&range.min), duration_label(&range.max));
            let start = range.min.ms() as f64;
            let end = range.max.ms() as f64;
            for (edge, value) in range_slider(start, end, min_duration_ms, max_duration_ms)
                .skew(skew)
                .align_left()
                .label(&label)
                .down(PAD * 2.0)
                .set(ids.source_editor_selected_soundscape_playback_duration_slider, ui)
            {
                let duration = {
                    let (unit, value) = utils::human_readable_ms(&Ms(value as _));
                    let (unit, value) = unit.to_finer_unit(value);
                    unit.to_ms(value.round())
                };

                // Update the local copy.
                let new_duration = {
                    let soundscape = expect_soundscape_mut(sources, &id);
                    match edge {
                        widget::range_slider::Edge::Start => {
                            soundscape.playback_duration.min = duration;
                        },
                        widget::range_slider::Edge::End => {
                            soundscape.playback_duration.max = duration;
                        }
                    }
                    soundscape.playback_duration
                };

                // Update the soundscape copy.
                channels
                    .soundscape
                    .send(move |soundscape| {
                        soundscape.update_source(&id, |source| {
                            source.playback_duration = new_duration;
                        });
                    })
                    .expect("failed to send new playback duration to source on soundscape thread");
            }

            /////////////////////
            // Attack Duration //
            /////////////////////

            widget::Text::new("Fade-In Duration")
                .align_left()
                .down(PAD * 2.0)
                .font_size(SMALL_FONT_SIZE)
                .set(ids.source_editor_selected_soundscape_attack_duration_text, ui);

            let min_duration = Ms(0.0);
            let max_duration = audio::source::MAX_ATTACK_DURATION;
            let min_duration_ms = min_duration.ms();
            let max_duration_ms = max_duration.ms();
            let range = attack_duration;
            let label = format!("{} to {}", duration_label(&range.min), duration_label(&range.max));
            let start = range.min.ms() as f64;
            let end = range.max.ms() as f64;
            for (edge, value) in range_slider(start, end, min_duration_ms, max_duration_ms)
                .skew(audio::source::skew::ATTACK)
                .align_left()
                .label(&label)
                .down(PAD * 2.0)
                .set(ids.source_editor_selected_soundscape_attack_duration_slider, ui)
            {
                let duration = {
                    let (unit, value) = utils::human_readable_ms(&Ms(value as _));
                    let (unit, value) = unit.to_finer_unit(value);
                    unit.to_ms(value.round())
                };

                // Update the local copy.
                let new_duration = {
                    let soundscape = expect_soundscape_mut(sources, &id);
                    match edge {
                        widget::range_slider::Edge::Start => {
                            soundscape.attack_duration.min = duration;
                        },
                        widget::range_slider::Edge::End => {
                            soundscape.attack_duration.max = duration;
                        }
                    }
                    soundscape.attack_duration
                };

                // Update the soundscape copy.
                channels
                    .soundscape
                    .send(move |soundscape| {
                        soundscape.update_source(&id, |source| {
                            source.attack_duration = new_duration;
                        });
                    })
                    .expect("failed to send source attack duration to soundscape thread");
            }

            //////////////////////
            // Release Duration //
            //////////////////////

            widget::Text::new("Fade-Out Duration")
                .align_left()
                .down(PAD * 2.0)
                .font_size(SMALL_FONT_SIZE)
                .set(ids.source_editor_selected_soundscape_release_duration_text, ui);

            let min_duration = Ms(0.0);
            let max_duration = audio::source::MAX_RELEASE_DURATION;
            let min_duration_ms = min_duration.ms();
            let max_duration_ms = max_duration.ms();
            let range = release_duration;
            let label = format!("{} to {}", duration_label(&range.min), duration_label(&range.max));
            let start = range.min.ms();
            let end = range.max.ms();
            for (edge, value) in range_slider(start, end, min_duration_ms, max_duration_ms)
                .skew(audio::source::skew::RELEASE)
                .align_left()
                .label(&label)
                .down(PAD * 2.0)
                .set(ids.source_editor_selected_soundscape_release_duration_slider, ui)
            {
                let duration = {
                    let (unit, value) = utils::human_readable_ms(&Ms(value as _));
                    let (unit, value) = unit.to_finer_unit(value);
                    unit.to_ms(value.round())
                };

                // Update the local copy.
                let new_duration = {
                    let soundscape = expect_soundscape_mut(sources, &id);
                    match edge {
                        widget::range_slider::Edge::Start => {
                            soundscape.release_duration.min = duration;
                        },
                        widget::range_slider::Edge::End => {
                            soundscape.release_duration.max = duration;
                        }
                    }
                    soundscape.release_duration
                };

                // Update the soundscape copy.
                channels
                    .soundscape
                    .send(move |soundscape| {
                        soundscape.update_source(&id, |source| {
                            source.release_duration = new_duration;
                        });
                    })
                    .expect("failed to send source release duration to soundscape thread");
            }

            //////////////////////////////////
            // Soundscape Group Assignments //
            //////////////////////////////////

            widget::Text::new("Soundscape Groups")
                .align_left()
                .down(PAD * 2.0)
                .font_size(SMALL_FONT_SIZE)
                .set(ids.source_editor_selected_soundscape_groups_text, ui);

            let mut groups_vec: Vec<_> = soundscape_groups.iter().collect();
            groups_vec.sort_by(|a, b| a.1.name.cmp(&b.1.name));
            let (mut events, scrollbar) = widget::ListSelect::multiple(groups_vec.len())
                .item_size(ITEM_HEIGHT)
                .h(SOUNDSCAPE_GROUP_LIST_H)
                .down(PAD * 2.0)
                .kid_area_w_of(ids.source_editor_selected_soundscape_canvas)
                .scrollbar_next_to()
                .scrollbar_color(color::LIGHT_CHARCOAL)
                .set(ids.source_editor_selected_soundscape_groups_list, ui);

            let is_selected = |idx: usize| groups.contains(&groups_vec[idx].0);
            while let Some(event) = events.next(ui, &is_selected) {
                use self::ui::widget::list_select::Event;
                match event {
                    // Instantiate a button for each group.
                    Event::Item(item) => {
                        let selected = is_selected(item.i);
                        let (&group_id, _) = groups_vec[item.i];
                        let soundscape = expect_soundscape_mut(sources, &id);
                        let color = if selected { ui::color::BLUE } else { ui::color::BLACK };
                        let button = widget::Button::new()
                            .label(&groups_vec[item.i].1.name)
                            .label_font_size(SMALL_FONT_SIZE)
                            .color(color);

                        for _click in item.set(button, ui) {
                            // Update the local copies.
                            if selected {
                                soundscape.groups.remove(&group_id);
                            } else {
                                soundscape.groups.insert(group_id);
                            }

                            // Update the soundscape copy.
                            channels
                                .soundscape
                                .send(move |soundscape| {
                                    soundscape.update_source(&id, move |source| {
                                        if selected {
                                            source.groups.remove(&group_id);
                                        } else {
                                            source.groups.insert(group_id);
                                        }
                                    });
                                })
                                .expect("failed to send source soundscape group update to soundscape thread");
                        }

                    }
                    _ => (),
                }
            }

            if let Some(scrollbar) = scrollbar {
                scrollbar.set(ui);
            }

            /////////////////////////
            // Soundscape Movement //
            /////////////////////////

            widget::Text::new("Movement")
                .mid_left_of(ids.source_editor_selected_soundscape_canvas)
                .down(PAD * 2.0)
                .font_size(SMALL_FONT_SIZE)
                .set(ids.source_editor_selected_soundscape_movement_text, ui);

            // A rightward flowing list for the movement kinds.
            let canvas_kid_area = ui.kid_area_of(ids.source_editor_selected_soundscape_canvas).unwrap();
            let n_items = audio::source::Movement::VARIANT_COUNT;
            let item_w = canvas_kid_area.w() / n_items as Scalar;
            let (mut events, _scrollbar) = widget::ListSelect::single(n_items)
                .flow_right()
                .down(PAD * 2.0)
                .align_left()
                .w(canvas_kid_area.w())
                .h(BUTTON_H)
                .item_size(item_w)
                .set(ids.source_editor_selected_soundscape_movement_mode_list, ui);
            let selected_index = movement.to_index();
            let is_selected = |i| i == selected_index;
            while let Some(event) = events.next(ui, &is_selected) {
                use nannou::ui::widget::list_select::Event;
                match event {
                    Event::Item(item) => {
                        let index = item.i;
                        let selected = is_selected(index);
                        let color = if selected { color::BLUE } else { color::DARK_CHARCOAL };
                        let label = audio::source::Movement::label_from_index(index);
                        let button = widget::Button::new()
                            .label(&label)
                            .label_font_size(SMALL_FONT_SIZE)
                            .color(color);

                        // If the button was clicked.
                        for _click in item.set(button, ui) {
                            let movement = match audio::source::Movement::from_index(index) {
                                None => continue,
                                Some(m) => m,
                            };

                            // Update local copy.
                            let soundscape = expect_soundscape_mut(sources, &id);
                            soundscape.movement = movement.clone();

                            // Update the soundsape thread copy.
                            channels
                                .soundscape
                                .send(move |soundscape| {
                                    // Update the source and all associated active sounds.
                                    soundscape.update_source_movement(&id, &movement);
                                })
                                .expect("could not update movement field on soundscape thread");
                        }
                    },
                    _ => (),
                }
            }

            // Depending on the selected movement, display the relevant widgets.
            let generative = match movement {
                audio::source::Movement::Fixed(position) => {

                    /////////////////////
                    // POSITION XY PAD //
                    /////////////////////

                    let x = position.x;
                    let y = position.y;
                    let w = canvas_kid_area.w();
                    let h = w;
                    for (new_x, new_y) in widget::XYPad::new(x, 0.0, 1.0, y, 0.0, 1.0)
                        .label("Installation Position")
                        .value_font_size(SMALL_FONT_SIZE)
                        .w(w)
                        .h(h)
                        .down_from(ids.source_editor_selected_soundscape_movement_mode_list, PAD)
                        .align_left_of(ids.source_editor_selected_soundscape_movement_mode_list)
                        .color(ui::color::DARK_CHARCOAL)
                        .set(ids.source_editor_selected_soundscape_movement_fixed_point, ui)
                    {
                        // Update the local copy.
                        let soundscape = expect_soundscape_mut(sources, &id);
                        let point = pt2(new_x, new_y);
                        let movement = audio::source::Movement::Fixed(point);
                        soundscape.movement = movement.clone();

                        // Update the soundsape thread copy.
                        channels
                            .soundscape
                            .send(move |soundscape| {
                                // Update the source and all active sounds that use the given source.
                                soundscape.update_source_movement(&id, &movement);
                            })
                            .expect("could not update movement field on soundscape thread");
                    }

                    return area.id;
                },
                audio::source::Movement::Generative(generative) => generative,
            };

            ////////////////////////////////////
            // Generative Soundscape Movement //
            ////////////////////////////////////

            // A rightward flowing list for the generative kinds.
            let n_items = audio::source::movement::Generative::VARIANT_COUNT;
            let item_w = canvas_kid_area.w() / n_items as Scalar;
            let (mut events, _scrollbar) = widget::ListSelect::single(n_items)
                .flow_right()
                .down_from(ids.source_editor_selected_soundscape_movement_mode_list, PAD)
                .align_left_of(ids.source_editor_selected_soundscape_movement_mode_list)
                .w(canvas_kid_area.w())
                .h(BUTTON_H)
                .item_size(item_w)
                .set(ids.source_editor_selected_soundscape_movement_generative_list, ui);
            let selected_index = generative.to_index();
            let is_selected = |i| i == selected_index;
            while let Some(event) = events.next(ui, &is_selected) {
                use nannou::ui::widget::list_select::Event;
                match event {
                    Event::Item(item) => {
                        let index = item.i;
                        let selected = is_selected(index);
                        let color = if selected { color::BLUE } else { color::DARK_CHARCOAL };
                        let label = audio::source::movement::Generative::label_from_index(index);
                        let button = widget::Button::new()
                            .label(&label)
                            .label_font_size(SMALL_FONT_SIZE)
                            .color(color);

                        // If the button was clicked.
                        for _click in item.set(button, ui) {
                            let gen = match audio::source::movement::Generative::from_index(index) {
                                None => continue,
                                Some(m) => m,
                            };

                            // Update local copy.
                            let soundscape = expect_soundscape_mut(sources, &id);
                            let movement = audio::source::Movement::Generative(gen);
                            soundscape.movement = movement.clone();

                            // Update the soundsape thread copy.
                            channels
                                .soundscape
                                .send(move |soundscape| {
                                    // Update the source and all active sounds that use the given source.
                                    soundscape.update_source_movement(&id, &movement);
                                })
                                .expect("could not update movement field on soundscape thread");
                        }
                    },
                    _ => (),
                }
            }

            // Depending on the selected generative movement, display the relevant widgets.
            match generative {
                // Agent-specific widgets.
                audio::source::movement::Generative::Agent(mut agent) => {
                    /////////////////
                    // Directional //
                    /////////////////

                    let on_off = if agent.directional { "ON" } else { "OFF" };
                    let label = format!("Directional: {}", on_off);
                    for new_directional in widget::Toggle::new(agent.directional)
                        .label(&label)
                        .label_font_size(SMALL_FONT_SIZE)
                        .mid_left_of(ids.source_editor_selected_soundscape_canvas)
                        .down(PAD * 2.0)
                        .h(ITEM_HEIGHT)
                        .w(canvas_kid_area.w())
                        .color(ui::color::LIGHT_CHARCOAL)
                        .set(ids.source_editor_selected_soundscape_movement_agent_directional, ui)
                    {
                        // Update local copy.
                        agent.directional = new_directional;
                        let generative = audio::source::movement::Generative::Agent(agent.clone());
                        let movement = audio::source::Movement::Generative(generative);
                        let soundscape = expect_soundscape_mut(sources, &id);
                        soundscape.movement = movement.clone();

                        // Update the soundsape thread copy.
                        channels
                            .soundscape
                            .send(move |soundscape| {
                                // Update all active sounds.
                                soundscape.update_active_sounds_with_source(id, |_, sound| {
                                    let gen = match sound.movement {
                                        soundscape::Movement::Generative(ref mut gen) => gen,
                                        _ => return,
                                    };
                                    let agent = match *gen {
                                        soundscape::movement::Generative::Agent(ref mut agent) => agent,
                                        _ => return,
                                    };
                                    agent.directional = new_directional;
                                });

                                // Update the source.
                                soundscape.update_source(&id, |source| {
                                    let gen = match source.movement {
                                        audio::source::Movement::Generative(ref mut gen) => gen,
                                        _ => return,
                                    };
                                    let agent = match *gen {
                                        audio::source::movement::Generative::Agent(ref mut agent) => agent,
                                        _ => return,
                                    };
                                    agent.directional = new_directional;
                                });
                            })
                            .expect("failed to send source movement update to soundscape thread");
                    }

                    ///////////////
                    // Max Speed //
                    ///////////////

                    widget::Text::new("Max Speed")
                        .mid_left_of(ids.source_editor_selected_soundscape_canvas)
                        .down(PAD * 2.0)
                        .font_size(SMALL_FONT_SIZE)
                        .set(ids.source_editor_selected_soundscape_movement_agent_max_speed_text, ui);

                    let min = agent.max_speed.min;
                    let max = agent.max_speed.max;
                    let total_min = 0.0;
                    let total_max = audio::source::movement::MAX_SPEED;
                    let label = format!("{:.2} to {:.2} metres per second", min, max);
                    for (edge, value) in range_slider(min, max, total_min, total_max)
                        .skew(audio::source::movement::MAX_SPEED_SKEW)
                        .align_left()
                        .label(&label)
                        .down(PAD * 2.0)
                        .set(ids.source_editor_selected_soundscape_movement_agent_max_speed_slider, ui)
                    {
                        match edge {
                            widget::range_slider::Edge::Start => agent.max_speed.min = value,
                            widget::range_slider::Edge::End => agent.max_speed.max = value,
                        }

                        // Update local copy.
                        let generative = audio::source::movement::Generative::Agent(agent.clone());
                        let movement = audio::source::Movement::Generative(generative);
                        let soundscape = expect_soundscape_mut(sources, &id);
                        soundscape.movement = movement.clone();

                        // Update the soundsape thread copy.
                        let new_max_speed = agent.max_speed;
                        channels
                            .soundscape
                            .send(move |soundscape| {
                                // Update all active sounds.
                                soundscape.update_active_sounds_with_source(id, |_, sound| {
                                    let gen = match sound.movement {
                                        soundscape::Movement::Generative(ref mut gen) => gen,
                                        _ => return,
                                    };
                                    let agent = match *gen {
                                        soundscape::movement::Generative::Agent(ref mut agent) => agent,
                                        _ => return,
                                    };
                                    agent.max_speed = new_max_speed.clamp(agent.max_speed);
                                });

                                // Update the source.
                                soundscape.update_source(&id, |source| {
                                    let gen = match source.movement {
                                        audio::source::Movement::Generative(ref mut gen) => gen,
                                        _ => return,
                                    };
                                    let agent = match *gen {
                                        audio::source::movement::Generative::Agent(ref mut agent) => agent,
                                        _ => return,
                                    };
                                    agent.max_speed = new_max_speed;
                                });
                            })
                            .expect("failed to send movement update to soundscape thread");
                    }

                    ///////////////
                    // Max Force //
                    ///////////////

                    widget::Text::new("Max Force")
                        .mid_left_of(ids.source_editor_selected_soundscape_canvas)
                        .down(PAD * 2.0)
                        .font_size(SMALL_FONT_SIZE)
                        .set(ids.source_editor_selected_soundscape_movement_agent_max_force_text, ui);

                    let min = agent.max_force.min;
                    let max = agent.max_force.max;
                    let total_min = 0.0;
                    let total_max = audio::source::movement::MAX_FORCE;
                    let label = format!("{:.2} to {:.2} metres per second squared", min, max);
                    for (edge, value) in range_slider(min, max, total_min, total_max)
                        .skew(audio::source::movement::MAX_FORCE_SKEW)
                        .align_left()
                        .label(&label)
                        .down(PAD * 2.0)
                        .set(ids.source_editor_selected_soundscape_movement_agent_max_force_slider, ui)
                    {
                        match edge {
                            widget::range_slider::Edge::Start => agent.max_force.min = value,
                            widget::range_slider::Edge::End => agent.max_force.max = value,
                        }

                        // Update local copy.
                        let soundscape = expect_soundscape_mut(sources, &id);
                        let generative = audio::source::movement::Generative::Agent(agent.clone());
                        let movement = audio::source::Movement::Generative(generative);
                        soundscape.movement = movement.clone();

                        // Update the soundsape thread copy.
                        let new_max_force = agent.max_force;
                        channels
                            .soundscape
                            .send(move |soundscape| {
                                // Update all active sounds.
                                soundscape.update_active_sounds_with_source(id, |_, sound| {
                                    let gen = match sound.movement {
                                        soundscape::Movement::Generative(ref mut gen) => gen,
                                        _ => return,
                                    };
                                    let agent = match *gen {
                                        soundscape::movement::Generative::Agent(ref mut agent) => agent,
                                        _ => return,
                                    };
                                    agent.max_force = new_max_force.clamp(agent.max_force);
                                });

                                // Update the source.
                                soundscape.update_source(&id, |source| {
                                    let gen = match source.movement {
                                        audio::source::Movement::Generative(ref mut gen) => gen,
                                        _ => return,
                                    };
                                    let agent = match *gen {
                                        audio::source::movement::Generative::Agent(ref mut agent) => agent,
                                        _ => return,
                                    };
                                    agent.max_force = new_max_force;
                                });
                            })
                            .expect("failed to send movement update to soundscape thread");
                    }

                    //////////////////
                    // Max Rotation //
                    //////////////////

                    widget::Text::new("Max Rotation")
                        .mid_left_of(ids.source_editor_selected_soundscape_canvas)
                        .down(PAD * 2.0)
                        .font_size(SMALL_FONT_SIZE)
                        .set(ids.source_editor_selected_soundscape_movement_agent_max_rotation_text, ui);

                    let min = agent.max_rotation.min;
                    let max = agent.max_rotation.max;
                    let total_min = 0.0;
                    let total_max = audio::source::movement::MAX_ROTATION;
                    let label = format!("{:.2} to {:.2} radians per second", min, max);
                    for (edge, value) in range_slider(min, max, total_min, total_max)
                        .skew(audio::source::movement::MAX_ROTATION_SKEW)
                        .align_left()
                        .label(&label)
                        .down(PAD * 2.0)
                        .set(ids.source_editor_selected_soundscape_movement_agent_max_rotation_slider, ui)
                    {
                        match edge {
                            widget::range_slider::Edge::Start => agent.max_rotation.min = value,
                            widget::range_slider::Edge::End => agent.max_rotation.max = value,
                        }

                        // Update local copy.
                        let soundscape = expect_soundscape_mut(sources, &id);
                        let generative = audio::source::movement::Generative::Agent(agent.clone());
                        let movement = audio::source::Movement::Generative(generative);
                        soundscape.movement = movement.clone();

                        // Update the soundsape thread copy.
                        let new_max_rotation = agent.max_rotation;
                        channels
                            .soundscape
                            .send(move |soundscape| {
                                // Update all active sounds.
                                soundscape.update_active_sounds_with_source(id, |_, sound| {
                                    let gen = match sound.movement {
                                        soundscape::Movement::Generative(ref mut gen) => gen,
                                        _ => return,
                                    };
                                    let agent = match *gen {
                                        soundscape::movement::Generative::Agent(ref mut agent) => agent,
                                        _ => return,
                                    };
                                    agent.max_rotation = new_max_rotation.clamp(agent.max_rotation);
                                });

                                // Update the source.
                                soundscape.update_source(&id, |source| {
                                    let gen = match source.movement {
                                        audio::source::Movement::Generative(ref mut gen) => gen,
                                        _ => return,
                                    };
                                    let agent = match *gen {
                                        audio::source::movement::Generative::Agent(ref mut agent) => agent,
                                        _ => return,
                                    };
                                    agent.max_rotation = new_max_rotation;
                                });
                            })
                            .expect("failed to send movement update to soundscape thread");
                    }
                },

                // Ngon-specific widgets.
                audio::source::movement::Generative::Ngon(mut ngon) => {

                    //////////////
                    // Vertices //
                    //////////////

                    widget::Text::new("Vertices")
                        .mid_left_of(ids.source_editor_selected_soundscape_canvas)
                        .down(PAD * 2.0)
                        .font_size(SMALL_FONT_SIZE)
                        .set(ids.source_editor_selected_soundscape_movement_ngon_vertices_text, ui);

                    let min = ngon.vertices.min as f64;
                    let max = ngon.vertices.max as f64;
                    let total_min = 0.0;
                    let total_max = audio::source::movement::MAX_VERTICES as f64;
                    let label = format!("{} to {} vertices", min, max);
                    for (edge, value) in range_slider(min, max, total_min, total_max)
                        .skew(audio::source::movement::VERTICES_SKEW)
                        .align_left()
                        .label(&label)
                        .down(PAD * 2.0)
                        .set(ids.source_editor_selected_soundscape_movement_ngon_vertices_slider, ui)
                    {
                        let value = value as usize;
                        match edge {
                            widget::range_slider::Edge::Start => ngon.vertices.min = value,
                            widget::range_slider::Edge::End => ngon.vertices.max = value,
                        }

                        // Update local copy.
                        let soundscape = expect_soundscape_mut(sources, &id);
                        let generative = audio::source::movement::Generative::Ngon(ngon.clone());
                        let movement = audio::source::Movement::Generative(generative);
                        soundscape.movement = movement.clone();

                        // Update the soundsape thread copy.
                        let new_vertices = ngon.vertices;
                        channels
                            .soundscape
                            .send(move |soundscape| {
                                // Update all active sounds.
                                soundscape.update_active_sounds_with_source(id, |_, sound| {
                                    let gen = match sound.movement {
                                        soundscape::Movement::Generative(ref mut gen) => gen,
                                        _ => return,
                                    };
                                    let ngon = match *gen {
                                        soundscape::movement::Generative::Ngon(ref mut ngon) => ngon,
                                        _ => return,
                                    };
                                    ngon.vertices = new_vertices.clamp(ngon.vertices);
                                });

                                // Update the source.
                                soundscape.update_source(&id, |source| {
                                    let gen = match source.movement {
                                        audio::source::Movement::Generative(ref mut gen) => gen,
                                        _ => return,
                                    };
                                    let ngon = match *gen {
                                        audio::source::movement::Generative::Ngon(ref mut ngon) => ngon,
                                        _ => return,
                                    };
                                    ngon.vertices = new_vertices;
                                });
                            })
                            .expect("failed to send movement update to soundscape thread");
                    }

                    ////////////////
                    // Nth Vertex //
                    ////////////////

                    widget::Text::new("Step")
                        .mid_left_of(ids.source_editor_selected_soundscape_canvas)
                        .down(PAD * 2.0)
                        .font_size(SMALL_FONT_SIZE)
                        .set(ids.source_editor_selected_soundscape_movement_ngon_step_text, ui);

                    let min = ngon.nth.min as f64;
                    let max = ngon.nth.max as f64;
                    let total_min = 0.0;
                    let total_max = ngon.vertices.max as f64;
                    let label = format!("{} to {} vertices", min, max);
                    for (edge, value) in range_slider(min, max, total_min, total_max)
                        .skew(audio::source::movement::NTH_SKEW)
                        .align_left()
                        .label(&label)
                        .down(PAD * 2.0)
                        .set(ids.source_editor_selected_soundscape_movement_ngon_step_slider, ui)
                    {
                        let value = value as usize;
                        match edge {
                            widget::range_slider::Edge::Start => ngon.nth.min = value,
                            widget::range_slider::Edge::End => ngon.nth.max = value,
                        }

                        // Update local copy.
                        let soundscape = expect_soundscape_mut(sources, &id);
                        let generative = audio::source::movement::Generative::Ngon(ngon.clone());
                        let movement = audio::source::Movement::Generative(generative);
                        soundscape.movement = movement.clone();

                        // Update the soundsape thread copy.
                        let new_nth = ngon.nth;
                        channels
                            .soundscape
                            .send(move |soundscape| {
                                // Update all active sounds.
                                soundscape.update_active_sounds_with_source(id, |_, sound| {
                                    let gen = match sound.movement {
                                        soundscape::Movement::Generative(ref mut gen) => gen,
                                        _ => return,
                                    };
                                    let ngon = match *gen {
                                        soundscape::movement::Generative::Ngon(ref mut ngon) => ngon,
                                        _ => return,
                                    };
                                    ngon.nth = new_nth.clamp(ngon.nth);
                                });

                                // Update the source.
                                soundscape.update_source(&id, |source| {
                                    let gen = match source.movement {
                                        audio::source::Movement::Generative(ref mut gen) => gen,
                                        _ => return,
                                    };
                                    let ngon = match *gen {
                                        audio::source::movement::Generative::Ngon(ref mut ngon) => ngon,
                                        _ => return,
                                    };
                                    ngon.nth = new_nth;
                                });
                            })
                            .expect("failed to send movement update to soundscape thread");
                    }


                    ////////////////
                    // Dimensions //
                    ////////////////

                    widget::Text::new("Normalised Dimensions")
                        .mid_left_of(ids.source_editor_selected_soundscape_canvas)
                        .down(PAD * 2.0)
                        .font_size(SMALL_FONT_SIZE)
                        .set(ids.source_editor_selected_soundscape_movement_ngon_dimensions_text, ui);

                    let slider = |value, min, max| {
                        widget::Slider::new(value, min, max)
                            .h(SLIDER_H)
                            .w(canvas_kid_area.w())
                            .label_font_size(SMALL_FONT_SIZE)
                            .color(ui::color::LIGHT_CHARCOAL)
                    };

                    ///////////
                    // Width //
                    ///////////

                    let label = format!("{:.2}% of installation width", ngon.normalised_dimensions.x * 100.0);
                    for new_width in slider(ngon.normalised_dimensions.x, 0.0, 1.0)
                        .align_left()
                        .label(&label)
                        .down(PAD * 2.0)
                        .set(ids.source_editor_selected_soundscape_movement_ngon_width_slider, ui)
                    {
                        // Update local copy.
                        let soundscape = expect_soundscape_mut(sources, &id);
                        if let audio::source::Movement::Generative(ref mut gen) = soundscape.movement {
                            if let audio::source::movement::Generative::Ngon(ref mut ngon) = *gen {
                                ngon.normalised_dimensions.x = new_width;
                            }
                        }

                        // Update the soundsape thread copy.
                        channels
                            .soundscape
                            .send(move |soundscape| {
                                // Update all active sounds.
                                soundscape.update_active_sounds_with_source(id, |_, sound| {
                                    let gen = match sound.movement {
                                        soundscape::Movement::Generative(ref mut gen) => gen,
                                        _ => return,
                                    };
                                    let ngon = match *gen {
                                        soundscape::movement::Generative::Ngon(ref mut ngon) => ngon,
                                        _ => return,
                                    };
                                    ngon.normalised_dimensions.x = new_width;
                                });
                                // Update the source.
                                soundscape.update_source(&id, |source| {
                                    let gen = match source.movement {
                                        audio::source::Movement::Generative(ref mut gen) => gen,
                                        _ => return,
                                    };
                                    let ngon = match *gen {
                                        audio::source::movement::Generative::Ngon(ref mut ngon) => ngon,
                                        _ => return,
                                    };
                                    ngon.normalised_dimensions.x = new_width;
                                });
                            })
                            .expect("failed to send movement update to soundscape thread");
                    }

                    ////////////
                    // Height //
                    ////////////

                    let label = format!("{:.2}% of installation height", ngon.normalised_dimensions.y * 100.0);
                    for new_height in slider(ngon.normalised_dimensions.y, 0.0, 1.0)
                        .align_left()
                        .label(&label)
                        .down(PAD)
                        .set(ids.source_editor_selected_soundscape_movement_ngon_height_slider, ui)
                    {
                        // Update local copy.
                        let soundscape = expect_soundscape_mut(sources, &id);
                        if let audio::source::Movement::Generative(ref mut gen) = soundscape.movement {
                            if let audio::source::movement::Generative::Ngon(ref mut ngon) = *gen {
                                ngon.normalised_dimensions.y = new_height;
                            }
                        }

                        // Update the soundsape thread copy.
                        channels
                            .soundscape
                            .send(move |soundscape| {
                                // Update all active sounds.
                                soundscape.update_active_sounds_with_source(id, |_, sound| {
                                    let gen = match sound.movement {
                                        soundscape::Movement::Generative(ref mut gen) => gen,
                                        _ => return,
                                    };
                                    let ngon = match *gen {
                                        soundscape::movement::Generative::Ngon(ref mut ngon) => ngon,
                                        _ => return,
                                    };
                                    ngon.normalised_dimensions.y = new_height;
                                });
                                // Update the source.
                                soundscape.update_source(&id, |source| {
                                    let gen = match source.movement {
                                        audio::source::Movement::Generative(ref mut gen) => gen,
                                        _ => return,
                                    };
                                    let ngon = match *gen {
                                        audio::source::movement::Generative::Ngon(ref mut ngon) => ngon,
                                        _ => return,
                                    };
                                    ngon.normalised_dimensions.y = new_height;
                                });
                            })
                            .expect("failed to send movement update to soundscape thread");
                    }

                    ////////////////////
                    // Radians Offset //
                    ////////////////////

                    widget::Text::new("Rotation Offset")
                        .mid_left_of(ids.source_editor_selected_soundscape_canvas)
                        .down(PAD * 2.0)
                        .font_size(SMALL_FONT_SIZE)
                        .set(ids.source_editor_selected_soundscape_movement_ngon_radians_text, ui);

                    let min = ngon.radians_offset.min;
                    let max = ngon.radians_offset.max;
                    let total_min = 0.0;
                    let total_max = audio::source::movement::MAX_RADIANS_OFFSET;
                    let label = format!("{:.2} to {:.2} radians", min, max);
                    for (edge, value) in range_slider(min, max, total_min, total_max)
                        .align_left()
                        .label(&label)
                        .down(PAD * 2.0)
                        .set(ids.source_editor_selected_soundscape_movement_ngon_radians_slider, ui)
                    {
                        match edge {
                            widget::range_slider::Edge::Start => ngon.radians_offset.min = value,
                            widget::range_slider::Edge::End => ngon.radians_offset.max = value,
                        }

                        // Update local copy.
                        let soundscape = expect_soundscape_mut(sources, &id);
                        let generative = audio::source::movement::Generative::Ngon(ngon.clone());
                        let movement = audio::source::Movement::Generative(generative);
                        soundscape.movement = movement.clone();

                        // Update the soundsape thread copy.
                        let new_radians_offset = ngon.radians_offset;
                        channels
                            .soundscape
                            .send(move |soundscape| {
                                // Update all active sounds.
                                soundscape.update_active_sounds_with_source(id, |_, sound| {
                                    let gen = match sound.movement {
                                        soundscape::Movement::Generative(ref mut gen) => gen,
                                        _ => return,
                                    };
                                    let ngon = match *gen {
                                        soundscape::movement::Generative::Ngon(ref mut ngon) => ngon,
                                        _ => return,
                                    };
                                    ngon.radians_offset = new_radians_offset.clamp(ngon.radians_offset);
                                });

                                // Update the source.
                                soundscape.update_source(&id, |source| {
                                    let gen = match source.movement {
                                        audio::source::Movement::Generative(ref mut gen) => gen,
                                        _ => return,
                                    };
                                    let ngon = match *gen {
                                        audio::source::movement::Generative::Ngon(ref mut ngon) => ngon,
                                        _ => return,
                                    };
                                    ngon.radians_offset = new_radians_offset;
                                });
                            })
                            .expect("failed to send movement update to soundscape thread");
                    }

                    ///////////
                    // Speed //
                    ///////////

                    widget::Text::new("Speed")
                        .mid_left_of(ids.source_editor_selected_soundscape_canvas)
                        .down(PAD * 2.0)
                        .font_size(SMALL_FONT_SIZE)
                        .set(ids.source_editor_selected_soundscape_movement_ngon_speed_text, ui);

                    let min = ngon.speed.min;
                    let max = ngon.speed.max;
                    let total_min = 0.0;
                    let total_max = audio::source::movement::MAX_SPEED;
                    let label = format!("{:.2} to {:.2} metres per second", min, max);
                    for (edge, value) in range_slider(min, max, total_min, total_max)
                        .skew(audio::source::movement::MAX_SPEED_SKEW)
                        .align_left()
                        .label(&label)
                        .down(PAD * 2.0)
                        .set(ids.source_editor_selected_soundscape_movement_ngon_speed_slider, ui)
                    {
                        match edge {
                            widget::range_slider::Edge::Start => ngon.speed.min = value,
                            widget::range_slider::Edge::End => ngon.speed.max = value,
                        }

                        // Update local copy.
                        let soundscape = expect_soundscape_mut(sources, &id);
                        let generative = audio::source::movement::Generative::Ngon(ngon.clone());
                        let movement = audio::source::Movement::Generative(generative);
                        soundscape.movement = movement.clone();

                        // Update the soundsape thread copy.
                        let new_speed = ngon.speed;
                        channels
                            .soundscape
                            .send(move |soundscape| {
                                // Update all active sounds.
                                soundscape.update_active_sounds_with_source(id, |_, sound| {
                                    let gen = match sound.movement {
                                        soundscape::Movement::Generative(ref mut gen) => gen,
                                        _ => return,
                                    };
                                    let ngon = match *gen {
                                        soundscape::movement::Generative::Ngon(ref mut ngon) => ngon,
                                        _ => return,
                                    };
                                    ngon.speed = new_speed.clamp(ngon.speed);
                                });

                                // Update the source.
                                soundscape.update_source(&id, |source| {
                                    let gen = match source.movement {
                                        audio::source::Movement::Generative(ref mut gen) => gen,
                                        _ => return,
                                    };
                                    let ngon = match *gen {
                                        audio::source::movement::Generative::Ngon(ref mut ngon) => ngon,
                                        _ => return,
                                    };
                                    ngon.speed = new_speed;
                                });
                            })
                            .expect("failed to send movement update to soundscape thread");
                    }
                },
            }
        },

        // For interactive sounds, allow the user specify the location. NOTE: Option - just work
        // this sound out from the location of the speakers?
        Some(Role::Interactive) => {
        },

        // For scribbles, allow a specific location from which the speaking appears.
        Some(Role::Scribbles) => {
        },

        // If it has no role, no specific stuff to be done.
        None => (),
    }

    area.id
}
