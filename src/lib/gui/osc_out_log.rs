use crate::gui::{collapsible_area, info_text, Gui};
use conrod_core as ui;
use nannou_conrod::prelude::*;
use ui::{color, widget};

pub fn set(last_area_id: widget::Id, gui: &mut Gui) -> widget::Id {
    let is_open = gui.state.is_open.osc_out_log;
    let log_canvas_h = 200.0;
    let (area, event) = collapsible_area(is_open, "OSC Output Log", gui.ids.side_menu)
        .align_middle_x_of(gui.ids.side_menu)
        .down_from(last_area_id, 0.0)
        .set(gui.ids.osc_out_log, gui);
    if let Some(event) = event {
        gui.state.is_open.osc_out_log = event.is_open();
    }
    if let Some(area) = area {
        // The canvas on which the log will be placed.
        let canvas = widget::Canvas::new()
            .scroll_kids()
            .pad(10.0)
            .h(log_canvas_h);
        area.set(canvas, gui);

        // The text widget used to display the log.
        let log_string = match gui.state.osc_out_log.len() {
            0 => "No messages sent yet.".into(),
            _ => gui.state.osc_out_log.format(),
        };
        info_text(&log_string)
            .top_left_of(area.id)
            .kid_area_w_of(area.id)
            .set(gui.ids.osc_out_log_text, gui);

        // Scrollbars.
        widget::Scrollbar::y_axis(area.id)
            .color(color::LIGHT_CHARCOAL)
            .auto_hide(false)
            .set(gui.ids.osc_out_log_scrollbar_y, gui);
        widget::Scrollbar::x_axis(area.id)
            .color(color::LIGHT_CHARCOAL)
            .auto_hide(true)
            .set(gui.ids.osc_out_log_scrollbar_x, gui);

        area.id
    } else {
        gui.ids.osc_out_log
    }
}
