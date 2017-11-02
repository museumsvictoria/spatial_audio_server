use nannou::ui::{self, widget};
use nannou::ui::theme::WidgetDefault;
use std;

/// The default width for a widget within a column.
pub const DEFAULT_WIDTH: ui::Scalar = 220.0;


fn common_style(w: ui::Scalar, h: ui::Scalar) -> widget::CommonStyle {
    widget::CommonStyle {
        maybe_x_dimension: Some(ui::position::Dimension::Absolute(w)),
        maybe_y_dimension: Some(ui::position::Dimension::Absolute(h)),
        ..widget::CommonStyle::default()
    }
}

fn id<T>() -> std::any::TypeId
where
    T: std::any::Any,
{
    std::any::TypeId::of::<T>()
}


/// The theme to use for the SynthEditor.
pub fn construct() -> ui::Theme {
    ui::Theme {
        name: "Monochroma".to_owned(),
        padding: ui::position::Padding {
            x: ui::Range::new(20.0, 20.0),
            y: ui::Range::new(20.0, 20.0),
        },
        background_color: ui::color::DARK_CHARCOAL,
        shape_color: ui::color::BLACK,
        border_width: 0.0,
        border_color: ui::color::BLACK,
        label_color: ui::color::WHITE,

        widget_styling: {
            let mut map = ui::theme::StyleMap::default();

            map.insert(
                id::<widget::button::Style>(),
                WidgetDefault {
                    common: common_style(DEFAULT_WIDTH, 32.0),
                    style: Box::new(widget::button::Style::default()),
                },
            );

            map.insert(
                id::<widget::drop_down_list::Style>(),
                WidgetDefault {
                    common: common_style(DEFAULT_WIDTH, 32.0),
                    style: Box::new(widget::drop_down_list::Style::default()),
                },
            );

            map.insert(
                id::<widget::number_dialer::Style>(),
                WidgetDefault {
                    common: common_style(DEFAULT_WIDTH, 32.0),
                    style: Box::new(widget::number_dialer::Style::default()),
                },
            );

            map.insert(
                id::<widget::slider::Style>(),
                WidgetDefault {
                    common: common_style(DEFAULT_WIDTH, 32.0),
                    style: Box::new(widget::slider::Style {
                        color: Some(ui::color::LIGHT_CHARCOAL),
                        border_color: Some(ui::color::rgb(0.1, 0.1, 0.1)),
                        ..widget::slider::Style::default()
                    }),
                },
            );

            map.insert(
                id::<widget::text_box::Style>(),
                WidgetDefault {
                    common: common_style(DEFAULT_WIDTH, 36.0),
                    style: Box::new(widget::text_box::Style::default()),
                },
            );

            map.insert(
                id::<widget::toggle::Style>(),
                WidgetDefault {
                    common: common_style(DEFAULT_WIDTH, 32.0),
                    style: Box::new(widget::toggle::Style::default()),
                },
            );

            map.insert(
                id::<widget::canvas::Style>(),
                WidgetDefault {
                    common: widget::CommonStyle::default(),
                    style: {
                        let mut style = widget::canvas::Style::default();
                        let luminance = ui::color::DARK_CHARCOAL.luminance() * 0.5;
                        let color = ui::color::DARK_CHARCOAL.with_luminance(luminance);
                        style.title_bar_color = Some(Some(color));
                        Box::new(style)
                    },
                },
            );

            map
        },

        ..ui::Theme::default()
    }
}
