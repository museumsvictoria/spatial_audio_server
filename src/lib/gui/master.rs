//! A "Master" side-bar widget providing control over master volume and input latency.

use audio;
use gui::{collapsible_area, Gui, State};
use gui::{ITEM_HEIGHT, SMALL_FONT_SIZE};
use nannou::ui::prelude::*;
use serde_json;
use std::fs::File;
use std::path::Path;
use time_calc::Ms;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Master {
    /// The master volume for the exhibition, between 0.0 (silence) and 1.0 (full).
    #[serde(default = "default_master_volume")]
    pub volume: f32,
    /// The latency applied to real-time input sources.
    #[serde(default = "default_realtime_source_latency")]
    pub realtime_source_latency: Ms,
    /// Whether or not the collapsible panel is open.
    pub is_open: bool,
}

impl Default for Master {
    fn default() -> Self {
        Master {
            volume: default_master_volume(),
            realtime_source_latency: default_realtime_source_latency(),
            is_open: false,
        }
    }
}

fn default_master_volume() -> f32 {
    audio::DEFAULT_MASTER_VOLUME
}

fn default_realtime_source_latency() -> Ms {
    audio::DEFAULT_REALTIME_SOURCE_LATENCY
}

impl Master {
    /// Load the master state from the json at the given path.
    pub fn load(master_path: &Path) -> Self {
        File::open(&master_path)
            .ok()
            .and_then(|f| serde_json::from_reader(f).ok())
            .unwrap_or_else(Default::default)
    }
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
    const MASTER_H: Scalar = PAD + MASTER_VOLUME_H + PAD + LATENCY_H + PAD;

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

    // The master volume slider.
    let label = format!("Exhibition Volume: {:.2}", master.volume);
    for new_volume in widget::Slider::new(master.volume, 0.0, 1.0)
        .label(&label)
        .label_font_size(SMALL_FONT_SIZE)
        .h(MASTER_VOLUME_H)
        .kid_area_w_of(area.id)
        .mid_top_of(area.id)
        .set(ids.master_volume, ui)
    {
        // Update the local master volume.
        master.volume = new_volume;

        // Update the audio output thread's master volume.
        channels.audio_output.send(move |audio| {
            audio.master_volume = new_volume;
        }).ok();
    }

    // The master volume slider.
    let label = format!("Realtime Source Latency: {:.2} ms", master.realtime_source_latency.ms());
    let max_latency = 2_000.0;
    for new_latency in widget::Slider::new(master.realtime_source_latency.ms(), 0.0, max_latency)
        .label(&label)
        .label_font_size(SMALL_FONT_SIZE)
        .h(LATENCY_H)
        .kid_area_w_of(area.id)
        .align_middle_x_of(area.id)
        .down(PAD)
        .set(ids.master_realtime_source_latency, ui)
    {
        // Update the local master volume.
        master.realtime_source_latency = Ms(new_latency);
    }

    area.id
}
