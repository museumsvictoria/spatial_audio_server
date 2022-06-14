use crate::audio;
use crate::gui::{collapsible_area, Gui, ProjectState};
use crate::gui::{DARK_A, ITEM_HEIGHT, SMALL_FONT_SIZE};
use crate::project::{self, Project};
use crate::soundscape;
use conrod_core::{Borderable, Colorable, Labelable, Positionable, Sizeable, Widget};
use nannou_conrod as ui;
use ui::{FontSize, Scalar};

/// Runtime state related to the speaker editor GUI panel.
#[derive(Default)]
pub struct SpeakerEditor {
    /// The index of the selected speaker within the project.
    pub selected: Option<usize>,
}

/// Convert the given map into a sorted list of speaker Id.
pub fn sorted_speakers_vec(speakers: &project::Speakers) -> Vec<audio::speaker::Id> {
    let mut speakers_vec: Vec<_> = speakers.keys().cloned().collect();
    speakers_vec.sort_by(|a, b| a.0.cmp(&b.0));
    speakers_vec
}

// Instantiate the sidebar speaker editor widgets.
pub fn set(
    last_area_id: ui::widget::Id,
    gui: &mut Gui,
    project: &mut Project,
    project_state: &mut ProjectState,
) -> ui::widget::Id {
    let Gui {
        ref mut ui,
        ref mut state,
        ref ids,
        ref channels,
        ..
    } = *gui;

    let Project {
        state:
            project::State {
                ref camera,
                ref installations,
                ref mut speakers,
                ..
            },
        ..
    } = *project;

    let ProjectState {
        ref mut speaker_editor,
        ..
    } = *project_state;

    let is_open = state.is_open.speaker_editor;
    const LIST_HEIGHT: Scalar = 140.0;
    const PAD: Scalar = 6.0;
    const TEXT_PAD: Scalar = 20.0;
    const INSTALLATION_LIST_H: Scalar = ITEM_HEIGHT * 3.0;
    const INSTALLATIONS_CANVAS_H: Scalar =
        PAD + ITEM_HEIGHT * 2.0 + PAD + INSTALLATION_LIST_H + PAD;
    const SELECTED_CANVAS_H: Scalar = ITEM_HEIGHT * 2.0 + PAD * 4.0 + INSTALLATIONS_CANVAS_H;
    let speaker_editor_canvas_h = LIST_HEIGHT + ITEM_HEIGHT + SELECTED_CANVAS_H;

    let (area, event) = collapsible_area(is_open, "Speaker Editor", ids.side_menu)
        .align_middle_x_of(ids.side_menu)
        .down_from(last_area_id, 0.0)
        .set(ids.speaker_editor, ui);
    if let Some(event) = event {
        state.is_open.speaker_editor = event.is_open();
    }

    // Only continue if the collapsible area is open.
    let area = match area {
        None => return ids.speaker_editor,
        Some(area) => area,
    };

    // The canvas on which the log will be placed.
    let canvas = ui::widget::Canvas::new()
        .pad(0.0)
        .h(speaker_editor_canvas_h);
    area.set(canvas, ui);

    // Convert the given map into a sorted list of speaker Id.
    fn sorted_speakers_vec(speakers: &project::Speakers) -> Vec<audio::speaker::Id> {
        let mut speakers_vec: Vec<_> = speakers.keys().cloned().collect();
        speakers_vec.sort_by(|a, b| a.0.cmp(&b.0));
        speakers_vec
    }

    // The vec of sorted speakers.
    let mut speakers_vec = sorted_speakers_vec(speakers);

    // If there are no speakers, display a message saying how to add some.
    if speakers.is_empty() {
        ui::widget::Text::new("Add some speaker outputs with the `+` button")
            .padded_w_of(area.id, TEXT_PAD)
            .mid_top_with_margin_on(area.id, TEXT_PAD)
            .font_size(SMALL_FONT_SIZE)
            .center_justify()
            .set(ids.speaker_editor_no_speakers, ui);

    // Otherwise display the speaker list.
    } else {
        // Convert the `speakers` map into a Vec for display.
        let mut speakers_vec = sorted_speakers_vec(speakers);

        let num_items = speakers_vec.len();
        let (mut list_events, scrollbar) = ui::widget::ListSelect::single(num_items)
            .item_size(ITEM_HEIGHT)
            .h(LIST_HEIGHT)
            .align_middle_x_of(area.id)
            .align_top_of(area.id)
            .scrollbar_next_to()
            .scrollbar_color(ui::color::LIGHT_CHARCOAL)
            .set(ids.speaker_editor_list, ui);

        // If a speaker was removed, process it after the whole list is instantiated to avoid
        // invalid indices.
        let mut maybe_remove_index = None;

        while let Some(event) = list_events.next(ui, |i| speaker_editor.selected == Some(i)) {
            use self::ui::widget::list_select::Event;
            match event {
                // Instantiate a button for each speaker.
                Event::Item(item) => {
                    let selected = speaker_editor.selected == Some(item.i);
                    let speaker_id = speakers_vec[item.i];
                    let label = {
                        let speaker = &speakers[&speaker_id];
                        let label = format!(
                            "{} - CH {} - ({}mx, {}my)",
                            speaker.name,
                            speaker.channel + 1,
                            (speaker.point.x * 100.0).trunc() / 100.0,
                            (speaker.point.y * 100.0).trunc() / 100.0
                        );
                        label
                    };

                    // Blue if selected, gray otherwise.
                    let color = if selected {
                        ui::color::BLUE
                    } else {
                        ui::color::CHARCOAL
                    };

                    // Use `Button`s for the selectable items.
                    let button = ui::widget::Button::new()
                        .label(&label)
                        .label_font_size(SMALL_FONT_SIZE)
                        .label_x(ui::position::Relative::Place(ui::position::Place::Start(
                            Some(10.0),
                        )))
                        .color(color);
                    item.set(button, ui);

                    // If the button or any of its children are capturing the mouse, display
                    // the `remove` button.
                    let show_remove_button = ui
                        .global_input()
                        .current
                        .widget_capturing_mouse
                        .map(|id| {
                            id == item.widget_id
                                || ui
                                    .widget_graph()
                                    .does_recursive_depth_edge_exist(item.widget_id, id)
                        })
                        .unwrap_or(false);

                    if !show_remove_button {
                        continue;
                    }

                    if ui::widget::Button::new()
                        .label("X")
                        .label_font_size(SMALL_FONT_SIZE)
                        .color(ui::color::DARK_RED.alpha(0.5))
                        .w_h(ITEM_HEIGHT, ITEM_HEIGHT)
                        .align_right_of(item.widget_id)
                        .align_middle_y_of(item.widget_id)
                        .parent(item.widget_id)
                        .set(ids.speaker_editor_remove, ui)
                        .was_clicked()
                    {
                        maybe_remove_index = Some(item.i);
                    }
                }

                // Update the selected speaker.
                Event::Selection(idx) => speaker_editor.selected = Some(idx),

                _ => (),
            }
        }

        // The scrollbar for the list.
        if let Some(s) = scrollbar {
            s.set(ui);
        }

        // Remove a speaker if necessary.
        if let Some(i) = maybe_remove_index {
            // Unselect the speaker.
            if Some(i) == speaker_editor.selected {
                speaker_editor.selected = None;
            }

            // Remove the local copy.
            let speaker_id = speakers_vec.remove(i);
            speakers.remove(&speaker_id);

            // Remove the speaker from the audio output thread.
            channels
                .audio_output
                .send(move |audio| {
                    audio.remove_speaker(speaker_id);
                })
                .expect("failed to remove speaker from audio output thread");

            // Remove the soundscape copy.
            channels
                .soundscape
                .send(move |soundscape| {
                    soundscape.remove_speaker(&speaker_id);
                })
                .expect("failed to remove speaker from soundscape thread");
        }
    }

    // Only display the `add_speaker` button if there are less than `max` num channels.
    let show_add_button = speakers.len() < audio::MAX_CHANNELS;

    if show_add_button {
        let plus_size = (ITEM_HEIGHT * 0.66) as FontSize;
        if ui::widget::Button::new()
            .color(DARK_A)
            .label("+")
            .label_font_size(plus_size)
            .align_middle_x_of(area.id)
            .mid_top_with_margin_on(area.id, LIST_HEIGHT)
            .w_of(area.id)
            .parent(area.id)
            .set(ids.speaker_editor_add, ui)
            .was_clicked()
        {
            let id = project::next_speaker_id(speakers);
            let name = format!("S{}", id.0);
            let channel = project::next_available_speaker_channel(speakers);
            let audio = audio::Speaker {
                point: camera.position,
                channel: channel,
                installations: Default::default(),
            };

            // Update the audio output copy.
            let speaker = audio.clone();
            channels
                .audio_output
                .send(move |audio| {
                    audio.insert_speaker(id, speaker);
                })
                .expect("failed to send speaker to audio output thread");

            // Update the soundscape copy.
            let soundscape_speaker = soundscape::Speaker::from_audio_speaker(&audio);
            channels
                .soundscape
                .send(move |soundscape| {
                    soundscape.insert_speaker(id, soundscape_speaker);
                })
                .expect("failed to send speaker to soundscape thread");

            // Update the local copy.
            let speaker = project::Speaker { name, audio };
            speakers.insert(id, speaker);
            speakers_vec.push(id);
            speaker_editor.selected = Some(speakers.len() - 1);
        }
    }

    let area_rect = ui.rect_of(area.id).unwrap();
    let start = area_rect.y.start;
    let end = start + SELECTED_CANVAS_H;
    let selected_canvas_y = ui::Range { start, end };

    ui::widget::Canvas::new()
        .pad(PAD)
        .w_of(ids.side_menu)
        .h(SELECTED_CANVAS_H)
        .y(selected_canvas_y.middle())
        .align_middle_x_of(ids.side_menu)
        .set(ids.speaker_editor_selected_canvas, ui);

    // If a speaker is selected, display its info.
    let i = match speaker_editor.selected {
        None => {
            // Otherwise no speaker is selected.
            ui::widget::Text::new("No speaker selected")
                .padded_w_of(area.id, TEXT_PAD)
                .mid_top_with_margin_on(ids.speaker_editor_selected_canvas, TEXT_PAD)
                .font_size(SMALL_FONT_SIZE)
                .center_justify()
                .set(ids.speaker_editor_selected_none, ui);
            return area.id;
        }
        Some(i) => i,
    };

    // The unique ID of the selected speaker.
    let id = speakers_vec[i];

    // The name of the speaker.
    for event in ui::widget::TextBox::new(&speakers[&id].name)
        .mid_top_of(ids.speaker_editor_selected_canvas)
        .kid_area_w_of(ids.speaker_editor_selected_canvas)
        .parent(gui.ids.speaker_editor_selected_canvas)
        .h(ITEM_HEIGHT)
        .color(DARK_A)
        .font_size(SMALL_FONT_SIZE)
        .set(ids.speaker_editor_selected_name, ui)
    {
        if let ui::widget::text_box::Event::Update(string) = event {
            speakers.get_mut(&id).unwrap().name = string;
        }
    }

    let channel_vec: Vec<String> = (0..audio::MAX_CHANNELS)
        .map(|ch| {
            speakers_vec
                .iter()
                .map(|id| &speakers[id])
                .enumerate()
                .find(|&(ix, ref s)| i != ix && s.audio.channel == ch)
                .map(|(_ix, s)| format!("CH {} (swap with {})", ch + 1, s.name))
                .unwrap_or_else(|| format!("CH {}", ch + 1))
        })
        .collect();
    let selected_channel = speakers[&id].audio.channel;

    // The drop down list for channel selection.
    for new_index in ui::widget::DropDownList::new(&channel_vec, Some(selected_channel))
        .down_from(ids.speaker_editor_selected_name, PAD)
        .align_middle_x_of(ids.side_menu)
        .kid_area_w_of(ids.speaker_editor_selected_canvas)
        .h(ITEM_HEIGHT)
        .parent(ids.speaker_editor_selected_canvas)
        .scrollbar_on_top()
        .max_visible_items(5)
        .color(DARK_A)
        .border_color(ui::color::LIGHT_CHARCOAL)
        .label_font_size(SMALL_FONT_SIZE)
        .set(ids.speaker_editor_selected_channel, ui)
    {
        // Update the local copy.
        speakers.get_mut(&id).unwrap().audio.channel = new_index;

        // Update the audio output copy.
        let speaker = speakers[&id].audio.clone();
        gui.channels
            .audio_output
            .send(move |audio| {
                audio.insert_speaker(id, speaker);
            })
            .expect("failed to send speaker to audio output thread");

        // If an existing speaker was assigned to `index`, swap it with the original
        // selection.
        let maybe_index = speakers_vec
            .iter()
            .map(|id| &speakers[id])
            .enumerate()
            .find(|&(ix, ref s)| i != ix && s.audio.channel == new_index)
            .map(|(ix, _)| ix);
        if let Some(ix) = maybe_index {
            // Update the local copy.
            let other_id = speakers_vec[ix];
            let other_speaker = speakers.get_mut(&other_id).unwrap();
            other_speaker.audio.channel = selected_channel;

            // Update the audio output copy.
            let other_speaker = other_speaker.audio.clone();
            gui.channels
                .audio_output
                .send(move |audio| {
                    audio.insert_speaker(other_id, other_speaker);
                })
                .expect("failed to send speaker to audio output thread");
        }
    }

    // A canvas on which installation selection widgets are instantiated.
    ui::widget::Canvas::new()
        .kid_area_w_of(ids.speaker_editor_selected_canvas)
        .h(INSTALLATIONS_CANVAS_H)
        .mid_bottom_of(ids.speaker_editor_selected_canvas)
        .pad(PAD)
        .color(ui::color::CHARCOAL)
        .set(ids.speaker_editor_selected_installations_canvas, ui);

    // A header for the installations editing area.
    ui::widget::Text::new("Installations")
        .top_left_of(ids.speaker_editor_selected_installations_canvas)
        .font_size(SMALL_FONT_SIZE)
        .set(ids.speaker_editor_selected_installations_text, ui);

    // A dropdownlist for assigning installations to the speaker.
    //
    // Only show installations that aren't yet assigned.
    let installations_vec = installations
        .keys()
        .filter(|inst| !speakers[&id].installations.contains(inst))
        .cloned()
        .collect::<Vec<_>>();
    let installation_strs = installations_vec
        .iter()
        .map(|inst_id| &installations[&inst_id].name)
        .collect::<Vec<_>>();
    for index in ui::widget::DropDownList::new(&installation_strs, None)
        .align_middle_x_of(ids.speaker_editor_selected_installations_canvas)
        .down_from(ids.speaker_editor_selected_installations_text, PAD * 2.0)
        .h(ITEM_HEIGHT)
        .kid_area_w_of(ids.speaker_editor_selected_installations_canvas)
        .label("ADD INSTALLATION")
        .label_font_size(SMALL_FONT_SIZE)
        .set(ids.speaker_editor_selected_installations_ddl, ui)
    {
        let installation = installations_vec[index];
        let speaker = speakers.get_mut(&id).unwrap();

        // Update the local copy.
        speaker.installations.insert(installation);

        // Update the audio output copy.
        gui.channels
            .audio_output
            .send(move |audio| {
                audio.insert_speaker_installation(id, installation);
            })
            .expect("failed to update speaker installation for audio output thread");

        // Update the soundscape copy.
        gui.channels
            .soundscape
            .send(move |soundscape| {
                soundscape.update_speaker(&id, |speaker| {
                    speaker.installations.insert(installation);
                });
            })
            .expect("failed to update speaker installations for soundscape thread");
    }

    // A scrollable list showing each of the assigned installations.
    let mut selected_installations = speakers[&id]
        .audio
        .installations
        .iter()
        .cloned()
        .collect::<Vec<_>>();
    selected_installations.sort_by(|a, b| installations[a].name.cmp(&installations[b].name));
    let (mut items, scrollbar) = ui::widget::List::flow_down(selected_installations.len())
        .item_size(ITEM_HEIGHT)
        .h(INSTALLATION_LIST_H)
        .kid_area_w_of(ids.speaker_editor_selected_installations_canvas)
        .align_middle_x_of(ids.speaker_editor_selected_installations_canvas)
        .down_from(ids.speaker_editor_selected_installations_ddl, PAD)
        .scrollbar_next_to()
        .scrollbar_color(ui::color::LIGHT_CHARCOAL)
        .set(ids.speaker_editor_selected_installations_list, ui);
    let mut maybe_remove_index = None;
    while let Some(item) = items.next(ui) {
        let inst = selected_installations[item.i];
        let label = &installations[&inst].name;

        // Use `Button`s for the selectable items.
        let button = ui::widget::Button::new()
            .label(&label)
            .label_font_size(SMALL_FONT_SIZE)
            .label_x(ui::position::Relative::Place(ui::position::Place::Start(
                Some(10.0),
            )));
        item.set(button, ui);

        // If the button or any of its children are capturing the mouse, display
        // the `remove` button.
        let show_remove_button = ui
            .global_input()
            .current
            .widget_capturing_mouse
            .map(|id| {
                id == item.widget_id
                    || ui
                        .widget_graph()
                        .does_recursive_depth_edge_exist(item.widget_id, id)
            })
            .unwrap_or(false);

        if !show_remove_button {
            continue;
        }

        if ui::widget::Button::new()
            .label("X")
            .label_font_size(SMALL_FONT_SIZE)
            .color(ui::color::DARK_RED.alpha(0.5))
            .w_h(ITEM_HEIGHT, ITEM_HEIGHT)
            .align_right_of(item.widget_id)
            .align_middle_y_of(item.widget_id)
            .parent(item.widget_id)
            .set(gui.ids.speaker_editor_selected_installations_remove, ui)
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
        let speaker = speakers.get_mut(&id).unwrap();

        // Remove the local copy.
        speaker.audio.installations.remove(&inst);

        // Remove the audio output copy.
        gui.channels
            .audio_output
            .send(move |audio| {
                audio.remove_speaker_installation(id, &inst);
            })
            .expect("failed to remove installation from speaker on audio output thread");

        // Update the soundscape copy.
        gui.channels
            .soundscape
            .send(move |soundscape| {
                soundscape.update_speaker(&id, |speaker| {
                    speaker.installations.remove(&inst);
                });
            })
            .expect("failed to remove installation from speaker on soundscape thread");
    }

    area.id
}
