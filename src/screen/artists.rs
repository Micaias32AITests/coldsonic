use std::sync::Arc;

use async_compat::Compat;
use iced::{
    Length::Fill,
    Task,
    widget::{Column, button, scrollable, text},
};
use opensubsonic::{Client, data::ArtistId3};

use crate::{Action, Never, Screen};

#[derive(Debug, Clone)]
pub struct State {
    client: Arc<Client>,
    artists: Vec<ArtistId3>,
}

#[derive(Debug, Clone)]
pub enum Msg {
    ArtistsLoaded(Vec<ArtistId3>),
    ArtistsLoadFailed,
    ArtistClicked(String),
}

impl Screen for State {
    type Msg = Msg;

    type Emit = Never;

    type InitData = Arc<Client>;

    fn init(client: Self::InitData) -> (Self, Action<Self::Msg, Self::Emit>)
    where
        Self: Sized,
    {
        let client_for_load = client.clone();
        (
            Self {
                artists: Vec::new(),
                client,
            },
            Action::task(Task::perform(
                Compat::new(async move { client_for_load.get_artists(None).await }),
                |res| match res {
                    Ok(artists) => Msg::ArtistsLoaded(
                        artists
                            .index
                            .into_iter()
                            .flat_map(|index| index.artist)
                            .collect(),
                    ),
                    Err(_) => Msg::ArtistsLoadFailed,
                },
            )),
        )
    }

    fn update(&mut self, msg: Self::Msg) -> Action<Self::Msg, Self::Emit> {
        match msg {
            Msg::ArtistsLoaded(artists) => self.artists = artists,
            Msg::ArtistsLoadFailed => {
                eprintln!("FUCK");
            }
            Msg::ArtistClicked(_) => {}
        }
        Action::none()
    }

    fn view(&self) -> iced::Element<'_, Self::Msg> {
        if self.artists.is_empty() {
            return text("loading...").into();
        }
        let list = Column::with_children(self.artists.iter().map(|artist| {
            button(text(artist.name.clone()))
                .width(Fill)
                .on_press(Msg::ArtistClicked(artist.id.clone()))
                .into()
        }))
        .spacing(4)
        .padding(8);
        scrollable(list).width(Fill).height(Fill).into()
    }
}
