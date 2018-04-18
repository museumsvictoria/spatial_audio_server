use gui::{collapsible_area, info_text, Gui};
use nannou::ui::prelude::*;
use project::Project;

pub fn set(
    last_area_id: widget::Id,
    gui: &mut Gui,
    project: &Project,
) -> widget::Id {
    let is_open = gui.state.is_open.interaction_log;
    let log_canvas_h = 200.0;
    let (area, event) = collapsible_area(is_open, "Interaction Log", gui.ids.side_menu)
        .align_middle_x_of(gui.ids.side_menu)
        .down_from(last_area_id, 0.0)
        .set(gui.ids.interaction_log, gui);
    if let Some(event) = event {
        gui.state.is_open.interaction_log = event.is_open();
    }

    if let Some(area) = area {
        // The canvas on which the log will be placed.
        let canvas = widget::Canvas::new()
            .scroll_kids()
            .pad(10.0)
            .h(log_canvas_h);
        area.set(canvas, gui);

        // The text widget used to display the log.
        let log_string = match gui.state.interaction_log.len() {
            0 => format!(
                "No interactions received yet.\nListening on port {}...",
                project.config.osc_input_port,
            ),
            _ => gui.state.interaction_log.format(),
        };
        info_text(&log_string)
            .top_left_of(area.id)
            .kid_area_w_of(area.id)
            .set(gui.ids.interaction_log_text, gui);

        // Scrollbars.
        widget::Scrollbar::y_axis(area.id)
            .color(color::LIGHT_CHARCOAL)
            .auto_hide(false)
            .set(gui.ids.interaction_log_scrollbar_y, gui);
        widget::Scrollbar::x_axis(area.id)
            .color(color::LIGHT_CHARCOAL)
            .auto_hide(true)
            .set(gui.ids.interaction_log_scrollbar_x, gui);

        area.id
    } else {
        gui.ids.interaction_log
    }
}
