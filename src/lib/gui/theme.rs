use conrod::{self, widget};
use conrod::theme::WidgetDefault;
use std;

/// The default width for a widget within a column.
pub const DEFAULT_WIDTH: conrod::Scalar = 220.0;


fn common_style(w: conrod::Scalar, h: conrod::Scalar) -> widget::CommonStyle {
    widget::CommonStyle {
        maybe_x_dimension: Some(conrod::position::Dimension::Absolute(w)),
        maybe_y_dimension: Some(conrod::position::Dimension::Absolute(h)),
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
pub fn construct() -> conrod::Theme {
    conrod::Theme {
        name: "Monochroma".to_owned(),
        padding: conrod::position::Padding {
            x: conrod::Range::new(20.0, 20.0),
            y: conrod::Range::new(20.0, 20.0),
        },
        background_color: conrod::color::DARK_CHARCOAL,
        shape_color: conrod::color::BLACK,
        border_width: 0.0,
        border_color: conrod::color::BLACK,
        label_color: conrod::color::WHITE,

        widget_styling: {
            let mut map = conrod::theme::StyleMap::default();

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
                        color: Some(conrod::color::LIGHT_CHARCOAL),
                        border_color: Some(conrod::color::rgb(0.1, 0.1, 0.1)),
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
                        let luminance = conrod::color::DARK_CHARCOAL.luminance() * 0.5;
                        let color = conrod::color::DARK_CHARCOAL.with_luminance(luminance);
                        style.title_bar_color = Some(Some(color));
                        Box::new(style)
                    },
                },
            );

            map
        },

        ..conrod::Theme::default()
    }
}
