//! A "Master" side-bar widget providing control over master volume and input latency.

use audio;
use gui::{collapsible_area, Gui, State};
use gui::{ITEM_HEIGHT, SMALL_FONT_SIZE};
use nannou::ui;
use nannou::ui::prelude::*;
use time_calc::Ms;

#[derive(Clone, Debug)]
pub struct Master {
    /// Controllable, serializable parameters.
    pub params: Parameters,
    /// Whether or not the collapsible panel is open.
    pub is_open: bool,
    /// The last received peak master volume value.
    pub peak: f32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Parameters {
    /// The master volume for the exhibition, between 0.0 (silence) and 1.0 (full).
    #[serde(default = "default_master_volume")]
    pub volume: f32,
    /// The latency applied to real-time input sources.
    #[serde(default = "default_realtime_source_latency")]
    pub realtime_source_latency: Ms,
    /// The rolloff decibel amount, used to attenuate speaker gains over distances.
    #[serde(default = "default_dbap_rolloff_db")]
    pub dbap_rolloff_db: f64,
}

impl Default for Master {
    fn default() -> Self {
        Self::from_params(Default::default())
    }
}

impl Default for Parameters {
    fn default() -> Self {
        let volume = default_master_volume();
        let realtime_source_latency = default_realtime_source_latency();
        let dbap_rolloff_db = default_dbap_rolloff_db();
        Parameters { volume, realtime_source_latency, dbap_rolloff_db }
    }
}

impl Master {
    /// Construct a `Master` from the given set of parameters.
    pub fn from_params(params: Parameters) -> Self {
        let is_open = false;
        let peak = 0.0;
        Master { is_open, peak, params }
    }
}

fn default_master_volume() -> f32 {
    audio::DEFAULT_MASTER_VOLUME
}

fn default_realtime_source_latency() -> Ms {
    audio::DEFAULT_REALTIME_SOURCE_LATENCY
}

fn default_dbap_rolloff_db() -> f64 {
    audio::DEFAULT_DBAP_ROLLOFF_DB
}

pub fn set(gui: &mut Gui) -> widget::Id {
    let &mut Gui {
        ref mut ui,
        state: &mut State {
            ref mut master,
            ..
        },
        ref ids,
        ref channels,
        ..
    } = gui;

    // The height of the list of installations.
    const PAD: Scalar = 6.0;
    const MASTER_VOLUME_H: Scalar = ITEM_HEIGHT;
    const LATENCY_H: Scalar = ITEM_HEIGHT;
    const DECIBEL_H: Scalar = ITEM_HEIGHT;
    const MASTER_H: Scalar = PAD + MASTER_VOLUME_H + PAD + LATENCY_H + PAD + DECIBEL_H + PAD;

    // The collapsible area widget.
    let is_open = master.is_open;
    let (area, event) = collapsible_area(is_open, "Master", ids.side_menu)
        .mid_top_of(ids.side_menu)
        .set(ids.master, ui);
    if let Some(event) = event {
        master.is_open = event.is_open();
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
    let left_hsla = ui::color::DARK_GREEN.to_hsl();
    let right_hsla = ui::color::DARK_RED.to_hsl();
    let hue_diff = right_hsla.0 - left_hsla.0;
    let hue = (left_hsla.0 + hue_diff * master.peak).min(1.0);
    let mut peak_hsla = right_hsla;
    peak_hsla.0 = hue;
    let left_color: ui::Color = left_hsla.into();
    let peak_color: ui::Color = peak_hsla.into();
    let left_rgba = left_color.into();
    let peak_rgba = peak_color.into();
    let canvas_kid_rect = ui.rect_of(area.id).unwrap().pad(PAD);
    let w = (canvas_kid_rect.w() * master.peak as f64).min(canvas_kid_rect.w());
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
    let label = format!("Exhibition Volume: {:.2}", master.params.volume);
    for new_volume in widget::Slider::new(master.params.volume, 0.0, 1.0)
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
        master.params.volume = new_volume;

        // Update the audio output thread's master volume.
        channels.audio_output.send(move |audio| {
            audio.master_volume = new_volume;
        }).ok();
    }

    // The realtime source latency slider.
    let label = format!("Realtime Source Latency: {:.2} ms", master.params.realtime_source_latency.ms());
    let max_latency_ms = 2_000.0;
    let ms = master.params.realtime_source_latency.ms();
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
        master.params.realtime_source_latency = Ms(new_latency);

        // Update the soundscape copy.
        channels
            .soundscape
            .send(move |soundscape| {
                soundscape.realtime_source_latency = Ms(new_latency);
            })
            .ok();
    }

    // The master volume slider.
    let label = format!("DBAP Rolloff: {:.2} db", master.params.dbap_rolloff_db);
    let max_rolloff = 6.0;
    for new_rolloff in widget::Slider::new(master.params.dbap_rolloff_db, 1.0, max_rolloff)
        .label(&label)
        .label_font_size(SMALL_FONT_SIZE)
        .h(DECIBEL_H)
        .kid_area_w_of(area.id)
        .align_middle_x_of(area.id)
        .down(PAD)
        .set(ids.master_dbap_rolloff, ui)
    {
        // Update the local rolloff.
        master.params.dbap_rolloff_db = new_rolloff;

        // Update the audio output thread's rolloff.
        channels.audio_output.send(move |audio| {
            audio.dbap_rolloff_db = new_rolloff;
        }).ok();
    }

    area.id
}
