//! A `Soundscape` panel displaying:
//!
//! - Play/Pause toggle for the soundscape.
//! - Groups panel for creating/removing soundscape source groups.

use gui::{collapsible_area, Gui, State};
use gui::{ITEM_HEIGHT, SMALL_FONT_SIZE};
use nannou::ui::prelude::*;
use serde_json;
use soundscape;
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;

/// GUI state related to the soundscape editor area.
pub struct SoundscapeEditor {
    pub is_open: bool,
    pub groups: HashMap<soundscape::group::Id, soundscape::group::Name>,
    pub next_group_id: soundscape::group::Id,
}

/// JSON friendly representation of the soundscape editor GUI state.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Stored {
    pub groups: HashMap<soundscape::group::Id, soundscape::group::Name>,
    pub next_group_id: soundscape::group::Id,
}

impl Stored {
    /// Load the stored soundscape groups from the given path.
    ///
    /// If the path is invalid or the JSON can't be read, `Stored::default` will be called.
    pub fn load(soundscape_path: &Path) -> Self {
        let stored = File::open(&soundscape_path)
            .ok()
            .and_then(|f| serde_json::from_reader(f).ok())
            .unwrap_or_else(Stored::default);
        stored
    }
}

/// Sets all widgets in the soundscape area and returns the `Id` of the last area.
pub fn set(last_area_id: widget::Id, gui: &mut Gui) -> widget::Id {
    let &mut Gui {
        ref mut ui,
        ref ids,
        channels,
        state:
            &mut State {
                ref mut soundscape_editor,
                ..
            },
        ..
    } = gui;

    // Constants to use as widget heights.
    const PAD: Scalar = 6.0;
    const IS_PLAYING_H: Scalar = ITEM_HEIGHT;
    let soundscape_editor_canvas_h = PAD + IS_PLAYING_H + PAD;

    // The collapsible area.
    let is_open = soundscape_editor.is_open;
    let (area, event) = collapsible_area(is_open, "Soundscape Editor", ids.side_menu)
        .align_middle_x_of(ids.side_menu)
        .down_from(last_area_id, 0.0)
        .set(ids.soundscape_editor, ui);
    if let Some(event) = event {
        soundscape_editor.is_open = event.is_open();
    }

    // If the area is open, get the area.
    let area = match area {
        Some(area) => area,
        None => return gui.ids.soundscape_editor,
    };

    // The canvas on which the soundscape editor will be placed.
    let canvas = widget::Canvas::new()
        .scroll_kids()
        .pad(PAD)
        .h(soundscape_editor_canvas_h);
    area.set(canvas, ui);

    // The toggle for whether or not the soundscape should be playing back.
    let is_playing = channels.soundscape.is_playing();
    let label = match is_playing {
        true => format!(">> PLAYING >>"),
        false => format!("|| PAUSED ||"),
    };
    for new_is_playing in widget::Toggle::new(is_playing)
        .h(ITEM_HEIGHT)
        .mid_top_of(area.id)
        .kid_area_w_of(area.id)
        .label(&label)
        .label_font_size(SMALL_FONT_SIZE)
        .set(ids.soundscape_editor_is_playing, ui)
    {
        if new_is_playing {
            channels.soundscape.play().ok();
        } else {
            channels.soundscape.pause().ok();
        }
    }

    //////////////////
    // GROUP EDITOR //
    //////////////////

    // If there are no groups, display some text for adding a group.
    if soundscape_editor.groups.is_empty() {

    } else {

    }

    area.id
}
