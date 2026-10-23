use crate::prelude::*;

#[derive(Debug, Clone)]
pub enum Msg {
    Artists,
    Albums,
    Tracks,
    Playlists,
    Settings,
}

/// Region 01: the navigation sidebar.
///
/// `on_press` maps a sidebar intent into the parent screen's message type, so
/// the sidebar itself has no dependency on the coordinator.
pub fn view<'a, F>(on_press: F) -> Element<'a, crate::screen::Msg>
where
    F: Fn(Msg) -> crate::screen::Msg + 'a,
{
    fn nav_style(theme: &Theme, _status: button::Status) -> button::Style {
        button::Style {
            background: Some(iced::Background::Color(Color::TRANSPARENT)),
            text_color: theme.extended_palette().secondary.base.text,
            border: iced::Border::default(),
            shadow: iced::Shadow::default(),
            snap: false,
        }
    }

    widget::column![
        container(text("Settings").size(12))
            .padding(4)
            .width(Fill)
            .center_x(Fill),
        button(text("Settings"))
            .on_press(on_press(Msg::Settings))
            .style(nav_style)
            .width(Fill),
        rule::horizontal(1),
        button(text("Artists"))
            .on_press(on_press(Msg::Artists))
            .style(nav_style)
            .width(Fill),
        button(text("Albums"))
            .on_press(on_press(Msg::Albums))
            .style(nav_style)
            .width(Fill),
        button(text("Tracks"))
            .on_press(on_press(Msg::Tracks))
            .style(nav_style)
            .width(Fill),
        button(text("Playlists"))
            .on_press(on_press(Msg::Playlists))
            .style(nav_style)
            .width(Fill),
    ]
    .padding(8)
    .spacing(4)
    .width(Fill)
    .height(Fill)
    .into()
}
