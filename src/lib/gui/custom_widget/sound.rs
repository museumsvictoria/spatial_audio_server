//! A visual representation of a `Sound` for displaying over the floorplan.

use metres::Metres;
use nannou::ui::Color;
use nannou::ui::prelude::*;
use std;

#[derive(Clone, WidgetCommon)]
pub struct Sound<'a> {
    #[conrod(common_builder)]
    common: widget::CommonBuilder,
    style: Style,
    // Amplitude per channel.
    channels: &'a [f32],
    // The distance from each channel to the centre of the sound as a Scalar value over the
    // floorplan.
    spread: Scalar,
    // The direction that the sound is facing.
    radians: f64,
    // The rotation offset for the channels around the sound's centre.
    channel_radians: f64,
}

#[derive(Copy, Clone, Debug, Default, PartialEq, WidgetStyle)]
pub struct Style {
    #[conrod(default = "Sound::DEFAULT_COLOR")]
    pub color: Option<Color>,
}

widget_ids! {
    struct Ids {
        circle,
        triangle,
        channel_lines[],
        channel_circles[],
        channel_labels[],
    }
}

pub struct State {
    ids: Ids,
}

pub fn dimension_metres(amplitude: f32) -> Metres {
    let min = Sound::MIN_DIMENSION.0;
    let max = Sound::MAX_DIMENSION.0;
    Metres(min + (max - min) * amplitude as Scalar)
}

impl<'a> Sound<'a> {
    pub const DEFAULT_COLOR: Color = color::BLUE;
    pub const MIN_DIMENSION: Metres = Metres(0.5);
    pub const MAX_DIMENSION: Metres = Metres(1.0);

    pub fn new(channels: &'a [f32], spread: Scalar, radians: f64, channel_radians: f64) -> Self {
        Sound {
            common: widget::CommonBuilder::default(),
            style: Style::default(),
            channels,
            spread,
            radians,
            channel_radians,
        }
    }
}

impl<'a> Widget for Sound<'a> {
    type State = State;
    type Style = Style;
    type Event = ();

    fn init_state(&self, id_gen: widget::id::Generator) -> Self::State {
        State {
            ids: Ids::new(id_gen),
        }
    }

    fn style(&self) -> Self::Style {
        self.style.clone()
    }

    fn update(self, args: widget::UpdateArgs<Self>) -> Self::Event {
        let widget::UpdateArgs { id, state, style, rect, ui, .. } = args;
        let Sound { channels, spread, radians, channel_radians, .. } = self;

        let (x, y, w, _) = rect.x_y_w_h();
        let radius = w / 2.0;

        // The circle of the sound's source position.
        let color = style.color(&ui.theme);
        let color = match ui.widget_input(id).mouse() {
            Some(mouse) =>
                if mouse.buttons.left().is_down() { color.clicked() }
                else { color.highlighted() },
            None => color,
        };
        widget::Circle::fill(radius)
            .x_y(x, y)
            .color(color)
            .graphics_for(id)
            .parent(id)
            .set(state.ids.circle, ui);

        // Calculate the position of each channel around the sound's position.
        //
        // TODO: This is a copy of audio::channel_point but for scalar values instead of metres -
        // should probably abstract the common stuff between these.
        fn channel_point(
            (sound_x, sound_y): (Scalar, Scalar),
            channel_index: usize,
            total_channels: usize,
            spread: Scalar,
            radians: f64,
        ) -> (Scalar, Scalar)
        {
            assert!(channel_index < total_channels);
            if total_channels == 1 {
                (sound_x, sound_y)
            } else {
                let phase = channel_index as f64 / total_channels as f64;
                let default_radians = phase * std::f64::consts::PI * 2.0;
                let radians = radians + default_radians;
                let rel_x = -radians.cos() * spread;
                let rel_y = radians.sin() * spread;
                let x = sound_x + rel_x;
                let y = sound_y + rel_y;
                (x, y)
            }
        }

        // Ensure there is an ID for each channel.
        if state.ids.channel_circles.len() < channels.len() {
            let id_gen = &mut ui.widget_id_generator();
            state.update(|state| {
                state.ids.channel_circles.resize(channels.len(), id_gen);
                state.ids.channel_lines.resize(channels.len(), id_gen);
                state.ids.channel_labels.resize(channels.len(), id_gen);
            });
        }

        // Instantiate a circle for each channel position.
        for (i, &amp) in channels.iter().enumerate() {
            let circle_id = state.ids.channel_circles[i];
            let line_id = state.ids.channel_lines[i];
            let label_id = state.ids.channel_labels[i];
            let (ch_x, ch_y) = channel_point((x, y), i, channels.len(), spread, radians + channel_radians);

            let base_thickness = 1.0;
            let amp_thickness = amp as f64 * 10.0;
            let thickness = base_thickness + amp_thickness;
            widget::Line::abs([x, y], [ch_x, ch_y])
                .color(color.alpha(0.5))
                .thickness(thickness)
                .graphics_for(id)
                .parent(id)
                .set(line_id, ui);

            let radius_amp = radius * amp as f64;
            let channel_radius = radius * 0.75 + radius_amp;
            widget::Circle::fill(channel_radius)
                .x_y(ch_x, ch_y)
                .color(color)
                .graphics_for(id)
                .parent(id)
                .set(circle_id, ui);

            let label = format!("{}", i+1);
            widget::Text::new(&label)
                .font_size((radius * 0.8) as FontSize)
                .x_y(ch_x, ch_y + radius / 6.0)
                .color(color.plain_contrast())
                .graphics_for(id)
                .parent(id)
                .set(label_id, ui);
        }

        // The triangle pointing in the direction that the sound is facing.
        let tri_radius = radius * 0.4;
        let front_to_back_radians = std::f64::consts::PI * 2.5 / 3.0;
        let br_radians = radians + front_to_back_radians;
        let bl_radians = radians - front_to_back_radians;
        let rel_front = [x + -radians.cos() * tri_radius, y + radians.sin() * tri_radius];
        let rel_back_right = [x + -bl_radians.cos() * tri_radius, y + bl_radians.sin() * tri_radius];
        let rel_back_left = [x + -br_radians.cos() * tri_radius, y + br_radians.sin() * tri_radius];
        let points = [rel_front, rel_back_right, rel_back_left];
        widget::Polygon::centred_fill(points.iter().cloned())
            .x_y(x, y)
            .color(color.plain_contrast().alpha(0.5))
            .graphics_for(id)
            .parent(id)
            .set(state.ids.triangle, ui);
    }
}

impl<'a> Colorable for Sound<'a> {
    fn color(mut self, color: Color) -> Self {
        self.style.color = Some(color);
        self
    }
}
