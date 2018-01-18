use gui::{collapsible_area, Channels, Gui, State};
use gui::{ITEM_HEIGHT, SMALL_FONT_SIZE};
use installation::{self, Installation};
use nannou;
use nannou::osc::Connected;
use nannou::ui;
use nannou::ui::prelude::*;
use osc;
use std::collections::HashMap;
use std::net;

#[derive(Clone, Debug)]
#[derive(Deserialize, Serialize)]
pub struct Address {
    // The IP address of the target installation computer.
    pub socket: net::SocketAddrV4,
    // The OSC address string.
    pub osc_addr: String,
}

pub type AddressMap = HashMap<Installation, Address>;

pub struct InstallationEditor {
    pub is_open: bool,
    pub selected: Option<Selected>,
    pub address_map: AddressMap,
}

pub struct Selected {
    index: usize,
    socket_string: String,
    osc_addr: String,
}

pub fn set(gui: &mut Gui) -> widget::Id {
    let &mut Gui {
        ref mut ui,
        ref ids,
        channels,
        state: &mut State {
            installation_editor: InstallationEditor {
                ref mut is_open,
                ref mut selected,
                ref mut address_map,
            },
            ..
        },
        ..
    } = gui;

    // The height of the list of installations.
    const LIST_HEIGHT: Scalar = ITEM_HEIGHT * 5.0;
    const PAD: Scalar = 6.0;
    const TEXT_PAD: Scalar = PAD * 2.0;

    // The height of the canvas displaying options for the selected installation.
    //
    // These options include:
    //
    // - Music Data OSC Output (Text and TextBox)
    let osc_canvas_h = PAD * 2.0 + ITEM_HEIGHT * 3.0;
    let selected_canvas_h  = osc_canvas_h + PAD * 2.0;

    // The total height of the installation editor as a sum of the previous heights plus necessary
    // padding..
    let installation_editor_h = LIST_HEIGHT + selected_canvas_h;

    let (area, event) = collapsible_area(*is_open, "Installation Editor", ids.side_menu)
        .mid_top_of(ids.side_menu)
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
    let canvas = widget::Canvas::new()
        .pad(0.0)
        .h(installation_editor_h);
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
                let installation = Installation::from_usize(item.i).expect("no installation for index");
                let is_selected = selected.as_ref().map(|s| s.index) == Some(item.i);
                // Blue if selected, gray otherwise.
                let color = if is_selected { color::BLUE } else { color::CHARCOAL };
                let label = installation.display_str();

                // Use `Button`s for the selectable items.
                let button = widget::Button::new()
                    .label(&label)
                    .label_font_size(SMALL_FONT_SIZE)
                    .label_x(position::Relative::Place(position::Place::Start(Some(10.0))))
                    .color(color);
                item.set(button, ui);
            },

            // Update the selected source.
            Event::Selection(index) => {
                let installation = Installation::from_usize(index).expect("no installation for index");
                let (socket_string, osc_addr) = {
                    let address = &address_map[&installation];
                    (format!("{}", address.socket), address.osc_addr.clone())
                };
                *selected = Some(Selected { index, socket_string, osc_addr });
            },

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
    let selected_canvas_kid_area = ui.kid_area_of(ids.installation_editor_selected_canvas).unwrap();

    let selected = match *selected {
        Some(ref mut selected) => selected,
        None => return area.id,
    };

    let installation = Installation::from_usize(selected.index).expect("no installation for index");

    // The canvas for displaying the osc output address editor.
    widget::Canvas::new()
        .mid_top_of(ids.installation_editor_selected_canvas)
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

    fn osc_sender(socket: &net::SocketAddrV4) -> nannou::osc::Sender<Connected> {
        nannou::osc::sender()
            .expect("failed to create OSC sender")
            .connect(socket)
            .expect("failed to connect OSC sender")
    }

    fn update_addr(
        installation: Installation,
        selected: &Selected,
        channels: &Channels,
        address_map: &mut AddressMap,
    ) {
        let socket = match selected.socket_string.parse() {
            Ok(s) => s,
            Err(_) => return,
        };
        let osc_tx = osc_sender(&socket);
        let osc_addr = selected.osc_addr.clone();
        let add = osc::output::OscTarget::Add(installation, osc_tx, osc_addr.clone());
        let msg = osc::output::Message::Osc(add);
        if channels.osc_out_msg_tx.send(msg).ok().is_some() {
            let addr = Address { socket, osc_addr };
            address_map.insert(installation, addr);
        }
    }

    // The textbox for editing the OSC output IP address.
    let color = match selected.socket_string.parse::<net::SocketAddrV4>() {
        Ok(socket) => match address_map[&installation].socket == socket {
            true => color::BLACK,
            false => color::DARK_GREEN.with_luminance(0.1),
        },
        Err(_) => color::DARK_RED.with_luminance(0.1),
    };
    for event in widget::TextBox::new(&selected.socket_string)
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
                update_addr(installation, &selected, channels, address_map);
            },
            Event::Update(new_string) => {
                selected.socket_string = new_string;
            },
        }
    }

    // The textbox for editing the OSC output address.
    for event in widget::TextBox::new(&selected.osc_addr)
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
                update_addr(installation, &selected, channels, address_map);
            },
            Event::Update(new_string) => {
                selected.osc_addr = new_string;
            },
        }
    }

    area.id
}
