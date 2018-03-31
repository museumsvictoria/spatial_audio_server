use gui::{collapsible_area, Channels, Gui, State};
use gui::{ITEM_HEIGHT, SMALL_FONT_SIZE};
use installation::{self, ComputerId, Installation};
use nannou;
use nannou::osc::Connected;
use nannou::ui;
use nannou::ui::prelude::*;
use osc;
use std::collections::HashMap;
use std::{io, net};
use std::path::Path;
use utils;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Address {
    // The IP address of the target installation computer.
    pub socket: net::SocketAddrV4,
    // The OSC address string.
    pub osc_addr: String,
}

pub type AddressMap = HashMap<ComputerId, Address>;
pub type ComputerMap = HashMap<Installation, AddressMap>;

pub struct InstallationEditor {
    pub is_open: bool,
    pub selected: Option<Selected>,
    pub computer_map: ComputerMap,
}

pub struct Selected {
    index: usize,
    selected_computer: Option<SelectedComputer>,
}

pub struct SelectedComputer {
    computer: ComputerId,
    socket_string: String,
    osc_addr: String,
}

/// Create the default computer map.
pub fn default_computer_map() -> ComputerMap {
    installation::ALL
        .iter()
        .map(|&inst| {
            let map = (0..inst.default_num_computers())
                .map(|i| {
                    let computer = ComputerId(i);
                    let socket = "127.0.0.1:9002".parse().unwrap();
                    let osc_addr_base = inst.default_osc_addr_str().to_string();
                    let osc_addr = format!("/{}/{}", osc_addr_base, i);
                    let addr = Address { socket, osc_addr };
                    (computer, addr)
                })
                .collect();
            (inst, map)
        })
        .collect()
}

/// Load the computer map from file or fall back to the default.
pub fn load_computer_map(path: &Path) -> ComputerMap {
    utils::load_from_json(path).ok().unwrap_or_else(default_computer_map)
}

pub fn set(last_area_id: widget::Id, gui: &mut Gui) -> widget::Id {
    let &mut Gui {
        ref mut ui,
        ref ids,
        channels,
        state:
            &mut State {
                installation_editor:
                    InstallationEditor {
                        ref mut is_open,
                        ref mut selected,
                        ref mut computer_map,
                    },
                ..
            },
        ..
    } = gui;

    // The height of the list of installations.
    const LIST_HEIGHT: Scalar = ITEM_HEIGHT * 5.0;
    const COMPUTER_LIST_HEIGHT: Scalar = ITEM_HEIGHT * 3.0;
    const PAD: Scalar = 6.0;
    const TEXT_PAD: Scalar = PAD * 2.0;

    // The height of the canvas displaying options for the selected installation.
    //
    // These options include:
    //
    // - Music Data OSC Output (Text and TextBox)
    let osc_canvas_h = PAD + ITEM_HEIGHT * 3.0 + PAD;
    let computer_canvas_h = ITEM_HEIGHT + PAD + ITEM_HEIGHT + PAD + COMPUTER_LIST_HEIGHT;
    let selected_canvas_h = PAD + computer_canvas_h + PAD + osc_canvas_h + PAD;

    // The total height of the installation editor as a sum of the previous heights plus necessary
    // padding.
    let installation_editor_h = LIST_HEIGHT + selected_canvas_h;

    let (area, event) = collapsible_area(*is_open, "Installation Editor", ids.side_menu)
        .align_middle_x_of(ids.side_menu)
        .down_from(last_area_id, 0.0)
        .set(ids.installation_editor, ui);
    if let Some(event) = event {
        *is_open = event.is_open();
    }

    // If the area is open, continue. If its closed, return the editor id as the last id.
    let area = match area {
        Some(area) => area,
        None => return ids.installation_editor,
    };

    // The canvas on which the installation editor widgets will be placed.
    let canvas = widget::Canvas::new().pad(0.0).h(installation_editor_h);
    area.set(canvas, ui);

    // Display the installation list.
    let num_items = installation::ALL.len();
    let (mut events, scrollbar) = widget::ListSelect::single(num_items)
        .item_size(ITEM_HEIGHT)
        .h(LIST_HEIGHT)
        .align_middle_x_of(area.id)
        .align_top_of(area.id)
        .scrollbar_color(color::LIGHT_CHARCOAL)
        .scrollbar_next_to()
        .set(ids.installation_editor_list, ui);

    while let Some(event) = events.next(ui, |i| selected.as_ref().map(|s| s.index) == Some(i)) {
        use self::ui::widget::list_select::Event;
        match event {
            // Instantiate a button for each installation.
            Event::Item(item) => {
                let installation =
                    Installation::from_usize(item.i).expect("no installation for index");
                let is_selected = selected.as_ref().map(|s| s.index) == Some(item.i);
                // Blue if selected, gray otherwise.
                let color = if is_selected {
                    color::BLUE
                } else {
                    color::CHARCOAL
                };
                let label = installation.display_str();

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
                let installation =
                    Installation::from_usize(index).expect("no installation for index");
                let addresses = &computer_map[&installation];
                let selected_computer = match addresses.len() {
                    0 => None,
                    _ => {
                        let computer = ComputerId(0);
                        let (socket_string, osc_addr) = {
                            let address = &computer_map[&installation][&computer];
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
                    index,
                    selected_computer,
                });
            }

            _ => (),
        }
    }

    if let Some(scrollbar) = scrollbar {
        scrollbar.set(ui);
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
        ref mut index,
        ref mut selected_computer,
    } = *selected;
    let installation = Installation::from_usize(*index).expect("no installation for index");

    // The canvas for displaying the computer selection / editor.
    widget::Canvas::new()
        .mid_top_of(ids.installation_editor_selected_canvas)
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
    let n_computers = computer_map[&installation].len();
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
        let computers = computer_map.get_mut(&installation).unwrap();
        if n_computers < n {
            for i in n_computers..n {
                let computer = ComputerId(i);
                let socket = "127.0.0.1:9002".parse().unwrap();
                let osc_addr_base = installation.default_osc_addr_str();
                let osc_addr = format!("/{}/{}", osc_addr_base, i);
                let osc_tx = match osc_sender(&socket) {
                    Err(err) => {
                        eprintln!("failed to connect localhost OSC sender: {}", err);
                        break;
                    },
                    Ok(tx) => tx,
                };
                let add =
                    osc::output::OscTarget::Add(installation, computer, osc_tx, osc_addr.clone());
                let msg = osc::output::Message::Osc(add);
                if channels.osc_out_msg_tx.send(msg).ok().is_some() {
                    let addr = Address { socket, osc_addr };
                    computers.insert(computer, addr);
                }
            }
        } else if n_computers > n {
            for i in n..n_computers {
                let computer = ComputerId(i);
                let rem = osc::output::OscTarget::Remove(installation, computer);
                let msg = osc::output::Message::Osc(rem);
                if channels.osc_out_msg_tx.send(msg).ok().is_some() {
                    computers.remove(&computer);
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
    let n_computers = computer_map[&installation].len();
    let (mut events, scrollbar) = widget::ListSelect::single(n_computers)
        .item_size(ITEM_HEIGHT)
        .h(COMPUTER_LIST_HEIGHT)
        .align_middle_x_of(ids.installation_editor_computer_canvas)
        .down_from(ids.installation_editor_computer_number, PAD)
        .scrollbar_color(color::LIGHT_CHARCOAL)
        .scrollbar_next_to()
        .set(ids.installation_editor_computer_list, ui);

    while let Some(event) = events.next(ui, |i| {
        selected_computer.as_ref().map(|s| s.computer) == Some(ComputerId(i))
    }) {
        use self::ui::widget::list_select::Event;
        match event {
            // Instantiate a button for each computer.
            Event::Item(item) => {
                let computer = ComputerId(item.i);
                let is_selected = selected_computer.as_ref().map(|s| s.computer) == Some(computer);
                // Blue if selected, gray otherwise.
                let color = if is_selected {
                    color::BLUE
                } else {
                    color::BLACK
                };
                let addr = &computer_map[&installation][&computer];
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
                let computer = ComputerId(index);
                let addr = &computer_map[&installation][&computer];
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
        installation: Installation,
        selected: &SelectedComputer,
        channels: &Channels,
        computer_map: &mut ComputerMap,
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
            osc::output::OscTarget::Add(installation, selected.computer, osc_tx, osc_addr.clone());
        let msg = osc::output::Message::Osc(add);
        if channels.osc_out_msg_tx.send(msg).ok().is_some() {
            let addr = Address { socket, osc_addr };
            computer_map
                .get_mut(&installation)
                .unwrap()
                .insert(selected.computer, addr);
        }
    }

    // The textbox for editing the OSC output IP address.
    let color = match selected_computer.socket_string.parse::<net::SocketAddrV4>() {
        Ok(socket) => {
            match computer_map[&installation][&selected_computer.computer].socket == socket {
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
                update_addr(installation, &selected_computer, channels, computer_map);
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
                update_addr(installation, &selected_computer, channels, computer_map);
            }
            Event::Update(new_string) => {
                selected_computer.osc_addr = new_string;
            }
        }
    }

    area.id
}
