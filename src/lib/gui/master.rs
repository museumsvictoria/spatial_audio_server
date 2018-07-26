//! A "Master" side-bar widget providing control over master volume and input latency.

use gui::{collapsible_area, Gui};
use gui::{ITEM_HEIGHT, SMALL_FONT_SIZE};
use project::{self, Project};
use nannou::ui;
use nannou::ui::prelude::*;
use time_calc::Ms;
use metres::Metres;

pub fn set(last_area_id: widget::Id, gui: &mut Gui, project: &mut Project) -> widget::Id {
    let Gui {
        ref mut ui,
        ref audio_monitor,
        ref ids,
        ref channels,
        ref mut state,
        ..
    } = *gui;
    let Project {
        state: project::State {
            ref mut master,
            ..
        },
        ..
    } = *project;

    // The height of the list of installations.
    const PAD: Scalar = 6.0;
    const MASTER_VOLUME_H: Scalar = ITEM_HEIGHT;
    const LATENCY_H: Scalar = ITEM_HEIGHT;
    const DECIBEL_H: Scalar = ITEM_HEIGHT;
    const PROXIMITY_H: Scalar = ITEM_HEIGHT;
    const MASTER_H: Scalar = PAD + MASTER_VOLUME_H + PAD + LATENCY_H + PAD + DECIBEL_H + PAD + PROXIMITY_H + PAD;

    // The collapsible area widget.
    let is_open = state.is_open.master;
    let (area, event) = collapsible_area(is_open, "Master", ids.side_menu)
        .down_from(last_area_id, 0.0)
        .align_middle_x_of(last_area_id)
        .set(ids.master, ui);
    if let Some(event) = event {
        state.is_open.master = event.is_open();
    }

    // Return early if the panel is not open.
    let area = match area {
        None => return ids.master,
        Some(area) => area,
    };

    // The canvas on which the controls will be placed.
    let canvas = widget::Canvas::new().pad(PAD).h(MASTER_H);
    area.set(canvas, ui);

    // Display the peak volume as a gradient underlay below the slider.
    let peak = audio_monitor.master_peak;
    let left_hsla = ui::color::DARK_GREEN.to_hsl();
    let right_hsla = ui::color::DARK_RED.to_hsl();
    let hue_diff = right_hsla.0 - left_hsla.0;
    let hue = (left_hsla.0 + hue_diff * peak).min(1.0);
    let mut peak_hsla = right_hsla;
    peak_hsla.0 = hue;
    let left_color: ui::Color = left_hsla.into();
    let peak_color: ui::Color = peak_hsla.into();
    let left_rgba = left_color.into();
    let peak_rgba = peak_color.into();
    let canvas_kid_rect = ui.rect_of(area.id).unwrap().pad(PAD);
    let w = (canvas_kid_rect.w() * peak as f64).min(canvas_kid_rect.w());
    let rect = ui::Rect::from_xy_dim([0.0, 0.0], [w, MASTER_VOLUME_H])
        .align_top_of(canvas_kid_rect)
        .align_left_of(canvas_kid_rect);
    let tl = (rect.top_left(), left_rgba);
    let bl = (rect.bottom_left(), left_rgba);
    let tr = (rect.top_right(), peak_rgba);
    let br = (rect.bottom_right(), peak_rgba);
    let tri_a = widget::triangles::Triangle([tl, tr, br]);
    let tri_b = widget::triangles::Triangle([tl, br, bl]);
    let tris = Some(tri_a).into_iter().chain(Some(tri_b));
    widget::Triangles::multi_color(tris)
        .with_bounding_rect(rect)
        .set(ids.master_peak_meter, ui);

    // The master volume slider.
    let label = format!("Exhibition Volume: {:.2}", master.volume);
    for new_volume in widget::Slider::new(master.volume, 0.0, 1.0)
        .label(&label)
        .label_font_size(SMALL_FONT_SIZE)
        .kid_area_w_of(area.id)
        .h_of(ids.master_peak_meter)
        .align_middle_y_of(ids.master_peak_meter)
        .align_middle_x_of(area.id)
        .parent(ids.master_peak_meter)
        .border_color(ui::color::TRANSPARENT)
        .color(ui::color::LIGHT_CHARCOAL.alpha(0.6))
        .set(ids.master_volume, ui)
    {
        // Update the local master volume.
        master.volume = new_volume;

        // Update the audio output thread's master volume.
        channels
            .audio_output
            .send(move |audio| {
                audio.master_volume = new_volume;
            })
            .expect("failed to send updated master volume to audio output thread");
    }

    // The realtime source latency slider.
    let label = format!("Realtime Source Latency: {:.2} ms", master.realtime_source_latency.ms());
    let max_latency_ms = 2_000.0;
    let ms = master.realtime_source_latency.ms();
    for new_latency in widget::Slider::new(ms, 0.0, max_latency_ms)
        .label(&label)
        .label_font_size(SMALL_FONT_SIZE)
        .h(LATENCY_H)
        .kid_area_w_of(area.id)
        .align_middle_x_of(area.id)
        .down(PAD)
        .set(ids.master_realtime_source_latency, ui)
    {
        // Update the local copy.
        master.realtime_source_latency = Ms(new_latency);

        // Update the soundscape copy.
        channels
            .soundscape
            .send(move |soundscape| {
                soundscape.realtime_source_latency = Ms(new_latency);
            })
            .expect("failed to send updated realtime source latency volume to soundscape thread");
    }

    // The dbap slider.
    let label = format!("DBAP Rolloff: {:.2} db", master.dbap_rolloff_db);
    let max_rolloff = 6.0;
    for new_rolloff in widget::Slider::new(master.dbap_rolloff_db, 1.0, max_rolloff)
        .label(&label)
        .label_font_size(SMALL_FONT_SIZE)
        .h(DECIBEL_H)
        .kid_area_w_of(area.id)
        .align_middle_x_of(area.id)
        .down(PAD)
        .set(ids.master_dbap_rolloff, ui)
    {
        // Update the local rolloff.
        master.dbap_rolloff_db = new_rolloff;

        // Update the audio output thread's rolloff.
        channels
            .audio_output
            .send(move |audio| {
                audio.dbap_rolloff_db = new_rolloff;
            })
            .expect("failed to send updated DBAP rolloff to audio output thread");
    }
    
    // The proximity slider.
    // Proximity limit is stored as a squared value so sqrt() is called here
    let label = format!("Proximity Limit: {:.2} metres", master.proximity_limit_2.0.sqrt());
    for new_proximity in widget::Slider::new(master.proximity_limit_2.0.sqrt(), 0.0, 10.0)
        .label(&label)
        .label_font_size(SMALL_FONT_SIZE)
        .h(PROXIMITY_H)
        .kid_area_w_of(area.id)
        .align_middle_x_of(area.id)
        .down(PAD)
        .set(ids.master_proximity_limit, ui)
        {
            // Update the local rolloff.
            master.proximity_limit_2 = Metres(new_proximity * new_proximity);

            // Update the audio output thread's rolloff.
            channels
                .audio_output
                .send(move |audio| {
                    // The proximity squared (for more efficient distance comparisons).
                    audio.proximity_limit_2 = Metres(new_proximity * new_proximity);
                })
            .expect("failed to send updated proximity limit to audio output thread");
        }


    area.id
}
