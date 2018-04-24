use gui::{self, collapsible_area, Channels, Gui, ProjectState, State};
use gui::{ITEM_HEIGHT, SMALL_FONT_SIZE};
use installation;
use nannou::{self, ui};
use nannou::osc::Connected;
use nannou::ui::prelude::*;
use osc;
use project::{self, Project};
use std::{io, net};

/// Runtime state relevant to the installation editor GUI.
#[derive(Default)]
pub struct InstallationEditor {
    pub selected: Option<Selected>,
}

/// The currently selected installation.
pub struct Selected {
    id: installation::Id,
    name: String,
    selected_computer: Option<SelectedComputer>,
}

/// The currently selected installation computer.
pub struct SelectedComputer {
    computer: installation::computer::Id,
    socket_string: String,
    osc_addr: String,
}

pub fn set(
    last_area_id: widget::Id,
    gui: &mut Gui,
    project: &mut Project,
    project_state: &mut gui::ProjectState,
) -> widget::Id {
    let Gui {
        ref mut ui,
        ref ids,
        channels,
        state: &mut State {
            ref mut is_open,
            ..
        },
        ..
    } = *gui;
    let Project {
        state: project::State {
            ref mut installations,
            ..
        },
        ..
    } = *project;
    let ProjectState {
        installation_editor:
            InstallationEditor {
                ref mut selected,
            },
        ..
    } = *project_state;

    // The height of the list of installations.
    const LIST_HEIGHT: Scalar = ITEM_HEIGHT * 4.0;
    const ADD_H: Scalar = ITEM_HEIGHT;
    const NAME_H: Scalar = ITEM_HEIGHT;
    const COMPUTER_LIST_HEIGHT: Scalar = ITEM_HEIGHT * 3.0;
    const PAD: Scalar = 6.0;
    const TEXT_PAD: Scalar = PAD * 2.0;
    const SLIDER_H: Scalar = ITEM_HEIGHT;

    // The height of the canvas displaying options for the selected installation.
    //
    // These options include:
    //
    // - Music Data OSC Output (Text and TextBox)
    let osc_canvas_h = PAD + ITEM_HEIGHT * 3.0 + PAD;
    let computer_canvas_h = ITEM_HEIGHT + PAD + ITEM_HEIGHT + PAD + COMPUTER_LIST_HEIGHT;
    let soundscape_canvas_h = PAD + PAD * 3.0 + PAD + SLIDER_H + PAD;
    let selected_canvas_h = PAD
        + NAME_H + PAD
        + computer_canvas_h + PAD
        + osc_canvas_h + PAD
        + soundscape_canvas_h + PAD;

    // The total height of the installation editor as a sum of the previous heights plus necessary
    // padding.
    let installation_editor_h = LIST_HEIGHT + ADD_H + selected_canvas_h;

    let (area, event) = collapsible_area(is_open.installation_editor, "Installation Editor", ids.side_menu)
        .align_middle_x_of(ids.side_menu)
        .down_from(last_area_id, 0.0)
        .set(ids.installation_editor, ui);
    if let Some(event) = event {
        is_open.installation_editor = event.is_open();
    }

    // If the area is open, continue. If its closed, return the editor id as the last id.
    let area = match area {
        Some(area) => area,
        None => return ids.installation_editor,
    };

    // The canvas on which the installation editor widgets will be placed.
    let canvas = widget::Canvas::new().pad(0.0).h(installation_editor_h);
    area.set(canvas, ui);

    // A button for adding new installations.
    for _click in widget::Button::new()
        .label("+")
        .kid_area_w_of(area.id)
        .mid_top_with_margin_on(area.id, LIST_HEIGHT)
        .h(ADD_H)
        .set(ids.installation_editor_add, ui)
    {
        // Add a new installation.
        let installation = installation::Installation::default();
        let id = project::next_installation_id(installations);
        let clone = installation.soundscape.clone();
        let name = installation.name.clone();
        installations.insert(id, installation);
        let selected_computer = None;
        *selected = Some(Selected { id, name, selected_computer });

        // Update the soundscape thread.
        channels
            .soundscape
            .send(move |soundscape| {
                soundscape.insert_installation(id, clone);
            })
            .ok();

        // Update the audio output.
        channels
            .audio_output
            .send(move |audio| {
                audio.insert_installation(id);
            })
            .ok();
    }

    // If there are no installations, display some text for adding one.
    if installations.is_empty() {
        widget::Text::new("Add an installation with the \"+\" button below!")
            .font_size(SMALL_FONT_SIZE)
            .align_middle_x_of(area.id)
            .down(PAD + ITEM_HEIGHT)
            .set(ids.installation_editor_none, ui);
        return area.id;
    }

    // The list of installation names sorted in alphabetical order.
    let mut installations_vec: Vec<_> = installations.keys().cloned().collect();

    // Sort the names.
    installations_vec.sort_by(|a, b| a.0.cmp(&b.0));

    // Display the installation list.
    let num_items = installations.len();
    let (mut events, scrollbar) = widget::ListSelect::single(num_items)
        .item_size(ITEM_HEIGHT)
        .h(LIST_HEIGHT)
        .align_middle_x_of(area.id)
        .align_top_of(area.id)
        .scrollbar_color(color::LIGHT_CHARCOAL)
        .scrollbar_next_to()
        .set(ids.installation_editor_list, ui);

    // Track whether or not an item was removed.
    let mut maybe_remove_index = None;

    // The index of the selected installation.
    let selected_index = selected.as_ref()
        .and_then(|s| installations_vec.iter().position(|&id| id == s.id));

    while let Some(event) = events.next(ui, |i| selected_index == Some(i)) {
        use nannou::ui::widget::list_select::Event;
        match event {
            // Instantiate a button for each installation.
            Event::Item(item) => {
                let id = installations_vec[item.i];
                let label = &installations[&id].name;
                let is_selected = selected_index == Some(item.i);

                // Blue if selected, gray otherwise.
                let color = if is_selected {
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
                    .set(ids.installation_editor_remove, ui)
                    .was_clicked()
                {
                    maybe_remove_index = Some(item.i);
                }
            }

            // Update the selected installation.
            Event::Selection(index) => {
                let id = installations_vec[index];
                let installation = &installations[&id];
                let name = installation.name.clone();
                let selected_computer = match installation.computers.len() {
                    0 => None,
                    _ => {
                        let computer = installation::computer::Id(0);
                        let (socket_string, osc_addr) = {
                            let address = &installation.computers[&computer];
                            (format!("{}", address.socket), address.osc_addr.clone())
                        };
                        Some(SelectedComputer {
                            computer,
                            socket_string,
                            osc_addr,
                        })
                    }
                };
                *selected = Some(Selected {
                    id,
                    name,
                    selected_computer,
                });
            }

            _ => (),
        }
    }

    // Instantiate the scrollbar widget if necessary.
    if let Some(scrollbar) = scrollbar {
        scrollbar.set(ui);
    }

    // Remove an installation if necessary.
    if let Some(i) = maybe_remove_index {
        let id = installations_vec.remove(i);

        // Unselect the removed group.
        if selected.as_ref().map(|s| s.id) == Some(id) {
            *selected = None;
        }

        // Remove the local copy from the map.
        installations.remove(&id);

        // Remove this installation from the soundscape thread.
        channels
            .soundscape
            .send(move |soundscape| {
                soundscape.remove_installation(&id);
            })
            .ok();

        // Remove this installation from the audio output thread.
        channels
            .audio_output
            .send(move |audio| {
                audio.remove_installation(&id);
            })
            .ok();

        // Remove this installation from the OSC output thread.
        let rem = osc::output::OscTarget::RemoveInstallation(id);
        let msg = osc::output::Message::Osc(rem);
        channels
            .osc_out_msg_tx
            .send(msg)
            .ok();
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
        .set(ids.installation_editor_selected_canvas, ui);
    let selected_canvas_kid_area = ui.kid_area_of(ids.installation_editor_selected_canvas)
        .unwrap();

    // If an installation is selected, display the installation computer canvas.
    let selected = match *selected {
        Some(ref mut selected) => selected,
        None => return area.id,
    };

    let Selected {
        id,
        ref mut name,
        ref mut selected_computer,
    } = *selected;

    // A textbox for editing the name of the installation.
    let color = if name == &installations[&id].name {
        color::BLACK
    } else {
        if installations.values().any(|inst| &inst.name == name) || name == "" {
            color::DARK_RED
        } else {
            color::DARK_GREEN
        }
    };
    for event in widget::TextBox::new(name)
        .w(selected_canvas_kid_area.w())
        .h(NAME_H)
        .color(color)
        .font_size(SMALL_FONT_SIZE)
        .mid_top_of(ids.installation_editor_selected_canvas)
        .set(ids.installation_editor_name, ui)
    {
        use nannou::ui::widget::text_box::Event;
        match event {
            Event::Update(s) => *name = s,
            Event::Enter => {
                // Update name and computer OSC addresses.
                let installation = installations.get_mut(&id).unwrap();
                installation.name = name.clone();

                // Send updated addresses.
                let osc_addr_base = installation::osc_addr_string(&installation.name);
                for (&c_id, computer) in installation.computers.iter_mut() {
                    let osc_addr = format!("/{}", osc_addr_base);

                    // Update local copy.
                    computer.osc_addr = osc_addr.clone();

                    // Update osc copy.
                    let update = osc::output::OscTarget::UpdateAddr(id, c_id, osc_addr);
                    let msg = osc::output::Message::Osc(update);
                    channels
                        .osc_out_msg_tx
                        .send(msg)
                        .expect("could not update OSC target computer address");
                }
            },
        }
    }

    // The canvas for displaying the computer selection / editor.
    widget::Canvas::new()
        .middle_of(ids.installation_editor_selected_canvas)
        .down_from(ids.installation_editor_name, PAD)
        .color(color::CHARCOAL)
        .w(selected_canvas_kid_area.w())
        .h(computer_canvas_h)
        .pad(PAD)
        .set(ids.installation_editor_computer_canvas, ui);

    // OSC output address header.
    widget::Text::new("Installation Computers")
        .font_size(SMALL_FONT_SIZE)
        .top_left_of(ids.installation_editor_computer_canvas)
        .set(ids.installation_editor_computer_text, ui);

    fn osc_sender(socket: &net::SocketAddrV4) -> io::Result<nannou::osc::Sender<Connected>> {
        nannou::osc::sender()
            .expect("failed to create OSC sender")
            .connect(socket)
    }

    // A number dialer to control the number of computers in the installation.
    let n_computers = installations[&id].computers.len();
    let min_cpus = 0.0;
    let max_cpus = 128.0;
    let precision = 0;
    for n in widget::NumberDialer::new(n_computers as f32, min_cpus, max_cpus, precision)
        .align_middle_x_of(ids.installation_editor_computer_canvas)
        .down_from(ids.installation_editor_computer_text, TEXT_PAD)
        .kid_area_w_of(ids.installation_editor_computer_canvas)
        .h(ITEM_HEIGHT)
        .label("Number of Computers")
        .label_font_size(SMALL_FONT_SIZE)
        .set(ids.installation_editor_computer_number, ui)
    {
        let n = n as usize;
        let installation = installations.get_mut(&id).unwrap();
        if n_computers < n {
            for i in n_computers..n {
                let computer = installation::computer::Id(i);
                let socket = "127.0.0.1:9002".parse().unwrap();
                let osc_addr = installation::osc_addr_string(&installation.name);
                let osc_tx = match osc_sender(&socket) {
                    Err(err) => {
                        eprintln!("failed to connect localhost OSC sender: {}", err);
                        break;
                    },
                    Ok(tx) => tx,
                };
                let add = osc::output::OscTarget::Add(id, computer, osc_tx, osc_addr.clone());
                let msg = osc::output::Message::Osc(add);
                if channels.osc_out_msg_tx.send(msg).ok().is_some() {
                    let addr = installation::computer::Address { socket, osc_addr };
                    installation.computers.insert(computer, addr);
                }
            }
        } else if n_computers > n {
            for i in n..n_computers {
                let computer = installation::computer::Id(i);
                let rem = osc::output::OscTarget::Remove(id, computer);
                let msg = osc::output::Message::Osc(rem);
                if channels.osc_out_msg_tx.send(msg).ok().is_some() {
                    installation.computers.remove(&computer);
                }
            }
            if selected_computer
                .as_ref()
                .map(|s| s.computer.0 >= n)
                .unwrap_or(true)
            {
                *selected_computer = None;
            }
        }
    }

    // Display the computer list for this installation.
    let n_computers = installations[&id].computers.len();
    let (mut events, scrollbar) = widget::ListSelect::single(n_computers)
        .item_size(ITEM_HEIGHT)
        .h(COMPUTER_LIST_HEIGHT)
        .align_middle_x_of(ids.installation_editor_computer_canvas)
        .down_from(ids.installation_editor_computer_number, PAD)
        .scrollbar_color(color::LIGHT_CHARCOAL)
        .scrollbar_next_to()
        .set(ids.installation_editor_computer_list, ui);

    while let Some(event) = events.next(ui, |i| {
        selected_computer.as_ref().map(|s| s.computer) == Some(installation::computer::Id(i))
    }) {
        use self::ui::widget::list_select::Event;
        match event {
            // Instantiate a button for each computer.
            Event::Item(item) => {
                let computer = installation::computer::Id(item.i);
                let is_selected = selected_computer.as_ref().map(|s| s.computer) == Some(computer);
                // Blue if selected, gray otherwise.
                let color = if is_selected {
                    color::BLUE
                } else {
                    color::BLACK
                };
                let addr = &installations[&id].computers[&computer];
                let label = format!("{} {}", addr.socket, addr.osc_addr);

                // Use `Button`s for the selectable items.
                let button = widget::Button::new()
                    .label(&label)
                    .label_font_size(SMALL_FONT_SIZE)
                    .label_x(position::Relative::Place(position::Place::Start(Some(
                        10.0,
                    ))))
                    .color(color);
                item.set(button, ui);
            }

            // Update the selected source.
            Event::Selection(index) => {
                let computer = installation::computer::Id(index);
                let addr = &installations[&id].computers[&computer];
                let socket_string = format!("{}", addr.socket);
                let osc_addr = addr.osc_addr.clone();
                *selected_computer = Some(SelectedComputer {
                    computer,
                    socket_string,
                    osc_addr,
                });
            }

            _ => (),
        }
    }

    if let Some(scrollbar) = scrollbar {
        scrollbar.set(ui);
    }

    // If a computer within the installation is selected, display the cpu stuff.
    let selected_computer = match *selected_computer {
        Some(ref mut selected_computer) => selected_computer,
        None => return area.id,
    };

    // The canvas for displaying the osc output address editor.
    widget::Canvas::new()
        .align_middle_x_of(ids.installation_editor_selected_canvas)
        .down_from(ids.installation_editor_computer_canvas, PAD)
        .color(color::CHARCOAL)
        .w(selected_canvas_kid_area.w())
        .h(osc_canvas_h)
        .pad(PAD)
        .set(ids.installation_editor_osc_canvas, ui);

    // OSC output address header.
    widget::Text::new("Audio Data OSC Output")
        .font_size(SMALL_FONT_SIZE)
        .top_left_of(ids.installation_editor_osc_canvas)
        .set(ids.installation_editor_osc_text, ui);

    fn update_addr(
        id: installation::Id,
        selected: &SelectedComputer,
        channels: &Channels,
        computers: &mut installation::Computers,
    ) {
        let socket = match selected.socket_string.parse() {
            Ok(s) => s,
            Err(_) => {
                eprintln!("could not parse socket string");
                return
            },
        };
        let osc_tx = match osc_sender(&socket) {
            Ok(tx) => tx,
            Err(err) => {
                eprintln!("coulc not connect osc_sender: {}", err);
                return;
            }
        };
        let osc_addr = selected.osc_addr.clone();
        let add =
            osc::output::OscTarget::Add(id, selected.computer, osc_tx, osc_addr.clone());
        let msg = osc::output::Message::Osc(add);
        if channels.osc_out_msg_tx.send(msg).ok().is_some() {
            let addr = installation::computer::Address { socket, osc_addr };
            computers.insert(selected.computer, addr);
        }
    }

    // The textbox for editing the OSC output IP address.
    let color = match selected_computer.socket_string.parse::<net::SocketAddrV4>() {
        Ok(socket) => {
            match installations[&id].computers[&selected_computer.computer].socket == socket {
                true => color::BLACK,
                false => color::DARK_GREEN.with_luminance(0.1),
            }
        }
        Err(_) => color::DARK_RED.with_luminance(0.1),
    };
    for event in widget::TextBox::new(&selected_computer.socket_string)
        .align_middle_x_of(ids.installation_editor_osc_canvas)
        .down(TEXT_PAD)
        .parent(ids.installation_editor_osc_canvas)
        .kid_area_w_of(ids.installation_editor_osc_canvas)
        .h(ITEM_HEIGHT)
        .font_size(SMALL_FONT_SIZE)
        .color(color)
        .set(ids.installation_editor_osc_ip_text_box, ui)
    {
        use nannou::ui::conrod::widget::text_box::Event;
        match event {
            Event::Enter => {
                let computers = &mut installations.get_mut(&id).unwrap().computers;
                update_addr(id, &selected_computer, channels, computers);
            }
            Event::Update(new_string) => {
                selected_computer.socket_string = new_string;
            }
        }
    }

    // The textbox for editing the OSC output address.
    for event in widget::TextBox::new(&selected_computer.osc_addr)
        .align_middle_x_of(ids.installation_editor_osc_canvas)
        .down(PAD)
        .parent(ids.installation_editor_osc_canvas)
        .kid_area_w_of(ids.installation_editor_osc_canvas)
        .h(ITEM_HEIGHT)
        .font_size(SMALL_FONT_SIZE)
        .set(ids.installation_editor_osc_address_text_box, ui)
    {
        use nannou::ui::conrod::widget::text_box::Event;
        match event {
            Event::Enter => {
                let computers = &mut installations.get_mut(&id).unwrap().computers;
                update_addr(id, &selected_computer, channels, computers);
            }
            Event::Update(new_string) => {
                selected_computer.osc_addr = new_string;
            }
        }
    }

    // The canvas for displaying the osc output address editor.
    widget::Canvas::new()
        .align_middle_x_of(ids.installation_editor_selected_canvas)
        .down_from(ids.installation_editor_osc_canvas, PAD)
        .parent(ids.installation_editor_selected_canvas)
        .color(color::CHARCOAL)
        .w(selected_canvas_kid_area.w())
        .h(soundscape_canvas_h)
        .pad(PAD)
        .set(ids.installation_editor_soundscape_canvas, ui);

    // OSC output address header.
    widget::Text::new("Soundscape - Simultaneous Sounds")
        .font_size(SMALL_FONT_SIZE)
        .top_left_of(ids.installation_editor_soundscape_canvas)
        .set(ids.installation_editor_soundscape_text, ui);

    /////////////////////////
    // SIMULTANEOUS SOUNDS //
    /////////////////////////

    let range = installations[&id].soundscape.simultaneous_sounds;
    let label = format!("{} to {} sounds at once", range.min, range.max);
    let total_min_num = 0.0;
    let total_max_num = 100.0;
    let min = range.min as f64;
    let max = range.max as f64;
    let total_min = total_min_num as f64;
    let total_max = total_max_num as f64;
    for (edge, value) in widget::RangeSlider::new(min, max, total_min, total_max)
        .skew(0.5)
        .kid_area_w_of(ids.installation_editor_soundscape_canvas)
        .h(SLIDER_H)
        .label_font_size(SMALL_FONT_SIZE)
        .color(ui::color::LIGHT_CHARCOAL)
        .align_left()
        .label(&label)
        .down(PAD * 2.0)
        .set(ids.installation_editor_soundscape_simultaneous_sounds_slider, ui)
    {
        let num = value as usize;

        // Update the local copy.
        let new_range = {
            let installation = installations.get_mut(&id).unwrap();
            match edge {
                widget::range_slider::Edge::Start => {
                    installation.soundscape.simultaneous_sounds.min = num;
                },
                widget::range_slider::Edge::End => {
                    installation.soundscape.simultaneous_sounds.max = num;
                }
            }
            installation.soundscape.simultaneous_sounds
        };

        // Update the soundscape copy.
        channels.soundscape.send(move |soundscape| {
            soundscape.update_installation(&id, |installation| {
                installation.simultaneous_sounds = new_range;
            });
        }).ok();
    }

    area.id
}
