use audio;
use gui::{collapsible_area, Gui};
use gui::{ITEM_HEIGHT, SMALL_FONT_SIZE, DARK_A};
use installation::{self, Installation};
use nannou::ui;
use nannou::ui::prelude::*;
use serde_json;
use std::fs::File;
use std::path::Path;

pub struct SpeakerEditor {
    pub is_open: bool,
    // The list of speaker outputs.
    pub speakers: Vec<Speaker>,
    // The index of the selected speaker.
    pub selected: Option<usize>,
    // The next ID to be used for a new speaker.
    pub next_id: audio::speaker::Id,
}

#[derive(Deserialize, Serialize)]
pub struct Speaker {
    // Speaker state shared with the audio thread.
    pub audio: audio::Speaker,
    pub name: String,
    pub id: audio::speaker::Id,
}

// A data structure from which the speaker layout can be saved and loaded.
#[derive(Deserialize, Serialize)]
pub struct StoredSpeakers {
    #[serde(default = "first_speaker_id")]
    pub next_id: audio::speaker::Id,
    #[serde(default = "Vec::new")]
    pub speakers: Vec<Speaker>,
}

pub fn first_speaker_id() -> audio::speaker::Id {
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
    pub fn load(path: &Path) -> Self {
        File::open(&path)
            .ok()
            .and_then(|f| serde_json::from_reader(f).ok())
            .unwrap_or_else(StoredSpeakers::new)
    }
}

// Instantiate the sidebar speaker editor widgets.
pub fn set(last_area_id: widget::Id, gui: &mut Gui) -> widget::Id {
    let is_open = gui.state.speaker_editor.is_open;
    const LIST_HEIGHT: Scalar = 140.0;
    const PAD: Scalar = 6.0;
    const TEXT_PAD: Scalar = 20.0;
    const INSTALLATION_LIST_H: Scalar = ITEM_HEIGHT * 3.0;
    const INSTALLATIONS_CANVAS_H: Scalar = PAD + ITEM_HEIGHT * 2.0 + PAD + INSTALLATION_LIST_H + PAD;
    const SELECTED_CANVAS_H: Scalar = ITEM_HEIGHT * 2.0 + PAD * 4.0 + INSTALLATIONS_CANVAS_H;
    let speaker_editor_canvas_h = LIST_HEIGHT + ITEM_HEIGHT + SELECTED_CANVAS_H;

    let (area, event) = collapsible_area(is_open, "Speaker Editor", gui.ids.side_menu)
        .align_middle_x_of(gui.ids.side_menu)
        .down_from(last_area_id, 0.0)
        .set(gui.ids.speaker_editor, gui);
    if let Some(event) = event {
        gui.state.speaker_editor.is_open = event.is_open();
    }

    // Only continue if the collapsible area is open.
    let area = match area {
        None => return gui.ids.speaker_editor,
        Some(area) => area,
    };

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
        let (mut list_events, scrollbar) = widget::ListSelect::single(num_items)
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

        while let Some(event) = list_events.next(gui, |i| gui.state.speaker_editor.selected == Some(i)) {
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
            if Some(i) == gui.state.speaker_editor.selected {
                gui.state.speaker_editor.selected = None;
            }
            let speaker = gui.state.speaker_editor.speakers.remove(i);
            let speaker_id = speaker.id;
            gui.channels.audio_output.send(move |audio| {
                audio.remove_speaker(speaker_id);
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
                installations: Default::default(),
            };

            let speaker = audio.clone();
            gui.channels.audio_output.send(move |audio| {
                audio.insert_speaker(id, speaker);
            }).ok();

            let speaker = Speaker { id, name, audio };
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
    let i = match gui.state.speaker_editor.selected {
        None => {
            // Otherwise no speaker is selected.
            widget::Text::new("No speaker selected")
                .padded_w_of(area.id, TEXT_PAD)
                .mid_top_with_margin_on(gui.ids.speaker_editor_selected_canvas, TEXT_PAD)
                .font_size(SMALL_FONT_SIZE)
                .center_justify()
                .set(gui.ids.speaker_editor_selected_none, gui);
            return area.id;
        },
        Some(i) => i,
    };

    let Gui { ref mut state, ref mut ui, ref ids, .. } = *gui;
    let SpeakerEditor { ref mut speakers, .. } = state.speaker_editor;

    // The name of the speaker.
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
    let selected_channel = speakers[i].audio.channel;

    // The drop down list for channel selection.
    for new_index in widget::DropDownList::new(&channel_vec, Some(selected_channel))
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
        let id = speakers[i].id;
        let speaker = speakers[i].audio.clone();
        gui.channels.audio_output.send(move |audio| {
            audio.insert_speaker(id, speaker);
        }).ok();

        // If an existing speaker was assigned to `index`, swap it with the original
        // selection.
        let maybe_index = speakers.iter()
            .enumerate()
            .find(|&(ix, s)| i != ix && s.audio.channel == new_index)
            .map(|(ix, _)| ix);
        if let Some(ix) = maybe_index {
            let speaker = &mut speakers[ix];
            speaker.audio.channel = selected_channel;
            let id = speaker.id;
            let speaker = speaker.audio.clone();
            gui.channels.audio_output.send(move |audio| {
                audio.insert_speaker(id, speaker);
            }).ok();
        }
    }

    // A canvas on which installation selection widgets are instantiated.
    widget::Canvas::new()
        .kid_area_w_of(ids.speaker_editor_selected_canvas)
        .h(INSTALLATIONS_CANVAS_H)
        .mid_bottom_of(ids.speaker_editor_selected_canvas)
        .pad(PAD)
        .color(color::CHARCOAL)
        .set(ids.speaker_editor_selected_installations_canvas, ui);

    // A header for the installations editing area.
    widget::Text::new("Installations")
        .top_left_of(ids.speaker_editor_selected_installations_canvas)
        .font_size(SMALL_FONT_SIZE)
        .set(ids.speaker_editor_selected_installations_text, ui);

    // A dropdownlist for assigning installations to the speaker.
    //
    // Only show installations that aren't yet assigned.
    let installations = installation::ALL
        .iter()
        .filter(|inst| !speakers[i].audio.installations.contains(inst))
        .cloned()
        .collect::<Vec<_>>();
    let installation_strs = installations
        .iter()
        .map(Installation::display_str)
        .collect::<Vec<_>>();
    for index in widget::DropDownList::new(&installation_strs, None)
        .align_middle_x_of(ids.speaker_editor_selected_installations_canvas)
        .down_from(ids.speaker_editor_selected_installations_text, PAD * 2.0)
        .h(ITEM_HEIGHT)
        .kid_area_w_of(ids.speaker_editor_selected_installations_canvas)
        .label("ADD INSTALLATION")
        .label_font_size(SMALL_FONT_SIZE)
        .set(ids.speaker_editor_selected_installations_ddl, ui)
    {
        let installation = installations[index];
        let speaker = &mut speakers[i];
        let speaker_id = speaker.id;
        speaker.audio.installations.insert(installation);
        gui.channels.audio_output.send(move |audio| {
            audio.insert_speaker_installation(speaker_id, installation);
        }).ok();
    }

    // A scrollable list showing each of the assigned installations.
    let mut selected_installations = speakers[i].audio.installations
        .iter()
        .cloned()
        .collect::<Vec<_>>();
    selected_installations.sort_by(|a, b| a.display_str().cmp(b.display_str()));
    let (mut items, scrollbar) = widget::List::flow_down(selected_installations.len())
        .item_size(ITEM_HEIGHT)
        .h(INSTALLATION_LIST_H)
        .kid_area_w_of(ids.speaker_editor_selected_installations_canvas)
        .align_middle_x_of(ids.speaker_editor_selected_installations_canvas)
        .down_from(ids.speaker_editor_selected_installations_ddl, PAD)
        .scrollbar_next_to()
        .scrollbar_color(color::LIGHT_CHARCOAL)
        .set(ids.speaker_editor_selected_installations_list, ui);
    let mut maybe_remove_index = None;
    while let Some(item) = items.next(ui) {
        let inst = selected_installations[item.i];
        let label = inst.display_str();

        // Use `Button`s for the selectable items.
        let button = widget::Button::new()
            .label(&label)
            .label_font_size(SMALL_FONT_SIZE)
            .label_x(position::Relative::Place(position::Place::Start(Some(10.0))));
        item.set(button, ui);

        // If the button or any of its children are capturing the mouse, display
        // the `remove` button.
        let show_remove_button = ui.global_input().current.widget_capturing_mouse
            .map(|id| {
                id == item.widget_id ||
                ui.widget_graph()
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
        let speaker = &mut speakers[i];
        let speaker_id = speaker.id;
        speaker.audio.installations.remove(&inst);
        gui.channels.audio_output.send(move |audio| {
            audio.remove_speaker_installation(speaker_id, &inst);
        }).ok();
    }

    area.id
}
