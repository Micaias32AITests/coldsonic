use std::time::Duration;

use opensubsonic::Client;

use crate::prelude::*;

pub mod artists;
pub mod login;

#[derive(Debug, Clone)]
pub struct MainScreen {
    client: Arc<Client>,
    current_screen: SubScreen,
    now_playing: Option<SongPlaying>,
}

#[derive(Debug, Clone)]
pub enum SubScreen {
    Artists(artists::State),
}

#[derive(Debug, Clone)]
pub struct SongPlaying {
    cover: Option<Arc<[u8]>>,
    duration: Duration,
    position: Duration,
}

#[derive(Debug, Clone)]
pub enum Msg {
    ArtistsScreenMsg(artists::Msg),
}

impl Screen for MainScreen {
    type Msg = Msg;

    type Emit = Never;

    type InitData = Client;

    fn init(data: Self::InitData) -> (Self, Action<Self::Msg, Self::Emit>)
    where
        Self: Sized,
    {
        let client: Arc<_> = data.into();
        let (state, action) = artists::State::init(client.clone());
        (
            Self {
                client,
                current_screen: SubScreen::Artists(state),
                now_playing: None,
            },
            action.map_task(|t| t.map(Msg::ArtistsScreenMsg)),
        )
    }

    fn update(&mut self, msg: Self::Msg) -> Action<Self::Msg, Self::Emit> {
        match msg {
            Msg::ArtistsScreenMsg(inner) => {
                let SubScreen::Artists(screen) = &mut self.current_screen;
                screen
                    .update(inner)
                    .map_task(|task| task.map(Msg::ArtistsScreenMsg))
            }
        }
    }

    fn view(&self) -> Element<'_, Self::Msg> {
        widget::column![self.view_subscreen(), self.view_now_playing()]
            .width(Fill)
            .height(Fill)
            .into()
    }
}

impl MainScreen {
    fn view_subscreen(&self) -> Element<'_, Msg> {
        match &self.current_screen {
            SubScreen::Artists(screen) => screen.view().map(Msg::ArtistsScreenMsg),
        }
    }

    fn view_now_playing(&self) -> Element<'_, Msg> {
        match &self.now_playing {
            Some(song) => row![
                match &song.cover {
                    Some(cover) => Element::from(
                        image(image::Handle::from_bytes(cover.to_vec()))
                            .width(48.0)
                            .height(48.0),
                    ),
                    None => Element::from(space().width(48.0).height(48.0)),
                },
                text(format!(
                    "{} / {}",
                    fmt_duration(song.position),
                    fmt_duration(song.duration)
                ))
            ]
            .align_y(Center)
            .spacing(8)
            .padding(8)
            .into(),
            None => text("Nothing playing").into(),
        }
    }
}

fn fmt_duration(duration: Duration) -> String {
    let secs = duration.as_secs();
    format!("{:02}:{:02}", secs / 60, secs % 60)
}
